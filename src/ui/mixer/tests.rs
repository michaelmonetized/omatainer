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
                egui::CentralPanel::default().show(ctx, |ui| self.app.mixer_panel(ui));
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
