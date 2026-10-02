//! Regression boundary for #10: warm sample processing must not allocate or
//! destroy slot storage. Slot construction is outside this measurement.
use super::*;
use fx::{FxChain, FxId, FxSlot};
use std::hint::black_box;

fn chain(mode: usize) -> FxChain {
    let mut chain = FxChain::new(48_000.0);
    if mode > 0 {
        for (i, id) in FxId::all().iter().enumerate() {
            let mut slot = FxSlot::new(*id, 48_000.0);
            slot.on = mode == 1 || (mode == 3 && i % 2 == 0);
            chain.slots.push(slot);
        }
    }
    chain
}

#[test]
fn empty_enabled_disabled_and_mixed_fx_chains_have_no_warm_heap_traffic() {
    for mode in 0..4 {
        let mut chain = chain(mode);
        for _ in 0..128 {
            black_box(chain.process_stereo([0.2, -0.1], 48_000.0));
        }
        let mut energy = 0.0;
        let measured = test_alloc::measure(|| {
            for frame in 0..96_000 {
                let input = [(frame as f32 * 0.01).sin() * 0.2, -0.1];
                let output = black_box(chain.process_stereo(input, 48_000.0));
                energy += output[0] * output[0] + output[1] * output[1];
            }
        });
        assert_eq!(measured, test_alloc::Counts::default(), "mode {mode}");
        assert!(energy.is_finite() && energy > 0.0);
    }
}

#[test]
fn stopped_track_fx_fixture_has_no_per_frame_slot_allocations() {
    let (_tx, rx) = crossbeam_channel::bounded(256);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let mut buffer = [0.0; 2048];
    for mode in 0..5 {
        for track in &mut rt.tracks {
            track.playing = None;
            track.fx = if mode == 4 {
                // Original defect fixture: 8 stopped tracks, 1 compressor each.
                let mut chain = FxChain::new(rt.sr);
                chain.slots.push(FxSlot::new(FxId::Comp, rt.sr));
                chain
            } else {
                chain(mode)
            };
        }
        rt.scene_fx = (0..SCENES).map(|_| FxChain::new(rt.sr)).collect();
        rt.process(&mut buffer);
        // Snapshot building is separately tracked by #25/#59. Match the
        // original 1024-frame fixture without crossing a publication boundary.
        rt.frames_done = 0;
        let measured = test_alloc::measure(|| rt.process(&mut buffer));
        assert_eq!(measured, test_alloc::Counts::default(), "mode {mode}");
        assert!(buffer.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn settled_bypass_freezes_dsp_and_resumes_the_same_tail_without_heap_work() {
    for id in FxId::all() {
        let mut actual = FxChain::new(48_000.0);
        actual.slots.push(FxSlot::new(*id, 48_000.0));
        // Exercise Spread and every stateful effect rather than neutral input.
        if *id == FxId::Spread {
            actual.slots[0].p[0] = 0.9;
        }
        for frame in 0..15_000 {
            actual.process_stereo([(frame as f32 * 0.1).sin() * 0.3, 0.2], 48_000.0);
        }
        actual.slots[0].on = false;
        let fade = test_alloc::measure(|| {
            for _ in 0..240 {
                black_box(actual.process_stereo([0.27, -0.38], 48_000.0));
            }
        });
        assert_eq!(fade, test_alloc::Counts::default(), "fade {id:?}");
        let mut frozen_reference = actual.clone();
        let mut dry = true;
        let measured = test_alloc::measure(|| {
            for _ in 0..15_000 {
                dry &= actual.process_stereo([0.27, -0.38], 48_000.0) == [0.27, -0.38];
            }
        });
        assert!(dry, "{id:?}");
        assert_eq!(measured, test_alloc::Counts::default(), "{id:?}");
        actual.slots[0].on = true;
        frozen_reference.slots[0].on = true;
        let mut matches = true;
        let measured = test_alloc::measure(|| {
            for _ in 0..15_000 {
                matches &= actual.process_stereo([0.0; 2], 48_000.0)
                    == frozen_reference.process_stereo([0.0; 2], 48_000.0);
            }
        });
        assert!(matches, "bypass advanced {id:?} processing state");
        assert_eq!(measured, test_alloc::Counts::default(), "{id:?}");
    }
}

#[test]
fn queued_fx_edits_take_effect_at_block_boundaries_and_warm_processing_stays_bounded() {
    let (tx, rx) = control::CommandPort::channel(256);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    tx.send(Command::OpenFxTrack(1)).unwrap();
    tx.send(Command::FxAdd(5)).unwrap(); // Delay
    tx.send(Command::FxMix {
        slot: 0,
        value: 0.75,
    })
    .unwrap();
    tx.send(Command::FxParam {
        slot: 0,
        p: 1,
        value: 0.25,
    })
    .unwrap();
    tx.send(Command::FxToggle(0)).unwrap();
    assert!(rt.tracks[1].fx.slots.is_empty());
    let mut output = [0.0; 2048];
    rt.process(&mut output);
    let slot = &rt.tracks[1].fx.slots[0];
    assert_eq!(slot.id(), FxId::Delay);
    assert_eq!(slot.mix, 0.75);
    assert_eq!(slot.p[1], 0.25);
    assert!(!slot.on);
    tx.send(Command::FxToggle(0)).unwrap();
    tx.send(Command::FxMix {
        slot: 0,
        value: 0.5,
    })
    .unwrap();
    rt.frames_done = 0;
    let measured = test_alloc::measure(|| rt.process(&mut output));
    assert_eq!(measured, test_alloc::Counts::default());
    assert!(rt.tracks[1].fx.slots[0].on);
    assert_eq!(rt.tracks[1].fx.slots[0].mix, 0.5);
    assert!(rt.tracks[0].fx.slots.is_empty());
}
