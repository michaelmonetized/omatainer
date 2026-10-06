use super::*;
use crate::engine::{audio::OutputCallback, dsp::Sample, Command, Engine};
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-performance-record-{}",
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
fn request(
    r: &Recorder,
    channels: Option<Vec<u16>>,
    format: Format,
    rate: u32,
    seconds: u32,
) -> Request {
    Request {
        alias: STANDARD_OUTPUT,
        options: Options {
            format,
            rate,
            channels: channels.as_ref().map_or(2, |c| c.len() as u16),
            dither: false,
            normalize: false,
        },
        seconds,
        output_channels: channels,
        epoch: r.epoch(),
    }
}
fn start(
    r: &Recorder,
    q: Request,
    path: &Path,
) -> std::thread::JoinHandle<Result<Outcome, String>> {
    let r = r.clone();
    let p = path.to_owned();
    std::thread::spawn(move || r.write_delivery(&q, &p, &AtomicBool::new(false)))
}
fn activate(r: &Recorder, rate: u32) {
    let end = Instant::now() + Duration::from_secs(15);
    while r.alias() == 0 {
        r.begin_delivery(rate);
        assert!(Instant::now() < end, "recording never activated");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn converted_output_records_exact_channel_order_integer_conversion_and_failure_prefixes() {
    let files = Files::new();
    let r = Recorder::default();
    for (n, format) in [
        Format::Float32,
        Format::Pcm16,
        Format::Pcm24,
        Format::Flac16,
        Format::Flac24,
        Format::Mp3,
    ]
    .into_iter()
    .enumerate()
    {
        let folder = files.0.join(format!("format-{n}"));
        let job = start(&r, request(&r, Some(vec![3, 1]), format, 48000, 1), &folder);
        activate(&r, 48000);
        let input: Vec<i16> = (0..4096)
            .flat_map(|i| {
                [
                    0,
                    ((i as f32 * 0.05).sin() * 8192.) as i16,
                    0,
                    ((i as f32 * 0.08).cos() * 4096.) as i16,
                ]
            })
            .collect();
        assert_eq!(
            crate::engine::test_alloc::measure(|| r.converted(&input, 4)),
            crate::engine::test_alloc::Counts::default()
        );
        r.stop();
        let out = job.join().unwrap().unwrap();
        assert_eq!((out.frames, out.channels, out.rate), (4096, 2, 48000));
        assert!(out.warning.is_none(), "{:?}", out.warning);
        let decoded = crate::engine::decode::decode_audio(&out.files[0]).unwrap();
        assert_eq!(decoded.sample.ch, 2);
        assert!(decoded.sample.frames() >= 4096);
        if format != Format::Mp3 {
            assert_eq!(decoded.sample.frames(), 4096);
            for (i, frame) in decoded.sample.data.chunks_exact(2).enumerate() {
                assert_eq!(
                    frame,
                    [
                        f32::from(input[i * 4 + 3]) / 32768.,
                        f32::from(input[i * 4 + 1]) / 32768.
                    ]
                );
            }
        }
        assert!(folder.join("take-0001.wav").exists());
    }
    for failure in [1, 3, 4] {
        let folder = files.0.join(format!("failure-{failure}"));
        let job = start(
            &r,
            request(&r, Some(vec![0, 1]), Format::Float32, 48000, 1),
            &folder,
        );
        activate(&r, 48000);
        r.converted(&[0.125_f32, -0.25].repeat(17), 2);
        match failure {
            1 => r.converted(&[0.0_f32], 1),
            3 => r.invalidate(),
            _ => r.converted(&[f32::NAN, 0.0], 2),
        }
        let out = job.join().unwrap().unwrap();
        assert_eq!(out.frames, 17);
        assert!(out.warning.is_some());
        assert_eq!(
            crate::engine::decode::decode_audio(&out.files[0])
                .unwrap()
                .sample
                .data,
            [0.125, -0.25].repeat(17)
        );
    }
}
#[test]
fn final_callback_capture_is_bit_identical_to_the_audience_buffer_and_does_not_change_monitoring() {
    let files = Files::new();
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.quantize = false;
    rt.master = 0.25;
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample {
            spectrum: None,
            name: "record parity".into(),
            sr: 48000,
            ch: 2,
            data: (0..48000)
                .flat_map(|i| [(i as f32 * 0.07).sin() * 0.4, (i as f32 * 0.11).cos() * 0.2])
                .collect(),
            peaks: Vec::new().into(),
            bpm: 120.,
            path: String::new(),
        }),
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    let r = engine.routing.recorder.clone();
    let mut callback = OutputCallback::new(rt, 2);
    let folder = files.0.join("actual-callback");
    let job = start(
        &r,
        request(&r, Some(vec![1, 0]), Format::Float32, 48000, 1),
        &folder,
    );
    let end = Instant::now() + Duration::from_secs(15);
    let mut buffer = [0_f32; 512];
    let mut expected = Vec::new();
    while expected.len() < 4096 * 2 {
        let was_active = r.alias() != 0;
        callback.render(&mut buffer);
        if was_active || r.alias() != 0 {
            expected.extend(buffer.chunks_exact(2).flat_map(|f| [f[1], f[0]]));
            assert!(!r.monitoring_inputs());
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    r.stop();
    let out = job.join().unwrap().unwrap();
    let decoded = crate::engine::decode::decode_audio(&out.files[0]).unwrap();
    assert_eq!(decoded.sample.data, expected);
    assert!(expected.iter().any(|v| v.abs() > 0.001));
}
#[test]
fn raw_sources_and_queue_overflow_preserve_exact_prefix_and_restart_cleanly() {
    let files = Files::new();
    let r = Recorder::default();
    let mut q = request(&r, None, Format::Float32, 48000, 1);
    q.alias = 7;
    let job = start(&r, q, &files.0.join("raw"));
    activate(&r, 48000);
    assert!(r.monitoring_inputs());
    for _ in 0..31 {
        r.capture(7, [0.25; 32], true);
    }
    r.capture(7, [0.0; 32], false);
    let out = job.join().unwrap().unwrap();
    assert_eq!(out.frames, 31);
    assert!(out.warning.unwrap().contains("complete"));
    assert_eq!(
        crate::engine::decode::decode_audio(&out.files[0])
            .unwrap()
            .sample
            .data,
        vec![0.25; 62]
    );
    let job = start(
        &r,
        request(&r, Some(vec![0]), Format::Float32, 48000, 2),
        &files.0.join("overflow"),
    );
    activate(&r, 48000);
    let receiver = r.receiver.lock();
    for _ in 0..=super::super::CAPACITY {
        r.converted(&[0.125_f32], 1);
    }
    assert!(r.shared.fault.load(Ordering::Acquire));
    drop(receiver);
    let out = job.join().unwrap().unwrap();
    assert!(out.warning.unwrap().contains("overflow"));
    assert_eq!(out.frames, super::super::CAPACITY as u64);
    assert_eq!(
        crate::engine::decode::decode_audio(&out.files[0])
            .unwrap()
            .sample
            .data,
        vec![0.125; super::super::CAPACITY]
    );
    let job = start(
        &r,
        request(&r, Some(vec![0]), Format::Float32, 48000, 1),
        &files.0.join("restarted"),
    );
    activate(&r, 48000);
    r.converted(&[0.5_f32; 13], 1);
    r.stop();
    assert_eq!(job.join().unwrap().unwrap().frames, 13);
}
#[test]
fn suspended_producers_and_activation_locks_never_wait_on_the_callback_or_allocate() {
    let r = Recorder::default();
    r.shared.busy.store(true, Ordering::Release);
    r.shared.request.store(1, Ordering::Release);
    let mut producer = r.starts.lock();
    let chunk = producer.write_chunk_uninit(1).unwrap();
    assert_eq!(
        crate::engine::test_alloc::measure(|| for _ in 0..128 {
            r.begin_delivery(48000)
        }),
        crate::engine::test_alloc::Counts::default()
    );
    assert_eq!(r.alias(), 0);
    let start = DeliveryStart {
        request: 1,
        alias: STANDARD_OUTPUT,
        epoch: r.epoch(),
        rate: 48000,
        limit: 48000,
        width: 1,
        channels: [0; 26],
    };
    assert_eq!(chunk.fill_from_iter([start]), 1);
    drop(producer);
    let lock = r.starting.lock();
    r.begin_delivery(48000);
    assert_eq!(r.alias(), 0);
    drop(lock);
    r.begin_delivery(48000);
    assert_eq!(r.alias(), STANDARD_OUTPUT);
    let held = r.sender.lock();
    let begin = Instant::now();
    assert_eq!(
        crate::engine::test_alloc::measure(|| r.converted(&[0.25_f32; 256], 1)),
        crate::engine::test_alloc::Counts::default()
    );
    assert!(begin.elapsed() < Duration::from_millis(100));
    assert!(r.shared.fault.load(Ordering::Acquire));
    drop(held);
}
#[test]
fn stale_rate_epoch_preview_cancel_and_existing_destinations_refuse_activation() {
    let files = Files::new();
    let r = Recorder::default();
    let q = request(&r, Some(vec![0, 1]), Format::Float32, 48000, 1);
    let existing = files.0.join("existing");
    std::fs::create_dir(&existing).unwrap();
    assert!(r
        .write_delivery(&q, &existing, &AtomicBool::new(false))
        .is_err());
    let cancelled = files.0.join("cancelled");
    assert!(r
        .write_delivery(&q, &cancelled, &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    assert!(!cancelled.exists());
    r.preview(true);
    assert!(r
        .write_delivery(&q, &files.0.join("preview"), &AtomicBool::new(false))
        .is_err());
    r.preview(false);
    r.invalidate();
    assert!(r
        .write_delivery(&q, &files.0.join("retired"), &AtomicBool::new(false))
        .is_err());
    let job = start(
        &r,
        request(&r, Some(vec![0, 1]), Format::Float32, 48000, 1),
        &files.0.join("wrong-rate"),
    );
    let end = Instant::now() + Duration::from_secs(15);
    while !job.is_finished() {
        r.begin_delivery(44100);
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(job.join().unwrap().unwrap_err().contains("rate changed"));
    assert_eq!(r.alias(), 0);
}
#[test]
fn two_hour_software_stream_splits_without_missing_frames_or_growing_callback_memory() {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    let files = Files::new();
    let r = Recorder::default();
    let job = start(
        &r,
        request(&r, Some(vec![0]), Format::Float32, 12000, 7200),
        &files.0.join("two-hours"),
    );
    activate(&r, 12000);
    let frames = 12000_u64 * 7200;
    let block: Vec<f32> = (0..1024).map(|i| i as f32 / 2048. - 0.25).collect();
    let mut expected = Sha256::new();
    let mut submitted = 0;
    let deadline = Instant::now() + Duration::from_secs(240);
    while submitted < frames {
        let n = (frames - submitted).min(1024) as usize;
        while r.sender.lock().slots() < n {
            assert!(
                !r.shared.stop.load(Ordering::Acquire),
                "recording stopped at {submitted}"
            );
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            crate::engine::test_alloc::measure(|| r.converted(&block[..n], 1)),
            crate::engine::test_alloc::Counts::default()
        );
        for v in &block[..n] {
            expected.update(v.to_le_bytes());
        }
        submitted += n as u64;
    }
    r.stop();
    let out = job.join().unwrap().unwrap();
    assert_eq!(out.frames, frames);
    assert_eq!(out.files.len(), 2);
    assert!(out.warning.is_none(), "{:?}", out.warning);
    let mut actual = Sha256::new();
    let mut total = 0;
    for p in &out.files {
        let mut f = std::fs::File::open(p).unwrap();
        let mut header = [0; 44];
        f.read_exact(&mut header).unwrap();
        let bytes = u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64;
        assert_eq!(f.metadata().unwrap().len(), 44 + bytes);
        assert!(bytes <= SEGMENT_BYTES);
        total += bytes / 4;
        f.seek(SeekFrom::Start(44)).unwrap();
        let mut buffer = [0; 65536];
        loop {
            let n = f.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            actual.update(&buffer[..n]);
        }
    }
    assert_eq!(total, frames);
    assert_eq!(actual.finalize(), expected.finalize());
    let plan =
        crate::audio_delivery::recovery::review(&out.folder, &AtomicBool::new(false)).unwrap();
    assert_eq!((plan.frames, plan.repairs), (frames, 0));
}
