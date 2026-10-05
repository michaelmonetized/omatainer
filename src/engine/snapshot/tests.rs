use super::*;
use crate::engine::test_alloc;
use std::sync::mpsc;

pub(super) fn engine() -> RtEngine {
    let (_tx, rx) = CommandPort::channel(256);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.decks.iter_mut().for_each(|deck| deck.audio = None);
    rt.apply(Command::LaunchScene { scene: 0 });
    rt.process(&mut [0.0; 1024]);
    rt.publish_for_test();
    rt.publish_for_test();
    wait(|| rt.publisher.free.len() == FRAMES);
    rt
}

pub(super) fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "snapshot worker did not advance");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn publish_without_allocating(rt: &mut RtEngine) {
    wait(|| rt.publisher.free.len() > 0);
    let before = rt.publisher.sequence;
    let counts = test_alloc::measure(|| rt.publish());
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(
        rt.publisher.sequence,
        before + 1,
        "publication did not acquire a frame"
    );
}

#[test]
fn snapshot_publication_preserves_controller_worker_counters() {
    let mut rt = engine();
    let feedback = midi::FeedbackStats { sent: 90, failed: 2, connected: 3 };
    let input = midi::InputStats { received: 7, dispatched: 4, ..Default::default() };
    {
        let mut snapshot = rt.snap.lock();
        snapshot.midi_feedback = feedback;
        snapshot.midi_input = input;
    }
    rt.publish_initial();
    assert_eq!(rt.snap.lock().midi_feedback, feedback);
    assert_eq!(rt.snap.lock().midi_input, input);
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().midi_feedback, feedback);
    assert_eq!(rt.snap.lock().midi_input, input);
}

#[test]
fn snapshot_periodic_publication_allocates_and_frees_nothing_on_audio() {
    let mut rt = engine();
    let mut output = [0.0; 1024];
    for _ in 0..12 {
        wait(|| rt.publisher.free.len() == FRAMES);
        rt.frames_done = rt.sr as u64 / 60 - 256;
        let before = rt.publisher.sequence;
        let counts = test_alloc::measure(|| rt.process(&mut output));
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(
            rt.publisher.sequence,
            before + 1,
            "block missed periodic publish"
        );
        assert!(output.iter().any(|sample| sample.abs() > 0.0));
    }
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().beat, rt.beat);
}

#[test]
fn snapshot_eighty_ms_reader_cannot_delay_rendering_or_exhaust_ownership() {
    let mut rt = engine();
    let mut output = [0.0; 1024];
    let start = Instant::now();
    for _ in 0..64 {
        rt.process(&mut output);
    }
    let baseline = start.elapsed();
    rt.publish_for_test();
    wait(|| rt.publisher.free.len() == FRAMES);
    let before = rt.publisher.sequence;
    let public = rt.snap.clone();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _guard = public.lock();
        let held = Instant::now();
        held_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        release_rx.recv().unwrap();
        held.elapsed()
    });
    held_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let audio = std::thread::spawn(move || {
        let start = Instant::now();
        let counts = test_alloc::measure(|| {
            for _ in 0..64 {
                rt.process(&mut output);
            }
        });
        done_tx.send(start.elapsed()).unwrap();
        (rt, counts)
    });
    // The reader stays locked until explicitly released, even if 80 ms pass.
    // This broad machine-relative bound detects lock dependence, not underruns.
    let bound = (baseline * 50).max(Duration::from_millis(250));
    let completed = done_rx.recv_timeout(bound);
    release_tx.send(()).unwrap();
    let held = reader.join().unwrap();
    let (mut rt, counts) = audio.join().unwrap();
    assert!(
        completed.is_ok(),
        "renderer waited for reader; baseline {baseline:?}, bound {bound:?}"
    );
    assert!(held >= Duration::from_millis(80));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(
        rt.publisher.sequence - before,
        FRAMES as u64,
        "held reader did not exhaust the fixed two-frame pool"
    );
    rt.publish_for_test();
    assert_eq!(
        rt.snap.lock().beat,
        rt.beat,
        "publication failed to catch up after skipped states"
    );
    println!(
        "snapshot reader held {held:?}; 64-block baseline {baseline:?}; held-reader renderer {:?}",
        completed.unwrap()
    );
}

#[test]
fn snapshot_large_metadata_grows_off_audio_and_old_media_retires_on_worker() {
    let mut rt = engine();
    rt.tracks[0].name = "track".repeat(65_536);
    rt.tracks[0].clips[0].name = "clip".repeat(32_768);
    rt.sampler_banks = (0..5).map(|_| sampler::test_bank(&rt, "bank".repeat(1024), std::array::from_fn(|_| None))).collect();
    rt.fx_view = session::SCENE_FX_BASE + 3;
    rt.scene_fx[3].slots = (0..512)
        .map(|index| {
            let mut slot = fx::FxSlot::new(fx::FxId::Dist, rt.sr);
            slot.p[0] = index as f32 / 512.0;
            slot
        })
        .collect();
    let media = Arc::new(Sample {
        name: "large media".repeat(8192),
        sr: 48_000,
        ch: 2,
        data: vec![0.0; 1_048_576],
        peaks: vec![[0.1, 0.2, 0.3]; 262_144].into(),
        bpm: 120.0,
        path: String::new(),
    });
    let retired_media = Arc::downgrade(&media);
    rt.decks[0].title = media.name.clone();
    rt.decks[0].audio = Some(media);
    rt.snap.lock().midi = vec!["persistent MIDI input".into()];

    // First attempt cannot fit: request growth without allocating on audio.
    publish_without_allocating(&mut rt);
    rt.publish_for_test();
    rt.publish_for_test();
    wait(|| rt.publisher.free.len() == FRAMES);
    {
        let snap = rt.snap.lock();
        assert_eq!(snap.tracks[0].name, rt.tracks[0].name);
        assert_eq!(snap.tracks[0].clips[0].name, rt.tracks[0].clips[0].name);
        assert_eq!(snap.decks[0].title, rt.decks[0].title);
        assert_eq!(snap.decks[0].peaks.len(), 262_144);
        assert_eq!(snap.sampler_banks, rt.sampler_banks.iter().map(|bank| bank.name().to_owned()).collect::<Vec<_>>());
        assert_eq!(snap.fx_slots.len(), 512);
        assert_eq!(snap.fx_slots[511].3[0], 511.0 / 512.0);
        assert_eq!(snap.midi, ["persistent MIDI input"]);
    }
    let retired_peaks = Arc::downgrade(&rt.snap.lock().decks[0].peaks);
    let public = rt.snap.clone();
    let reader = public.lock();
    publish_without_allocating(&mut rt);
    // Worker is waiting on the reader with the first frame; its second frame
    // queues media references that must outlive engine source replacement.
    wait(|| rt.publisher.ready.is_empty() && rt.publisher.free.len() == 1);
    publish_without_allocating(&mut rt);
    assert_eq!(rt.publisher.free.len(), 0);
    rt.decks[0].audio = None;
    assert!(
        retired_media.upgrade().is_some(),
        "queue lost ownership of old media"
    );
    let counts = test_alloc::measure(|| {
        for _ in 0..20 {
            rt.publish();
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    drop(reader);
    wait(|| retired_media.upgrade().is_none());
    assert!(
        retired_peaks.upgrade().is_some(),
        "published waveform must retain its data"
    );

    rt.tracks[0].name = "short".into();
    rt.tracks[0].clips[0].name = "short clip".into();
    rt.decks[0].title.clear();
    rt.sampler_banks = vec![sampler::test_bank(&rt, "one bank".into(), std::array::from_fn(|_| None))];
    rt.scene_fx[3].slots.clear();
    publish_without_allocating(&mut rt);
    rt.publish_for_test();
    let snap = rt.snap.lock();
    assert_eq!(snap.tracks[0].name, "short");
    assert!(snap.decks[0].peaks.is_empty());
    assert!(retired_peaks.upgrade().is_none());
    assert_eq!(snap.sampler_banks, ["one bank"]);
    assert!(snap.fx_slots.is_empty());
    assert_eq!(snap.midi, ["persistent MIDI input"]);
}

fn isolated_publisher(capacity: usize) -> (Publisher, Receiver<Box<Frame>>, Sender<Box<Frame>>) {
    let (free_tx, free) = bounded(FRAMES);
    let (ready, receiver) = bounded(capacity);
    (
        Publisher {
            free,
            ready,
            pending: None,
            disconnected: false,
            sequence: 0,
            empty_peaks: Arc::new(Vec::new()),
            published: Arc::new(AtomicU64::new(0)),
        },
        receiver,
        free_tx,
    )
}

fn large_frame() -> (Box<Frame>, std::sync::Weak<Sample>) {
    let mut frame = Box::new(Frame::new(Arc::new(Vec::new())));
    frame.values.tracks[0].name = "old name".repeat(32_768);
    let sample = Arc::new(Sample {
        name: "old".into(),
        sr: 48_000,
        ch: 1,
        data: vec![0.0; 262_144],
        peaks: vec![[0.0; 3]; 65_536].into(),
        bpm: 120.0,
        path: String::new(),
    });
    let weak = Arc::downgrade(&sample);
    frame.samples[0] = Some(sample);
    (frame, weak)
}

#[test]
fn snapshot_full_and_disconnected_handoffs_retain_payload_without_callback_drops() {
    let (mut publisher, receiver, free_tx) = isolated_publisher(1);
    publisher.ready.send(Box::new(Frame::new(Arc::new(Vec::new())))).unwrap();
    let (frame, retired) = large_frame();
    let counts = test_alloc::measure(|| publisher.submit(frame));
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(publisher.pending.is_some());
    assert!(retired.upgrade().is_some());
    let counts = test_alloc::measure(|| {
        assert!(publisher.acquire().is_none());
    });
    assert_eq!(counts, test_alloc::Counts::default());
    drop(receiver.recv().unwrap());
    let counts = test_alloc::measure(|| {
        assert!(publisher.acquire().is_none());
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(publisher.pending.is_none());
    drop(receiver.recv().unwrap());
    assert!(retired.upgrade().is_none());
    drop(receiver);

    let (frame, retired) = large_frame();
    let counts = test_alloc::measure(|| publisher.submit(frame));
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(publisher.disconnected);
    assert!(retired.upgrade().is_some());
    let counts = test_alloc::measure(|| {
        for _ in 0..100 {
            assert!(publisher.acquire().is_none());
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    drop(publisher); // lifecycle teardown is outside rendering
    drop(free_tx);
    assert!(retired.upgrade().is_none());
}
