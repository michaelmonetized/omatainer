use super::*;
use crate::engine::{test_alloc, RtEngine, SamplerInstrument, Snapshot, SynthInstrument};
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};

#[test]
fn closing_gate_observes_a_producer_already_waiting_for_admission_without_taking_its_mutex() {
    let (port, receiver) = CommandPort::channel(32);
    let producer_mutex = port.admission.lock();
    let producer = port.clone();
    let worker = std::thread::spawn(move || producer.send(Command::Master(0.37)));
    // The held admission mutex gives a deterministic point after lease claim
    // and before queue publication, without adding any production test hook.
    let until = Instant::now() + Duration::from_secs(2);
    while port.shared.project_writers.load(Ordering::Acquire) != 1 {
        assert!(Instant::now() < until, "producer did not acquire its lease");
        std::thread::yield_now();
    }
    let counts = test_alloc::measure(|| {
        assert!(!receiver.begin_project_install());
        assert!(!receiver.begin_project_install());
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(receiver.is_empty());
    // Rejection happens before the still-held producer mutex and all GUI paths.
    for command in [
        Command::Master(0.9),
        Command::Browse(1.0),
        Command::DeckLoadSelected { deck: 0 },
    ] {
        assert_eq!(port.send(command), Err(SubmissionError::ProjectChanging));
    }
    drop(producer_mutex);
    assert_eq!(worker.join().unwrap(), Ok(SubmissionOutcome::Accepted));
    assert!(receiver.begin_project_install());
    assert!(matches!(receiver.try_recv(), Ok(Command::Master(value)) if value == 0.37));
    assert!(receiver.is_empty());
    assert_eq!(
        port.stats().last_error,
        Some(SubmissionError::ProjectChanging)
    );
    assert_eq!(
        serde_json::to_string(&SubmissionError::ProjectChanging).unwrap(),
        "\"project_changing\""
    );
    receiver.end_project_install();
    assert_eq!(
        port.send(Command::Master(0.9)),
        Ok(SubmissionOutcome::Accepted)
    );
    assert_eq!(port.shared.project_writers.load(Ordering::Acquire), 0);
}

fn lease_retired(port: &CommandPort) {
    assert_eq!(port.shared.project_writers.load(Ordering::Acquire), 0);
}

#[test]
fn every_existing_early_exit_retires_its_lease_and_gui_success_is_gated() {
    use crate::engine::media_source::{BuiltinStem, LibSource, Selection};
    let (port, receiver) = CommandPort::channel(32);
    assert_eq!(
        port.send(Command::Master(0.4)),
        Ok(SubmissionOutcome::Accepted)
    );
    lease_retired(&port);
    assert_eq!(
        port.send(Command::LiveNoteOff {
            source: 1,
            ch: 0,
            note: 60
        }),
        Ok(SubmissionOutcome::Coalesced)
    );
    lease_retired(&port);
    assert_eq!(
        port.send(Command::FxWet {
            slot: 255,
            value: 0.5
        }),
        Err(SubmissionError::InvalidTarget)
    );
    lease_retired(&port);
    assert_eq!(
        port.send(Command::DeckLoadSelected { deck: 0 }),
        Err(SubmissionError::UiUnavailable)
    );
    lease_retired(&port);
    let gui = port.take_ui_receiver().unwrap();
    assert_eq!(
        port.send(Command::DeckLoadSelected { deck: 0 }),
        Err(SubmissionError::UncapturedSelection)
    );
    lease_retired(&port);
    gui.publish_selection(Some(Arc::new(Selection {
        source: LibSource::Builtin(BuiltinStem::Harmony),
        title: "Harmony".into(),
    })));
    assert_eq!(
        port.send(Command::DeckLoadSelected { deck: 1 }),
        Ok(SubmissionOutcome::Accepted)
    );
    lease_retired(&port);
    while port.send(Command::DeckLoadSelected { deck: 1 }).is_ok() {
        lease_retired(&port);
    }
    assert_eq!(port.stats().last_error, Some(SubmissionError::UiFull));
    lease_retired(&port);
    assert!(receiver.begin_project_install());
    assert_eq!(
        port.send(Command::DeckLoadSelected { deck: 1 }),
        Err(SubmissionError::ProjectChanging)
    );
    receiver.end_project_install();
    assert_eq!(port.send(Command::Stop), Ok(SubmissionOutcome::Accepted));
    assert_eq!(port.send(Command::Play), Err(SubmissionError::StopPending));
    lease_retired(&port);
    while port.send(Command::Master(0.4)).is_ok() {
        lease_retired(&port);
    }
    assert_eq!(port.stats().last_error, Some(SubmissionError::Full));
    lease_retired(&port);
    drop(receiver);
    assert_eq!(
        port.send(Command::Master(0.4)),
        Err(SubmissionError::Disconnected)
    );
    lease_retired(&port);
}

#[test]
fn physical_releases_and_reserved_stops_survive_closed_gate_and_an_aborted_install() {
    let (port, receiver) = CommandPort::channel(64);
    let mut rt = RtEngine::new(
        48000.0,
        receiver,
        Arc::new(parking_lot::Mutex::new(Snapshot::default())),
    );
    rt.selected_track = 1;
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Keys,
    )));
    for command in [
        Command::LiveNoteOn {
            source: 7,
            ch: 0,
            note: 60,
            vel: 100,
        },
        Command::LiveNoteOn {
            source: 8,
            ch: 0,
            note: 62,
            vel: 100,
        },
        Command::SamplerPad { pad: 3, on: true },
        Command::DeckTouch { deck: 0, on: true },
        Command::MidiDeckTouch {
            source: 8,
            deck: 1,
            on: true,
        },
    ] {
        assert_eq!(port.send(command), Ok(SubmissionOutcome::Accepted));
    }
    rt.process(&mut [0.0; 32]);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| matches!(v.env.stage, 1..=3)));
    assert!(rt.decks.iter().all(|deck| deck.touching));
    assert!(rt.cmd_rx.begin_project_install());
    assert_eq!(
        port.send(Command::LiveNoteOn {
            source: 9,
            ch: 0,
            note: 64,
            vel: 100
        }),
        Err(SubmissionError::ProjectChanging)
    );
    for command in [
        Command::LiveNoteOff {
            source: 7,
            ch: 0,
            note: 60,
        },
        Command::LiveNoteOn {
            source: 8,
            ch: 0,
            note: 62,
            vel: 0,
        },
        Command::SamplerPad { pad: 3, on: false },
        Command::DeckTouch { deck: 0, on: false },
        Command::MidiDeckTouch {
            source: 8,
            deck: 1,
            on: false,
        },
        Command::StopTrack { track: 1 },
        Command::Stop,
    ] {
        assert_eq!(port.send(command), Ok(SubmissionOutcome::Accepted));
    }
    assert_eq!(port.admission.lock().held, 0);
    // Abort preserves the old graph; accepted releases still retire its gates.
    rt.cmd_rx.end_project_install();
    rt.process(&mut [0.0; 32]);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .all(|v| !matches!(v.env.stage, 1..=3)));
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .all(|v| !matches!(v.env.stage, 1..=3)));
    assert!(rt.decks.iter().all(|deck| !deck.touching));
    assert!(!rt.playing);
    assert_eq!(
        port.send(Command::LiveNoteOn {
            source: 7,
            ch: 0,
            note: 60,
            vel: 100
        }),
        Ok(SubmissionOutcome::Accepted)
    );
    rt.process(&mut [0.0; 32]);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| matches!(v.env.stage, 1..=3)));
}

#[test]
fn gate_toggle_is_allocation_free_idempotent_and_critical_writer_is_counted() {
    let (port, receiver) = CommandPort::channel(32);
    let counts = test_alloc::measure(|| {
        for _ in 0..1000 {
            assert!(receiver.begin_project_install());
            assert!(receiver.begin_project_install());
            receiver.end_project_install();
            receiver.end_project_install();
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(receiver.begin_project_install());
    let producer_mutex = port.admission.lock();
    let producer = port.clone();
    let worker = std::thread::spawn(move || producer.send(Command::Stop));
    let until = Instant::now() + Duration::from_secs(2);
    while port.shared.project_writers.load(Ordering::Acquire) != PROJECT_CLOSED + 1 {
        assert!(
            Instant::now() < until,
            "critical producer did not acquire its lease"
        );
        std::thread::yield_now();
    }
    assert!(!receiver.begin_project_install());
    drop(producer_mutex);
    assert_eq!(worker.join().unwrap(), Ok(SubmissionOutcome::Accepted));
    assert!(receiver.begin_project_install());
    assert!(matches!(
        receiver.try_recv(),
        Ok(Command::ReservedStop { lane: 0, .. })
    ));
    receiver.end_project_install();
    lease_retired(&port);
    // Raw receivers used by numerical tests have no producers to gate.
    let (_, raw) = crossbeam_channel::bounded::<Command>(1);
    let raw = CommandReceiver::from(raw);
    assert!(raw.begin_project_install());
    raw.end_project_install();
}
