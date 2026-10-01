use super::*;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Gui {
    app: App,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    controls: Arc<owner::tests::Controls>,
    dir: PathBuf,
}
fn inventory() -> config::Inventory {
    let device = config::Device {
        name: "Fixture".into(),
        default: true,
        defaults: Some((2, 48000, cpal::SampleFormat::F32)),
        ranges: vec![config::Range {
            channels: 2,
            min_rate: 44100,
            max_rate: 192000,
            format: cpal::SampleFormat::F32,
            buffer: Some((64, 2048)),
        }],
        error: None,
    };
    config::Inventory {
        backend: "Fixture".into(),
        devices: vec![device.clone()],
        inputs: vec![device],
        input_error: None,
        truncated: false,
    }
}
impl Gui {
    fn new(rate: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "omatainer-audio-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (engine, controls) = owner::tests::engine_fixture();
        let handle = engine.audio_handle();
        let mut app = App::with_loader(engine, Theme::default(), None);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut preferences = crate::preferences::Preferences::defaults(&dir);
        for profile in preferences.profiles.values_mut() {
            profile.startup.scan_library = false;
            profile.audio.sample_rate = Some(rate);
            profile.audio.buffer_frames = Some(128);
            profile.audio.calibration.buffer_frames = Some(128);
        }
        let path = dir.join("preferences.json");
        std::fs::write(&path, serde_json::to_vec(&preferences).unwrap()).unwrap();
        app.initialize_preferences(
            &ctx,
            crate::preferences::worker::Startup::read(path, dir.clone()),
            Audio::default(),
        );
        app.audio_settings = Panel::with_discovery(handle, || Ok(inventory()));
        let mut gui = Self {
            app,
            ctx,
            nodes: vec![],
            time: 0.0,
            controls,
            dir,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.025;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1700.0, 1300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
    }
    fn click(&mut self, name: &str) {
        let target = self
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.label() == Some(name) && node.supports_action(Action::Click)).then_some(*id)
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing action {name}; {:?}",
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
        let deadline = Instant::now() + std::time::Duration::from_secs(4);
        while self.app.audio_settings.busy() {
            self.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "{}",
                self.app.audio_settings.message
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
    fn open(&mut self) {
        self.click("Preferences");
        self.click("Audio devices and latency");
        assert!(self.app.audio_settings.open);
    }
    fn preview(&mut self) {
        self.click("Preview saved audio");
        self.wait();
        assert!(self
            .app
            .audio_settings
            .preview
            .as_ref()
            .unwrap()
            .output
            .is_ok());
    }
    fn apply(&mut self) {
        self.click("Use saved audio now");
        self.click("Stop and change output");
        self.wait();
    }
    fn measure(&mut self) {
        self.click("Measure loopback");
        self.click("Cable ready: stop and measure");
        self.wait();
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
#[test]
fn real_ui_requires_confirmation_preserves_cancel_and_reopens_saved_supported_rates() {
    for rate in [44100, 48000, 96000, 192000] {
        let mut gui = Gui::new(rate);
        gui.open();
        gui.preview();
        assert_eq!(gui.app.engine.sr(), 48000);
        gui.click("Use saved audio now");
        gui.click("Keep current audio");
        assert_eq!(gui.controls.opens.load(Ordering::Acquire), 1);
        gui.apply();
        assert_eq!(gui.app.engine.sr(), rate);
        assert!(!gui.app.settings.pending_restart());
        let stored = crate::preferences::storage::load(
            &gui.dir.join("preferences.json"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            stored.preferences.current().unwrap().audio.sample_rate,
            Some(rate)
        );
        assert!(gui.app.audio_settings.message.contains("applied"));
    }
}
#[test]
fn real_ui_displays_rollback_offline_recovery_and_keeps_saved_intent() {
    let mut gui = Gui::new(96000);
    gui.open();
    gui.preview();
    gui.controls.failures.lock().extend([true, false]);
    gui.apply();
    assert_eq!(gui.app.engine.sr(), 48000);
    assert!(gui.app.settings.pending_restart());
    assert!(gui
        .app
        .audio_settings
        .message
        .contains("Previous output restored"));
    gui.controls.failures.lock().extend([true, true]);
    gui.apply();
    assert_eq!(
        gui.app
            .audio_settings
            .handle
            .as_ref()
            .unwrap()
            .status()
            .phase,
        owner::Phase::Offline
    );
    let captured = gui
        .app
        .engine
        .project
        .capture(&AtomicBool::new(false))
        .unwrap();
    assert!(!captured.media.is_empty());
    gui.preview();
    gui.apply();
    assert_eq!(gui.app.engine.sr(), 96000);
    assert!(!gui.app.settings.pending_restart());
}
#[test]
fn real_ui_calibration_qualifies_identity_and_clears_prior_result_on_cancel_or_missing_loopback() {
    let mut gui = Gui::new(48000);
    gui.open();
    gui.preview();
    gui.measure();
    let handle = gui.app.audio_settings.handle.clone().unwrap();
    let measured = handle.status();
    assert_eq!(
        measured.measurement.as_ref().unwrap().identity.profile,
        "Studio"
    );
    assert_eq!(gui.app.engine.sr(), 48000);
    gui.controls
        .calibration_block
        .store(true, Ordering::Release);
    gui.click("Measure loopback");
    gui.click("Cable ready: stop and measure");
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while !gui.controls.entering_calibration.load(Ordering::Acquire) {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(handle.status().measurement.is_none());
    gui.click("Cancel audio operation");
    gui.wait();
    assert!(handle.status().measurement.is_none());
    assert!(gui.app.audio_settings.message.contains("cancelled"));
    gui.controls
        .calibration_block
        .store(false, Ordering::Release);
    for mode in [1, 2] {
        gui.controls.calibration_mode.store(mode, Ordering::Release);
        gui.measure();
        assert!(handle.status().measurement.is_none());
        assert_eq!(handle.status().phase, owner::Phase::Running);
    }
}
#[test]
fn saved_changes_invalidate_preview_and_invalid_devices_cannot_reach_confirmation() {
    let mut gui = Gui::new(96000);
    gui.open();
    gui.preview();
    gui.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .audio
        .device = Some("Missing".into());
    gui.frame(vec![]);
    assert!(gui.app.audio_settings.preview.is_none());
    gui.click("Preview saved audio");
    gui.wait();
    assert!(gui
        .app
        .audio_settings
        .preview
        .as_ref()
        .unwrap()
        .output
        .is_err());
    let node = gui
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Use saved audio now"))
        .unwrap();
    assert!(node.1.is_disabled());
    assert_eq!(gui.controls.opens.load(Ordering::Acquire), 1);
}

#[test]
fn capability_selectors_edit_the_real_preference_draft_and_save_exact_format() {
    let mut gui = Gui::new(48000);
    gui.open();
    gui.preview();
    gui.app.audio_settings.open = false;
    gui.frame(vec![]);
    gui.app.settings.worker = Some(
        crate::preferences::worker::Worker::with_discovery(
            gui.dir.join("preferences.json"),
            || Ok(inventory()),
        )
        .unwrap(),
    );
    gui.click("Output sample format");
    gui.click("f32");
    assert_eq!(
        gui.app.settings.draft.profiles["Studio"].audio.format,
        Some(AudioFormat::F32)
    );
    gui.click("Sample rate Hz");
    gui.click("192000");
    assert_eq!(
        gui.app.settings.draft.profiles["Studio"].audio.sample_rate,
        Some(192000)
    );
    gui.click("Preview changes");
    let end = Instant::now() + std::time::Duration::from_secs(3);
    while gui.app.settings.busy() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    gui.frame(vec![]);
    gui.click("Apply and save");
    while gui.app.settings.busy() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    gui.frame(vec![]);
    let saved = crate::preferences::storage::load(
        &gui.dir.join("preferences.json"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        saved.preferences.current().unwrap().audio.format,
        Some(AudioFormat::F32)
    );
    assert_eq!(
        saved.preferences.current().unwrap().audio.sample_rate,
        Some(192000)
    );
    assert_eq!(
        gui.app.engine.sr(),
        48000,
        "Saving did not silently switch the live stream"
    );
}

#[test]
fn closed_audio_window_still_surfaces_pending_and_failed_operations() {
    let mut gui = Gui::new(96000);
    gui.open();
    gui.preview();
    gui.controls.block_open.store(true, Ordering::Release);
    gui.click("Use saved audio now");
    gui.click("Stop and change output");
    gui.app.audio_settings.open = false;
    gui.frame(vec![]);
    assert!(gui
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Audio operation pending")));
    gui.controls.failures.lock().extend([true, true]);
    gui.controls.block_open.store(false, Ordering::Release);
    gui.wait();
    assert!(gui
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Audio offline")));
    gui.click("Audio offline");
    assert!(gui.app.audio_settings.open);
    assert!(gui.app.audio_settings.message.contains("Session retained"));
}
