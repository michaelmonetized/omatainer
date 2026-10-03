use super::*;
use crate::engine::{Command, Engine, SamplerInstrument, SubmissionError, SynthInstrument};
use std::time::Instant;

fn wait(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < Duration::from_secs(8));
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn token() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn fault(audio: &AudioOut, controls: &tests::Controls) {
    controls
        .active_fault
        .lock()
        .as_ref()
        .unwrap()
        .store(true, Ordering::Release);
    wait(|| audio.handle.status().phase == Phase::Offline);
}
fn reconnect(handle: &Handle) -> Result<Arc<Status>, String> {
    handle.reconnect_permitted(
        token(),
        handle.performance_permit()?,
        handle.status().generation,
    )
}

#[test]
fn lost_output_finalizes_real_recording_and_preserves_project_route_and_input_recovery() {
    let (engine, audio, controls) = tests::fixture();
    for command in [
        Command::Master(0.37),
        Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)),
        Command::ComposeArm { track: 4, scene: 3 },
        Command::Play,
        Command::SamplerPad { pad: 0, on: true },
    ] {
        engine.cmd.send(command).unwrap();
    }
    wait(|| engine.snapshot().compose_target.is_some() && engine.snapshot().playing);
    engine.cmd.send(Command::PerformanceMode(true)).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let before = engine.project.capture(&AtomicBool::new(false)).unwrap();
    fault(&audio, &controls);
    let state = audio.handle.status();
    let target = state.recovery.as_ref().unwrap();
    assert_eq!(target.plan.rate, 48000);
    assert_eq!(target.identity.as_deref(), Some("unit-1"));
    assert!(engine.cmd.performance().status().stopped);
    assert!(engine.cmd.performance().status().recovery);
    assert!(matches!(
        engine.cmd.send(Command::Play),
        Err(SubmissionError::Performance(
            crate::engine::performance::Error::Recovery
        ))
    ));
    let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(captured.state.master, 0.37);
    assert_eq!(captured.media.len(), before.media.len());
    let notes = captured.state.tracks[4].clips[3].notes.clone();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].len > 0.0);
    let path =
        std::env::temp_dir().join(format!("omatainer-device-loss-{}.omat", std::process::id()));
    let bundle = crate::project_file::Bundle {
        state: captured.state,
        media: captured.media,
    };
    crate::project_file::save(
        &path,
        &bundle,
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.state.tracks[4].clips[3].notes, notes);
    std::fs::remove_file(path).unwrap();
    let restored = reconnect(&audio.handle).unwrap();
    assert_eq!(restored.phase, Phase::Running);
    wait(|| !engine.snapshot().playing && engine.snapshot().compose_target.is_none());
    assert!(engine.cmd.send(Command::Play).is_err());
    engine
        .cmd
        .performance()
        .acknowledge_inputs_released()
        .unwrap();
    wait(|| !engine.cmd.performance().status().recovery);
    engine.cmd.send(Command::Play).unwrap();
    wait(|| engine.snapshot().playing);
    let after = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(after.state.tracks[4].clips[3].notes, notes);
}

#[test]
fn recovery_survives_renumbering_but_refuses_a_different_default_and_never_uses_reset_as_fallback()
{
    let (engine, audio, controls) = tests::fixture();
    let mut settings = crate::preferences::Audio::default();
    settings.sample_rate = Some(96000);
    settings.channels = Some(4);
    settings.buffer_frames = Some(256);
    let plan = audio
        .handle
        .switch(settings, token())
        .unwrap()
        .active
        .as_ref()
        .unwrap()
        .plan
        .clone();
    fault(&audio, &controls);
    let opens = controls.opens.load(Ordering::Acquire);
    controls.alternate_device.store(true, Ordering::Release);
    assert!(reconnect(&audio.handle).is_err());
    assert_eq!(controls.opens.load(Ordering::Acquire), opens);
    assert!(audio
        .handle
        .reset_permitted(token(), audio.handle.performance_permit().unwrap())
        .is_err());
    assert_eq!(controls.opens.load(Ordering::Acquire), opens);
    controls.alternate_device.store(false, Ordering::Release);
    controls.device_renamed.store(true, Ordering::Release);
    let status = reconnect(&audio.handle).unwrap();
    let current = &status.active.as_ref().unwrap().plan;
    let mut expected = plan;
    expected.device = "Renumbered fixture".into();
    assert_eq!(current, &expected);
    assert_eq!(engine.sr(), 96000);
    assert!(!engine.snapshot().playing);
}

#[test]
fn canceled_or_failed_reconnect_retains_graph_and_target_for_later_retry_and_save() {
    let (engine, audio, controls) = tests::fixture();
    fault(&audio, &controls);
    controls.failures.lock().push_back(true);
    assert!(reconnect(&audio.handle).is_err());
    assert!(audio.handle.status().recovery.is_some());
    engine.project.capture(&AtomicBool::new(false)).unwrap();
    controls.block_open.store(true, Ordering::Release);
    let handle = audio.handle.clone();
    let cancel = token();
    let cancelled = cancel.clone();
    let operation = std::thread::spawn(move || {
        handle.reconnect_permitted(
            cancel,
            handle.performance_permit().unwrap(),
            handle.status().generation,
        )
    });
    wait(|| controls.entering_open.load(Ordering::Acquire));
    cancelled.store(true, Ordering::Release);
    controls.block_open.store(false, Ordering::Release);
    assert!(operation.join().unwrap().is_err());
    assert_eq!(audio.handle.status().phase, Phase::Offline);
    assert!(audio.handle.status().recovery.is_some());
    engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(reconnect(&audio.handle).unwrap().phase, Phase::Running);
}

#[test]
fn callback_stall_retires_the_actual_callback_without_a_backend_error_and_aliases_need_explicit_fallback(
) {
    let (engine, audio, controls) = tests::fixture();
    controls.stalled.store(true, Ordering::Release);
    wait(|| audio.handle.status().phase == Phase::Offline);
    assert_eq!(controls.dropped.load(Ordering::Acquire), 1);
    assert!(engine.cmd.performance().status().recovery);
    engine.project.capture(&AtomicBool::new(false)).unwrap();
    controls.stalled.store(false, Ordering::Release);
    assert_eq!(reconnect(&audio.handle).unwrap().phase, Phase::Running);
    engine
        .cmd
        .performance()
        .acknowledge_inputs_released()
        .unwrap();
    wait(|| !engine.cmd.performance().status().recovery);
    controls.identity_unavailable.store(true, Ordering::Release);
    audio
        .handle
        .switch(crate::preferences::Audio::default(), token())
        .unwrap();
    fault(&audio, &controls);
    let opens = controls.opens.load(Ordering::Acquire);
    assert!(reconnect(&audio.handle).is_err());
    assert_eq!(controls.opens.load(Ordering::Acquire), opens);
    audio
        .handle
        .switch(crate::preferences::Audio::default(), token())
        .unwrap();
    assert!(!engine.snapshot().playing);
}

#[test]
fn managed_transport_resume_ramps_exactly_two_milliseconds_without_callback_heap_work() {
    fn renderer() -> RtEngine {
        let (_, mut rt) = Engine::headless_for_test(48000, 256);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: Arc::new(crate::engine::dsp::Sample {
                name: "ramp".into(),
                sr: 48000,
                ch: 2,
                data: vec![0.25; 4000],
                peaks: vec![].into(),
                bpm: 120.0,
                path: String::new(),
            }),
        });
        rt.xfader = 0.0;
        rt
    }
    let mut reference = OutputCallback::new(renderer(), 2);
    let (send, receive) = bounded(1);
    let mut managed = OutputCallback::managed(
        Box::new(renderer()),
        2,
        send,
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let mut actual = [0.0f32; 512];
    let mut expected = [0.0f32; 512];
    managed.render(&mut actual);
    reference.render(&mut expected);
    managed.rt.decks[0].playing = true;
    reference.rt.decks[0].playing = true;
    reference.render(&mut expected);
    assert_eq!(
        crate::engine::test_alloc::measure(|| managed.render(&mut actual)),
        crate::engine::test_alloc::Counts::default()
    );
    assert!(expected.iter().any(|sample| sample.abs() > 0.01));
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let gain = (index / 2).min(96) as f32 / 96.0;
        assert!(
            (*actual - expected * gain).abs() < 1e-7,
            "frame {}",
            index / 2
        );
    }
    drop(managed);
    assert!(receive.recv().is_ok());
}

#[test]
fn retained_reconnect_keeps_emergency_mute_latched_until_deliberate_stopped_reset() {
    let (engine, audio, controls) = tests::fixture();
    engine
        .cmd
        .send(Command::SafetyStop(
            crate::engine::performance::Safety::Silence,
        ))
        .unwrap();
    wait(|| {
        engine.cmd.performance().status().output_muted && engine.cmd.performance().status().stopped
    });
    fault(&audio, &controls);
    reconnect(&audio.handle).unwrap();
    assert!(engine.cmd.performance().status().output_muted);
    engine
        .cmd
        .performance()
        .acknowledge_inputs_released()
        .unwrap();
    wait(|| !engine.cmd.performance().status().recovery);
    assert!(engine.cmd.performance().status().output_muted);
    audio
        .handle
        .reset_permitted(token(), audio.handle.performance_permit().unwrap())
        .unwrap();
    wait(|| !engine.cmd.performance().status().output_muted);
    assert!(!engine.snapshot().playing);
}

#[test]
#[ignore = "Requires scripts/check-audio-recovery.py and its private PipeWire server"]
fn private_native_pipewire_restart_retains_recorded_document_and_requires_explicit_fallback() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("OMATAINER_NATIVE_RECOVERY_DIR").expect("private fixture directory"),
    );
    assert_eq!(
        std::env::var_os("ALSA_CONFIG_PATH").unwrap(),
        dir.join("alsa.conf")
    );
    assert_eq!(
        std::env::var_os("XDG_RUNTIME_DIR").unwrap(),
        dir.join("runtime")
    );
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let settings = crate::preferences::Audio {
        sample_rate: Some(48000),
        channels: Some(2),
        format: Some(crate::preferences::AudioFormat::F32),
        buffer_frames: Some(128),
        ..Default::default()
    };
    let audio = super::super::start_with_settings(rt, &settings).unwrap();
    wait(|| engine.cmd.audio_metrics().callbacks >= 20);
    for command in [
        Command::Master(0.37),
        Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)),
        Command::ComposeArm { track: 4, scene: 3 },
        Command::Play,
        Command::SamplerPad { pad: 0, on: true },
    ] {
        engine.cmd.send(command).unwrap();
    }
    wait(|| engine.snapshot().playing && engine.snapshot().compose_target.is_some());
    engine.cmd.send(Command::PerformanceMode(true)).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    std::fs::write(dir.join("started.json"),serde_json::to_vec(&serde_json::json!({
        "callbacks":engine.cmd.audio_metrics().callbacks,"output":audio.handle.status().active.as_ref().unwrap().plan.device,
    })).unwrap()).unwrap();
    wait(|| audio.handle.status().phase == Phase::Offline);
    let status = audio.handle.status();
    assert!(status.recovery.as_ref().unwrap().identity.is_none());
    assert!(engine.cmd.performance().status().recovery);
    assert!(engine.cmd.performance().status().stopped);
    let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
    let notes = captured.state.tracks[4].clips[3].notes.clone();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].len > 0.0);
    let bundle = crate::project_file::Bundle {
        state: captured.state,
        media: captured.media,
    };
    let path = dir.join("retained.omat");
    crate::project_file::save(
        &path,
        &bundle,
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.state.tracks[4].clips[3].notes, notes);
    assert_eq!(reopened.state.master, 0.37);
    assert!(reconnect(&audio.handle).is_err());
    let metrics = engine.cmd.audio_metrics();
    std::fs::write(dir.join("offline.json"),serde_json::to_vec(&serde_json::json!({
        "callbacks":metrics.callbacks,"backend_errors":metrics.backend_errors,"device_lost":metrics.device_lost,
        "recorded_notes":notes.len(),"recorded_duration":notes[0].len,"message":status.message,"project_reopened":true,
    })).unwrap()).unwrap();
    wait(|| dir.join("restart.ready").is_file());
    assert_eq!(
        audio.handle.switch(settings, token()).unwrap().phase,
        Phase::Running
    );
    let before = engine.cmd.audio_metrics().callbacks;
    wait(|| engine.cmd.audio_metrics().callbacks >= before + 20);
    assert!(!engine.snapshot().playing);
    assert!(engine.cmd.send(Command::Play).is_err());
    engine
        .cmd
        .performance()
        .acknowledge_inputs_released()
        .unwrap();
    wait(|| !engine.cmd.performance().status().recovery);
    engine.cmd.send(Command::Play).unwrap();
    wait(|| engine.snapshot().playing);
    let after = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(after.state.tracks[4].clips[3].notes, notes);
    std::fs::write(dir.join("done.json"),serde_json::to_vec(&serde_json::json!({
        "callbacks":engine.cmd.audio_metrics().callbacks,"explicit_fallback":true,"explicit_input_acknowledgment":true,
        "explicit_play":true,"recorded_notes_retained":true,"scope":"real CPAL/ALSA plus private PipeWire null sink; no physical devices",
    })).unwrap()).unwrap();
}
