use super::*;
use crate::engine::audio::OutputCallback;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-native-audio-export-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    callback: OutputCallback,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut g = Self {
            app,
            callback: OutputCallback::new(rt, 2),
            ctx,
            nodes: vec![],
            time: 0.0,
        };
        g.wait();
        g.click("Audio export & recording");
        g
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        self.callback.renderer_mut_for_test().publish_for_test();
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
        self.callback.render(&mut [0.0_f32; 512]);
    }
    fn click(&mut self, label: &str) {
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {label}"));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn wait(&mut self) {
        let end = Instant::now() + std::time::Duration::from_secs(15);
        loop {
            self.frame(vec![]);
            if self.app.audio_delivery.job.is_none() && !self.app.library_metadata.active() {
                break;
            }
            assert!(Instant::now() < end, "{}", self.app.audio_delivery.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}
#[test]
fn native_source_review_export_decode_and_stale_review_preserve_live_session() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.click("Review export source");
    g.wait();
    assert!(g.app.audio_delivery.plan.is_some());
    let before = g.app.snap.project_revision;
    g.app.audio_delivery.request.end = 0.04;
    g.app.audio_delivery.request.tail = 0.0;
    let folder = files.0.join("delivery");
    g.app.audio_delivery.path = folder.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Saved"),
        "{}",
        g.app.audio_delivery.message
    );
    let decoded = crate::engine::decode::decode_audio(&folder.join("master.wav")).unwrap();
    assert_eq!(decoded.sample.frames(), 1920);
    assert_eq!(g.app.snap.project_revision, before);
    assert!(!g.callback.renderer_for_test().playing);
    assert!(g
        .callback
        .renderer_for_test()
        .decks
        .iter()
        .all(|d| !d.playing));
    g.callback
        .renderer_mut_for_test()
        .apply(Command::SetBpm(130.0));
    let refused = files.0.join("stale");
    g.app.audio_delivery.path = refused.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(
        g.app
            .audio_delivery
            .message
            .contains("changed since export review"),
        "{}",
        g.app.audio_delivery.message
    );
    assert!(!refused.exists());
    assert!(folder.join("master.wav").exists());
}

#[test]
fn native_performance_record_stop_decode_and_reviewed_recovery_work_in_performance_mode() {
    use std::io::{Seek, SeekFrom, Write};
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.click("Review export source");
    g.wait();
    g.callback
        .renderer_mut_for_test()
        .apply(Command::PerformanceMode(true));
    g.callback
        .renderer_mut_for_test()
        .apply(Command::DeckPlay { deck: 0 });
    let revision = g.app.engine.project.revision();
    let folder = files.0.join("performance");
    g.app.audio_delivery.record_path = folder.to_string_lossy().into_owned();
    g.app.audio_delivery.record_seconds = 60;
    g.frame(vec![]);
    g.click("Start performance recording");
    let end = Instant::now() + std::time::Duration::from_secs(15);
    while g.app.engine.routing.recorder.frames() < 2048 {
        g.frame(vec![]);
        assert!(Instant::now() < end, "{}", g.app.audio_delivery.message);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    g.click("Stop performance recording");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Recorded"),
        "{}",
        g.app.audio_delivery.message
    );
    let audio = folder.join("take-0001.wav");
    let decoded = crate::engine::decode::decode_audio(&audio).unwrap();
    assert!(decoded.sample.frames() >= 2048);
    assert!(decoded.sample.data.iter().any(|v| v.abs() > 0.001));
    assert_eq!(g.app.engine.project.revision(), revision);
    assert!(g.callback.renderer_for_test().decks[0].playing);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&audio)
        .unwrap();
    file.seek(SeekFrom::Start(40)).unwrap();
    file.write_all(&0_u32.to_le_bytes()).unwrap();
    drop(file);
    g.app.audio_delivery.recovery_path = folder.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Review recording recovery");
    g.wait();
    assert_eq!(g.app.audio_delivery.recovery.as_ref().unwrap().repairs, 1);
    g.click("Recover reviewed recording");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Recovered"),
        "{}",
        g.app.audio_delivery.message
    );
    assert_eq!(
        crate::engine::decode::decode_audio(&audio)
            .unwrap()
            .sample
            .data,
        decoded.sample.data
    );
    g.app.audio_delivery.path = files
        .0
        .join("protected-export")
        .to_string_lossy()
        .into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(!files.0.join("protected-export").exists());
}
