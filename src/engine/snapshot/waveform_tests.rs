use super::tests::{engine, wait};
use super::*;
use crate::engine::test_alloc;

fn media(name: &str, buckets: usize) -> Arc<Sample> {
    Arc::new(Sample {
        name: name.into(),
        sr: 48_000,
        ch: 2,
        data: vec![0.25; 48_000 * 2],
        peaks: vec![[0.1, 0.2, 0.3]; buckets].into(),
        bpm: 120.0,
        path: String::new(),
    })
}

fn assert_periodic_identity(rt: &mut RtEngine, expected: &Arc<Vec<[f32; 3]>>) {
    let mut output = [0.0; 1024];
    for _ in 0..12 {
        wait(|| rt.publisher.free.len() == FRAMES);
        let sequence = rt.publisher.sequence;
        rt.frames_done = 6000 - 256;
        let counts = test_alloc::measure(|| rt.process(&mut output));
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(rt.publisher.sequence, sequence + 1);
        wait(|| rt.publisher.published.load(Ordering::Acquire) > sequence);
        assert!(Arc::ptr_eq(&rt.snap.lock().decks[0].peaks, expected));
    }
}

#[test]
fn waveform_identity_changes_only_with_media_across_real_periodic_snapshots() {
    let mut rt = engine();
    let empty = rt.snap.lock().decks[0].peaks.clone();
    assert_periodic_identity(&mut rt, &empty);

    let first = media("first", 2048);
    let first_peaks = first.peaks.clone();
    let original_data = first.data.as_ptr();
    let original_peaks = first.peaks.as_ptr();
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: first.clone(),
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.publish_for_test();
    assert_eq!(
        rt.decks[0].audio.as_ref().unwrap().data.as_ptr(),
        original_data
    );
    assert_eq!(rt.snap.lock().decks[0].peaks.as_ptr(), original_peaks);
    assert_periodic_identity(&mut rt, &first_peaks);

    // New media with identical analysis values still has its own identity.
    let second = media("second", 2048);
    let second_peaks = second.peaks.clone();
    assert_eq!(*first_peaks, *second_peaks);
    assert!(!Arc::ptr_eq(&first_peaks, &second_peaks));
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: second,
    });
    rt.publish_for_test();
    assert_periodic_identity(&mut rt, &second_peaks);
    assert_eq!(rt.snap.lock().decks[0].title, "second");
    assert_eq!(first_peaks[0], [0.1, 0.2, 0.3]);

    rt.apply(Command::DeckUnload { deck: 0 });
    rt.publish_for_test();
    assert_periodic_identity(&mut rt, &empty);
    // Reloading the same immutable built-in/file object reuses that object's
    // metadata. It does not duplicate peaks just because a deck was unloaded.
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: first,
    });
    rt.publish_for_test();
    assert_periodic_identity(&mut rt, &first_peaks);
}

#[test]
fn held_snapshot_keeps_only_waveform_data_alive_after_media_replacement() {
    let mut rt = engine();
    let first = media("held", 8192);
    let retired_media = Arc::downgrade(&first);
    let retired_peaks = Arc::downgrade(&first.peaks);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: first,
    });
    rt.publish_for_test();
    wait(|| rt.publisher.free.len() == FRAMES);
    let held = rt.snap.lock().clone();
    rt.apply(Command::DeckUnload { deck: 0 });
    rt.publish_for_test();
    wait(|| rt.publisher.free.len() == FRAMES);
    assert!(retired_media.upgrade().is_none());
    assert!(retired_peaks.upgrade().is_some());
    assert_eq!(held.decks[0].peaks[8191], [0.1, 0.2, 0.3]);
    assert!(rt.snap.lock().decks[0].peaks.is_empty());
    drop(held);
    assert!(retired_peaks.upgrade().is_none());
}

#[test]
fn snapshot_worker_allocation_budget_is_independent_of_waveform_length() {
    let mut rt = engine();
    let mut frame = Frame::new(rt.publisher.empty_peaks.clone());
    let mut budgets = Vec::new();
    for buckets in [1, 2048, 1_048_576] {
        let sample = media("same title", buckets);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: sample.clone(),
        });
        frame.capture(&rt);
        if !frame.complete {
            frame.prepare();
            frame.capture(&rt);
        }
        assert!(frame.complete);
        let refs_before = Arc::strong_count(&sample.peaks);
        let mut next = None;
        let counts = test_alloc::measure(|| next = Some(frame.materialize()));
        let next = next.unwrap();
        assert!(Arc::ptr_eq(&next.decks[0].peaks, &sample.peaks));
        assert_eq!(Arc::strong_count(&sample.peaks), refs_before + 1);
        assert!(frame.samples.iter().all(Option::is_none));
        assert_eq!(next.decks[0].peaks.len(), buckets);
        budgets.push(counts);
        drop(next);
        assert_eq!(Arc::strong_count(&sample.peaks), refs_before);
    }
    assert_eq!(budgets[0], budgets[1]);
    assert_eq!(budgets[0], budgets[2]);
    // Metadata remains worker-allocated. This fixture documents its measured
    // cost; it is independent of peak count, not a universal project-size cap.
    println!("snapshot materialization metadata budget: {:?}", budgets[0]);
}
