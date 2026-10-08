use super::*;
use crate::engine::{Engine, RtEngine, Command, midi_edit::Outcome, test_alloc};
use std::time::{Duration, Instant};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("omatainer-record-placement-{}", crate::sampler_bank::BankId::new().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn capture(engine: &Engine, rt: &mut RtEngine) -> project::Captured {
    let project = engine.project.clone();
    let job = std::thread::spawn(move || project.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !job.is_finished() { rt.process(&mut []); assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
    job.join().unwrap()
}
fn recording(path: &Path, rate: u32, origin: f64, delay: u32, bpm: f32, fault: u8) {
    let recorder = super::super::Recorder::default();
    let output = path.to_owned();
    let writer_recorder = recorder.clone();
    let epoch = recorder.epoch();
    let writer = std::thread::spawn(move || writer_recorder.write_timed(7, 2, rate, 1, &output, &AtomicBool::new(false), epoch, None, bpm));
    let deadline = Instant::now() + Duration::from_secs(10);
    while recorder.alias() != 7 { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
    for frame in 0..2048 {
        let seconds = origin + if fault == 1 { 0.0 } else { f64::from(frame) / f64::from(rate) }
            + if fault == 3 && frame >= 512 { 1.0 } else { 0.0 };
        let tempo = if fault == 2 && frame >= 512 { bpm + 1.0 } else { bpm };
        let current_delay = delay + u32::from(fault == 4 && frame >= 512);
        assert_eq!(test_alloc::measure(|| {
            recorder.mark_clock(None, tempo);
            recorder.mark_origin(7, current_delay, seconds);
            let mut data = [0.0; 32];
            data[0] = if frame == 1000 { 0.25 } else { 0.0 };
            data[1] = -(frame as f32) / 10000.0;
            recorder.capture(7, data, true);
        }), Default::default());
    }
    recorder.stop();
    recorder.mark_clock(None, bpm + 1.0);
    recorder.mark_origin(7, delay + 1, origin + 10.0);
    assert_eq!(writer.join().unwrap().unwrap(), path);
}

#[test]
fn captured_delay_places_editable_audio_with_exact_tempo_undo_and_native_file_reopen() {
    let files = Files::new();
    for rate in [44100, 48000, 96000] {
        let path = files.0.join(format!("{rate}.wav"));
        let bpm = 103.9606_f32;
        let origin = 4.0;
        let delay = rate / 100;
        recording(&path, rate, origin, delay, bpm, 0);
        let original_hash = hash_file(&File::open(&path).unwrap(), &AtomicBool::new(false)).unwrap();
        let (engine, mut rt) = Engine::headless_for_test(rate, 256);
        rt.apply(Command::Stop);
        let captured = capture(&engine, &mut rt);
        let target = captured.state.session.as_ref().unwrap().reference(session::Axis::Track, 2).unwrap();
        let reviewed = Review::inspect(captured, &path, target, &AtomicBool::new(false)).unwrap();
        let seconds = origin - f64::from(delay) / f64::from(rate);
        assert!((reviewed.start - seconds * f64::from(bpm) / 60.0).abs() < 1e-12);
        assert!((reviewed.duration - 2048.0 / f64::from(rate) * f64::from(bpm) / 60.0).abs() < 1e-12);
        let (request, ack) = reviewed.prepare(capture(&engine, &mut rt), &AtomicBool::new(false)).unwrap();
        assert_eq!(test_alloc::measure(|| rt.apply(Command::ArrangementEdit(request))), Default::default());
        assert_eq!(ack.state(), Outcome::Applied);
        let retained = capture(&engine, &mut rt);
        let song = retained.state.arrangement.as_ref().unwrap();
        assert!(!song.enabled);
        assert_eq!(song.instances[0].track, target);
        assert_eq!(song.sources[0].audio_clock.as_ref().unwrap().exact_bpm, Some(f64::from(bpm)));
        let native = files.0.join(format!("{rate}.omat"));
        crate::project_file::save(&native, &crate::project_file::Bundle { state: retained.state, media: retained.media }, crate::project_file::Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
        let loaded = crate::project_file::load::<project::State>(&native, &Default::default(), &AtomicBool::new(false)).unwrap();
        let reopened = project::Prepared::from_state(loaded.state.clone(), loaded.media.clone(), rate).unwrap().into_offline();
        assert!(reopened.arrangement.plan.is_some());
        let source = &loaded.state.arrangement.as_ref().unwrap().sources[0];
        assert_eq!(loaded.media[source.clip.audio.unwrap()].data[2000], 0.25);
        assert_eq!(source.audio_clock.as_ref().unwrap().exact_bpm, Some(f64::from(bpm)));
        let mut legacy = serde_json::to_value(&loaded.state).unwrap();
        legacy["version"] = 35.into();
        assert!(serde_json::from_value::<project::State>(legacy).is_err());
        rt.apply(Command::Undo);
        assert!(capture(&engine, &mut rt).state.arrangement.is_none());
        rt.apply(Command::Redo);
        assert_eq!(capture(&engine, &mut rt).state.arrangement.unwrap().instances[0].start, reviewed.start);
        assert_eq!(hash_file(&File::open(&path).unwrap(), &AtomicBool::new(false)).unwrap(), original_hash);
    }
}

#[test]
fn negative_origin_trims_only_the_native_region_and_discontinuous_clock_keeps_the_wav() {
    let files = Files::new();
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    let target = rt.session.reference(session::Axis::Track, 2).unwrap();
    let negative = files.0.join("negative.wav");
    recording(&negative, 48000, 0.005, 480, 120.0, 0);
    let reviewed = Review::inspect(capture(&engine, &mut rt), &negative, target, &AtomicBool::new(false)).unwrap();
    assert_eq!(reviewed.trimmed_frames, 240);
    assert_eq!(reviewed.start, 0.0);
    assert_eq!(reviewed.model.sources[0].clip.audio_region.unwrap().start, 240);
    assert_eq!(crate::engine::decode::decode_audio(&negative).unwrap().sample.frames(), 2048);
    for fault in [1, 2, 3, 4] {
        let path = files.0.join(format!("fault-{fault}.wav"));
        recording(&path, 48000, 4.0, 480, 120.0, fault);
        assert!(path.is_file());
        assert!(Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).err().unwrap().contains("continuous"));
    }
}

#[test]
fn changed_files_stale_project_malformed_receipt_and_cancellation_never_admit_a_placement() {
    let files = Files::new();
    let path = files.0.join("guarded.wav");
    recording(&path, 48000, 4.0, 480, 120.0, 0);
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    let target = rt.session.reference(session::Axis::Track, 2).unwrap();
    let review = Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).unwrap();
    assert!(review.prepare(capture(&engine, &mut rt), &AtomicBool::new(true)).err().unwrap().contains("cancelled"));
    rt.apply(Command::Master(0.75));
    assert!(review.prepare(capture(&engine, &mut rt), &AtomicBool::new(false)).err().unwrap().contains("Project changed"));
    let review = Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).unwrap();
    let wav = std::fs::read(&path).unwrap();
    let mut changed = wav.clone();
    changed[44] ^= 1;
    std::fs::write(&path, changed).unwrap();
    assert!(review.prepare(capture(&engine, &mut rt), &AtomicBool::new(false)).err().unwrap().contains("WAV changed"));
    assert!(Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).err().unwrap().contains("no longer matches"));
    std::fs::write(&path, wav).unwrap();
    let review = Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).unwrap();
    let sidecar = path.with_file_name("guarded.wav.omatainer.json");
    let timing = std::fs::read(&sidecar).unwrap();
    std::fs::write(&sidecar, b"{}").unwrap();
    assert!(review.prepare(capture(&engine, &mut rt), &AtomicBool::new(false)).err().unwrap().contains("receipt changed"));
    assert!(Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(false)).is_err());
    std::fs::write(&sidecar, timing).unwrap();
    assert!(Review::inspect(capture(&engine, &mut rt), &path, target, &AtomicBool::new(true)).is_err());
    assert!(capture(&engine, &mut rt).state.arrangement.is_none());
}
