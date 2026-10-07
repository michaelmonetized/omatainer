use super::*;
use crate::engine::{
    midi_data::{ControlKind, Label, Lanes},
    ClipKind, RtEngine,
};
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionRequest};
use std::path::PathBuf;
struct File(PathBuf);
impl File {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "omat-midi-transform-{}.omat",
            crate::sampler_bank::BankId::new().unwrap()
        )))
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn open(gui: &mut Gui) {
    gui.screen = Vec2::new(1600.0, 2600.0);
    gui.frame(vec![]);
    gui.click("Sampler: Edit selected MIDI clip");
    gui.settle();
    assert!(
        gui.app.piano_roll.draft.is_some(),
        "{:?}",
        gui.app.piano_roll.error
    );
}
fn tools(gui: &mut Gui) {
    if gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("MIDI piano roll: MIDI editor controls: Vertical scroll"))
    {
        gui.action(
            "MIDI piano roll: MIDI editor controls: Vertical scroll",
            Action::SetValue,
            Some(egui::accesskit::ActionData::NumericValue(f64::MAX)),
        );
    }
    gui.click_text("MIDI transformations");
    gui.frame(vec![]);
}
fn draft(gui: &Gui) -> &Draft {
    gui.app.piano_roll.draft.as_ref().unwrap()
}
fn operation(gui: &mut Gui, current: &str, next: &str) {
    gui.click_text(current);
    gui.click_text(next);
}
fn preview(gui: &mut Gui) {
    gui.click("MIDI piano roll: Preview MIDI transformation");
    gui.settle();
    assert!(
        gui.app.piano_roll.error.is_none(),
        "{:?}",
        gui.app.piano_roll.error
    );
    assert!(draft(gui).tools.preview.is_some());
}
fn save(gui: &mut Gui, file: &File) {
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&file.0);
    gui.click_text("Save");
    gui.settle();
    assert_eq!(
        gui.app.project_result_for_test().0.as_ref(),
        Some(&file.0),
        "{:?}",
        gui.app.project_result_for_test().1
    );
}
#[test]
fn native_preview_settings_restore_chain_and_apply_are_one_undoable_change() {
    let file = File::new();
    let mut gui = Box::new(Gui::new());
    open(&mut gui);
    for (pitch, start, length, velocity) in [
        (60.0, 0.12, 0.3, 80.0),
        (64.0, 0.38, 0.25, 90.0),
        (67.0, 0.9, 0.4, 100.0),
    ] {
        gui.number("Note pitch", pitch);
        gui.number("Note start", start);
        gui.number("Note length", length);
        gui.number("Note velocity", velocity);
        gui.click("MIDI piano roll: Add note");
    }
    gui.apply();
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    gui.click("MIDI piano roll: Select all notes");
    tools(&mut gui);
    operation(&mut gui, "Quantize", "Stretch");
    gui.number("Stretch factor", 1.5);
    preview(&mut gui);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert!(gui.node("MIDI piano roll: Note pitch").1.is_disabled());
    assert_eq!(draft(&gui).notes[0].start, original[0].start);
    assert!((draft(&gui).notes[1].start - 0.51).abs() < 1e-6);
    gui.click("MIDI piano roll: Restore original MIDI preview");
    assert_eq!(draft(&gui).notes, original);
    assert!(!draft(&gui).dirty);
    assert!(!gui.node("MIDI piano roll: Note pitch").1.is_disabled());
    gui.number("Stretch factor", 2.0);
    preview(&mut gui);
    gui.number("Stretch factor", 3.0);
    preview(&mut gui);
    assert!((draft(&gui).notes[1].start - 0.90).abs() < 1e-6);
    assert!((draft(&gui).notes[2].start - 2.46).abs() < 1e-6);
    gui.click("MIDI piano roll: Previous transform settings");
    assert_eq!(draft(&gui).tools.params.stretch, 2.0);
    preview(&mut gui);
    assert!((draft(&gui).notes[1].start - 0.64).abs() < 1e-6);
    gui.click("MIDI piano roll: Keep MIDI preview in draft");
    assert!(draft(&gui).tools.preview.is_none());
    assert!(!gui.node("MIDI piano roll: Note pitch").1.is_disabled());
    let stretched = draft(&gui).notes.clone();
    operation(&mut gui, "Stretch", "Recombine");
    gui.number("Property rotation", 1.0);
    preview(&mut gui);
    let transformed = draft(&gui).notes.clone();
    assert_eq!(
        transformed.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [67, 60, 64]
    );
    assert_eq!(
        transformed.iter().map(|n| n.start).collect::<Vec<_>>(),
        stretched.iter().map(|n| n.start).collect::<Vec<_>>()
    );
    let cursor = gui.app.engine.undo.view().cursor;
    gui.apply();
    assert_eq!(gui.rt.tracks[2].clips[7].notes, transformed);
    assert_eq!(gui.app.engine.undo.view().cursor, cursor + 1);
    assert!(draft(&gui).tools.preview.is_none());
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    gui.app.engine.send(Command::Redo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, transformed);
    save(&mut gui, &file);
    let mut reopened = Box::new(Gui::new());
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&file.0);
    reopened.click_text("Open");
    reopened.settle();
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, transformed);
    open(&mut reopened);
    assert_eq!(draft(&reopened).notes, transformed);
    println!("MIDI_TRANSFORM_NATIVE {{\"native_egui_accesskit\":true,\"reversible_preview\":true,\"settings_history\":true,\"single_undo\":true,\"native_save_reopen\":true,\"physical_devices_opened\":false}}");
}
fn expressive(rt: &mut RtEngine) {
    let note = |channel, pitch, start, duration, orders: (u32, u32), velocity| {
        MidiNote::from_smf(
            &crate::midi_file::Note {
                channel,
                pitch,
                start_tick: start,
                duration_ticks: duration,
                start_order: orders.0,
                end_order: orders.1,
                velocity,
                release_velocity: 17,
            },
            960,
        )
        .unwrap()
    };
    let message = |tick, order, bytes: [u8; 3]| crate::midi_file::Message {
        tick,
        order,
        bytes,
        length: if bytes[0] & 0xf0 == 0xd0 { 2 } else { 3 },
    };
    let clip = &mut rt.tracks[2].clips[7];
    clip.kind = ClipKind::Midi;
    clip.name = "Expressive phrase".into();
    clip.region = Some(Region::full(4.0));
    clip.bars = 4.0;
    clip.notes = vec![
        note(1, 60, 48, 432, (1, 4), 80),
        note(2, 67, 96, 576, (5, 9), 100),
        note(3, 72, 800, 100, (12, 13), 90),
    ];
    clip.lanes = Some(
        Lanes::named(
            960,
            15360,
            vec![
                message(48, 0, [0xe1, 0, 64]),
                message(264, 2, [0xd1, 75, 0]),
                message(400, 3, [0xe1, 8, 70]),
                message(300, 6, [0xb2, 74, 87]),
                message(384, 7, [0xa2, 67, 66]),
                message(240, 10, [0xe0, 0, 64]),
                message(300, 11, [0xb1, 1, 23]),
                message(850, 14, [0xd3, 22, 0]),
            ],
            vec![crate::midi_file::Meta {
                tick: 0,
                order: 15,
                value: crate::midi_file::MetaValue::Text {
                    kind: 3,
                    bytes: b"Expression source".to_vec(),
                },
            }],
            vec![Label {
                channel: 1,
                control: ControlKind::Pressure,
                name: "Finger pressure".into(),
            }],
        )
        .unwrap(),
    );
    rt.publish_for_test();
}
fn select(gui: &mut Gui, row: usize, extend: bool) {
    let prefix = format!("MIDI piano roll: Note {row}:");
    let label = gui
        .nodes
        .iter()
        .filter_map(|(_, n)| n.label())
        .find(|label| label.starts_with(&prefix))
        .unwrap()
        .to_owned();
    if extend {
        let target = gui.node(&label).0;
        gui.frame(vec![
            egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            },
            egui::Event::AccessKitActionRequest(ActionRequest {
                action: Action::Click,
                target,
                data: None,
            }),
        ]);
        gui.frame(vec![]);
    } else {
        gui.click(&label);
    }
}
#[test]
fn native_quantize_links_mpe_and_poly_pressure_to_selected_notes_and_reopens_exact_lanes() {
    let file = File::new();
    let mut gui = Box::new(Gui::new());
    expressive(&mut gui.rt);
    gui.frame(vec![]);
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    let lanes = gui.rt.tracks[2].clips[7].lanes.clone().unwrap();
    open(&mut gui);
    select(&mut gui, 1, false);
    select(&mut gui, 2, true);
    assert_eq!(
        draft(&gui).selected,
        original[..2].iter().map(|n| n.id).collect()
    );
    tools(&mut gui);
    gui.number("Transform strength", 0.5);
    operation(&mut gui, "Poly pressure only", "MPE lower zone");
    preview(&mut gui);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert_eq!(gui.rt.tracks[2].clips[7].lanes.as_ref().unwrap(), &lanes);
    assert_eq!(draft(&gui).notes[2], original[2]);
    let content = draft(&gui).controls.content(&draft(&gui).notes);
    for (order, tick) in [(0, 24), (2, 240), (3, 376), (6, 252), (7, 336)] {
        assert_eq!(
            content
                .messages
                .iter()
                .find(|m| m.order == order)
                .unwrap()
                .tick,
            tick
        );
    }
    for order in [10, 11, 14] {
        assert_eq!(
            content.messages.iter().find(|m| m.order == order),
            lanes.messages.iter().find(|m| m.order == order)
        );
    }
    gui.apply();
    let transformed = gui.rt.tracks[2].clips[7].notes.clone();
    let transformed_lanes = gui.rt.tracks[2].clips[7].lanes.clone().unwrap();
    assert_eq!(transformed_lanes.meta, lanes.meta);
    assert_eq!(transformed_lanes.labels, lanes.labels);
    assert_eq!(transformed[0].source_timing.unwrap().start, 24);
    assert_eq!(transformed[1].source_timing.unwrap().start, 48);
    assert_eq!(transformed[2], original[2]);
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert_eq!(gui.rt.tracks[2].clips[7].lanes.as_ref().unwrap(), &lanes);
    gui.app.engine.send(Command::Redo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, transformed);
    save(&mut gui, &file);
    let mut reopened = Box::new(Gui::new());
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&file.0);
    reopened.click_text("Open");
    reopened.settle();
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, transformed);
    assert_eq!(
        reopened.rt.tracks[2].clips[7].lanes.as_ref().unwrap(),
        &transformed_lanes
    );
    println!("MIDI_TRANSFORM_EXPRESSION {{\"linked_events\":5,\"unselected_notes_and_events_unchanged\":true,\"metadata_and_labels_retained\":true,\"single_undo_redo\":true,\"native_save_reopen\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_worker_stale_and_cancelled_results_preserve_draft_and_history_is_bounded() {
    let mut gui = Box::new(Gui::new());
    open(&mut gui);
    gui.number("Note start", 0.12);
    gui.click("MIDI piano roll: Add note");
    gui.apply();
    gui.click("MIDI piano roll: Select all notes");
    let original = draft(&gui).notes.clone();
    let mut tools = std::mem::take(&mut gui.app.piano_roll.draft.as_mut().unwrap().tools);
    tools.preview(draft(&gui)).unwrap();
    gui.app.piano_roll.draft.as_mut().unwrap().notes[0].vel = 77;
    gui.app.piano_roll.draft.as_mut().unwrap().dirty = true;
    gui.app.piano_roll.draft.as_mut().unwrap().tools = tools;
    gui.settle();
    assert_eq!(draft(&gui).notes[0].vel, 77);
    assert!(gui
        .app
        .piano_roll
        .error
        .as_ref()
        .unwrap()
        .contains("draft changed"));
    assert!(draft(&gui).tools.preview.is_none());
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    let before = draft(&gui).notes.clone();
    let mut tools = std::mem::take(&mut gui.app.piano_roll.draft.as_mut().unwrap().tools);
    tools.preview(draft(&gui)).unwrap();
    tools
        .worker
        .as_ref()
        .unwrap()
        .cancel
        .store(true, Ordering::Release);
    gui.app.piano_roll.draft.as_mut().unwrap().tools = tools;
    gui.settle();
    assert_eq!(draft(&gui).notes, before);
    assert!(gui
        .app
        .piano_roll
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
    for index in 0..35 {
        let mut tools = std::mem::take(&mut gui.app.piano_roll.draft.as_mut().unwrap().tools);
        tools.params.strength = index as f64 / 35.0;
        tools.params.seed = index;
        tools.seed = index.to_string();
        tools.preview(draft(&gui)).unwrap();
        tools.params.seed = 1000;
        tools.seed = "1000".into();
        gui.app.piano_roll.draft.as_mut().unwrap().tools = tools;
        gui.settle();
        assert!(draft(&gui)
            .tools
            .message
            .contains(&format!("seed {index}.")));
        assert_eq!(draft(&gui).tools.history.back().unwrap().seed, index);
    }
    assert_eq!(draft(&gui).tools.history.len(), 32);
    assert_eq!(draft(&gui).tools.history_index, Some(31));
    let mut tools = std::mem::take(&mut gui.app.piano_roll.draft.as_mut().unwrap().tools);
    tools
        .restore(gui.app.piano_roll.draft.as_mut().unwrap())
        .unwrap();
    gui.app.piano_roll.draft.as_mut().unwrap().tools = tools;
    assert_eq!(draft(&gui).notes, before);
    assert!(draft(&gui).dirty);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    let mut tools = std::mem::take(&mut gui.app.piano_roll.draft.as_mut().unwrap().tools);
    tools.preview(draft(&gui)).unwrap();
    let cancellation = tools.worker.as_ref().unwrap().cancel.clone();
    gui.app.piano_roll.draft.as_mut().unwrap().tools = tools;
    gui.app.piano_roll.discard(&gui.app.engine);
    assert!(cancellation.load(Ordering::Acquire));
    assert!(gui.app.piano_roll.draft.is_none());
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    println!("MIDI_TRANSFORM_WORKER {{\"actual_worker\":true,\"stale_result_preserves_draft\":true,\"cancel_preserves_draft\":true,\"history_limit\":32,\"discard_cancels\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn native_drawn_velocity_points_and_cyclic_preset_apply_only_velocities() {
    let mut gui = Box::new(Gui::new());
    open(&mut gui);
    for i in 0..16 {
        gui.number("Note pitch", 60.0 + (i % 3) as f64);
        gui.number("Note start", i as f64 * 0.25);
        gui.number("Note velocity", 80.0 + i as f64);
        gui.click("MIDI piano roll: Add note");
    }
    gui.apply();
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    gui.click("MIDI piano roll: Select all notes");
    tools(&mut gui);
    operation(&mut gui, "Quantize", "Velocity curve");
    gui.number("Velocity minimum", 20.0);
    gui.number("Velocity maximum", 120.0);
    for _ in 0..3 {
        gui.frame(vec![]);
    }
    let rect = gui
        .node("MIDI piano roll: Draw velocity curve")
        .1
        .bounds()
        .unwrap();
    let point = |x: f64, y: f64| {
        Pos2::new(
            (rect.x0 + (rect.x1 - rect.x0) * x) as f32,
            (rect.y1 - (rect.y1 - rect.y0) * y) as f32,
        )
    };
    let start = point(0.10, 0.8);
    gui.frame(vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    gui.frame(vec![egui::Event::PointerMoved(point(0.12, 0.8))]);
    gui.frame(vec![egui::Event::PointerMoved(point(0.80, 0.2))]);
    let end = point(0.80, 0.2);
    gui.frame(vec![egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(draft(&gui).tools.shape, 6);
    assert!(draft(&gui).tools.params.curve[4] > 0.75);
    assert!(draft(&gui).tools.params.curve[25] < 0.25);
    assert!(draft(&gui).tools.params.curve[15] > 0.45 && draft(&gui).tools.params.curve[15] < 0.55);
    gui.click_text("Velocity curve points");
    gui.number("Velocity point 1", 0.3);
    gui.number("Velocity point 32", 0.7);
    preview(&mut gui);
    let shaped = draft(&gui).notes.clone();
    assert_eq!(shaped[0].vel, 50);
    assert_eq!(shaped[15].vel, 90);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    for (old, new) in original.iter().zip(&shaped) {
        let mut restored = new.clone();
        restored.vel = old.vel;
        assert_eq!(restored, *old);
    }
    gui.click("MIDI piano roll: Restore original MIDI preview");
    operation(&mut gui, "Custom", "Pulse");
    gui.number("Velocity cycles", 2.0);
    gui.number("Velocity phase", 0.25);
    preview(&mut gui);
    let cyclic = draft(&gui).notes.clone();
    assert_eq!(cyclic[0].vel, 120);
    assert!(cyclic.iter().any(|n| n.vel == 20));
    assert!(cyclic.iter().any(|n| n.vel == 120));
    gui.apply();
    assert_eq!(gui.rt.tracks[2].clips[7].notes, cyclic);
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    println!("MIDI_TRANSFORM_DRAWN {{\"actual_pointer_curve\":true,\"native_numeric_points\":true,\"cyclic_preset\":true,\"velocity_only\":true,\"single_undo\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn disabled_preview_canvas_cancels_actual_pointer_drag_and_retains_notes_during_keyboard_input() {
    let mut gui = Box::new(Gui::new());
    open(&mut gui);
    gui.number("Note pitch", 70.0);
    gui.number("Note start", 0.12);
    gui.number("Note length", 0.4);
    gui.click("MIDI piano roll: Add note");
    gui.apply();
    let committed = gui.rt.tracks[2].clips[7].notes.clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let theme = Theme::default();
    let mut time = 0.0;
    let mut render = |draft: &mut Draft, enabled: bool, events: Vec<egui::Event>| {
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
                    ui.add_enabled_ui(enabled, |ui| {
                        canvas::show(ui, &theme, draft, None).unwrap();
                    });
                });
            },
        )
    };
    let draft = gui.app.piano_roll.draft.as_mut().unwrap();
    let mut output = render(draft, true, vec![]);
    for _ in 0..3 {
        output = render(draft, true, vec![]);
    }
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, n)| {
            n.label()
                .is_some_and(|label| label.contains(" · velocity "))
                .then(|| n.bounds().unwrap())
        })
        .unwrap();
    let start = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    render(
        draft,
        true,
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    render(
        draft,
        true,
        vec![egui::Event::PointerMoved(start + Vec2::new(12.0, 0.0))],
    );
    assert!(draft.drag.is_some(), "Actual pointer gesture did not start");
    let before = draft.notes.clone();
    let selected = draft.selected.clone();
    render(
        draft,
        false,
        vec![
            egui::Event::PointerMoved(start + Vec2::new(112.0, 36.0)),
            egui::Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key: Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(draft.notes, before);
    assert_eq!(draft.selected, selected);
    assert!(draft.drag.is_none());
    render(
        draft,
        false,
        vec![egui::Event::PointerButton {
            pos: start + Vec2::new(112.0, 36.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(draft.notes, before);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, committed);
    println!("MIDI_TRANSFORM_DISABLED {{\"actual_pointer_drag\":true,\"disabled_canvas_preserves_draft\":true,\"keyboard_cannot_edit_preview\":true,\"committed_clip_unchanged\":true,\"physical_devices_opened\":false}}");
}
