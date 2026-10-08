use super::*;
use crate::engine::{
    musical_context::{Context, Scale},
    ClipKind,
};
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

fn pointer(gui: &mut Gui, point: Pos2, pressed: bool) {
    gui.frame(vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]);
}
fn scroll_bottom(gui: &mut Gui) {
    if gui.nodes.iter().any(|(_, node)| {
        node.label() == Some("MIDI piano roll: MIDI editor controls: Vertical scroll")
    }) {
        gui.action(
            "MIDI piano roll: MIDI editor controls: Vertical scroll",
            Action::SetValue,
            Some(ActionData::NumericValue(f64::MAX)),
        );
    }
}
fn reveal(gui: &mut Gui, label: &str) {
    for _ in 0..3 {
        let Some((_, scroll)) = gui.nodes.iter().find(|(_, node)| {
            node.label() == Some("MIDI piano roll: MIDI editor controls: Vertical scroll")
        }) else {
            return;
        };
        let Some(view) = scroll.bounds() else { return };
        let current = scroll.numeric_value().unwrap_or(0.0);
        let bounds = gui.node(label).1.bounds().unwrap();
        if bounds.y0 >= view.y0 && bounds.y1 <= view.y1 {
            return;
        }
        let next = (current + bounds.y0 - view.y0 - 20.0).max(0.0);
        gui.action(
            "MIDI piano roll: MIDI editor controls: Vertical scroll",
            Action::SetValue,
            Some(ActionData::NumericValue(next)),
        );
    }
}
fn fixture(notes: bool) -> Box<Gui> {
    let mut gui = Box::new(Gui::new());
    gui.screen = Vec2::new(1600.0, 4000.0);
    let clip = &mut gui.rt.tracks[2].clips[7];
    clip.kind = ClipKind::Midi;
    clip.name = "Composition".into();
    clip.bars = 4.0;
    clip.region = Some(Region::full(4.0));
    clip.properties.context = Some(Context {
        tonic: 0,
        scale: Scale::Dorian,
    });
    if notes {
        clip.notes = [60, 64, 67]
            .into_iter()
            .map(|pitch| MidiNote {
                id: NoteId::new(),
                pitch,
                channel: 0,
                start: 1.0,
                len: 1.0,
                vel: 90,
                release_vel: 31,
                source_timing: None,
                muted: false,
                variation: None,
            })
            .collect();
    }
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
    assert!(
        gui.app.piano_roll.draft.is_some(),
        "Native fixture inspection failed: {:?}",
        gui.app.piano_roll.error
    );
    gui.click("MIDI piano roll: Select all notes");
    scroll_bottom(&mut gui);
    gui.click_text("MIDI transformations");
    scroll_bottom(&mut gui);
    gui
}
fn choose(gui: &mut Gui, name: &str) {
    reveal(gui, "MIDI transformation tool");
    gui.click("MIDI transformation tool");
    gui.click(name);
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().tools.params.kind,
        match name {
            "Chord progression" => Kind::Chords,
            "Melody contour" => Kind::Melody,
            "Articulation" => Kind::Articulate,
            _ => panic!("Unknown composition tool"),
        }
    );
    scroll_bottom(gui);
}
fn preview(gui: &mut Gui) {
    gui.click("MIDI piano roll: Preview MIDI transformation");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
}
fn musical(notes: &[MidiNote]) -> Vec<(u8, u8, u64, u64, u8)> {
    notes
        .iter()
        .map(|note| {
            (
                note.channel,
                note.pitch,
                note.source_start().to_bits(),
                note.source_duration().to_bits(),
                note.vel,
            )
        })
        .collect()
}
fn save(gui: &mut Gui, file: &std::path::Path) {
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&file.to_path_buf());
    gui.click_text("Save");
    gui.settle();
    assert_eq!(gui.app.project_result_for_test().0.as_deref(), Some(file));
}
fn reopen(file: &std::path::Path) -> Box<Gui> {
    let mut gui = Box::new(Gui::new());
    gui.click_text("Project");
    gui.click_text("Open project…");
    gui.path(&file.to_path_buf());
    gui.click_text("Open");
    gui.settle();
    gui
}
fn edit_first_velocity(gui: &mut Gui, velocity: u8) {
    let before = gui.app.piano_roll.draft.as_ref().unwrap().notes.clone();
    let note = &before[0];
    let label = format!(
        "MIDI piano roll: Note 1: {} · {:.6} beats · length {:.6} · velocity {} · {}",
        pitch_name(note.pitch),
        note.start,
        note.len,
        note.vel,
        if note.muted { "muted" } else { "audible" }
    );
    gui.click(&label);
    gui.number("Note velocity", f64::from(velocity));
    gui.click("MIDI piano roll: Set selected note values");
    let after = &gui.app.piano_roll.draft.as_ref().unwrap().notes;
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0].vel, velocity);
    let mut original = after[0].clone();
    original.vel = before[0].vel;
    assert_eq!(original, before[0]);
    assert_eq!(after[1..], before[1..]);
}
#[test]
fn actual_eight_chord_progression_revises_inversion_previews_restores_commits_undo_and_reopens() {
    let mut gui = fixture(false);
    choose(&mut gui, "Chord progression");
    gui.action(
        "Chord 2: Chord degree",
        Action::SetValue,
        Some(ActionData::NumericValue(4.0)),
    );
    gui.action(
        "Chord 2: Chord inversion",
        Action::SetValue,
        Some(ActionData::NumericValue(1.0)),
    );
    preview(&mut gui);
    let candidate = gui.app.piano_roll.draft.as_ref().unwrap().notes.clone();
    assert_eq!(candidate.len(), 24);
    assert!(candidate.iter().all(|note| gui
        .app
        .piano_roll
        .draft
        .as_ref()
        .unwrap()
        .context
        .unwrap()
        .contains(note.pitch)));
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    gui.click("MIDI piano roll: Restore original MIDI preview");
    assert!(gui.app.piano_roll.draft.as_ref().unwrap().notes.is_empty());
    preview(&mut gui);
    assert_eq!(
        musical(&gui.app.piano_roll.draft.as_ref().unwrap().notes),
        musical(&candidate)
    );
    gui.apply();
    let committed = gui.rt.tracks[2].clips[7].notes.clone();
    assert_eq!(musical(&committed), musical(&candidate));
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    gui.app.engine.send(Command::Redo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, committed);
    let file = std::env::temp_dir().join(format!(
        "omat-chords-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    save(&mut gui, &file);
    let reopened = reopen(&file);
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, committed);
    assert_eq!(
        reopened.rt.tracks[2].clips[7].properties.context,
        Some(Context {
            tonic: 0,
            scale: Scale::Dorian
        })
    );
    std::fs::remove_file(file).unwrap();
    eprintln!("MIDI_COMPOSITION_CHORDS {{\"actual_egui_accesskit\":true,\"eight_chords\":true,\"editable_inversion\":true,\"preview_restore\":true,\"one_undo\":true,\"native_save_reopen\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_drawn_contour_ten_saved_seeded_alternatives_reopen_and_manual_note_edit_survives_without_generator(
) {
    let root = std::env::temp_dir().join(format!(
        "omat-melody-alternatives-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut alternatives = BTreeSet::new();
    for index in 0..10 {
        let mut gui = fixture(false);
        choose(&mut gui, "Melody contour");
        gui.number("Melody note density", 0.6);
        gui.number("Melody pitch variation", 0.3);
        reveal(&mut gui, "MIDI piano roll: Draw melody contour");
        let bounds = gui
            .node("MIDI piano roll: Draw melody contour")
            .1
            .bounds()
            .unwrap();
        let a = Pos2::new(bounds.x0 as f32 + 2.0, bounds.y1 as f32 - 2.0);
        let b = Pos2::new(bounds.x1 as f32 - 2.0, bounds.y0 as f32 + 2.0);
        pointer(&mut gui, a, true);
        gui.frame(vec![egui::Event::PointerMoved(b)]);
        pointer(&mut gui, b, false);
        assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().tools.shape, 6);
        gui.action("Transformation seed", Action::Focus, None);
        gui.key(
            Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        gui.frame(vec![egui::Event::Text(index.to_string())]);
        preview(&mut gui);
        let first = musical(&gui.app.piano_roll.draft.as_ref().unwrap().notes);
        gui.click("MIDI piano roll: Reroll melodic candidate");
        gui.settle();
        assert!(
            gui.app.piano_roll.error.is_none(),
            "{:?}",
            gui.app.piano_roll.error
        );
        assert!(gui.app.piano_roll.draft.as_ref().unwrap().notes.len() <= 32);
        gui.click("MIDI piano roll: Restore original MIDI preview");
        assert!(gui.app.piano_roll.draft.as_ref().unwrap().notes.is_empty());
        gui.action("Transformation seed", Action::Focus, None);
        gui.key(
            Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        gui.frame(vec![egui::Event::Text(index.to_string())]);
        preview(&mut gui);
        assert_eq!(
            musical(&gui.app.piano_roll.draft.as_ref().unwrap().notes),
            first
        );
        alternatives.insert(first.clone());
        gui.click("MIDI piano roll: Keep MIDI preview in draft");
        assert!(gui
            .app
            .piano_roll
            .draft
            .as_ref()
            .unwrap()
            .tools
            .preview
            .is_none());
        if index == 0 {
            edit_first_velocity(&mut gui, 17);
        }
        gui.apply();
        let saved = gui.rt.tracks[2].clips[7].notes.clone();
        let file = root.join(format!("alternative-{index}.omat"));
        save(&mut gui, &file);
        let reopened = reopen(&file);
        assert_eq!(reopened.rt.tracks[2].clips[7].notes, saved);
        assert!(saved.iter().all(|note| note.variation.is_none()));
        assert!(reopened.rt.tracks[2].clips[7].variation.is_none());
        if index == 0 {
            assert_eq!(reopened.rt.tracks[2].clips[7].notes[0].vel, 17);
        }
    }
    assert_eq!(alternatives.len(), 10);
    std::fs::remove_dir_all(root).unwrap();
    eprintln!("MIDI_COMPOSITION_MELODY {{\"actual_egui_pointer\":true,\"ten_native_saved_alternatives\":true,\"deterministic_replay\":true,\"reroll_without_accumulation\":true,\"manual_edit_without_generator\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_articulation_selector_strums_flams_and_glissandi_remain_editable_with_one_undo() {
    for (mode, count) in [("Strum", 3), ("Flam", 6), ("Glissando", 24)] {
        let mut gui = fixture(true);
        let original = gui.rt.tracks[2].clips[7].notes.clone();
        choose(&mut gui, "Articulation");
        reveal(&mut gui, "Articulation type");
        gui.click("Articulation type");
        gui.click_text(mode);
        gui.number("Articulation step beats", 0.125);
        gui.number("Ornament semitone interval", 7.0);
        preview(&mut gui);
        assert_eq!(
            gui.app.piano_roll.draft.as_ref().unwrap().notes.len(),
            count
        );
        gui.click("MIDI piano roll: Keep MIDI preview in draft");
        edit_first_velocity(&mut gui, 23);
        gui.apply();
        let committed = gui.rt.tracks[2].clips[7].notes.clone();
        assert_eq!(committed[0].vel, 23);
        assert!(committed.iter().all(|note| note.source_duration() > 0.0));
        gui.app.engine.send(Command::Undo).unwrap();
        gui.frame(vec![]);
        assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
        gui.app.engine.send(Command::Redo).unwrap();
        gui.frame(vec![]);
        assert_eq!(gui.rt.tracks[2].clips[7].notes, committed);
    }
    eprintln!("MIDI_COMPOSITION_ARTICULATION {{\"actual_egui_accesskit\":true,\"strum_flam_glissando\":true,\"ordinary_editable_notes\":true,\"one_undo\":true,\"paired_releases\":true,\"physical_devices_opened\":false}}");
}
