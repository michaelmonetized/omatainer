use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    reference: Option<Box<crate::engine::RtEngine>>,
    independent_blocks: usize,
}
impl Gui {
    fn new() -> Self {
        let fixture = Fixture::new(256);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture,
            ctx,
            nodes: Vec::new(),
            time: 1.0,
            reference: None,
            independent_blocks: 0,
        };
        gui.fixture.app.dj_fx_open = true;
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.fixture.rt.publish_for_test();
        self.fixture.app.snap = self.fixture.app.engine.snapshot();
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 2200.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.dj_fx_ui(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let mut actual = [0.0; 512];
        self.fixture.rt.process(&mut actual);
        if let Some(reference) = &mut self.reference {
            let mut expected = [0.0; 512]; reference.process(&mut expected);
            assert_eq!(actual, expected, "Native channel controls changed the independent playing deck");
            if actual.iter().any(|sample| *sample != 0.0) { self.independent_blocks += 1; }
        }
        self.fixture.rt.publish_for_test();
        self.fixture.app.snap = self.fixture.app.engine.snapshot();
    }
    fn node(&self, label: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, node)| (*id, node))
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(label).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
    fn value(&mut self, label: &str, value: f64) {
        self.action(
            label,
            Action::SetValue,
            Some(ActionData::NumericValue(value)),
        );
    }
    fn settings(&self, bank: usize) -> crate::engine::surface_controls::EffectBank {
        self.fixture.app.engine.snapshot().surfaces.fx[bank]
    }
}

#[test]
fn native_channel_types_knob_center_confirmed_feedback_and_undo_preserve_independent_playing_pcm() {
    use crate::engine::{channel_fx::Kind, dsp::Sample};
    let mut gui = Gui::new(); let mut reference = Fixture::new(256).rt;
    reference.process(&mut [0.0; 512]);
    let audio = std::sync::Arc::new(Sample { spectrum: None, name: "Independent stereo".into(), sr: 48000, ch: 2, data: (0..960000).map(|i| (i as f32 * if i % 2 == 0 { 0.03 } else { 0.09 }).sin() * 0.1).collect(), peaks: vec![].into(), bpm: 120.0, path: String::new() });
    for rt in [&mut *gui.fixture.rt, &mut *reference] {
        rt.apply(Command::DeckAudio { deck: 1, audio: audio.clone() }); rt.decks[1].playing = true; rt.decks[1].rate = 1.0;
        rt.decks[0].playing = false; rt.master = 1.0; rt.xfader = 1.0;
    }
    gui.reference = Some(reference); gui.frame(vec![]);
    for (kind, label) in [(Kind::Echo, "Echo"), (Kind::Room, "Room"), (Kind::Filter, "Filter")] {
        gui.click(&format!("Channel effect Deck A: {label}"));
        assert_eq!(gui.fixture.app.snap.decks[0].channel_effect, kind);
        assert_eq!(gui.node(&format!("Channel effect Deck A: {label}")).1.toggled(), Some(egui::accesskit::Toggled::True));
        for value in [0.0, 0.25, 0.5, 0.75, 1.0] { gui.value("Channel effect Deck A: Knob", value); assert_eq!(gui.fixture.app.snap.decks[0].filter, value as f32); }
        gui.click("Channel effect Deck A: Center"); assert_eq!(gui.fixture.app.snap.decks[0].filter, 0.5);
        for _ in 0..20 { gui.frame(vec![]); }
    }
    gui.click("Channel effect Deck A: Echo");
    gui.fixture.app.engine.send(Command::Undo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.fixture.app.snap.decks[0].channel_effect, Kind::Filter);
    gui.fixture.app.engine.send(Command::Redo).unwrap(); gui.frame(vec![]);
    assert_eq!(gui.fixture.app.snap.decks[0].channel_effect, Kind::Echo);
    assert!(gui.independent_blocks >= 100);
    println!("CHANNEL_EFFECT_NATIVE {{\"native_egui_accesskit\":true,\"confirmed_type_and_knob\":true,\"undo_redo\":true,\"exact_independent_pcm_blocks\":{},\"physical_devices_opened\":false}}", gui.independent_blocks);
}

#[test]
fn native_unit_assignment_timing_wet_bypass_and_settled_placement_are_exact_and_independent() {
    let mut gui = Gui::new();
    let other = serde_json::to_value(gui.settings(1)).unwrap();
    gui.click("Unit A: Master");
    gui.click("Unit A: Manual timing");
    gui.value("Unit A: Echo time ms", 53.0);
    gui.value("Unit A slot 1: Wet", 0.3);
    gui.value("Unit A slot 1: Feedback / cutoff", 0.25);
    gui.click("Unit A slot 1: Enabled");
    let settings = gui.settings(0);
    assert!(settings.master);
    assert_eq!(settings.timing, Timing::Manual);
    assert_eq!(settings.manual_ms, 53.0);
    assert_eq!(settings.wet[0], 0.3);
    assert_eq!(settings.parameter[0], 0.25);
    assert!(settings.on[0]);
    assert_eq!(serde_json::to_value(gui.settings(1)).unwrap(), other);
    assert!(gui.node("Unit A: Post fader").1.is_disabled());
    gui.click("Unit A slot 1: Enabled");
    for _ in 0..600 {
        gui.fixture.rt.process(&mut [0.0; 512]);
    }
    gui.frame(vec![]);
    assert!(!gui.settings(0).tails.iter().any(|tail| *tail));
    gui.click("Unit A: Post fader");
    assert_eq!(gui.settings(0).placement, Placement::PostFader);
    gui.click("Unit A: Beat timing");
    gui.click("Unit A: 3/16 beat");
    assert_eq!(gui.settings(0).beats, -2);
    assert_eq!(gui.settings(0).timing, Timing::Beat);
    gui.click("Unit A slot 1: Enabled");
    gui.fixture.app.dj_fx_open = false;
    gui.frame(vec![]);
    assert!(gui.settings(0).on[0]);
    println!("DJ_FX_NATIVE {{\"native_egui_accesskit\":true,\"independent_units\":true,\"applied_timing_wet_and_bypass\":true,\"settled_placement\":true,\"window_close_retains_live_controls\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn native_sampler_destination_selection_keeps_stable_track_ownership_and_distinct_unit_slot_controls(
) {
    let mut gui = Gui::new();
    let layout = gui.fixture.app.engine.snapshot().session.unwrap();
    let expected = layout
        .reference(crate::engine::session::Axis::Track, 1)
        .unwrap();
    gui.click("Unit A: Choose sampler destination");
    gui.click("Unit A: Sampler destination track 2");
    assert_eq!(gui.settings(0).sampler, Some(expected));
    assert!(gui.settings(1).sampler.is_none());
    for bank in ["A", "B"] {
        for slot in 1..=3 {
            let wet = gui.node(&format!("Unit {bank} slot {slot}: Wet")).0;
            let on = gui.node(&format!("Unit {bank} slot {slot}: Enabled")).0;
            assert_ne!(wet, on);
        }
    }
    gui.click("Unit A: Sampler destination");
    assert!(gui.settings(0).sampler.is_none());
    gui.value("Unit B slot 3: Wet", 0.72);
    assert_eq!(gui.settings(1).wet[2], 0.72);
    assert_eq!(gui.settings(0).wet[2], 0.5);
}
