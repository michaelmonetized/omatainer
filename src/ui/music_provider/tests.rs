use super::*;
use crate::music_provider::tests::{track, ContractProvider};
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::time::Duration;

const CONSENT: &str =
    "Enable for non-commercial use; I accept the current license and attribution requirement";
struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new(provider: Arc<dyn MusicProvider>) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut fixture = Fixture::new(256);
        fixture.app.music_provider.provider = provider;
        let mut gui = Self {
            fixture,
            ctx,
            nodes: vec![],
            time: 0.0,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.click("Music providers");
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1540.0, 960.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut [0.0; 128]);
        self.fixture.rt.publish_for_test();
    }
    fn click(&mut self, name: &str) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.fixture.app.music_provider.job.is_some() {
            assert!(Instant::now() < deadline);
            self.frame(vec![]);
            std::thread::yield_now();
        }
        self.frame(vec![]);
    }
}

#[test]
fn native_consent_search_preview_credits_stop_and_license_revocation_use_real_handlers() {
    let provider = Arc::new(ContractProvider::default());
    let mut gui = Gui::new(provider.clone());
    let library_count = gui.fixture.app.library.len();
    assert_eq!(gui.fixture.app.music_provider.license, License::Missing);
    gui.click("Search provider");
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    gui.click(CONSENT);
    gui.click("Search provider");
    gui.wait();
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    assert_eq!(
        gui.fixture.app.music_provider.page.as_ref().unwrap().tracks[0].id,
        track().id
    );
    gui.click("Preview Contract tone");
    gui.wait();
    let watch = gui.fixture.app.music_provider.preview.as_ref().unwrap();
    assert_eq!(watch.state.load(Ordering::Acquire), 1);
    assert!(watch.credits.contains("Contract tone by Omatainer test"));
    assert_eq!(gui.fixture.app.library.len(), library_count);
    gui.click("Copy preview credits");
    gui.click("Stop provider preview");
    assert_eq!(
        gui.fixture
            .app
            .music_provider
            .preview
            .as_ref()
            .unwrap()
            .state
            .load(Ordering::Acquire),
        3
    );
    gui.click("Preview Contract tone");
    gui.wait();
    gui.click(CONSENT);
    assert_eq!(gui.fixture.app.music_provider.license, License::Missing);
    assert!(gui.fixture.app.music_provider.page.is_none());
    let watch = gui.fixture.app.music_provider.preview.as_ref().unwrap();
    assert!(watch.cancel.load(Ordering::Acquire));
    assert_eq!(watch.state.load(Ordering::Acquire), 3);
    assert_eq!(gui.fixture.app.library.len(), library_count);
}

#[test]
fn native_cancel_close_and_performance_mode_do_not_publish_obsolete_audio() {
    let provider = Arc::new(ContractProvider {
        block: AtomicBool::new(true),
        ..ContractProvider::default()
    });
    let mut gui = Gui::new(provider.clone());
    gui.click(CONSENT);
    gui.click("Search provider");
    gui.click("Cancel provider request");
    gui.wait();
    assert!(gui.fixture.app.music_provider.page.is_none());
    provider.block.store(false, Ordering::Release);
    gui.click("Search provider");
    gui.wait();
    gui.click("Preview Contract tone");
    gui.wait();
    gui.fixture.app.music_provider.open = false;
    gui.fixture.app.music_provider.stop();
    gui.frame(vec![]);
    assert_eq!(
        gui.fixture
            .app
            .music_provider
            .preview
            .as_ref()
            .unwrap()
            .state
            .load(Ordering::Acquire),
        3
    );
    gui.fixture.app.music_provider.open = true;
    gui.frame(vec![]);
    gui.fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(true)
        .unwrap();
    gui.frame(vec![]);
    let count = provider.calls.load(Ordering::Acquire);
    gui.click("Search provider");
    assert_eq!(provider.calls.load(Ordering::Acquire), count);
    assert!(gui.fixture.app.music_provider.job.is_none());
}
