use super::*;
use crate::engine::{midi_tools::Parameters, ClipKind};
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};
fn fixture() -> Box<Gui> {
    let mut gui = Box::new(Gui::new());
    gui.screen = Vec2::new(1600.0, 2600.0);
    for (track, scene, name, start, end, tuning, pitches) in [
        (2, 7, "Bass", 0.0, 12.0, 432.0, [60, 60]),
        (3, 6, "Chords", 4.0, 28.0, 442.0, [64, 67]),
    ] {
        assert!(gui.rt.tracks[track].poly.set_tuning_hz(tuning));
        let clip = &mut gui.rt.tracks[track].clips[scene];
        clip.kind = ClipKind::Midi;
        clip.name = name.into();
        clip.bars = (end / 4.0) as f32;
        clip.region = Some(Region {
            start,
            end,
            loop_start: start,
            loop_end: end,
            loop_enabled: true,
        });
        clip.notes = pitches
            .into_iter()
            .enumerate()
            .map(|(i, pitch)| MidiNote {
                id: NoteId::new(),
                channel: 0,
                release_vel: 37,
                source_timing: None,
                muted: false,
                pitch,
                start: (start + 0.125 + i as f64 * 0.255) as f32,
                len: 0.25,
                vel: 80 + i as u8 * 10,
            })
            .collect();
    }
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
    assert_eq!(gui.app.piano_roll.comparison.tuning_hz, 432.0);
    gui
}
fn comparison(gui: &mut Gui) {
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
    gui.click_text("MIDI clip comparison");
    gui.frame(vec![]);
}
fn number(gui: &mut Gui, label: &str, value: f64) {
    gui.action(
        &format!("MIDI clip comparison: {label}"),
        Action::SetValue,
        Some(ActionData::NumericValue(value)),
    );
}
fn add(gui: &mut Gui) {
    comparison(gui);
    number(gui, "Compare track", 4.0);
    number(gui, "Compare scene", 7.0);
    gui.click("MIDI clip comparison: Add comparison clip");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert_eq!(gui.app.piano_roll.comparison.clips.len(), 1);
    assert_eq!(gui.app.piano_roll.comparison.clips[0].tuning_hz, 442.0);
}
fn focus(gui: &Gui) -> &Draft {
    gui.app.piano_roll.draft.as_ref().unwrap()
}
#[test]
fn native_shared_criteria_focus_and_combined_apply_preserve_unequal_regions_tuning_and_save_reopen()
{
    let mut gui = fixture();
    add(&mut gui);
    number(&mut gui, "Focused shared offset", 0.0);
    number(&mut gui, "Shared offset track 4 scene 7", 0.0);
    let bass = gui.rt.tracks[2].clips[7].notes.clone();
    let chords = gui.rt.tracks[3].clips[6].notes.clone();
    let regions = (
        gui.rt.tracks[2].clips[7].region,
        gui.rt.tracks[3].clips[6].region,
    );
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    assert_eq!(focus(&gui).selected.len(), 2);
    assert!(gui.app.piano_roll.comparison.clips[0]
        .draft
        .selected
        .is_empty());
    gui.click("MIDI piano roll: Transpose up");
    gui.apply();
    assert_eq!(
        gui.rt.tracks[2].clips[7]
            .notes
            .iter()
            .map(|n| n.pitch)
            .collect::<Vec<_>>(),
        [61, 61]
    );
    assert_eq!(gui.rt.tracks[3].clips[6].notes, chords);
    gui.click("MIDI clip comparison: Enable edits across explicitly enabled clips");
    gui.click("MIDI clip comparison: Enable track 4 scene 7 for shared edits");
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    assert_eq!(
        gui.app.piano_roll.comparison.clips[0].draft.selected.len(),
        2
    );
    let before = (
        gui.rt.tracks[2].clips[7].notes.clone(),
        gui.rt.tracks[3].clips[6].notes.clone(),
    );
    let cursor = gui.app.engine.undo.view().cursor;
    gui.click("MIDI clip comparison: Preview shared tool on enabled clips");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert!(gui.app.piano_roll.comparison.group.previewed());
    assert_eq!(gui.rt.tracks[2].clips[7].notes, before.0);
    assert_eq!(gui.rt.tracks[3].clips[6].notes, before.1);
    gui.apply();
    assert_eq!(gui.app.engine.undo.view().cursor, cursor + 1);
    let after = (
        gui.rt.tracks[2].clips[7].notes.clone(),
        gui.rt.tracks[3].clips[6].notes.clone(),
    );
    assert_ne!(after.0, before.0);
    assert_ne!(after.1, before.1);
    assert_eq!(
        (
            gui.rt.tracks[2].clips[7].region,
            gui.rt.tracks[3].clips[6].region
        ),
        regions
    );
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, before.0);
    assert_eq!(gui.rt.tracks[3].clips[6].notes, before.1);
    gui.app.engine.send(Command::Redo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, after.0);
    assert_eq!(gui.rt.tracks[3].clips[6].notes, after.1);
    let shared_left =
        focus(&gui).view_beat - focus(&gui).region.start + gui.app.piano_roll.comparison.offset;
    gui.click("MIDI clip comparison: Focus track 4 scene 7");
    assert_eq!(
        (focus(&gui).baseline.track, focus(&gui).baseline.scene),
        (3, 6)
    );
    assert_eq!(gui.app.piano_roll.comparison.tuning_hz, 442.0);
    assert_eq!(
        focus(&gui).view_beat - focus(&gui).region.start + gui.app.piano_roll.comparison.offset,
        shared_left
    );
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, after.0);
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    let file = std::env::temp_dir().join(format!(
        "omat-multi-clip-{}.omat",
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
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, after.0);
    assert_eq!(reopened.rt.tracks[3].clips[6].notes, after.1);
    assert_eq!(
        (
            reopened.rt.tracks[2].clips[7].region,
            reopened.rt.tracks[3].clips[6].region
        ),
        regions
    );
    reopened.screen = Vec2::new(1600.0, 2600.0);
    reopened.click("Sampler: Edit selected MIDI clip");
    reopened.settle();
    assert_eq!(reopened.app.piano_roll.comparison.tuning_hz, 432.0);
    add(&mut reopened);
    assert_eq!(
        bass.iter().map(|n| n.id).collect::<Vec<_>>(),
        after.0.iter().map(|n| n.id).collect::<Vec<_>>()
    );
    std::fs::remove_file(file).unwrap();
    println!("MIDI_COMPARISON_NATIVE {{\"native_egui_accesskit\":true,\"explicit_focus_and_permissions\":true,\"one_undo\":true,\"unequal_loops_and_offsets\":true,\"different_tuning\":true,\"native_save_reopen\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_group_worker_restore_and_stale_selection_refusal_retain_every_owned_draft() {
    let mut gui = fixture();
    add(&mut gui);
    gui.click("MIDI clip comparison: Enable edits across explicitly enabled clips");
    gui.click("MIDI clip comparison: Enable track 4 scene 7 for shared edits");
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    let original = focus(&gui).notes.clone();
    let peer = gui.app.piano_roll.comparison.clips[0].draft.notes.clone();
    gui.click("MIDI clip comparison: Preview shared tool on enabled clips");
    gui.settle();
    gui.click("MIDI clip comparison: Restore all clip previews");
    assert_eq!(focus(&gui).notes, original);
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, peer);
    assert!(!focus(&gui).dirty);
    assert!(!gui.app.piano_roll.comparison.clips[0].draft.dirty);
    let editor = &mut gui.app.piano_roll;
    let current = editor.draft.as_mut().unwrap();
    editor
        .comparison
        .group
        .start(
            Comparison::owners(&editor.comparison.clips, current, true),
            Parameters::default(),
        )
        .unwrap();
    current.selected.clear();
    gui.settle();
    assert!(gui
        .app
        .piano_roll
        .error
        .as_ref()
        .unwrap()
        .contains("changed during preparation"));
    assert_eq!(focus(&gui).notes, original);
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, peer);
}
#[test]
fn actual_compared_canvas_protects_ghost_clicks_and_preserves_negative_translated_shared_view() {
    let mut gui = fixture();
    add(&mut gui);
    number(&mut gui, "Focused shared offset", 0.0);
    number(&mut gui, "Shared offset track 4 scene 7", 0.0);
    let current = focus(&gui).notes.clone();
    let peer = gui.app.piano_roll.comparison.clips[0].draft.notes.clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let theme = Theme::default();
    let mut time = 0.0;
    let mut paint =
        |draft: &mut Draft, offset: f64, layers: &[canvas::Layer<'_>], events: Vec<egui::Event>| {
            time += 0.02;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 640.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        canvas::show_compared(ui, &theme, draft, None, offset, layers).unwrap()
                    });
                },
            )
        };
    let pointer = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    {
        let editor = &mut gui.app.piano_roll;
        let layers = editor.comparison.layers();
        let offset = editor.comparison.offset;
        let draft = editor.draft.as_mut().unwrap();
        let mut output = paint(draft, offset, &layers, vec![]);
        for _ in 0..3 {
            output = paint(draft, offset, &layers, vec![]);
        }
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, n)| {
                n.description()
                    .is_some_and(|s| s.contains("Protected ghost note"))
                    .then(|| n.bounds().unwrap())
            })
            .unwrap();
        let point = Pos2::new(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        paint(draft, offset, &layers, pointer(point, true));
        paint(draft, offset, &layers, pointer(point, false));
        assert_eq!(draft.notes, current);
        assert!(!draft.dirty);
        assert!(draft.selected.is_empty());
    }
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, peer);
    number(&mut gui, "Shared offset track 4 scene 7", 100.0);
    let shared_left =
        focus(&gui).view_beat - focus(&gui).region.start + gui.app.piano_roll.comparison.offset;
    gui.click("MIDI clip comparison: Focus track 4 scene 7");
    assert!(focus(&gui).view_beat < 0.0);
    assert_eq!(
        focus(&gui).view_beat - focus(&gui).region.start + gui.app.piano_roll.comparison.offset,
        shared_left
    );
    {
        let editor = &mut gui.app.piano_roll;
        let layers = editor.comparison.layers();
        let offset = editor.comparison.offset;
        let draft = editor.draft.as_mut().unwrap();
        let output = paint(draft, offset, &layers, vec![]);
        assert!(output.shapes.iter().any(
            |s| matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.text()=="Shared 0.000")
        ));
        let point = Pos2::new(550.0, 150.0);
        paint(draft, offset, &layers, pointer(point, true));
        paint(draft, offset, &layers, pointer(point, false));
        assert_eq!(draft.notes, peer);
        assert!(!draft.dirty);
    }
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, current);
    println!("MIDI_COMPARISON_POINTER {{\"actual_egui_pointer\":true,\"ghost_notes_protected\":true,\"translated_negative_source_view\":true,\"shared_ruler_retained\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn native_shared_pitch_time_velocity_overlap_and_inversion_edit_only_matching_owners() {
    let mut gui = fixture();
    add(&mut gui);
    number(&mut gui, "Focused shared offset", 0.0);
    number(&mut gui, "Shared offset track 4 scene 7", 0.0);
    gui.click("MIDI clip comparison: Enable edits across explicitly enabled clips");
    gui.click("MIDI clip comparison: Enable track 4 scene 7 for shared edits");
    number(&mut gui, "Selection pitch minimum", 60.0);
    number(&mut gui, "Selection pitch maximum", 60.0);
    number(&mut gui, "Selection velocity minimum", 80.0);
    number(&mut gui, "Selection velocity maximum", 80.0);
    number(&mut gui, "Selection shared time minimum", 0.2);
    number(&mut gui, "Selection shared time maximum", 0.3);
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    assert!(focus(&gui).selected.is_empty());
    assert!(gui.app.piano_roll.comparison.clips[0]
        .draft
        .selected
        .is_empty());
    gui.click("MIDI clip comparison: Include notes overlapping the shared time range");
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    let original = focus(&gui).notes.clone();
    let peer = gui.app.piano_roll.comparison.clips[0].draft.notes.clone();
    assert_eq!(focus(&gui).selected, BTreeSet::from([original[0].id]));
    assert!(gui.app.piano_roll.comparison.clips[0]
        .draft
        .selected
        .is_empty());
    gui.click("MIDI clip comparison: Preview shared tool on enabled clips");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert_ne!(focus(&gui).notes[0], original[0]);
    assert_eq!(focus(&gui).notes[1], original[1]);
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, peer);
    assert_eq!(gui.app.piano_roll.comparison.group.summaries.len(), 1);
    gui.click("MIDI clip comparison: Restore all clip previews");
    assert_eq!(focus(&gui).notes, original);
    assert_eq!(gui.app.piano_roll.comparison.clips[0].draft.notes, peer);
    gui.click("MIDI clip comparison: Invert pitch, time and velocity match");
    gui.click("MIDI clip comparison: Select enabled clips by criteria");
    assert_eq!(focus(&gui).selected, BTreeSet::from([original[1].id]));
    assert_eq!(
        gui.app.piano_roll.comparison.clips[0].draft.selected,
        peer.iter().map(|n| n.id).collect()
    );
    assert_eq!(focus(&gui).notes, original);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
}
