use super::*;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
struct Gui {
    app: App,
    rt: Box<crate::engine::RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    path: PathBuf,
}
impl Gui {
    fn new() -> Self {
        let fixture = test_support::Fixture::new(256);
        let mut app = fixture.app;
        let mut rt = fixture.rt;
        rt.apply(Command::Stop);
        rt.routing = Some(Box::new(
            crate::engine::audio::routing::prepared::Prepared::new(
                Arc::new(crate::engine::audio::routing::mic_aux::tests::model()),
                &rt.session,
            )
            .unwrap(),
        ));
        let path = std::env::temp_dir().join(format!(
            "omatainer-native-mic-aux-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        app.library_metadata =
            library_metadata::Metadata::with_hook(path.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut g = Self {
            app,
            rt,
            ctx,
            nodes: vec![],
            time: 0.0,
            path,
        };
        g.wait();
        g.click("Mic & aux");
        g
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        self.rt.publish_for_test();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        self.rt.process(&mut [0.0; 256]);
    }
    fn click(&mut self, label: &str) {
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
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
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        loop {
            self.frame(vec![]);
            if self.app.mic_aux.job.is_none()
                && self.app.mic_aux.ack.is_none()
                && !self.app.library_metadata.active()
            {
                break;
            }
            assert!(Instant::now() < deadline, "{}", self.app.mic_aux.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
#[test]
fn native_review_applies_choices_and_live_mute_tone_talkover_work_in_performance_mode() {
    let mut g = Gui::new();
    g.click("Review mic/aux sources");
    g.wait();
    assert!(g.app.mic_aux.draft.is_some());
    g.app.mic_aux.draft.as_mut().unwrap().configuration =
        crate::engine::audio::routing::mic_aux::tests::configuration();
    g.frame(vec![]);
    g.click("Apply mic/aux selection");
    g.wait();
    assert!(g
        .app
        .mic_aux
        .message
        .starts_with("Mic/aux selection applied"));
    assert_eq!(
        g.rt.mic_aux.configuration().unwrap().channels[0].mute,
        false
    );
    g.rt.apply(Command::PerformanceMode(true));
    g.frame(vec![]);
    g.click("Mic mute");
    assert!(g.rt.mic_aux.configuration().unwrap().channels[0].mute);
    g.click("Mic tone");
    assert!(g.rt.mic_aux.configuration().unwrap().channels[0].tone);
    g.click("Talkover ducking");
    assert!(g.rt.mic_aux.configuration().unwrap().duck.enabled);
    g.click("Hold duck");
    assert_eq!(
        g.rt.mic_aux.configuration().unwrap().duck.mode,
        Override::Held
    );
    g.click("Bypass duck");
    assert_eq!(
        g.rt.mic_aux.configuration().unwrap().duck.mode,
        Override::Off
    );
    g.click("Review mic/aux sources");
    g.wait();
    g.frame(vec![]);
    assert!(g
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Apply mic/aux selection"))
        .unwrap()
        .1
        .is_disabled());
    assert!(g.rt.performance.protected());
    g.rt.apply(Command::PerformanceMode(false));
    g.frame(vec![]);
    g.click("Remove mic/aux selection");
    g.wait();
    assert_eq!(g.rt.mic_aux.configuration(), None);
}
#[test]
fn native_stale_review_refuses_alias_change_and_preserves_existing_configuration() {
    let mut g = Gui::new();
    g.click("Review mic/aux sources");
    g.wait();
    g.app.mic_aux.draft.as_mut().unwrap().configuration =
        crate::engine::audio::routing::mic_aux::tests::configuration();
    g.rt.apply(Command::Master(0.2));
    g.frame(vec![]);
    g.click("Apply mic/aux selection");
    g.wait();
    assert!(g
        .app
        .mic_aux
        .message
        .starts_with("Selection was not applied"));
    assert_eq!(g.rt.mic_aux.configuration(), None);
}
