use super::*;
use crate::engine::{
    dsp::{InputKey, Sample},
    load_receipt, test_alloc, Engine, MidiNote, RtEngine,
};
fn fixture() -> (Engine, RtEngine) {
    Engine::headless_for_test(48_000, 256)
}
fn tick(rt: &mut RtEngine) {
    rt.process(&mut [0.0; 128]);
}
#[test]
fn protected_loads_are_checked_again_on_renderer_without_dropping_payloads() {
    let (engine, mut rt) = fixture();
    let original = rt.decks[0].audio.clone().unwrap();
    let audio = Arc::new(Sample {
        name: "queued replacement".into(),
        path: String::new(),
        sr: 48_000,
        ch: 2,
        data: vec![0.5; 8192],
        peaks: Arc::new(Vec::new()),
        bpm: 120.0,
    });
    let weak = Arc::downgrade(&audio);
    // Accepted before protection, but the preceding queued Play makes it unsafe
    // by the time the renderer reaches the replacement.
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    engine.send(Command::DeckAudio { deck: 0, audio }).unwrap();
    engine.send(Command::PerformanceMode(true)).unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(matches!(
        engine.send(Command::LoadBuiltin { deck: 0, stem: 1 }),
        Err(crate::engine::SubmissionError::Performance(
            Error::PlayingDeck
        ))
    ));
    assert!(matches!(
        engine.send(Command::SetNotes {
            track: 2,
            scene: 0,
            notes: Vec::new()
        }),
        Err(crate::engine::SubmissionError::Performance(
            Error::Protected
        ))
    ));
    engine.send(Command::Master(0.7)).unwrap();
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    tick(&mut rt);
    let receipt = load_receipt::Receipt::new();
    engine
        .send(Command::DeckLoadRequested {
            deck: 0,
            media: load_receipt::Media::Builtin(1),
            receipt: receipt.clone(),
        })
        .unwrap();
    tick(&mut rt);
    assert_eq!(receipt.state(), load_receipt::State::Current);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while weak.strong_count() != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn safe_stop_finalizes_capture_releases_all_owners_and_rejects_queued_onsets() {
    let (engine, mut rt) = fixture();
    rt.tracks[2].clips[7].notes = vec![MidiNote {
        pitch: 50,
        start: 0.0,
        len: 0.5,
        vel: 80,
    }];
    engine
        .send(Command::ComposeArm { track: 2, scene: 7 })
        .unwrap();
    engine.send(Command::Play).unwrap();
    engine.send(Command::Record).unwrap();
    engine
        .send(Command::LiveNoteOn {
            source: 2,
            ch: 1,
            note: 63,
            vel: 90,
        })
        .unwrap();
    tick(&mut rt);
    rt.process(&mut [0.0; 4800]);
    assert!(rt.has_held_project_notes());
    for source in 10..100 {
        engine
            .send(Command::LiveNoteOn {
                source,
                ch: 0,
                note: 68,
                vel: 100,
            })
            .unwrap();
    }
    engine.send(Command::SafetyStop(Safety::Stop)).unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(!rt.playing && !rt.recording && !rt.has_held_project_notes());
    assert!(rt.tracks.iter().all(|track| track
        .poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3))));
    assert!(rt.tracks[2].clips[7].notes.last().unwrap().len > 0.09);
    assert!(!engine.cmd.performance().status().output_muted);
    engine.send(Command::RecoverPerformance).unwrap();
    tick(&mut rt);
    assert!(
        engine.cmd.performance().status().recovery,
        "backlog must drain before reopening"
    );
    tick(&mut rt);
    assert!(!engine.cmd.performance().status().recovery);
    assert!(!rt.playing && rt.decks.iter().all(|d| !d.playing));
    engine
        .send(Command::LiveNoteOn {
            source: 2,
            ch: 1,
            note: 63,
            vel: 90,
        })
        .unwrap();
    tick(&mut rt);
    assert!(rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 2,
            ch: 1,
            note: 63
        })
        && matches!(v.env.stage, 1..=3)));
}
#[test]
fn emergency_output_ramps_then_stays_muted_after_explicit_ack_and_quiet_observation() {
    let (engine, mut rt) = fixture();
    rt.decks[0].playing = true;
    tick(&mut rt);
    engine.send(Command::SafetyStop(Safety::Silence)).unwrap();
    let mut block = [0.0; 256];
    rt.process(&mut block);
    assert!(block[192..].iter().all(|sample| *sample == 0.0));
    engine.send(Command::RecoverPerformance).unwrap();
    tick(&mut rt);
    assert!(!engine.cmd.performance().status().recovery);
    assert!(engine.cmd.performance().status().output_muted);
    // Even after the bounded observation completes, mute cannot self-clear.
    for _ in 0..760 {
        rt.process(&mut block);
    }
    let status = engine.cmd.performance().status();
    assert!(status.observation_complete && status.output_muted);
    assert!(block.iter().all(|sample| *sample == 0.0));
    let mut output = Output::default();
    output.stop(Safety::Silence, 48_000.0);
    output.observe([f32::NAN, 0.0], 48_000.0);
    assert!(output.nonfinite && output.quiet == 0);
}
#[test]
fn exclusive_jobs_and_optional_cancellation_close_mode_entry_races() {
    let handle = Handle::default();
    let permit = handle.audio_change().unwrap();
    assert!(permit.valid_for_audio(&handle));
    assert_eq!(handle.set_enabled(true), Err(Error::Changing));
    drop(permit);
    let work = handle.optional_work().unwrap();
    let cancel = work.cancel();
    handle.set_enabled(true).unwrap();
    assert!(cancel.load(Ordering::Acquire));
    assert!(matches!(handle.optional_work(), Err(Error::Protected)));
    assert!(matches!(handle.project_change(), Err(Error::Protected)));
    assert!(matches!(handle.audio_change(), Err(Error::Protected)));
    handle.request_safety(Safety::Stop);
    let request = handle.safety_request().unwrap().0;
    handle.stopped(request);
    let permit = handle.audio_change().unwrap();
    assert_eq!(handle.acknowledge_inputs_released(), Err(Error::Changing));
    drop(permit);
    handle.acknowledge_inputs_released().unwrap();
    let writer = handle.writer();
    assert!(!handle.try_recover(|| true));
    drop(writer);
    assert!(handle.try_recover(|| true));
    assert!(handle.status().protected);
}

#[test]
fn safety_mailbox_bypasses_full_owned_queue_and_retires_rejected_payloads_off_audio() {
    let (engine, mut rt) = fixture();
    let original = rt.decks[0].audio.clone().unwrap();
    let replacement = Arc::new(Sample { name: "rejected owned media".into(), path: String::new(), sr: 48_000, ch: 2, data: vec![0.5; 65_536], peaks: Arc::new(vec![[0.5; 3]; 512]), bpm: 120.0 });
    let weak = Arc::downgrade(&replacement);
    engine.send(Command::DeckAudio { deck: 0, audio: replacement }).unwrap();
    engine.send(Command::SetNotes { track: 2, scene: 7, notes: vec![MidiNote { pitch: 62, start: 0.0, len: 1.0, vel: 80 }; 128] }).unwrap();
    engine.send(Command::Gesture { id: 24, command: Box::new(Command::Master(0.1)) }).unwrap();
    while engine.send(Command::Tap(std::time::Instant::now())).is_ok() {}
    assert!(engine.cmd.len() > crate::engine::control::COMMANDS_PER_BLOCK);
    engine.send(Command::SafetyStop(Safety::Silence)).unwrap();
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 256]));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(engine.cmd.performance().status().stopped);
    assert!(engine.cmd.performance().status().output_muted);
    assert!(Arc::ptr_eq(&original, rt.decks[0].audio.as_ref().unwrap()));
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    while !rt.cmd_rx.is_empty() {
        let counts = test_alloc::measure(|| rt.process(&mut [0.0; 256]));
        assert_eq!((counts.allocations, counts.frees), (0, 0));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while weak.strong_count() != 0 { assert!(std::time::Instant::now() < deadline); std::thread::yield_now(); }
}

#[test]
fn optional_publication_rechecks_cancel_after_commit_claim() {
    let handle = Handle::default();
    let work = handle.optional_work().unwrap();
    let result = work.commit_after_check(|| {
        handle.set_enabled(true).unwrap();
        handle.set_enabled(false).unwrap();
    });
    assert!(matches!(result, Err(Error::Protected)));
    assert!(!handle.status().changing);
    assert!(matches!(work.commit(), Err(Error::Protected)));
}

#[test]
fn new_safety_request_cannot_reuse_old_recovery_cas_state() {
    let handle = Handle::default();
    handle.request_safety(Safety::Stop);
    handle.stopped(handle.safety_request().unwrap().0);
    handle.acknowledge_inputs_released().unwrap();
    assert!(!handle.try_recover(|| {
        handle.request_safety(Safety::Silence);
        handle.stopped(handle.safety_request().unwrap().0);
        handle.acknowledge_inputs_released().unwrap();
        true
    }));
    assert!(handle.status().recovery);
    assert!(handle.try_recover(|| true));
}

#[test]
fn protection_generation_rejects_publication_before_cancellation_flags_propagate() {
    let handle = Handle::default();
    let work = handle.optional_work().unwrap();
    // Pause mode entry after its admission-word linearization, before walking
    // the fixed cancellation slots; another operator can already leave mode.
    handle.0.admission.fetch_add(SAFETY_GENERATION, Ordering::AcqRel);
    assert!(!work.cancel().load(Ordering::Acquire));
    assert!(matches!(work.commit(), Err(Error::Protected)));
    assert!(!handle.status().changing);
}

#[test]
fn newer_safety_request_cannot_receive_an_older_stopped_publication_or_early_ack() {
    let handle = Handle::default();
    handle.request_safety(Safety::Stop);
    let old = handle.safety_request().unwrap().0;
    handle.stopped_after_check(old, || handle.request_safety(Safety::Silence));
    assert!(!handle.status().stopped);
    assert_eq!(handle.acknowledge_inputs_released(), Err(Error::PendingStop));
    let new = handle.safety_request().unwrap().0;
    assert_ne!(new, old);
    handle.stopped(new);
    assert!(!handle.try_recover(|| true), "new stop needs its own deliberate acknowledgment");
    handle.acknowledge_inputs_released().unwrap();
    assert!(handle.try_recover(|| true));
}
