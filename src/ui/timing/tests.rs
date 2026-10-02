use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

fn settle(gui: &mut Gui) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        gui.frame(vec![]);
        if !gui.app.timing.busy() && !gui.app.project_pending_for_test() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", gui.app.timing.error);
        std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn edit(gui: &mut Gui, label: &str, text: &str) {
    gui.action(label, Action::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text(text.into())]);
}
fn open(gui: &mut Gui) {
    gui.click_text("Project");
    gui.click_text("Tempo and meter…");
    gui.frame(vec![]);
    assert!(gui.app.timing.open);
}

#[test]
fn native_editor_applies_ramps_meters_and_pickup_preserves_notes_and_reopens_saved_project() {
    let mut gui = Gui::new();
    let notes = gui.rt.tracks[2].clips[0].notes.clone();
    open(&mut gui);
    edit(
        &mut gui,
        "Tempo points: beat BPM step/ramp",
        "0 120 ramp\n8.5 180 step",
    );
    edit(
        &mut gui,
        "Meter markers: beat numerator/denominator",
        "0 7/8\n3.5 5/4\n8.5 4/4",
    );
    for (label, value) in [
        ("Pickup length", 0.5),
        ("Click subdivisions (1, 2 or 4)", 2.0),
        ("Count-in bars", 2.0),
        ("Accent gain", 1.5),
        ("Beat gain", 0.75),
    ] {
        gui.action(
            label,
            Action::SetValue,
            Some(ActionData::NumericValue(value)),
        );
    }
    gui.click("Apply timing");
    settle(&mut gui);
    assert!(gui.app.timing.error.is_none(), "{:?}", gui.app.timing.error);
    let map = gui.rt.conductor.clone().unwrap();
    assert!(map.tempos[0].ramp);
    assert_eq!(
        map.native.unwrap(),
        TimingSettings {
            pickup: 0.5,
            subdivision: 2,
            count_in: 2,
            accent_gain: 1.5,
            beat_gain: 0.75
        }
    );
    assert_eq!(map.position(8.5).0, 3);
    assert_eq!(gui.rt.tracks[2].clips[0].notes, notes);
    assert!(!gui.app.timing.draft.as_ref().unwrap().dirty);
    gui.click("Close timing editor");
    gui.frame(vec![]);
    let path = std::env::temp_dir().join(format!(
        "omat-timing-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&path);
    gui.click_text("Save");
    settle(&mut gui);
    assert_eq!(gui.app.project_result_for_test().0, Some(path.clone()));
    let mut reopened = Gui::new();
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&path);
    reopened.click_text("Open");
    settle(&mut reopened);
    assert_eq!(reopened.rt.conductor.as_ref(), Some(&map));
    assert_eq!(reopened.rt.tracks[2].clips[0].notes, notes);
    reopened
        .app
        .engine
        .send(Command::Select { track: 2, scene: 0 })
        .unwrap();
    reopened.frame(vec![]);
    reopened.frame(vec![]);
    reopened.click_text("Project");
    reopened.click_text("Export MIDI file…");
    reopened.frame(vec![]);
    let midi_path = path.with_extension("mid");
    edit(
        &mut reopened,
        "MIDI file path",
        &midi_path.display().to_string(),
    );
    reopened.click("Export new MIDI file");
    let deadline = Instant::now() + Duration::from_secs(20);
    while reopened.app.midi_files.busy() {
        reopened.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        midi_path.exists(),
        "native MIDI export did not publish its file"
    );
    let exported = crate::midi_file::decode(&std::fs::read(&midi_path).unwrap()).unwrap();
    assert!(
        exported.tracks[0]
            .meta
            .iter()
            .filter(|m| matches!(m.value, crate::midi_file::MetaValue::Tempo(_)))
            .count()
            > 4000
    );
    std::fs::remove_file(midi_path).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn malformed_and_stale_timing_drafts_preserve_current_work_and_close_requires_discard() {
    let mut gui = Gui::new();
    open(&mut gui);
    edit(
        &mut gui,
        "Tempo points: beat BPM step/ramp",
        "0 NaN ramp\n8.5 180 step",
    );
    gui.click("Apply timing");
    settle(&mut gui);
    assert!(gui.app.timing.error.is_some());
    assert!(gui.rt.conductor.is_none());
    edit(&mut gui, "Tempo points: beat BPM step/ramp", "0 120 step");
    gui.rt.apply(Command::SetBpm(150.0));
    gui.rt.publish_for_test();
    gui.click("Apply timing");
    settle(&mut gui);
    assert!(gui.app.timing.error.as_ref().unwrap().contains("changed"));
    assert_eq!(gui.rt.bpm, 150.0);
    assert!(gui.rt.conductor.is_none());
    gui.click("Close timing editor");
    gui.frame(vec![]);
    assert!(gui.app.timing.confirm_discard && gui.app.timing.open);
    gui.click("Keep timing draft");
    gui.frame(vec![]);
    assert!(gui.app.timing.draft.as_ref().unwrap().dirty);
    gui.click("Close timing editor");
    gui.frame(vec![]);
    gui.click("Discard timing draft");
    gui.frame(vec![]);
    assert!(!gui.app.timing.open);
    assert!(gui.app.timing.draft.is_none());
}

#[test]
fn draft_parser_refuses_unordered_off_grid_and_invalid_meter_or_click_settings() {
    let mut gui = Gui::new();
    open(&mut gui);
    let good = gui.app.timing.draft.clone().unwrap();
    for text in [
        "1 120 step",
        "0 120 ramp",
        "0 120 step\n0 140 step",
        "0 120 step\n0.000001 140 step",
        "0 241 step",
        "0 120 jump",
    ] {
        let mut draft = good.clone();
        draft.tempos = text.into();
        assert!(draft.map().is_err(), "{text}");
    }
    for text in ["0 0/4", "0 4/0", "0 4/3", "0 4/256", "0 4/4\n0 7/8"] {
        let mut draft = good.clone();
        draft.meters = text.into();
        assert!(draft.map().is_err(), "{text}");
    }
    let mut draft = good.clone();
    draft.settings.subdivision = 3;
    assert!(draft.map().is_err());
    let mut draft = good;
    draft.settings.pickup = 4.0;
    assert!(draft.map().is_err());
}
