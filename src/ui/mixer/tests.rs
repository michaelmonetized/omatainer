use super::*;
use crate::engine::{session::Axis, RtEngine};
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt: Box::new(rt),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
        };
        gui.app.snap = gui.app.engine.snapshot();
        gui.frame(Vec::new());
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.app.snap = self.app.engine.snapshot();
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 1100.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                self.app
                    .undo_history
                    .begin_frame(ctx, &self.app.engine.undo);
                egui::CentralPanel::default().show(ctx, |ui| self.app.mixer_panel(ui));
                self.app.undo_history.end_frame(ctx);
            },
        );
        self.nodes = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.rt.process(&mut []);
        self.rt.publish_for_test();
        output
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| panic!("Missing {label}"))
            .0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(Vec::new());
    }
    fn select(&mut self, slots: &[usize]) {
        let layout = self.app.snap.session.as_ref().unwrap();
        self.app.mixer.selection = Some(
            slots
                .iter()
                .filter_map(|slot| {
                    layout
                        .reference(Axis::Track, *slot)
                        .map(|target| (*slot, target))
                })
                .collect(),
        );
        self.frame(Vec::new());
    }
}

#[test]
fn shared_inspector_edits_real_track_commands_mixed_reset_and_one_step_undo() {
    let mut gui = Gui::new();
    gui.rt.apply(Command::TrackGain {
        track: 0,
        value: 0.3,
    });
    gui.rt.apply(Command::TrackGain {
        track: 1,
        value: 0.7,
    });
    gui.rt.publish_for_test();
    gui.select(&[0, 1]);
    let output = gui.frame(Vec::new());
    assert!(output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Mixed")
    ));
    gui.action(
        "Track gain",
        Action::SetValue,
        Some(ActionData::NumericValue(55.0)),
    );
    assert_eq!((gui.rt.tracks[0].gain, gui.rt.tracks[1].gain), (0.55, 0.55));
    gui.rt.apply(Command::Undo);
    assert_eq!((gui.rt.tracks[0].gain, gui.rt.tracks[1].gain), (0.3, 0.7));
    gui.rt.apply(Command::Redo);
    assert_eq!((gui.rt.tracks[0].gain, gui.rt.tracks[1].gain), (0.55, 0.55));
    gui.action(
        "Track pan",
        Action::SetValue,
        Some(ActionData::NumericValue(-50.0)),
    );
    assert_eq!((gui.rt.tracks[0].pan, gui.rt.tracks[1].pan), (-0.5, -0.5));
    gui.action("Reset Track pan", Action::Click, None);
    assert_eq!((gui.rt.tracks[0].pan, gui.rt.tracks[1].pan), (0.0, 0.0));
    gui.action("Mute selected tracks", Action::Click, None);
    assert!(gui.rt.tracks[0].mute && gui.rt.tracks[1].mute);
    gui.rt.apply(Command::Undo);
    assert!(!gui.rt.tracks[0].mute && !gui.rt.tracks[1].mute);
}

#[test]
fn shared_inspector_empty_single_mixed_locked_labels_fit_all_scales() {
    let mut gui = Gui::new();
    for scale in [1.0, 1.5, 2.0] {
        gui.ctx.set_pixels_per_point(scale);
        for slots in [Vec::new(), vec![0], vec![0, 1]] {
            gui.select(&slots);
            for locked in [false, true] {
                gui.app.mixer.locked = locked;
                let output = gui.frame(Vec::new());
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if matches!(
                            text.galley.text(),
                            "Track gain" | "Track pan" | "Reset Track gain" | "Reset Track pan"
                        ) {
                            let bounds = text.visual_bounding_rect();
                            assert!(
                                shape.clip_rect.expand(1.0).contains_rect(bounds),
                                "clipped at {scale}: {:?}",
                                text.galley.text()
                            );
                        }
                    }
                }
                if !slots.is_empty() {
                    let node = gui
                        .nodes
                        .iter()
                        .find(|(_, node)| node.label() == Some("Track gain"))
                        .unwrap();
                    assert_eq!(node.1.supports_action(Action::SetValue), !locked);
                    if locked {
                        let before = gui.rt.tracks[slots[0]].gain;
                        gui.action(
                            "Track gain",
                            Action::SetValue,
                            Some(ActionData::NumericValue(130.0)),
                        );
                        assert_eq!(gui.rt.tracks[slots[0]].gain, before);
                    }
                }
            }
        }
    }
}

#[test]
fn stale_selection_never_retargets_replaced_tracks() {
    let mut gui = Gui::new();
    gui.select(&[0]);
    gui.app.mixer.selection.as_mut().unwrap()[0].1.namespace = [77, 88];
    gui.frame(Vec::new());
    assert!(gui.app.mixer.selection.as_ref().unwrap().is_empty());
    assert!(gui.app.mixer.message.contains("changed"));
}

#[test]
fn focused_track_field_keyboard_edit_and_error_keep_typed_undo() {
    let mut gui = Gui::new();
    gui.select(&[0]);
    let before = gui.rt.tracks[0].gain;
    gui.action("Track gain", Action::Focus, None);
    gui.frame(vec![egui::Event::Key {
        key: Key::ArrowUp,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    gui.frame(vec![egui::Event::Key {
        key: Key::ArrowUp,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Default::default(),
    }]);
    assert_ne!(gui.rt.tracks[0].gain, before);
    gui.rt.apply(Command::Undo);
    assert_eq!(gui.rt.tracks[0].gain, before);
    gui.app
        .submission_error
        .set(Some(crate::engine::SubmissionError::InvalidTarget));
    let output = gui.frame(Vec::new());
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("target"))));
}

#[test]
fn inspector_track_values_survive_native_project_save_and_reopen() {
    use crate::engine::project::{Prepared, State};
    use std::sync::atomic::AtomicBool;
    let mut gui = Gui::new();
    gui.select(&[0, 1]);
    gui.action(
        "Track gain",
        Action::SetValue,
        Some(ActionData::NumericValue(63.0)),
    );
    gui.action(
        "Track pan",
        Action::SetValue,
        Some(ActionData::NumericValue(25.0)),
    );
    let handle = gui.app.engine.project.clone();
    let worker = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !worker.is_finished() {
        gui.rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let captured = worker.join().unwrap().unwrap();
    let file =
        std::env::temp_dir().join(format!("omatainer-inspector-{}.omat", std::process::id()));
    let bundle = crate::project_file::Bundle {
        state: captured.state,
        media: captured.media,
    };
    crate::project_file::save(
        &file,
        &bundle,
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<State>(
        &file,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let prepared = Prepared::from_state(reopened.state, reopened.media, 48_000).unwrap();
    let (engine, mut restored) = Engine::headless_for_test(48_000, 256);
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    let worker =
        std::thread::spawn(move || handle.install(prepared, revision, &AtomicBool::new(false)));
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !worker.is_finished() {
        restored.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    worker.join().unwrap().unwrap();
    assert_eq!(
        (restored.tracks[0].gain, restored.tracks[1].gain),
        (0.63, 0.63)
    );
    assert_eq!(
        (restored.tracks[0].pan, restored.tracks[1].pan),
        (0.25, 0.25)
    );
    std::fs::remove_file(file).unwrap();
}
