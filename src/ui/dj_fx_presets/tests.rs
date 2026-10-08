use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
struct Gui { fixture: Fixture, ctx: egui::Context, nodes: Vec<(NodeId, Node)>, time: f64, reference: Box<crate::engine::RtEngine>, nonzero: usize }
impl Gui {
    fn new() -> Self {
        let fixture = Fixture::new(128); let reference = Fixture::new(128).rt; let ctx = egui::Context::default(); ctx.enable_accesskit();
        let mut gui = Self { fixture, reference, ctx, nodes: vec![], time: 1.0, nonzero: 0 }; gui.frame(vec![]); gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.fixture.rt.publish_for_test(); self.fixture.app.snap = self.fixture.app.engine.snapshot(); self.time += 0.02;
        let output = self.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 2200.0))), time: Some(self.time), focused: true, events, ..Default::default() }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| self.fixture.app.dj_fx_presets_ui(ui)); });
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let mut actual = [0.0; 512]; let mut expected = [0.0; 512]; self.fixture.rt.process(&mut actual); self.reference.process(&mut expected); assert_eq!(actual, expected, "Preset controls changed the independently playing deck"); if actual.iter().any(|value| *value != 0.0) { self.nonzero += 1; }
        self.fixture.rt.publish_for_test(); self.fixture.app.snap = self.fixture.app.engine.snapshot();
    }
    fn click(&mut self, label: &str) {
        let target = self.nodes.iter().find(|(_, node)| node.label().is_some_and(|actual| actual == label || actual.ends_with(&format!(": {label}")))).map(|(id, _)| *id).unwrap_or_else(|| panic!("Missing {label}: {:?}", self.nodes.iter().filter_map(|(_, node)| node.label()).collect::<Vec<_>>()));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { action: Action::Click, target, data: None })]); self.frame(vec![]);
    }
    fn wait(&mut self) { let deadline = Instant::now() + std::time::Duration::from_secs(3); while self.fixture.app.dj_fx_presets.busy() { self.frame(vec![]); assert!(Instant::now() < deadline, "Preset worker did not finish: {}", self.fixture.app.dj_fx_presets.message); std::thread::sleep(std::time::Duration::from_millis(1)); } self.frame(vec![]); }
}
#[test]
fn actual_native_save_inspect_review_atomic_recall_and_stale_refusal_preserve_playing_pcm() {
    let mut gui = Gui::new(); gui.click("Saved layouts");
    let audio = Arc::new(crate::engine::dsp::Sample { spectrum: None, name: "Independent playing B".into(), sr: 48000, ch: 2, data: (0..960000).map(|frame| (frame as f32 * if frame % 2 == 0 {0.03} else {0.09}).sin() * 0.1).collect(), peaks: vec![].into(), bpm: 120.0, path: String::new() });
    for rt in [&mut *gui.fixture.rt, &mut *gui.reference] { rt.apply(Command::DeckAudio { deck: 1, audio: audio.clone() }); rt.decks[1].playing = true; rt.decks[1].rate = 1.0; rt.master = 1.0; rt.xfader = 1.0; }
    let directory = std::env::temp_dir().join(format!("omatainer-native-fx-{}-{}", std::process::id(), SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())); std::fs::create_dir(&directory).unwrap();
    gui.fixture.app.dj_fx_presets.path = directory.join("routine.omatfx").to_string_lossy().into_owned(); gui.fixture.app.dj_fx_presets.names[0] = "Scratch echo".into(); gui.click("Apply Unit A name"); assert_eq!(gui.fixture.app.snap.surfaces.fx[0].name.as_str(), "Scratch echo");
    gui.click("Save new layout"); gui.wait(); assert!(directory.join("routine.omatfx").exists()); let bytes = std::fs::read(directory.join("routine.omatfx")).unwrap();
    gui.fixture.app.engine.send(Command::Surface(Input::FxValue { bank: 0, slot: 0, parameter: false, value: 0.2 })).unwrap(); gui.frame(vec![]);
    gui.click("Inspect layout"); gui.wait(); assert_eq!(gui.fixture.app.dj_fx_presets.review.unwrap().units[0].name.as_str(), "Scratch echo");
    gui.click("Review recall"); gui.click("Recall reviewed layout · reset tails"); gui.wait(); assert_eq!(gui.fixture.app.snap.surfaces.fx[0].wet[0], 0.5); assert!(gui.fixture.app.dj_fx_presets.message.starts_with("Recalled both units"));
    gui.click("Save new layout"); gui.wait(); assert_eq!(std::fs::read(directory.join("routine.omatfx")).unwrap(), bytes);
    gui.click("Review recall"); gui.fixture.app.engine.send(Command::Surface(Input::FxValue { bank: 0, slot: 0, parameter: false, value: 0.3 })).unwrap(); gui.frame(vec![]); gui.click("Recall reviewed layout · reset tails"); gui.wait(); assert_eq!(gui.fixture.app.snap.surfaces.fx[0].wet[0], 0.3); assert!(gui.fixture.app.dj_fx_presets.message.starts_with("Recall refused"));
    for _ in 0..100 { gui.frame(vec![]); } assert!(gui.nonzero >= 100);
    println!("DJ_FX_PRESET_NATIVE {{\"save_inspect_review_recall\":true,\"stale_refusal\":true,\"exact_independent_pcm_blocks\":{},\"physical_devices_opened\":false}}", gui.nonzero);
}

#[test]
fn native_foreign_sampler_routes_require_explicit_binding_or_removal_before_atomic_recall() {
    let mut gui = Gui::new(); gui.click("Saved layouts");
    for label in ["Preset name", "Preset path", "Unit A name", "Unit B name"] { assert!(gui.nodes.iter().any(|(_, node)| node.label().is_some_and(|value| value.ends_with(label))), "Unlabelled preset input {label}"); }
    let mut units = gui.fixture.app.snap.surfaces.fx.map(Settings::from);
    units[0].sampler = Some(crate::engine::session::Reference { namespace: [42, 43], id: crate::engine::session::Id(1) });
    gui.fixture.app.dj_fx_presets.review = Some(Preset { version: 1, name: Name::new("Foreign sampler layout").unwrap(), units }); gui.frame(vec![]);
    assert!(!route_valid(units[0], &gui.fixture.app.snap)); gui.click("Review recall"); assert!(gui.fixture.app.dj_fx_presets.expected.is_none());
    let target = gui.fixture.app.snap.session.as_ref().unwrap().reference(Axis::Track, gui.fixture.app.snap.selected_track).unwrap();
    gui.click("Bind Unit A to selected sampler track"); assert_eq!(gui.fixture.app.dj_fx_presets.review.unwrap().units[0].sampler, Some(target));
    gui.click("Review recall"); gui.click("Recall reviewed layout · reset tails"); gui.wait(); assert_eq!(gui.fixture.app.snap.surfaces.fx[0].sampler, Some(target));
    gui.click("Remove Unit A sampler route"); assert!(gui.fixture.app.dj_fx_presets.expected.is_none()); gui.click("Review recall"); gui.click("Recall reviewed layout · reset tails"); gui.wait(); assert_eq!(gui.fixture.app.snap.surfaces.fx[0].sampler, None);
    println!("DJ_FX_PRESET_BINDING {{\"foreign_route_requires_review\":true,\"explicit_binding_and_removal\":true,\"physical_devices_opened\":false}}");
}
