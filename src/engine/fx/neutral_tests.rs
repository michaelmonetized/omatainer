use super::*;

fn input(seed: &mut u32, frame: usize) -> [f32; 2] {
    if frame == 0 {
        return [1.0, 0.0];
    }
    if frame == 1 {
        return [0.0, -1.0];
    }
    if frame == 2 {
        return [-0.0, 0.0];
    }
    std::array::from_fn(|_| {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed as i32 as f32 / i32::MAX as f32
    })
}

fn bits(frame: [f32; 2]) -> [u32; 2] {
    frame.map(f32::to_bits)
}

#[test]
fn empty_and_entirely_neutral_racks_are_bit_exact_for_impulses_and_random_stereo() {
    for sr in [32_000.0, 44_100.0, 48_000.0, 96_000.0] {
        let mut empty = FxChain::new(sr);
        let mut neutral = FxChain::new(sr);
        for &id in FxId::all() {
            let mut slot = FxSlot::new(id, sr);
            slot.mix = 0.37; // identity cannot depend on fortunate mix rounding
            match id {
                FxId::Spread | FxId::Balance => slot.p[0] = 0.5,
                FxId::Comp => slot.p[1] = 0.0,
                FxId::Gate => slot.p[0] = 0.0,
                FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => slot.p[..3].fill(0.5),
                FxId::Arp => {}
                _ => slot.mix = 0.0,
            }
            neutral.slots.push(slot);
        }
        let mut seed = 17;
        for frame in 0..16_384 {
            let sample = input(&mut seed, frame);
            assert_eq!(bits(empty.process_stereo(sample, sr)), bits(sample));
            assert_eq!(bits(neutral.process_stereo(sample, sr)), bits(sample));
        }
    }
}

#[test]
fn neutral_or_initially_bypassed_spread_never_activates_delay_history() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for bypassed in [false, true] {
            let mut slot = FxSlot::new(FxId::Spread, sr);
            slot.on = !bypassed;
            slot.p[0] = if bypassed { 1.0 } else { 0.5 };
            let mut untouched = slot.clone();
            for _ in 0..4096 {
                assert_eq!(slot.tick_stereo([0.7, -0.4], sr), [0.7, -0.4]);
            }
            // Compare the actual processor states after making both
            // intentionally nonneutral. This deliberately isolates history
            // from the separately tested enable crossfade.
            slot.p[0] = 1.0;
            untouched.p[0] = 1.0;
            for frame in 0..2048 {
                let sample = if frame == 0 { [1.0, -0.5] } else { [0.0; 2] };
                assert_eq!(
                    slot.process_enabled(sample, sr),
                    untouched.process_enabled(sample, sr)
                );
            }
        }
        let mut active = FxSlot::new(FxId::Spread, sr);
        active.mix = 1.0;
        active.p[0] = 1.0;
        assert_eq!(active.tick_stereo([1.0, 1.0], sr), [1.0, 0.0]);
    }
}

#[test]
fn bypass_fades_to_exact_dry_and_back_in_five_ms_with_bounded_steps() {
    for sr in [32_000.0, 44_100.0, 48_000.0, 96_000.0] {
        let frames = (sr * 0.005_f32).ceil() as usize;
        let mut slot = FxSlot::new(FxId::Balance, sr);
        slot.mix = 1.0;
        slot.p[0] = 1.0;
        let sample = [0.75, -0.25];
        let processed = [0.0, -0.25];
        assert_eq!(slot.tick_stereo(sample, sr), processed);
        for on in [false, true, false, true] {
            slot.on = on;
            let mut previous = if on { sample } else { processed };
            for frame in 1..=frames {
                let progress = frame as f32 / frames as f32;
                let level = if on { progress } else { 1.0 - progress };
                let expected: [f32; 2] =
                    std::array::from_fn(|ch| sample[ch] * (1.0 - level) + processed[ch] * level);
                let actual = slot.tick_stereo(sample, sr);
                for ch in 0..2 {
                    assert!(
                        (actual[ch] - expected[ch]).abs() < 2e-7,
                        "sr={sr} on={on} frame={frame}"
                    );
                    assert!((actual[ch] - previous[ch]).abs() <= 0.75 / frames as f32 + 2e-7);
                }
                previous = actual;
            }
            assert_eq!(
                slot.tick_stereo(sample, sr),
                if on { processed } else { sample }
            );
        }
    }
}

#[test]
fn rapid_bypass_reversal_continues_from_current_level_without_a_jump() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let frames = (sr * 0.005_f32).ceil() as usize;
        let mut slot = FxSlot::new(FxId::Balance, sr);
        slot.mix = 1.0;
        slot.p[0] = 1.0;
        let sample = [1.0, -0.5];
        slot.tick_stereo(sample, sr);
        let mut last = [0.0, -0.5];
        for (on, count) in [
            (false, frames / 3),
            (true, frames / 4),
            (false, frames / 2),
            (true, frames),
        ] {
            slot.on = on;
            for _ in 0..count {
                let next = slot.tick_stereo(sample, sr);
                assert!((next[0] - last[0]).abs() <= 1.0 / frames as f32 + 2e-7);
                assert_eq!(next[1], -0.5);
                last = next;
            }
        }
        assert_eq!(last, [0.0, -0.5]);
    }
}

#[test]
fn neutral_eq_keeps_history_for_later_parameter_edits() {
    for id in [FxId::Eq3, FxId::Eq5, FxId::Eq8] {
        let mut actual = FxSlot::new(id, 48_000.0);
        let mut reference = actual.clone();
        for frame in 0..2048 {
            let sample = [(frame as f32 * 0.04).sin(), (frame as f32 * 0.017).cos()];
            assert_eq!(actual.tick_stereo(sample, 48_000.0), sample);
            // Run primitive processing directly with a nonflat gain. EQ gain
            // never feeds its filters, so this creates the same warm history.
            reference.p[..3].fill(0.75);
            reference.process_enabled(sample, 48_000.0);
        }
        actual.p = [0.1, 0.8, 0.4, 0.0];
        reference.p = actual.p;
        for _ in 0..1024 {
            assert_eq!(
                actual.tick_stereo([0.0; 2], 48_000.0),
                reference.tick_stereo([0.0; 2], 48_000.0)
            );
        }
    }
}
