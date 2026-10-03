use super::*;
use std::time::Duration;
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
fn native_reconnect_confirmation_cancel_and_resume_keep_saved_settings_and_finalize_held_input() {
    let mut gui=Gui::new(48000);gui.open();
    let saved=std::fs::read(gui.dir.join("preferences.json")).unwrap();
    let handle=gui.app.audio_settings.handle.clone().unwrap();
    gui.app.engine.cmd.send(Command::Play).unwrap();
    gui.controls.active_fault.lock().as_ref().unwrap().store(true,Ordering::Release);
    let deadline=Instant::now()+Duration::from_secs(3);
    while handle.status().phase!=owner::Phase::Offline {
        gui.frame(vec![]);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![]);let opens=gui.controls.opens.load(Ordering::Acquire);
    gui.click("Reconnect retained output…");gui.click("Keep current audio");
    assert_eq!(gui.controls.opens.load(Ordering::Acquire),opens);
    gui.controls.device_renamed.store(true,Ordering::Release);
    gui.click("Reconnect retained output…");gui.click("Confirm retained output reconnect");gui.wait();
    assert_eq!(handle.status().phase,owner::Phase::Running);
    assert_eq!(handle.status().active.as_ref().unwrap().plan.device,"Renumbered fixture");
    assert!(!gui.app.engine.snapshot().playing);
    assert!(gui.app.engine.cmd.performance().status().recovery);
    assert!(gui.app.engine.cmd.send(Command::Play).is_err());
    gui.app.audio_settings.open=false;gui.app.settings.open=false;gui.frame(vec![]);
    gui.click("Recover inputs…");gui.click("I have released the physical inputs");
    gui.click("Inputs released — keep playback stopped");
    let deadline=Instant::now()+Duration::from_secs(3);
    while gui.app.engine.cmd.performance().status().recovery {gui.frame(vec![]);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
    gui.app.engine.cmd.send(Command::Play).unwrap();
    assert_eq!(std::fs::read(gui.dir.join("preferences.json")).unwrap(),saved);
}
#[test]
fn native_reconnect_failure_and_stale_consent_never_open_a_different_output() {
    let mut gui=Gui::new(48000);gui.open();let handle=gui.app.audio_settings.handle.clone().unwrap();
    gui.controls.active_fault.lock().as_ref().unwrap().store(true,Ordering::Release);
    let deadline=Instant::now()+Duration::from_secs(3);
    while handle.status().phase!=owner::Phase::Offline {gui.frame(vec![]);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
    gui.frame(vec![]);let opens=gui.controls.opens.load(Ordering::Acquire);
    gui.controls.alternate_device.store(true,Ordering::Release);
    gui.click("Reconnect retained output…");gui.click("Confirm retained output reconnect");gui.wait();
    assert_eq!(handle.status().phase,owner::Phase::Offline);
    assert_eq!(gui.controls.opens.load(Ordering::Acquire),opens);
    assert!(gui.app.audio_settings.message.contains("unavailable"));
    gui.click("Reconnect retained output…");
    owner::tests::publish_phase_for_ui_test(&handle,owner::Phase::Running);gui.frame(vec![]);
    assert!(gui.app.audio_settings.confirm.is_none());
    assert_eq!(gui.controls.opens.load(Ordering::Acquire),opens);
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
    described(&gui.nodes, "Cancel audio operation", HelpControl::AudioCancel);
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
fn exposed_cancel_action_survives_async_audio_observation_layout_changes() {
    let mut gui = Gui::new(48000);
    gui.open();
    gui.preview();
    gui.controls.calibration_block.store(true, Ordering::Release);
    gui.click("Measure loopback");
    gui.click("Cable ready: stop and measure");
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while !gui.controls.entering_calibration.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let handle = gui.app.audio_settings.handle.clone().unwrap();
    // Reproduce the actual Running -> Calibrating publication between exposing
    // a native action and consuming it. The real owner remains blocked inside
    // calibration until the real UI cancellation token is set.
    owner::tests::publish_phase_for_ui_test(&handle, owner::Phase::Running);
    gui.frame(vec![]);
    let cancel = gui.app.audio_settings.worker.as_ref().unwrap().cancel.clone().unwrap();
    owner::tests::publish_phase_for_ui_test(&handle, owner::Phase::Calibrating);
    gui.click("Cancel audio operation");
    assert!(cancel.load(Ordering::Acquire), "exposed Cancel action was lost when asynchronous status changed the preceding layout");
    gui.wait();
    assert!(!gui.app.audio_settings.busy());
    assert!(gui.app.audio_settings.message.contains("cancelled"));
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
    described(&gui.nodes, "f32", HelpControl::AudioOutputFormat);
    gui.click("f32");
    assert_eq!(
        gui.app.settings.draft.profiles["Studio"].audio.format,
        Some(AudioFormat::F32)
    );
    gui.click("Sample rate Hz");
    described(&gui.nodes, "192000", HelpControl::PreferenceAudioRate);
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

fn described(nodes: &[(NodeId, Node)], name: &str, control: HelpControl) {
    let node = nodes.iter().find(|(_, n)| n.label() == Some(name))
        .unwrap_or_else(|| panic!("missing {name}"));
    let description = node.1.description().unwrap_or_default();
    let definition = control.definition();
    assert!(description.contains(definition.title), "{name}: {description}");
    assert!(description.contains(definition.units), "{name}: {description}");
    assert!(description.contains(definition.purpose), "{name}: {description}");
}

#[test]
fn actual_audio_confirmations_expose_canonical_help_without_starting_an_operation() {
    let mut gui = Gui::new(48000);
    gui.click("Preferences");
    described(&gui.nodes, "Audio devices and latency", HelpControl::AudioDevices);
    gui.click("Audio devices and latency");
    described(&gui.nodes, "Preview saved audio", HelpControl::AudioPreview);
    gui.preview();
    for (name, control) in [
        ("Use saved audio now", HelpControl::AudioUse),
        ("Measure loopback", HelpControl::AudioMeasure),
        ("Advertised input and output capabilities", HelpControl::AudioCapabilities),
        ("Dismiss audio notice", HelpControl::AudioNotice),
    ] { described(&gui.nodes, name, control); }
    gui.click("Use saved audio now");
    described(&gui.nodes, "Stop and change output", HelpControl::AudioConfirm);
    described(&gui.nodes, "Keep current audio", HelpControl::AudioKeep);
    gui.click("Keep current audio");
    gui.click("Measure loopback");
    described(&gui.nodes, "Cable ready: stop and measure", HelpControl::AudioProbeConfirm);
    // Focus + real F1 handling must display the exact confirmation's context.
    let target = gui.nodes.iter().find(|(_, n)| n.label() == Some("Cable ready: stop and measure")).unwrap().0;
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Focus, data: None })]);
    gui.frame(vec![egui::Event::Key { key: Key::F1, physical_key: Some(Key::F1), pressed: true, repeat: false, modifiers: Default::default() }]);
    gui.frame(vec![]);
    assert!(gui.app.keys_open);
    assert!(gui.nodes.iter().any(|(_, n)| n.value() == Some(HelpControl::AudioProbeConfirm.definition().purpose)
        || n.label() == Some(HelpControl::AudioProbeConfirm.definition().purpose)));
    assert_eq!(gui.controls.opens.load(Ordering::Acquire), 1, "reading help never starts a device change or probe");
    assert!(!gui.controls.entering_calibration.load(Ordering::Acquire));
}

#[test]
fn every_audio_preference_editor_has_explicit_units_and_saved_intent_description() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut audio = Audio::default();
    let original = audio.clone();
    let inventory = inventory();
    let mut nodes = Vec::new();
    for _ in 0..2 {
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1800.0))),
            ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| edit_profile(ui, &mut audio, Some(&inventory))); });
        nodes = output.platform_output.accesskit_update.unwrap().nodes;
    }
    for (name, control) in [
        ("Audio backend", HelpControl::AudioBackend),
        ("Output device", HelpControl::PreferenceAudioDevice),
        ("Sample rate Hz", HelpControl::PreferenceAudioRate),
        ("Output channels", HelpControl::PreferenceAudioChannels),
        ("Output sample format", HelpControl::AudioOutputFormat),
        ("Output buffer frames", HelpControl::PreferenceAudioBuffer),
        ("Calibration input device", HelpControl::AudioInputDevice),
        ("Input channels", HelpControl::AudioInputChannels),
        ("Input sample format", HelpControl::AudioInputFormat),
        ("Input buffer frames", HelpControl::AudioInputBuffer),
        ("Calibration input channel", HelpControl::AudioInputChannel),
        ("Probe output channel", HelpControl::AudioOutputChannel),
        ("Probe level", HelpControl::AudioProbeLevel),
    ] { described(&nodes, name, control); }
    assert_eq!(audio, original);
}


#[test]
fn retained_route_confirmation_cannot_apply_a_refreshed_default_device() {
    for (begin, confirm, calibrate) in [
        ("Use saved audio now", "Stop and change output", false),
        ("Measure loopback", "Cable ready: stop and measure", true),
    ] {
        let mut gui = Gui::new(48000);
        let controls = gui.controls.clone();
        gui.app.audio_settings = Panel::with_discovery(gui.app.engine.audio_handle(), move || {
            let mut devices = inventory();
            if controls.alternate_device.load(Ordering::Acquire) {
                devices.devices[0].name = "Alternate fixture".into();
                devices.inputs[0].name = "Alternate fixture".into();
            }
            Ok(devices)
        });
        let saved = gui.app.settings.profile().audio.clone();
        gui.open();
        gui.preview();
        gui.click(begin);
        let old_action = gui.nodes.iter().find_map(|(id, node)|
            (node.label() == Some(confirm) && node.supports_action(Action::Click)).then_some(*id)).unwrap();
        gui.controls.alternate_device.store(true, Ordering::Release);
        gui.preview();
        assert_eq!(gui.app.settings.profile().audio, saved);
        assert_eq!(gui.app.audio_settings.preview.as_ref().unwrap().output.as_ref().unwrap().device, "Alternate fixture");
        assert!(gui.app.audio_settings.confirm.is_none(), "refresh retained consent to the old route");
        // A fresh confirmation must not reuse the native ID exposed for the old
        // route. Clear-only fixes are insufficient for this delayed action.
        gui.click(begin);
        gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target: old_action, action: Action::Click, data: None,
        })]);
        assert!(!gui.app.audio_settings.busy(), "old route action admitted a different route");
        assert_eq!(gui.controls.opens.load(Ordering::Acquire), 1);
        assert!(!gui.controls.entering_calibration.load(Ordering::Acquire));
        assert!(gui.app.audio_settings.confirm.is_some());
        gui.click(confirm);
        gui.wait();
        let status = gui.app.engine.audio_handle().unwrap().status();
        if calibrate {
            assert_eq!(status.phase,owner::Phase::Offline);
            assert!(status.measurement.is_none());
            assert_eq!(status.recovery.as_ref().unwrap().plan.device,"Fixture");
            assert!(status.message.contains("could not reopen"));
        } else {
            assert_eq!(status.active.as_ref().unwrap().plan.device, "Alternate fixture");
        }
    }
}
