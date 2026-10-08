use super::*;
use crate::engine::{Command, ComposeTarget, Engine, SamplerInstrument, SynthInstrument};
use std::{path::Path, time::{Duration, Instant}};

fn wait(seconds: u64, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !ready() {
        assert!(Instant::now() < deadline, "Physical recovery action or application acknowledgment timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn marker(directory: &Path, action: &str, model: &str, trial: usize, key: &str) {
    std::fs::write(directory.join("action.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "action":action,"model":model,"trial":trial,"instance":key,"pid":std::process::id(),
    })).unwrap()).unwrap();
}

fn save(directory: &Path, engine: &Engine, trial: usize) -> serde_json::Value {
    let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
    let notes = captured.state.tracks[4].clips[3].notes.clone();
    assert_eq!(notes.len(), trial + 1);
    assert!(notes.iter().all(|note| note.len > 0.0));
    assert_eq!(captured.state.master, 0.0);
    let path = directory.join(format!("retained-{trial}.omatainer"));
    let document = crate::ui::project::Document { engine: captured.state, view: Default::default(),
        mapping_schema: crate::ui::project::FACTORY_MAPPING_SCHEMA };
    document.validate().unwrap();
    let metadata = serde_json::to_vec(&document).unwrap();
    crate::project_file::save(&path, &crate::project_file::Bundle { state: document, media: captured.media },
        crate::project_file::Overwrite::Never, &crate::project_file::Limits::default(), &AtomicBool::new(false)).unwrap();
    let reopened = crate::project_file::load::<crate::ui::project::Document>(&path, &crate::project_file::Limits::default(), &AtomicBool::new(false)).unwrap();
    reopened.state.validate().unwrap();
    assert_eq!(serde_json::to_vec(&reopened.state).unwrap(), metadata);
    serde_json::json!({"path":path,"notes":notes.len(),"metadata_bytes":metadata.len(),"native_document_reopened":true})
}

#[test]
#[ignore = "Requires exclusive powered original NS7 and APC40 MkII, project-owned XDG directories and OMATAINER_PHYSICAL_RECOVERY_DIR; waits for two physical unplug/replug rounds per device; master output is silent"]
fn native_physical_usb_recording_recovery_and_two_controller_reconnections() {
    use owner::{finish_shutdown, Phase};
    let directory = std::path::PathBuf::from(std::env::var_os("OMATAINER_PHYSICAL_RECOVERY_DIR").unwrap());
    assert!(directory.is_absolute() && directory.starts_with(env!("CARGO_MANIFEST_DIR")));
    for name in ["XDG_CONFIG_HOME", "XDG_STATE_HOME", "TMPDIR"] {
        assert!(std::path::PathBuf::from(std::env::var_os(name).unwrap()).starts_with(&directory));
    }
    std::fs::create_dir_all(&directory).unwrap();
    let (mut engine, mut rt) = Engine::headless_for_test(44100, 256);
    for command in [Command::Master(0.0), Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys))] {
        engine.cmd.send(command).unwrap();
    }
    rt.process(&mut [0.0; 2048]);
    let settings = crate::preferences::Audio { backend: Some("ALSA".into()), device: Some("hw:CARD=NS7,DEV=0".into()),
        sample_rate: Some(44100), channels: Some(4), format: Some(crate::preferences::AudioFormat::I32), buffer_frames: Some(512), ..Default::default() };
    let audio = start_with_settings(rt, &settings).unwrap();
    engine.midi = crate::engine::midi::MidiHub::start(engine.cmd.clone(), engine.snap.clone()).unwrap();
    let registry = engine.midi.profiles().unwrap().clone();
    wait(30, || { let view = registry.view(); view.ready && !view.busy && !engine.cmd.performance().status().changing
        && engine.midi.policy_status().is_some_and(|policy| !policy.pending()) && view.devices.iter().filter(|d| d.input_open).count() == 4
        && view.devices.iter().filter(|d| d.output_open).count() == 4 });
    let initial = registry.view();
    let ns7 = initial.devices.iter().find(|d| d.device.vendor == 0x15e4 && d.device.product == 0x0071 && d.device.port == 0 && d.input_open).unwrap().clone();
    let apc = initial.devices.iter().find(|d| d.device.vendor == 0x09e8 && d.device.product == 0x0029 && d.device.port == 0 && d.input_open).unwrap().clone();
    let accepted = audio.handle.status().active.as_ref().unwrap().plan.clone();
    assert!(recovery::identity(&accepted.device).is_none());
    let usb = std::path::PathBuf::from("/sys/bus/usb/devices").join(&ns7.device.topology);
    assert!(usb.exists());
    let mut trials = Vec::new();
    for trial in 0..2 {
        for command in [Command::ComposeArm { track: 4, scene: 3 }, Command::Play, Command::SamplerPad { pad: 0, on: true }] {
            engine.cmd.send(command).unwrap();
        }
        wait(5, || { let snapshot = engine.snapshot(); snapshot.playing && snapshot.compose_target == Some(ComposeTarget { track: 4, scene: 3 }) });
        let before = engine.midi.input_stats();
        let key = ns7.device.key();
        marker(&directory, "unplug", "NS7", trial, &key);
        wait(300, || !usb.exists());
        wait(15, || audio.handle.status().phase == Phase::Offline && !registry.view().devices.iter().any(|d| d.device.key() == key));
        let offline = audio.handle.status();
        assert!(offline.recovery.as_ref().unwrap().identity.is_none());
        assert!(engine.cmd.send(Command::Play).is_err());
        assert!(!engine.snapshot().playing && engine.snapshot().compose_target.is_none());
        let archive = save(&directory, &engine, trial);
        let missing_reconnect = audio.handle.reconnect_permitted(Arc::new(AtomicBool::new(false)),
            audio.handle.performance_permit().unwrap(), audio.handle.status().generation);
        assert!(missing_reconnect.is_err());
        assert_eq!(audio.handle.status().phase, Phase::Offline);
        marker(&directory, "replug", "NS7", trial, &key);
        wait(300, || usb.exists());
        wait(30, || registry.view().devices.iter().any(|d| d.device.key() == key && d.input_open && d.output_open));
        let connected = registry.view();
        let found = connected.devices.iter().find(|d| d.device.key() == key).unwrap();
        assert_ne!(found.device.connection, ns7.device.connection);
        assert_eq!(connected.devices.iter().filter(|d| d.input_open).count(), 4);
        assert_eq!(connected.devices.iter().filter(|d| d.output_open).count(), 4);
        assert_eq!(audio.handle.status().phase, Phase::Offline);
        let refused = audio.handle.reconnect_permitted(Arc::new(AtomicBool::new(false)), audio.handle.performance_permit().unwrap(), audio.handle.status().generation).unwrap_err();
        assert!(refused.contains("no verifiable physical identity"));
        assert_eq!(audio.handle.status().phase, Phase::Offline);
        let (_, reviewed) = config::select(&settings).unwrap();
        let restored = audio.handle.apply_preview(settings.clone(), reviewed, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(restored.phase, Phase::Running);
        let plan = &restored.active.as_ref().unwrap().plan;
        assert_eq!((plan.rate, plan.channels, plan.format, plan.buffer), (accepted.rate, accepted.channels, accepted.format, accepted.buffer));
        assert!(recovery::identity(&plan.device).is_none());
        assert!(engine.cmd.send(Command::Play).is_err());
        engine.cmd.performance().acknowledge_inputs_released().unwrap();
        wait(5, || !engine.cmd.performance().status().recovery);
        assert!(!engine.snapshot().playing);
        let after = engine.midi.input_stats();
        assert!(after.disconnected > before.disconnected && after.resets > before.resets);
        trials.push(serde_json::json!({"model":"NS7","trial":trial,"physical_identity":key,"old_usb_connection":ns7.device.connection,
            "new_usb_connection":found.device.connection,"output_identity":null,"route_preserved":true,"archive":archive,
            "missing_reconnect_refused":true,"placeholder_serial_reconnect_refused":true,"explicit_reviewed_fallback":true,"automatic_playback":false,"release_acknowledgment_required":true,
            "before_input":before,"after_input":after,"audio":engine.cmd.audio_metrics()}));
    }
    engine.cmd.send(Command::Play).unwrap();
    wait(5, || engine.snapshot().playing);
    for trial in 0..2 {
        let key = apc.device.key();
        let before = engine.midi.input_stats();
        marker(&directory, "unplug", "APC40 MkII", trial, &key);
        wait(300, || !registry.view().devices.iter().any(|d| d.device.key() == key));
        assert!(engine.snapshot().playing);
        assert_eq!(audio.handle.status().phase, Phase::Running);
        marker(&directory, "replug", "APC40 MkII", trial, &key);
        wait(300, || registry.view().devices.iter().any(|d| d.device.key() == key && d.input_open && d.output_open));
        let connected = registry.view();
        let found = connected.devices.iter().find(|d| d.device.key() == key).unwrap();
        assert_ne!(found.device.connection, apc.device.connection);
        assert_eq!(connected.devices.iter().filter(|d| d.input_open).count(), 4);
        assert_eq!(connected.devices.iter().filter(|d| d.output_open).count(), 4);
        assert!(engine.snapshot().playing);
        assert_eq!(audio.handle.status().phase, Phase::Running);
        let after = engine.midi.input_stats();
        assert!(after.disconnected > before.disconnected && after.resets > before.resets);
        trials.push(serde_json::json!({"model":"APC40 MkII","trial":trial,"physical_identity":key,"old_usb_connection":apc.device.connection,
            "new_usb_connection":found.device.connection,"playback_continued":true,"before_input":before,"after_input":after,"audio":engine.cmd.audio_metrics()}));
    }
    engine.cmd.send(Command::Stop).unwrap();
    wait(5, || !engine.snapshot().playing);
    std::fs::write(directory.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"status":"pass","physical_usb_actions":true,"trials":trials,"silent_master":true,
        "boundary":"Actual native audio and MIDI owners; command-initiated held MIDI notes, two physical removals per owned model and native saved-document reopen. NS7 has a placeholder serial: retained identity reconnect is refused; the fixture explicitly reviews and applies the returned NS7 output. MIDI pins are topology-qualified. No unique-unit serial, physical note-button holding, native mouse confirmation, audible performance, host suspend or stage-duration claim.",
    })).unwrap()).unwrap();
    marker(&directory, "complete", "NS7 and APC40 MkII", 2, "");
    drop(audio);
    finish_shutdown();
}
