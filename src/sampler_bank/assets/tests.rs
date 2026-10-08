use super::*;
use crate::{
    engine::test_alloc,
    sampler_bank::{resident::Voice, Controls},
};
use std::time::Instant;

fn wait(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < end, "sampler asset worker did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn sample(frames: usize) -> Arc<Sample> {
    Arc::new(Sample { spectrum: None,
        name: "Fixture".into(),
        path: String::new(),
        sr: 16_000,
        ch: 1,
        data: vec![0.25; frames],
        peaks: Arc::new(vec![[0.25; 3]; 2]),
        bpm: 0.0,
    })
}
fn data(sample: Arc<Sample>) -> Data {
    let mut audio = std::array::from_fn(|_| None);
    audio[0] = Some(sample);
    Data::prepare(
        Arc::new(Settings::empty("Embedded fixture".into()).unwrap()),
        audio,
        std::array::from_fn(|_| None),
    )
    .unwrap()
}
fn no_heap(f: impl FnOnce()) {
    let counts = test_alloc::measure(f);
    assert_eq!((counts.allocations, counts.frees), (0, 0));
}

#[test]
fn bank_history_voice_and_capture_final_releases_are_owned_by_the_worker() {
    let mut registry = Registry::default();
    let owner = registry.acquire(Budget::limits()).unwrap();
    let bank = owner.pin(data(sample(32))).unwrap();
    let weak_bank = Arc::downgrade(&bank.data);
    let weak_settings = Arc::downgrade(&bank.settings);
    let weak_sample = Arc::downgrade(bank.audio[0].as_ref().unwrap());
    let history = bank.clone();
    let capture_settings = bank.settings.clone();
    let capture_pcm = bank.audio[0].as_ref().unwrap().clone();
    let voice = Voice::new(
        capture_pcm.clone(),
        bank.ranges[0].unwrap(),
        Controls::default(),
        1.0,
        3,
    );
    no_heap(|| drop(bank));
    assert!(weak_bank.upgrade().is_some());
    no_heap(|| drop(history));
    wait(|| weak_bank.upgrade().is_none());
    assert!(weak_settings.upgrade().is_some());
    assert!(weak_sample.upgrade().is_some());
    no_heap(|| drop(voice));
    assert!(weak_sample.upgrade().is_some());
    no_heap(|| {
        drop(capture_pcm);
        drop(capture_settings);
    });
    wait(|| weak_sample.upgrade().is_none() && weak_settings.upgrade().is_none());
    assert_eq!(owner.available().unwrap(), Budget::limits());
    drop(owner);
    wait(|| registry.owner.upgrade().is_none());
}

#[test]
fn surviving_capture_joins_one_owner_after_graph_teardown_without_mutual_pins() {
    let mut registry = Registry::default();
    let owner = registry.acquire(Budget::limits()).unwrap();
    let id = owner.0.shared.id;
    let bank = owner.pin(data(sample(32))).unwrap();
    let captured_settings = bank.settings.clone();
    let captured_pcm = bank.audio[0].as_ref().unwrap().clone();
    let weak = Arc::downgrade(&captured_pcm);
    drop(bank);
    drop(owner);
    wait(|| registry.owner.upgrade().unwrap().state().banks.is_empty());
    let next = registry.acquire(Budget::limits()).unwrap();
    assert_eq!(next.0.shared.id, id);
    let mut audio = std::array::from_fn(|_| None);
    audio[0] = Some(captured_pcm.clone());
    let bank = next
        .pin(
            Data::prepare(
                captured_settings.clone(),
                audio,
                std::array::from_fn(|_| None),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(next.owns(&bank));
    assert_eq!(next.0.shared.state().samples.len(), 1);
    no_heap(|| {
        drop(captured_pcm);
        drop(captured_settings);
        drop(bank);
    });
    wait(|| weak.upgrade().is_none());
    drop(next);
    wait(|| registry.owner.upgrade().is_none());
    let fresh = registry.acquire(Budget::limits()).unwrap();
    assert_ne!(fresh.0.shared.id, id);
}

#[test]
fn panic_poison_retains_pins_until_final_external_release_and_refuses_new_owner() {
    let mut registry = Registry::default();
    let owner = registry.acquire(Budget::limits()).unwrap();
    let bank = owner.pin(data(sample(32))).unwrap();
    let voice = bank.audio[0].as_ref().unwrap().clone();
    let capture = bank.settings.clone();
    let weak = Arc::downgrade(&voice);
    let weak_settings = Arc::downgrade(&capture);
    owner.0.shared.panic_next.store(true, Ordering::Release);
    owner.0.shared.wake.notify_one();
    wait(|| owner.0.shared.health.poisoned.load(Ordering::Acquire));
    assert_eq!(owner.available(), Err(Error::Poisoned));
    assert!(matches!(
        registry.acquire(Budget::limits()),
        Err(Error::Poisoned)
    ));
    no_heap(|| drop(bank));
    wait(|| owner.0.shared.state().banks.is_empty());
    assert!(weak.upgrade().is_some());
    assert!(weak_settings.upgrade().is_some());
    no_heap(|| {
        drop(voice);
        drop(capture);
    });
    wait(|| weak.upgrade().is_none() && weak_settings.upgrade().is_none());
    drop(owner);
    wait(|| registry.owner.upgrade().is_none());
    assert!(matches!(
        registry.acquire(Budget::limits()),
        Err(Error::Poisoned)
    ));
}

#[test]
fn reservations_saturate_before_decode_and_return_all_credits_on_failure_or_cancel() {
    let limits = Budget {
        banks: 4,
        settings: 4,
        samples: 2,
        pcm_bytes: 64,
        metadata_bytes: 1024 * 1024,
    };
    let mut registry = Registry::default();
    let owner = registry.acquire(limits).unwrap();
    let audio = sample(8);
    let bank = owner.pin(data(audio.clone())).unwrap();
    let shared_bank = owner.pin(data(audio.clone())).unwrap();
    assert_eq!(
        owner.0.shared.state().samples.len(),
        1,
        "edits sharing PCM are charged once"
    );
    let budget = Budget {
        banks: 1,
        settings: 1,
        samples: 1,
        pcm_bytes: 32,
        metadata_bytes: 32 * 1024,
    };
    let pending = owner.reserve(budget).unwrap();
    assert_eq!(pending.budget(), budget);
    assert!(matches!(owner.reserve(budget), Err(Error::Full)));
    let too_large = Arc::new(data(sample(9)));
    assert!(matches!(
        pending.publish(too_large),
        Err(Error::Reservation)
    ));
    assert_eq!(owner.0.shared.state().reserved, Budget::default());
    let pending = owner.reserve(budget).unwrap();
    drop(pending); // cancellation on the worker
    assert_eq!(owner.0.shared.state().reserved, Budget::default());
    let pending = owner.reserve(budget).unwrap();
    let added = pending.publish(Arc::new(data(sample(8)))).unwrap();
    assert_eq!(owner.available().unwrap().pcm_bytes, 0);
    assert!(matches!(owner.pin(data(sample(1))), Err(Error::Full)));
    let second_owner = Registry::default().acquire(limits).unwrap();
    assert!(
        !second_owner.owns(&added),
        "a command cannot install another owner's certificate"
    );
    no_heap(|| {
        drop(bank);
        drop(shared_bank);
        drop(added);
    });
    drop(audio);
    wait(|| owner.available().unwrap() == limits);
}

#[test]
fn native_rate_cropped_voice_is_bounded_and_full_source_preserves_legacy_samples() {
    let sample = Arc::new(Sample {
        data: vec![0.0, 0.5, 1.0, -0.9, -0.8],
        ..(*sample(5)).clone()
    });
    let controls = Controls {
        gain: 0.5,
        start_seconds: 1.0 / sample.sr as f64,
        end_seconds: Some(3.0 / sample.sr as f64),
    };
    let range = controls.frames(sample.sr, sample.frames()).unwrap();
    let mut voice = Voice::new(sample.clone(), range, controls, 1.0, 6);
    let mut values = [0.0; 4];
    no_heap(|| {
        for value in &mut values {
            *value = voice.tick(32_000.0).unwrap().0;
        }
    });
    assert_eq!(values, [0.25, 0.375, 0.5, 0.5]);
    assert!(voice.tick(32_000.0).is_none());
    assert_eq!(voice.track, 6);
    for output_rate in [16_000.0, 44_100.0, 48_000.0, 96_000.0] {
        let mut full = Voice::new(
            sample.clone(),
            (0.0, sample.frames() as f64),
            Controls::default(),
            0.5,
            0,
        );
        let mut position = 0.0;
        while position < sample.frames() as f64 {
            assert_eq!(full.tick(output_rate), Some(sample.at(position)));
            position += 0.5 * sample.sr as f64 / output_rate;
        }
        assert!(full.tick(output_rate).is_none());
    }
}

#[test]
fn a_fully_sealed_healthy_owner_can_be_replaced_before_its_worker_arc_is_dropped() {
    let mut registry = Registry::default();
    let owner = registry.acquire(Budget::limits()).unwrap();
    let finishing = owner.0.shared.clone();
    let id = finishing.id;
    drop(owner);
    wait(|| !finishing.health.alive.load(Ordering::Acquire));
    assert!(finishing.state().sealed);
    let next = registry.acquire(Budget::limits()).unwrap();
    assert_ne!(next.0.shared.id, id);
    assert!(!next.0.shared.state().sealed);
    assert_eq!(next.available().unwrap(), Budget::limits());
}
