use super::*;
use crate::ui::piano_roll::tests::Gui;

fn fixture() -> Gui {
    let mut gui = Gui::new();
    gui.rt.tracks[2].clips[7] = crate::engine::mpe::tests::expressive_clip();
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
    assert!(
        gui.app.piano_roll.draft.is_some(),
        "{:?}",
        gui.app.piano_roll.error
    );
    let draft = gui.app.piano_roll.draft.as_mut().unwrap();
    let id = draft.notes[0].id;
    draft.selected.insert(id);
    gui.click_text("Per-note expression");
    gui
}

#[test]
fn actual_native_expression_controls_prepare_apply_and_undo_an_independent_curve() {
    let mut gui = fixture();
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    let lanes = gui.rt.tracks[2].clips[7].lanes.clone();
    gui.click("MIDI piano roll: Inspect selected note expression");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert!(gui
        .app
        .piano_roll
        .draft
        .as_ref()
        .unwrap()
        .expression
        .inspection
        .is_some());
    gui.click("MIDI piano roll: Note pressure envelope");
    gui.number("Expression beat within note", 0.75);
    gui.number("Per-note expression value", 44.0);
    gui.click("MIDI piano roll: Add or update note expression point");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().notes[0].id,
        original[0].id
    );
    gui.apply();
    let clip = &gui.rt.tracks[2].clips[7];
    assert_eq!(clip.notes[0].id, original[0].id);
    assert_eq!(clip.notes[1].id, original[1].id);
    let content = gui
        .app
        .piano_roll
        .draft
        .as_ref()
        .unwrap()
        .controls
        .content(&clip.notes);
    let envelope = envelope::inspect(&content, original[0].id, &AtomicBool::new(false)).unwrap();
    assert!(envelope
        .points
        .iter()
        .any(|point| point.dimension == Dimension::Pressure
            && point.beat == 0.75
            && point.value == 44.0));
    gui.app.engine.send(Command::Undo).unwrap();
    gui.settle();
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert_eq!(gui.rt.tracks[2].clips[7].lanes, lanes);
    gui.app.engine.send(Command::Redo).unwrap();
    gui.settle();
    assert_ne!(gui.rt.tracks[2].clips[7].lanes, lanes);
}

#[test]
fn actual_worker_refuses_stale_notes_and_native_fields_have_accessible_labels() {
    let mut gui = fixture();
    gui.click("MIDI piano roll: Inspect selected note expression");
    gui.settle();
    let (_, node) = gui.node("MIDI piano roll: Expression beat within note");
    assert!(node.supports_action(egui::accesskit::Action::SetValue));
    let (_, node) = gui.node("MIDI piano roll: Per-note expression value");
    assert!(node.supports_action(egui::accesskit::Action::SetValue));
    let draft = gui.app.piano_roll.draft.as_mut().unwrap();
    let mut controls = std::mem::take(&mut draft.expression);
    let note = draft.notes[0].id;
    controls
        .start(
            draft,
            &gui.app.engine,
            note,
            Operation::Edit(Edit {
                dimension: Dimension::Timbre,
                beat: 0.5,
                value: Some(100.0),
                original: None,
            }),
        )
        .unwrap();
    draft.notes[0].vel = 17;
    let preserved = draft.controls.content(&draft.notes);
    draft.expression = controls;
    gui.settle();
    assert!(
        gui.app
            .piano_roll
            .error
            .as_ref()
            .is_some_and(|error| error.contains("draft changed")),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert_eq!(
        gui.app
            .piano_roll
            .draft
            .as_ref()
            .unwrap()
            .controls
            .content(&gui.app.piano_roll.draft.as_ref().unwrap().notes),
        preserved
    );
}
