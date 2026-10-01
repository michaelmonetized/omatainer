use super::*;
use crate::engine::{master_fx::MasterSlot, FxKind, test_alloc};
use parts::Parts;
use tracker::Tracker;

fn direct(key: u64, value: [f32; 2]) -> Parts {
    Parts::transition(Parts::default(), key, value, 1.0)
}

#[test]
fn bounded_tracker_matches_single_source_fx_and_same_main_cue_route_without_heap_work() {
    let mut tracker = Tracker::new(48_000).unwrap();
    let kinds = [FxKind::Echo, FxKind::Reverb, FxKind::Filter];
    let wet = [0.21, 0.31, 0.44];
    let mut independent: [MasterSlot; 3] = std::array::from_fn(|_| MasterSlot::new(48_000.0));
    let spb = 48_000.0 * 60.0 / 172.0;
    tracker.configure(kinds, wet, spb);
    for i in 0..3 { independent[i].configure(wet[i], spb); }
    let counts = test_alloc::measure(|| {
        for i in 0..50_000 {
            let x = [(i as f32 * 0.013).sin() * 0.2, (i as f32 * 0.029).cos() * 0.3];
            let cue = (i % 100) as f32 / 99.0;
            let result = tracker.process([direct(101, x), Parts::default()], [101, 0],
                [x, [0.0; 2]], [0.25, 0.75], [true, false], cue);
            let mut expected = x.map(|v| v * 0.25);
            for slot in 0..3 { expected = independent[slot].process(expected, kinds[slot], wet[slot]); }
            expected = std::array::from_fn(|c| expected[c] * (1.0 - cue) + x[c] * cue);
            assert_eq!(result.values[0], expected);
            assert_eq!(result.episodes[0].unwrap().load, 101);
            assert!(!result.incomplete);
            assert!(result.error[0].iter().all(|x| x.is_finite() && *x >= 0.0));
        }
    });
    assert_eq!(counts.allocations, 0); assert_eq!(counts.frees, 0);
    // PFL contributes through cue_mix even when the crossfader is zero.
    let result = tracker.process([direct(101, [0.4; 2]), Parts::default()], [101, 0],
        [[0.4; 2], [0.0; 2]], [0.0, 1.0], [true, false], 1.0);
    assert_eq!(result.values[0], [0.4; 2]);
    tracker.reset();
    let result = tracker.process([direct(101, [0.4; 2]), Parts::default()], [101, 0],
        [[0.4; 2], [0.0; 2]], [0.0, 1.0], [true, false], 0.0);
    assert_eq!(result.values[0], [0.0; 2]);
}

#[test]
fn retiring_tails_keep_identity_and_four_lane_overflow_never_relabels_them() {
    let mut tracker = Tracker::new(8_000).unwrap();
    let kinds = [FxKind::Echo, FxKind::Echo, FxKind::Echo];
    tracker.configure(kinds, [0.5; 3], 8_000.0 * 60.0 / 400.0);
    for key in 1..=4 {
        for _ in 0..100 {
            let out = tracker.process([direct(key, [0.2; 2]), Parts::default()], [key, 0],
                [[0.2; 2], [0.0; 2]], [1.0, 0.0], [false; 2], 0.0);
            assert!(!out.incomplete);
        }
    }
    let fifth = tracker.process([direct(5, [0.2; 2]), Parts::default()], [5, 0],
        [[0.2; 2], [0.0; 2]], [1.0, 0.0], [false; 2], 0.0);
    assert!(fifth.incomplete);
    assert_eq!(fifth.episodes.map(|ep| ep.unwrap().load), [1, 2, 3, 4]);
    let mut tail = false;
    for _ in 0..8_000 * 45 {
        let out = tracker.process([Parts::default(); 2], [0; 2], [[0.0; 2]; 2],
            [1.0, 0.0], [false; 2], 0.0);
        tail |= out.episodes.iter().zip(out.values).any(|(ep, value)| ep.is_some_and(|e| e.load < 5) && value != [0.0; 2]);
    }
    assert!(tail);
    let after = tracker.process([direct(6, [0.2; 2]), Parts::default()], [6, 0],
        [[0.2; 2], [0.0; 2]], [1.0, 0.0], [false; 2], 0.0);
    assert_eq!(after.episodes.iter().flatten().map(|ep| ep.load).collect::<Vec<_>>(), [6]);
    assert!(after.incomplete, "recovering a lane cannot erase a prior gap");
}

#[test]
fn negotiated_rate_storage_is_bounded_and_invalid_rates_do_not_allocate() {
    for rate in [8_000, 48_000, 384_000] {
        let tracker = Tracker::new(rate).unwrap();
        assert!(tracker.storage_bytes() <= tracker::MAX_STORAGE);
    }
    let counts = test_alloc::measure(|| {
        for rate in [0, 7_999, 384_001, u32::MAX] { assert!(Tracker::new(rate).is_err()); }
    });
    assert_eq!(counts.allocations, 0); assert_eq!(counts.frees, 0);
}
