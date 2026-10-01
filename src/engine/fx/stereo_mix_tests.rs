use super::*;

// A delay-free Balance reference and an independent explicit ring for Spread.
// Neither uses FxSlot/FxChain or the DSP Delay implementation under test.
struct Reference {
    kind: FxId,
    amount: f32,
    mix: f32,
    ring: [Vec<f32>; 2],
    write: usize,
}

impl Reference {
    fn new(kind: FxId, amount: f32, mix: f32, sr: f32) -> Self {
        Self {
            kind,
            amount,
            mix,
            ring: std::array::from_fn(|_| vec![0.0; ((sr * 0.02) as usize).max(64)]),
            write: 0,
        }
    }
    fn process(&mut self, input: [f32; 2], sr: f32) -> [f32; 2] {
        let wet = if self.kind == FxId::Balance {
            let pan = (self.amount * 2.0 - 1.0).clamp(-1.0, 1.0);
            [
                input[0] * (1.0 - pan.max(0.0)).sqrt(),
                input[1] * (1.0 + pan.min(0.0)).sqrt(),
            ]
        } else if self.amount == 0.5 {
            input
        } else {
            let size = self.ring[0].len();
            let distance = ((self.amount - 0.5).abs() * sr * 0.012).clamp(1.0, size as f32 - 2.0);
            let read = (self.write as f32 - distance + size as f32) % size as f32;
            let left = read as usize;
            let right = (left + 1) % size;
            let fraction = read.fract();
            let delayed = std::array::from_fn::<_, 2, _>(|channel| {
                let a = self.ring[channel][left];
                let b = self.ring[channel][right];
                self.ring[channel][self.write] = input[channel];
                a + (b - a) * fraction
            });
            self.write = (self.write + 1) % size;
            if self.amount > 0.5 {
                [input[0], delayed[1]]
            } else {
                let amount = (0.5 - self.amount) * 2.0;
                let mid = (delayed[0] + delayed[1]) * 0.5;
                input.map(|x| x * (1.0 - amount) + mid * amount)
            }
        };
        if self.mix == 0.0 || wet == input {
            input
        } else if self.mix == 1.0 {
            wet
        } else {
            std::array::from_fn(|channel| {
                input[channel] * (1.0 - self.mix) + wet[channel] * self.mix
            })
        }
    }
}

fn slot(kind: FxId, amount: f32, mix: f32, sr: f32) -> FxSlot {
    let mut slot = FxSlot::new(kind, sr);
    slot.p[0] = amount;
    slot.mix = mix;
    slot
}
fn input(frame: usize) -> [f32; 2] {
    match frame {
        0 => [0.8, -0.3],
        1 => [-0.0, 0.0],
        2 => [0.0, -0.0],
        _ => [
            (frame as f32 * 0.117).sin() * 0.7,
            (frame as f32 * 0.053).cos() * 0.4,
        ],
    }
}
fn bits(frame: [f32; 2]) -> [u32; 2] {
    frame.map(f32::to_bits)
}

#[test]
fn zero_mix_is_bit_exact_for_every_amount_and_preserves_the_original_balance_fixture() {
    for sr in [32_000.0, 44_100.0, 48_000.0, 96_000.0] {
        for kind in [FxId::Spread, FxId::Balance] {
            for amount in 0..=128 {
                let mut processor = slot(kind, amount as f32 / 128.0, 0.0, sr);
                for frame in 0..64 {
                    let sample = input(frame);
                    assert_eq!(bits(processor.tick_stereo(sample, sr)), bits(sample));
                }
            }
        }
    }
    let mut balance = slot(FxId::Balance, 0.0, 0.0, 48_000.0);
    assert_eq!(balance.tick_stereo([0.8, 0.8], 48_000.0), [0.8, 0.8]);
}

#[test]
fn exhaustive_mix_grid_matches_independent_stereo_references_and_linear_interpolation() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for kind in [FxId::Spread, FxId::Balance] {
            for amount in [0.0, 0.17, 0.499, 0.5, 0.501, 0.83, 1.0] {
                for step in 0..=32 {
                    let mix = step as f32 / 32.0;
                    let mut actual = slot(kind, amount, mix, sr);
                    let mut reference = Reference::new(kind, amount, mix, sr);
                    let mut full = Reference::new(kind, amount, 1.0, sr);
                    for frame in 0..2048 {
                        let dry = input(frame);
                        let wet = full.process(dry, sr);
                        let expected = reference.process(dry, sr);
                        let actual = actual.tick_stereo(dry, sr);
                        assert_eq!(
                            bits(actual),
                            bits(expected),
                            "{kind:?} amount={amount} mix={mix} frame={frame} sr={sr}"
                        );
                        for channel in 0..2 {
                            let linear = dry[channel] * (1.0 - mix) + wet[channel] * mix;
                            assert!((actual[channel] - linear).abs() < 1e-7);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn duplicate_instances_and_reordered_stereo_chains_match_independent_series() {
    let sr = 48_000.0;
    for order in [
        [
            (FxId::Spread, 0.23, 0.37),
            (FxId::Balance, 0.9, 0.6),
            (FxId::Spread, 0.87, 0.8),
            (FxId::Balance, 0.1, 0.25),
        ],
        [
            (FxId::Balance, 0.1, 0.25),
            (FxId::Spread, 0.87, 0.8),
            (FxId::Balance, 0.9, 0.6),
            (FxId::Spread, 0.23, 0.37),
        ],
    ] {
        let mut chain = FxChain {
            slots: order
                .iter()
                .map(|&(kind, amount, mix)| slot(kind, amount, mix, sr))
                .collect(),
        };
        let mut reference: Vec<_> = order
            .iter()
            .map(|&(kind, amount, mix)| Reference::new(kind, amount, mix, sr))
            .collect();
        for frame in 0..8192 {
            let sample = input(frame);
            let mut expected = sample;
            for processor in &mut reference {
                expected = processor.process(expected, sr);
            }
            assert_eq!(bits(chain.process_stereo(sample, sr)), bits(expected));
        }
    }
    // Narrowing mixes channels, so balancing before it and after it must differ.
    let mut before = FxChain {
        slots: vec![
            slot(FxId::Balance, 0.0, 1.0, sr),
            slot(FxId::Spread, 0.0, 1.0, sr),
        ],
    };
    let mut after = FxChain {
        slots: vec![
            slot(FxId::Spread, 0.0, 1.0, sr),
            slot(FxId::Balance, 0.0, 1.0, sr),
        ],
    };
    let mut difference = 0.0;
    for frame in 0..2048 {
        let a = before.process_stereo(input(frame), sr);
        let b = after.process_stereo(input(frame), sr);
        difference += (a[0] - b[0]).abs() + (a[1] - b[1]).abs();
    }
    assert!(difference > 1.0);
}

#[test]
fn zero_mix_keeps_spread_history_warm_and_bypass_reversal_keeps_the_mixed_endpoint() {
    let sr = 48_000.0;
    let mut dry = slot(FxId::Spread, 0.83, 0.0, sr);
    let mut wet = slot(FxId::Spread, 0.83, 1.0, sr);
    for frame in 0..1024 {
        dry.tick_stereo(input(frame), sr);
        wet.tick_stereo(input(frame), sr);
    }
    dry.mix = 1.0;
    for frame in 1024..2048 {
        assert_eq!(
            bits(dry.tick_stereo(input(frame), sr)),
            bits(wet.tick_stereo(input(frame), sr))
        );
    }
    // A quarter-mix hard-left Balance is [0.8,0.6], not the full-wet [0.8,0].
    let mut balance = slot(FxId::Balance, 0.0, 0.25, sr);
    let sample = [0.8, 0.8];
    assert_eq!(balance.tick_stereo(sample, sr), [0.8, 0.6]);
    let mut previous = [0.8, 0.6];
    for (enabled, count) in [(false, 80), (true, 240), (false, 240)] {
        balance.on = enabled;
        for _ in 0..count {
            let next = balance.tick_stereo(sample, sr);
            assert!((next[1] - previous[1]).abs() <= 0.2 / 240.0 + 2e-7);
            assert!((next[0] - 0.8).abs() < 1e-7);
            previous = next;
        }
    }
    assert_eq!(balance.tick_stereo(sample, sr), sample);
}
