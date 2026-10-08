use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest};
struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
    output: egui::FullOutput,
}
impl Gui {
    fn new() -> Box<Self> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Box::new(Self {
            fixture: Fixture::new(128),
            ctx,
            time: 0.0,
            output: Default::default(),
        });
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.action("Deck A: Pad mode", Action::Click, None);
        gui.action("Deck A: Pad mode Slice", Action::Click, None);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        self.output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        self.frame(vec![]);
        let target = self
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
}
#[test]
fn native_slicer_domain_repeat_and_trigger_settings_are_accessible_independent_and_transport_neutral(
) {
    let mut gui = Gui::new();
    let positions = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    let mixer = (gui.fixture.rt.master, gui.fixture.rt.xfader);
    gui.click("Deck A: Slicer behavior");
    gui.click("Deck A: Slicer Repeating");
    gui.click("Deck A: Slicer domain");
    gui.click("Deck A: Slicer domain 16 beats");
    gui.click("Deck A: Slicer repeat length");
    gui.click("Deck A: Slicer repeat 1/4");
    gui.click("Deck A: Slicer trigger timing");
    gui.click("Deck A: Slicer trigger 0.5 beats");
    let state = gui.fixture.app.snap.decks[0].controls;
    assert!(state.slicer.repeating);
    assert_eq!(state.slice_domain, 4);
    assert_eq!(state.slice_quant, 2);
    assert_eq!(state.slicer.division, Some(2));
    let other = gui.fixture.app.snap.decks[1].controls;
    assert!(!other.slicer.repeating);
    assert_eq!(other.slice_domain, 3);
    assert_eq!(other.slice_quant, 0);
    assert_eq!(other.slicer.division, None);
    assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), positions);
    assert_eq!((gui.fixture.rt.master, gui.fixture.rt.xfader), mixer);
    assert!(gui.fixture.rt.decks.iter().all(|d| !d.playing));
}
#[test]
fn native_slicer_source_bar_and_pending_pad_report_the_actual_renderer_and_release_cancels_it() {
    let mut gui = Gui::new();
    gui.click("Deck A: Slicer trigger timing");
    gui.click("Deck A: Slicer trigger 4 beats");
    assert!(
        !egui::Popup::is_any_open(&gui.ctx),
        "Slicer choices must surrender native performance input"
    );
    gui.fixture
        .rt
        .apply(Command::DeckSeek { deck: 0, frac: 0.2 });
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 });
    gui.fixture.rt.process(&mut [0.0; 256]);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    gui.action(
        "Deck A: Slice pad 3: Slice 3",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    let controls = gui.fixture.app.snap.decks[0].controls;
    assert_eq!(
        controls.slicer.pending,
        Some(2),
        "playing={} position={} controls={controls:?} owners={:?}",
        gui.fixture.app.snap.decks[0].playing,
        gui.fixture.rt.decks[0].pos,
        gui.fixture.app.deck_pad_inputs.owners[0][2]
    );
    assert!(controls.slicer.active.is_none());
    let bounds = controls.slicer.bounds.unwrap();
    let rate = f64::from(gui.fixture.app.snap.decks[0].source_sample_rate);
    gui.frame(vec![]);
    let nodes = &gui
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    let boundary = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Deck A: Slicer source boundaries"))
        .unwrap();
    let description = boundary.1.description().unwrap();
    for i in 0..8 {
        assert!(description.contains(&format!(
            "Slice {}: {:.3}–{:.3} source seconds",
            i + 1,
            bounds[i] / rate,
            bounds[i + 1] / rate
        )));
    }
    let mut mesh_found = false;
    for shape in &gui.output.shapes {
        if let egui::epaint::Shape::Rect(rect) = &shape.shape {
            if rect.fill == Color32::from_rgb(255, 160, 40) && rect.rect.height() == 24.0 {
                mesh_found = true;
            }
        }
    }
    assert!(mesh_found, "Pending slice is actually painted");
    gui.action(
        "Deck A: Slice pad 3: Slice 3",
        Action::CustomAction,
        Some(ActionData::CustomAction(2)),
    );
    assert!(gui.fixture.app.snap.decks[0]
        .controls
        .slicer
        .pending
        .is_none());
    assert!(gui.fixture.app.snap.decks[0].controls.slice.is_none());
    assert!(gui.fixture.app.snap.decks[1]
        .controls
        .slicer
        .bounds
        .is_none());
    println!(
        "SLICER_NATIVE_RECEIPT {}",
        serde_json::json!({"accessible_independent_settings":true,"exact_eight_source_intervals":true,"actual_pending_marker_painted":true,"native_original_release_cancelled":true,"physical_devices_opened":false})
    );
}
