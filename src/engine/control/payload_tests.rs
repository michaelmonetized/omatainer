use super::*;
use crate::engine::{
    dsp::Sample,
    load_receipt::{Media, Receipt},
    test_alloc, MidiNote,
};
use std::sync::{atomic::Ordering::Acquire, Arc};

fn sample() -> Arc<Sample> {
    Arc::new(Sample {
        name: String::with_capacity(31),
        path: String::with_capacity(63),
        sr: 48_000,
        ch: 1,
        data: Vec::with_capacity(128),
        peaks: Arc::new(Vec::with_capacity(17)),
        bpm: 120.0,
    })
}
fn load(audio: &Arc<Sample>) -> Command {
    Command::DeckAudio {
        deck: 0,
        audio: audio.clone(),
    }
}
fn pending(port: &CommandPort) -> usize {
    port.shared.queued_payload_bytes.load(Acquire)
}

#[test]
fn variable_payload_counts_allocated_capacity_and_shared_media_conservatively() {
    let audio = sample();
    let bytes = owned_payload_bytes(&load(&audio));
    let loader = crate::engine::media_load::Loader::with_decoder(|path, _| {
        crate::engine::decode::decode_audio_with_cancel(path, || true)
    })
    .unwrap();
    let token = loader
        .request(0, std::path::PathBuf::from("/unavailable-payload-test.wav"))
        .unwrap();
    assert_eq!(
        bytes,
        std::mem::size_of::<Sample>()
            + 4 * std::mem::size_of::<usize>()
            + std::mem::size_of::<Vec<[f32; 3]>>()
            + audio.data.capacity() * 4
            + audio.peaks.capacity() * 12
            + audio.name.capacity()
            + audio.path.capacity()
    );
    assert_eq!(
        bytes,
        owned_payload_bytes(&Command::DeckLoadRequested {
            deck: 0,
            media: Media::Decoded {
                token: token,
                audio
            },
            receipt: Receipt::new(),
        })
    );
    let notes = Vec::<MidiNote>::with_capacity(57);
    let note_bytes = notes.capacity() * std::mem::size_of::<MidiNote>();
    assert_eq!(
        owned_payload_bytes(&Command::SetNotes {
            track: 0,
            scene: 0,
            notes
        }),
        note_bytes
    );
    let param = String::with_capacity(49);
    let param_bytes = param.capacity();
    assert_eq!(
        owned_payload_bytes(&Command::LearnCapture {
            param,
            ch: 0,
            d1: 0,
            d2: 0,
            status: 0
        }),
        param_bytes
    );
    assert_eq!(
        owned_payload_bytes(&Command::Gesture {
            id: 1,
            command: Box::new(Command::Master(0.3))
        }),
        std::mem::size_of::<Command>()
    );
    assert_eq!(MAX_QUEUED_PAYLOAD_BYTES, 256 * 1024 * 1024);
}

#[test]
fn queued_payload_limit_preserves_reserved_releases_and_recovers_on_dequeue() {
    let audio = sample();
    let bytes = owned_payload_bytes(&load(&audio));
    let (port, receiver) = CommandPort::channel_with_payload_limit(32, bytes);
    port.send(Command::SamplerPad { pad: 2, on: true }).unwrap();
    port.send(load(&audio)).unwrap();
    assert_eq!(pending(&port), bytes);
    assert_eq!(port.send(load(&audio)), Err(SubmissionError::PayloadFull));
    assert_eq!(port.stats().last_error, Some(SubmissionError::PayloadFull));
    assert_eq!(
        serde_json::to_string(&SubmissionError::PayloadFull).unwrap(),
        "\"payload_full\""
    );
    port.send(Command::SamplerPad { pad: 2, on: false })
        .unwrap();
    port.send(Command::Stop).unwrap();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        Command::SamplerPad { on: true, .. }
    ));
    assert_eq!(pending(&port), bytes);
    assert!(matches!(
        receiver.try_recv().unwrap(),
        Command::DeckAudio { .. }
    ));
    assert_eq!(pending(&port), 0);
    port.send(load(&audio)).unwrap();
    assert_eq!(pending(&port), bytes);
    while receiver.try_recv().is_ok() {}
    assert_eq!(pending(&port), 0);
}

#[test]
fn an_oversize_payload_and_full_queue_do_not_consume_memory_credit() {
    let audio = sample();
    let bytes = owned_payload_bytes(&load(&audio));
    let (small, _rx) = CommandPort::channel_with_payload_limit(32, bytes - 1);
    assert_eq!(small.send(load(&audio)), Err(SubmissionError::PayloadFull));
    assert_eq!(pending(&small), 0);
    let (port, receiver) = CommandPort::channel_with_payload_limit(32, bytes);
    while port.send(Command::Master(0.5)).is_ok() {}
    assert_eq!(port.send(load(&audio)), Err(SubmissionError::Full));
    assert_eq!(pending(&port), 0);
    drop(receiver);
    assert_eq!(port.send(load(&audio)), Err(SubmissionError::Disconnected));
    assert_eq!(pending(&port), 0);
}

#[test]
fn concurrent_producers_share_one_payload_budget() {
    let audio = sample();
    let bytes = owned_payload_bytes(&load(&audio));
    let (port, receiver) = CommandPort::channel_with_payload_limit(64, bytes * 2);
    let ready = Arc::new(std::sync::Barrier::new(9));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let port = port.clone();
            let audio = audio.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                port.send(load(&audio))
            })
        })
        .collect();
    ready.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(SubmissionError::PayloadFull))
            .count(),
        6
    );
    assert_eq!(pending(&port), 2 * bytes);
    assert_eq!(receiver.len(), 2);
    let batch = CommandBatch::receive(&receiver);
    assert_eq!(batch.received, 2);
    assert_eq!(pending(&port), 0);
}

#[test]
fn producer_cannot_refill_payload_credit_between_batch_pops() {
    let audio = sample();
    let bytes = owned_payload_bytes(&load(&audio));
    let (port, receiver) = CommandPort::channel_with_payload_limit(64, bytes * 2);
    port.send(load(&audio)).unwrap();
    port.send(load(&audio)).unwrap();
    let (request, requests) = std::sync::mpsc::sync_channel(0);
    let (reply, replies) = std::sync::mpsc::sync_channel(0);
    let producer = port.clone();
    let worker_audio = audio.clone();
    let worker = std::thread::spawn(move || {
        for _ in 0..2 {
            requests.recv().unwrap();
            reply.send(producer.send(load(&worker_audio))).unwrap();
        }
    });
    let batch = CommandBatch::receive_with(&receiver, || {
        request.send(()).unwrap();
        assert_eq!(
            replies
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            Err(SubmissionError::PayloadFull)
        );
        assert_eq!(pending(&port), bytes * 2);
    });
    worker.join().unwrap();
    assert_eq!(batch.received, 2);
    assert_eq!(pending(&port), 0);
    // The received batch and newly admitted queue each retain at most one cap.
    port.send(load(&audio)).unwrap();
    port.send(load(&audio)).unwrap();
    assert_eq!(port.send(load(&audio)), Err(SubmissionError::PayloadFull));
    assert_eq!(pending(&port), bytes * 2);
}

#[test]
fn coalesced_boxed_gestures_return_credit_without_callback_destruction() {
    let one = std::mem::size_of::<Command>();
    let (port, receiver) = CommandPort::channel_with_payload_limit(64, one * 3);
    for value in [0.1, 0.2, 0.3] {
        port.send(Command::Gesture {
            id: 41,
            command: Box::new(Command::Master(value)),
        })
        .unwrap();
    }
    assert_eq!(pending(&port), one * 3);
    let mut batch = None;
    let counts = test_alloc::measure(|| batch = Some(CommandBatch::receive(&receiver)));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let batch = batch.unwrap();
    assert_eq!((batch.received, batch.applied), (3, 1));
    assert_eq!(batch.discarded.iter().flatten().count(), 2);
    assert_eq!(pending(&port), 0);
    port.send(Command::Gesture {
        id: 42,
        command: Box::new(Command::Master(0.8)),
    })
    .unwrap();
    assert_eq!(pending(&port), one);
}

#[test]
fn producer_waiting_on_admission_rechecks_new_history_backpressure() {
    let (port, receiver) = CommandPort::channel(32);
    let locked = port.admission.lock();
    let (preflight, passed) = std::sync::mpsc::sync_channel(0);
    let producer = port.clone();
    let worker = std::thread::spawn(move || {
        producer.send_after_preflight(
            Command::SetNotes {
                track: 0,
                scene: 0,
                notes: Vec::with_capacity(128),
            },
            || {
                preflight.send(()).unwrap();
            },
        )
    });
    passed
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    receiver.set_history_available(false);
    drop(locked);
    assert_eq!(worker.join().unwrap(), Err(SubmissionError::HistoryBusy));
    assert!(receiver.is_empty());
    assert_eq!(pending(&port), 0);
    assert_eq!(port.stats().accepted, 0);
    // Emergency stop still has its reserved route after creative admission closes.
    port.send(Command::Stop).unwrap();
    assert!(matches!(
        receiver.try_recv(),
        Ok(Command::ReservedStop { .. })
    ));
}

#[test]
fn performance_packet_waiter_rechecks_safety_after_producer_mutex() {
    use crate::engine::performance::{Error, Safety};
    let (port, receiver) = CommandPort::channel(32);
    let producer = port.for_input_epoch(port.performance().input_epoch());
    let locked = port.admission.lock();
    let (preflight, passed) = std::sync::mpsc::sync_channel(0);
    let worker = std::thread::spawn(move || producer.send_after_preflight(
        Command::LiveNoteOn { source: 42, ch: 0, note: 60, vel: 100 },
        || preflight.send(()).unwrap(),
    ));
    passed.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    port.send(Command::SafetyStop(Safety::Stop)).unwrap();
    drop(locked);
    assert_eq!(worker.join().unwrap(), Err(SubmissionError::Performance(Error::Recovery)));
    assert!(receiver.is_empty());
    assert_eq!(pending(&port), 0);
    // Stale packet handles may still deliver safety releases; they cannot start
    // a new voice after an explicit recovery opens the creative gate again.
    let old = port.for_input_epoch(port.performance().input_epoch());
    port.performance().stopped(port.performance().safety_request().unwrap().0);
    port.send(Command::RecoverPerformance).unwrap();
    assert!(port.performance().try_recover(|| true));
    assert_eq!(old.send(Command::Master(0.2)), Err(SubmissionError::Performance(Error::Recovery)));
    assert!(old.send(Command::LiveNoteOff { source: 42, ch: 0, note: 60 }).is_ok());
}
