use super::*;
use crate::engine::{midi_tools, ClipKind};
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

fn open(gui: &mut Gui) {
    gui.screen = Vec2::new(1600.0, 3000.0);
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
}
fn panel(gui: &mut Gui) {
    if gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("MIDI piano roll: MIDI editor controls: Vertical scroll"))
    {
        gui.action(
            "MIDI piano roll: MIDI editor controls: Vertical scroll",
            Action::SetValue,
            Some(ActionData::NumericValue(f64::MAX)),
        );
    }
    gui.click_text("Saved key and scale");
    gui.frame(vec![]);
}
fn fixture() -> Box<Gui> {
    let mut gui = Box::new(Gui::new());
    let clip = &mut gui.rt.tracks[2].clips[7];
    clip.kind = ClipKind::Midi;
    clip.name = "Scale phrase".into();
    clip.bars = 4.0;
    clip.region = Some(Region::full(4.0));
    for (index, pitch) in [60, 61, 63, 66].into_iter().enumerate() {
        clip.notes.push(MidiNote {
            pitch,
            vel: 90,
            start: index as f32 * 0.5,
            len: 0.25,
            channel: 0,
            release_vel: 45,
            source_timing: None,
            id: NoteId::new(),
            muted: false,
        });
    }
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    open(&mut gui);
    gui
}
fn set_scale(gui: &mut Gui, scale: Scale) {
    panel(gui);
    gui.click("MIDI piano roll: Override clip key");
    gui.click("Override clip key scale");
    gui.click_text(scale.name());
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().context,
        Some(Context { tonic: 0, scale })
    );
    gui.apply();
}
fn save_reopen(gui: &mut Gui) -> Box<Gui> {
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    let file = std::env::temp_dir().join(format!(
        "omat-saved-scale-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&file);
    gui.click_text("Save");
    gui.settle();
    assert_eq!(gui.app.project_result_for_test().0.as_ref(), Some(&file));
    let mut reopened = Box::new(Gui::new());
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&file);
    reopened.click_text("Open");
    reopened.settle();
    std::fs::remove_file(file).unwrap();
    reopened
}

#[test]
fn native_three_nonmajor_keys_fold_highlight_degree_edit_and_reopen_preserve_chromatic_exceptions()
{
    for scale in [Scale::Dorian, Scale::Phrygian, Scale::HarmonicMinor] {
        let mut gui = fixture();
        let before = gui.rt.tracks[2].clips[7].notes.clone();
        set_scale(&mut gui, scale);
        assert_eq!(gui.rt.tracks[2].clips[7].notes, before);
        gui.click("MIDI piano roll: Fold to saved scale");
        assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().fold, 4);
        let rows = canvas::pitches(gui.app.piano_roll.draft.as_ref().unwrap());
        for note in &before {
            assert!(rows.contains(&note.pitch));
        }
        assert!(rows.contains(&match scale {
            Scale::HarmonicMinor => 71,
            _ => 70,
        }));
        gui.click("MIDI piano roll: Highlight saved scale");
        assert!(!gui.app.piano_roll.draft.as_ref().unwrap().highlight_scale);
        gui.click("MIDI piano roll: Highlight saved scale");
        assert!(gui.app.piano_roll.draft.as_ref().unwrap().highlight_scale);
        gui.click("MIDI piano roll: Select all notes");
        gui.click_text("MIDI transformations");
        gui.click_text("Quantize");
        gui.click_text("Scale degrees");
        gui.number("Scale degrees", 2.0);
        gui.click("MIDI piano roll: Preview MIDI transformation");
        gui.settle();
        assert!(
            gui.app.piano_roll.error.is_none(),
            "{:?}",
            gui.app.piano_roll.error
        );
        let context = Context { tonic: 0, scale };
        let expected: Vec<_> = before
            .iter()
            .map(|note| context.transpose(note.pitch, 2, false).unwrap())
            .collect();
        assert_eq!(
            gui.app
                .piano_roll
                .draft
                .as_ref()
                .unwrap()
                .notes
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(gui
            .node("MIDI piano roll: Override clip key")
            .1
            .is_disabled());
        let cursor = gui.app.engine.undo.view().cursor;
        gui.apply();
        assert_eq!(gui.app.engine.undo.view().cursor, cursor + 1);
        let after = gui.rt.tracks[2].clips[7].notes.clone();
        for (old, next) in before.iter().zip(&after) {
            assert_eq!(
                (old.id, old.start, old.len, old.channel, old.release_vel),
                (
                    next.id,
                    next.start,
                    next.len,
                    next.channel,
                    next.release_vel
                )
            );
            if !context.contains(old.pitch) {
                assert_eq!(next.pitch, old.pitch);
            }
        }
        gui.app.engine.send(Command::Undo).unwrap();
        gui.frame(vec![]);
        assert_eq!(gui.rt.tracks[2].clips[7].notes, before);
        gui.app.engine.send(Command::Redo).unwrap();
        gui.frame(vec![]);
        assert_eq!(gui.rt.tracks[2].clips[7].notes, after);
        let mut reopened = save_reopen(&mut gui);
        assert_eq!(
            reopened.rt.tracks[2].clips[7].properties.context,
            Some(context)
        );
        assert_eq!(reopened.rt.tracks[2].clips[7].notes, after);
        open(&mut reopened);
        assert_eq!(
            reopened
                .app
                .piano_roll
                .draft
                .as_ref()
                .unwrap()
                .resolved_context()
                .context,
            Some(context)
        );
    }
    println!("MIDI_SCALE_NATIVE {{\"actual_egui_accesskit\":true,\"three_nonmajor_scales\":true,\"one_undo\":true,\"native_save_reopen\":true,\"chromatic_exceptions_retained\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn native_song_key_scale_generation_and_participation_are_explicit_and_saved() {
    let mut gui = fixture();
    panel(&mut gui);
    gui.click("MIDI piano roll: Set song key");
    gui.click("Set song key scale");
    gui.click_text("Dorian");
    gui.settle();
    assert_eq!(
        gui.rt.musical_context,
        Some(Context {
            tonic: 0,
            scale: Scale::Dorian
        })
    );
    gui.click("MIDI piano roll: Refresh current clip");
    gui.settle();
    assert_eq!(
        gui.app
            .piano_roll
            .draft
            .as_ref()
            .unwrap()
            .resolved_context()
            .origin,
        Origin::Song
    );
    let before = gui.rt.tracks[2].clips[7].notes.clone();
    gui.click_text("Rhythm generator");
    gui.action(
        "Rhythm voice 1: Rhythm pitch",
        Action::SetValue,
        Some(ActionData::NumericValue(61.0)),
    );
    gui.click("MIDI piano roll: Generate rhythm in saved scale");
    gui.click("MIDI piano roll: Preview rhythm");
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    let context = gui.rt.musical_context.unwrap();
    let draft = gui.app.piano_roll.draft.as_ref().unwrap();
    assert!(draft.notes.len() > before.len());
    for note in &draft.notes[before.len()..] {
        assert!(context.contains(note.pitch));
    }
    assert_eq!(&draft.notes[..before.len()], before.as_slice());
    gui.apply();
    gui.click("MIDI piano roll: Instrument pads follow saved scale");
    gui.settle();
    assert!(gui.rt.sampler_scale);
    let reopened = save_reopen(&mut gui);
    assert_eq!(reopened.rt.musical_context, Some(context));
    assert!(reopened.rt.sampler_scale);
    assert_eq!(reopened.rt.tracks[2].clips[7].properties.context, None);
    println!("MIDI_SCALE_GENERATION {{\"actual_egui_accesskit\":true,\"explicit_song_context\":true,\"participating_generation\":true,\"original_absolute_pitches_retained\":true,\"saved_instrument_opt_in\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn shared_degree_worker_uses_each_captured_key_and_refuses_changed_context_without_partial_preview()
{
    let mut gui = fixture();
    let base = gui.app.piano_roll.draft.as_ref().unwrap().baseline.clone();
    let mut left = Draft::new(base.clone());
    let mut right = Draft::new(base);
    left.context = Some(Context {
        tonic: 0,
        scale: Scale::Dorian,
    });
    right.context = Some(Context {
        tonic: 0,
        scale: Scale::Phrygian,
    });
    left.selected = left.notes.iter().map(|n| n.id).collect();
    right.selected = right.notes.iter().map(|n| n.id).collect();
    let params = midi_tools::Parameters {
        kind: midi_tools::Kind::ScaleTranspose,
        degrees: 1,
        ..Default::default()
    };
    let mut comparison = comparison::Comparison::default();
    let group = &mut comparison.group;
    let original = left.notes.clone();
    group
        .start(
            std::collections::BTreeMap::from([((2, 7), &left), ((3, 6), &right)]),
            params.clone(),
        )
        .unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while group.busy() {
        group
            .poll(std::collections::BTreeMap::from([
                ((2, 7), &mut left),
                ((3, 6), &mut right),
            ]))
            .unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(left.notes[0].pitch, 62);
    assert_eq!(right.notes[0].pitch, 61);
    group
        .restore(std::collections::BTreeMap::from([
            ((2, 7), &mut left),
            ((3, 6), &mut right),
        ]))
        .unwrap();
    assert_eq!(left.notes, original);
    assert_eq!(right.notes, original);
    group
        .start(
            std::collections::BTreeMap::from([((2, 7), &left), ((3, 6), &right)]),
            params,
        )
        .unwrap();
    left.context = Some(Context {
        tonic: 0,
        scale: Scale::HarmonicMinor,
    });
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let mut refused = false;
    while group.busy() {
        if group
            .poll(std::collections::BTreeMap::from([
                ((2, 7), &mut left),
                ((3, 6), &mut right),
            ]))
            .is_err()
        {
            refused = true;
        }
        assert!(
            Instant::now() < deadline,
            "Actual shared scale worker did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(refused);
    assert_eq!(left.notes, original);
    assert_eq!(right.notes, original);
    assert!(!group.previewed());
    gui.rt.musical_context = Some(Context {
        tonic: 0,
        scale: Scale::Dorian,
    });
    gui.rt.tracks[2].clips[7].properties.context = None;
    gui.rt.tracks[3].clips[6] = gui.rt.tracks[2].clips[7].clone();
    for note in &mut gui.rt.tracks[3].clips[6].notes { note.id = NoteId::new(); }
    gui.rt.tracks[3].clips[6].properties.context = Some(Context {
        tonic: 2,
        scale: Scale::Phrygian,
    });
    gui.app.engine.send(Command::Quant(0.0)).unwrap();
    gui.app
        .engine
        .send(Command::FireClip {
            track: 2,
            scene: 7,
            looping: true,
        })
        .unwrap();
    gui.app
        .engine
        .send(Command::FireClip {
            track: 3,
            scene: 6,
            looping: true,
        })
        .unwrap();
    gui.frame(vec![]);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    assert!(gui.app.snap.active_scale.conflicted);
    assert!(gui.nodes.iter().any(|(_,node)| node.value().is_some_and(|value| value.contains("different keys")) || node.label().is_some_and(|label| label.contains("different keys"))));
    assert_eq!(
        gui.rt.musical_context,
        Some(Context {
            tonic: 0,
            scale: Scale::Dorian
        })
    );
    println!("MIDI_SCALE_SHARED {{\"actual_group_worker\":true,\"per_owner_context\":true,\"stale_context_refusal\":true,\"launched_key_conflict_visible\":true,\"physical_devices_opened\":false}}");
}
