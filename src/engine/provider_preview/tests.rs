use super::*;
use crate::engine::test_alloc;

fn request(rt: &RtEngine) -> Request {
    Request {
        audio: crate::music_provider::tests::audio(),
        cancel: Arc::new(AtomicBool::new(false)),
        state: Arc::new(AtomicU8::new(0)),
        transport_epoch: rt.transport_epoch,
        safety_epoch: rt.performance.safety_epoch(),
    }
}

#[test]
fn original_pitch_audio_and_cancel_retirement_have_no_callback_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    let preview = request(&rt);
    let control = preview.clone();
    let before = rt.project.revision();
    let decks = rt.decks.each_ref().map(|deck| deck.audio.clone());
    engine.send(Command::ProviderPreview(preview)).unwrap();
    let mut out = [0.0; 128];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        test_alloc::Counts::default()
    );
    assert_eq!(control.state.load(Ordering::Acquire), 1);
    assert!(out.iter().any(|x| *x != 0.0));
    assert_eq!(rt.project.revision(), before);
    assert!(rt
        .decks
        .iter()
        .zip(decks)
        .all(|(deck, before)| match (&deck.audio, before) {
            (Some(after), Some(before)) => Arc::ptr_eq(after, &before),
            (None, None) => true,
            _ => false,
        }));
    for frame in 0..64 {
        let (left, right) = control.audio.at(frame as f64);
        assert_eq!(
            out[frame * 2].to_bits(),
            limiter(left * 0.25 * rt.master).to_bits()
        );
        assert_eq!(
            out[frame * 2 + 1].to_bits(),
            limiter(right * 0.25 * rt.master).to_bits()
        );
    }
    control.cancel.store(true, Ordering::Release);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        test_alloc::Counts::default()
    );
    assert_eq!(out, [0.0; 128]);
    assert_eq!(control.state.load(Ordering::Acquire), 3);
    assert!(rt.provider_preview.is_none());
}

#[test]
fn obsolete_previews_are_rejected_and_native_stop_and_safety_stop_end_audio() {
    for change in 0..4 {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.process(&mut []);
        let preview = request(&rt);
        let state = preview.state.clone();
        if change == 0 {
            rt.transport_epoch = rt.transport_epoch.wrapping_add(1);
        }
        if change == 1 {
            preview.cancel.store(true, Ordering::Release);
        }
        engine.send(Command::ProviderPreview(preview)).unwrap();
        rt.process(&mut [0.0; 128]);
        if change < 2 {
            assert_eq!(state.load(Ordering::Acquire), 2);
            continue;
        }
        assert_eq!(state.load(Ordering::Acquire), 1);
        if change == 2 {
            engine.send(Command::Stop).unwrap();
        } else {
            engine
                .send(Command::SafetyStop(performance::Safety::Silence))
                .unwrap();
        }
        let mut out = [0.0; 128];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut out)),
            test_alloc::Counts::default()
        );
        assert_eq!(out, [0.0; 128]);
        assert_eq!(state.load(Ordering::Acquire), 3);
    }
}

#[test]
fn natural_end_and_replacement_retire_pcm_outside_callback() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    let first = request(&rt);
    let first_state = first.state.clone();
    engine.send(Command::ProviderPreview(first)).unwrap();
    rt.process(&mut [0.0; 128]);
    let second = request(&rt);
    let second_state = second.state.clone();
    engine.send(Command::ProviderPreview(second)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 128])),
        test_alloc::Counts::default()
    );
    assert_eq!(first_state.load(Ordering::Acquire), 3);
    for _ in 0..80 {
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [0.0; 128])),
            test_alloc::Counts::default()
        );
    }
    assert_eq!(second_state.load(Ordering::Acquire), 3);
    assert!(rt.provider_preview.is_none());
}

#[test]
fn actual_project_capture_does_not_collect_remote_preview_pcm() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    let preview = request(&rt);
    let remote_audio = preview.audio.clone();
    engine.send(Command::ProviderPreview(preview)).unwrap();
    rt.process(&mut [0.0; 128]);
    let project = engine.project.clone();
    let worker = std::thread::spawn(move || project.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !worker.is_finished() {
        assert!(std::time::Instant::now() < deadline);
        rt.process(&mut []);
        std::thread::yield_now();
    }
    let capture = worker.join().unwrap();
    assert!(capture
        .media
        .iter()
        .all(|sample| !Arc::ptr_eq(sample, &remote_audio)));
}

#[test]
fn a_musically_scheduled_stop_silences_preview_on_that_sample() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    rt.playing = true;
    let preview = request(&rt);
    let state = preview.state.clone();
    engine.send(Command::ProviderPreview(preview)).unwrap();
    engine
        .send(Command::Remote(remote::Request {
            namespace: rt.session.namespace,
            transport_epoch: rt.transport_epoch,
            safety_epoch: rt.performance.safety_epoch(),
            at: Some(17.0 / (f64::from(rt.sr) * 60.0 / f64::from(rt.bpm))),
            action: remote::Action::Stop,
            ack: crate::engine::midi_edit::Ack::new(),
        }))
        .unwrap();
    let mut out = [0.0; 128];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        test_alloc::Counts::default()
    );
    assert!(out[..34].iter().any(|x| *x != 0.0));
    assert!(out[34..].iter().all(|x| *x == 0.0));
    assert_eq!(state.load(Ordering::Acquire), 3);
}

#[test]
fn licensed_preview_never_enters_a_routed_record_source_and_stale_writers_refuse_activation() {
    use crate::engine::audio::routing::{model::*, prepared::Prepared};
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    let mut model = Model::default(); model.next_id = 3;
    model.ports.push(Port { id: 2, alias: "Main print".into(), direction: Direction::Record, channels: vec![0, 1] });
    model.connections.push(Connection { source: Source { group: Group::Main, tap: Tap::PostMixer }, destination: Group::Record(2),
        map: (0..2).map(|channel| ChannelMap { source: channel, destination: channel, gain: 1.0 }).collect() });
    rt.routing = Some(Box::new(Prepared::new(Arc::new(model), &rt.session).unwrap()));
    let path = std::env::temp_dir().join(format!("omatainer-preview-record-{}.wav", crate::sampler_bank::BankId::new().unwrap()));
    let recorder = engine.routing.recorder.clone(); let writer = recorder.clone(); let destination = path.clone(); let epoch = recorder.epoch();
    let capture = std::thread::spawn(move || writer.write(2, 2, 48000, 1, &destination, &AtomicBool::new(false), epoch));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while recorder.alias() != 2 { assert!(std::time::Instant::now() < deadline); std::thread::sleep(std::time::Duration::from_millis(1)); }
    rt.process(&mut [0.0; 256]);
    rt.apply(Command::ProviderPreview(request(&rt)));
    let mut out = [0.0; 256]; rt.process(&mut out);
    assert!(out.iter().any(|sample| sample.abs() > 0.0002));
    capture.join().unwrap().unwrap();
    let decoded = crate::engine::decode::decode_audio(&path).unwrap();
    assert_eq!(decoded.sample.frames(), 128);
    assert!(decoded.sample.data.iter().all(|sample| *sample == 0.0));
    std::fs::remove_file(&path).unwrap();
    assert!(recorder.write(2, 2, 48000, 1, &path, &AtomicBool::new(false), recorder.epoch()).unwrap_err().contains("preview"));
    assert!(!path.exists());
}
