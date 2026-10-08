use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

fn fixture() -> Box<Gui> {
    let mut gui = Box::new(Gui::new());
    gui.screen = Vec2::new(1600.0, 3600.0);
    let clip = &mut gui.rt.tracks[2].clips[7];
    clip.kind = crate::engine::ClipKind::Midi;
    clip.name = "Note choice phrase".into();
    clip.bars = 1.0;
    clip.region = Some(Region::full(1.0));
    clip.notes = [60, 64].into_iter().map(|pitch|MidiNote { variation: None, id: NoteId::new(), channel: 0, release_vel: 45, source_timing: None, muted: false, pitch, start: 0.0, len: 0.25, vel: 90 }).collect();
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
    gui.click("MIDI piano roll: Select all notes");
    if gui.nodes.iter().any(|(_,node)|node.label()==Some("MIDI piano roll: MIDI editor controls: Vertical scroll")) {
        gui.action("MIDI piano roll: MIDI editor controls: Vertical scroll", Action::SetValue, Some(ActionData::NumericValue(f64::MAX)));
    }
    gui.click_text("Note velocity and chance");
    if gui.nodes.iter().any(|(_,node)|node.label()==Some("MIDI piano roll: MIDI editor controls: Vertical scroll")) {
        gui.action("MIDI piano roll: MIDI editor controls: Vertical scroll", Action::SetValue, Some(ActionData::NumericValue(f64::MAX)));
    }
    gui.frame(vec![]);
    gui
}
fn choose_group(gui: &mut Gui, name: &str) {
    gui.click("Note probability group");
    gui.click_text(name);
    gui.frame(vec![]);
}
fn save_reopen(gui: &mut Gui) -> Box<Gui> {
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    let file = std::env::temp_dir().join(format!("omat-note-choice-{}.omat", crate::sampler_bank::BankId::new().unwrap()));
    gui.click_text("Project"); gui.click_text("Save project as…"); gui.path(&file); gui.click_text("Save"); gui.settle();
    assert_eq!(gui.app.project_result_for_test().0.as_ref(), Some(&file));
    let mut reopened = Box::new(Gui::new());
    reopened.click_text("Project"); reopened.click_text("Open project…"); reopened.path(&file); reopened.click_text("Open"); reopened.settle();
    std::fs::remove_file(file).unwrap();
    reopened
}
#[test]
fn actual_velocity_chance_lanes_linked_apply_undo_and_native_reopen_keep_notes_and_choices() {
    let mut gui = fixture();
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    gui.number("Lane note velocity", 56.0);
    gui.click("MIDI piano roll: Set selected note velocity");
    assert!(gui.app.piano_roll.draft.as_ref().unwrap().notes.iter().all(|note|note.vel == 56 && note.variation.is_none()));
    gui.number("Note chance percent", 37.0);
    gui.click("MIDI piano roll: Vary note velocity");
    gui.number("Minimum note velocity", 41.0);
    gui.number("Maximum note velocity", 79.0);
    choose_group(&mut gui, "Linked");
    gui.click("MIDI piano roll: Set selected note choices");
    assert!(gui.app.piano_roll.error.is_none(), "{:?}", gui.app.piano_roll.error);
    let draft = gui.app.piano_roll.draft.as_ref().unwrap().notes.clone();
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    for (before, after) in original.iter().zip(&draft) { let mut retained = after.clone(); retained.variation = None; retained.vel = before.vel; assert_eq!(&retained, before); assert_eq!(after.vel, 56); let properties = after.variation.unwrap(); assert_eq!(properties.chance, 3700); assert_eq!(properties.velocity, Some(Velocity { minimum: 41, maximum: 79 })); assert_eq!(properties.group.unwrap().kind, GroupKind::Linked); }
    assert_eq!(draft[0].variation.unwrap().group, draft[1].variation.unwrap().group);
    assert!(gui.nodes.iter().any(|(_,node)|node.label().is_some_and(|label|label.contains("velocity lane: 41–79; chance lane: 37.00%"))));
    gui.apply();
    assert_eq!(gui.rt.tracks[2].clips[7].notes, draft);
    let prepared = gui.rt.tracks[2].clips[7].variation.clone().unwrap();
    gui.app.engine.send(Command::Undo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert!(gui.rt.tracks[2].clips[7].variation.is_none());
    gui.app.engine.send(Command::Redo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, draft);
    assert!(Arc::ptr_eq(gui.rt.tracks[2].clips[7].variation.as_ref().unwrap(), &prepared));
    let reopened = save_reopen(&mut gui);
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, draft);
    assert!(reopened.rt.tracks[2].clips[7].variation.is_some());
    assert_eq!(reopened.rt.note_seed, note_variation::DEFAULT_SEED);
    eprintln!("MIDI_NOTE_VARIATION_NATIVE {{\"actual_egui_accesskit\":true,\"velocity_chance_lanes\":true,\"base_velocity_edited\":true,\"linked_groups\":true,\"one_undo\":true,\"native_save_reopen\":true,\"absolute_notes_retained\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_exclusive_weights_refuse_conflicts_and_full_width_seed_has_undo_and_native_reopen() {
    let mut gui = fixture();
    let original = gui.app.piano_roll.draft.as_ref().unwrap().notes.clone();
    gui.number("Note chance percent", 75.0);
    choose_group(&mut gui, "Exclusive");
    gui.click("MIDI piano roll: Set selected note choices");
    assert!(gui.app.piano_roll.error.as_ref().is_some_and(|error|error.contains("exceed 100%")));
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().notes, original);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    gui.number("Note chance percent", 25.0);
    gui.click("MIDI piano roll: Set selected note choices");
    assert!(gui.app.piano_roll.error.is_none(), "{:?}", gui.app.piano_roll.error);
    gui.apply();
    let notes = gui.rt.tracks[2].clips[7].notes.clone();
    gui.action("Note variation seed", Action::Focus, None);
    gui.key(Key::A, egui::Modifiers { ctrl: true, command: true, ..Default::default() });
    gui.frame(vec![egui::Event::Text(u64::MAX.to_string())]);
    gui.frame(vec![]);
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().variation.seed.text, u64::MAX.to_string());
    gui.click("MIDI piano roll: Apply saved note seed");
    gui.settle();
    assert!(gui.app.piano_roll.error.is_none(), "{:?}", gui.app.piano_roll.error);
    assert_eq!(gui.rt.note_seed, u64::MAX);
    assert_eq!(gui.app.snap.note_seed, u64::MAX);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, notes);
    gui.app.engine.send(Command::Undo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.rt.note_seed, note_variation::DEFAULT_SEED);
    gui.app.engine.send(Command::Redo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.rt.note_seed, u64::MAX);
    let reopened = save_reopen(&mut gui);
    assert_eq!(reopened.rt.note_seed, u64::MAX);
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, notes);
    let plan = reopened.rt.tracks[2].clips[7].variation.as_ref().unwrap();
    for cycle in 0..2000 { assert!(!(plan.velocity(0, u64::MAX, cycle, 90).is_some() && plan.velocity(1, u64::MAX, cycle, 90).is_some())); }
    eprintln!("MIDI_NOTE_VARIATION_SEED {{\"actual_egui_accesskit\":true,\"exclusive_weight_refusal\":true,\"full_u64_seed\":true,\"one_undo\":true,\"native_save_reopen\":true,\"notes_retained\":true,\"physical_devices_opened\":false}}");
}
