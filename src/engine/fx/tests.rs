use super::*;

const SR: f32 = 48_000.0;

// Reference state is explicitly independent per instance and per channel. It
// uses DSP primitives, never FxSlot/FxChain, to catch shared rack histories.
struct Reference {
    id: FxId,
    p: [f32; 4],
    mix: f32,
    env: f32,
    delay: [Delay; 2],
    reverb: [Reverb; 2],
    chorus: [Delay; 2],
    phase: f32,
    filter: [Svf; 2],
    eq: [[OnePole; 8]; 2],
    spread: [Delay; 2],
}

impl Reference {
    fn new(id: FxId, p: [f32; 4], mix: f32) -> Self {
        Self {
            id,
            p,
            mix,
            env: 0.0,
            delay: std::array::from_fn(|_| Delay::new((SR * 2.0) as usize)),
            reverb: std::array::from_fn(|_| Reverb::new()),
            chorus: std::array::from_fn(|_| {
                let mut delay = Delay::new((SR * 0.05) as usize);
                delay.fb = 0.0;
                delay
            }),
            phase: 0.0,
            filter: [Svf::default(); 2],
            eq: [[80.0, 160.0, 320.0, 640.0, 1280.0, 2560.0, 5120.0, 9000.0]
                .map(|hz| OnePole::lpf(SR, hz)); 2],
            spread: std::array::from_fn(|_| Delay::new((SR * 0.02) as usize)),
        }
    }

    fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.id == FxId::Chorus {
            self.phase = (self.phase + 0.7 / SR) % 1.0;
        }
        let gain = match self.id {
            FxId::Comp => {
                self.env = self.env * 0.995 + input[0].abs().max(input[1].abs()) * 0.005;
                let threshold = 0.05 + self.p[0] * 0.4;
                let reduction = if self.env > threshold {
                    threshold / self.env.max(1e-6)
                } else {
                    1.0
                };
                1.0 - self.p[1] + self.p[1] * reduction
            }
            FxId::Gate => {
                self.env = self.env * 0.98 + input[0].abs().max(input[1].abs()) * 0.02;
                if self.env < self.p[0] {
                    0.05
                } else {
                    1.0
                }
            }
            _ => 1.0,
        };
        let mut wet = [0.0; 2];
        for channel in 0..2 {
            let x = input[channel];
            wet[channel] = match self.id {
                FxId::Comp | FxId::Gate => x * gain,
                FxId::Delay => {
                    self.delay[channel].fb = self.p[1];
                    self.delay[channel].mix = 1.0;
                    self.delay[channel].tick(x)
                }
                FxId::Reverb => {
                    self.reverb[channel].mix = 1.0;
                    self.reverb[channel].tick(x)
                }
                FxId::Chorus => {
                    self.chorus[channel].time_samples =
                        SR * (0.008 + 0.006 * (self.phase * std::f32::consts::TAU).sin());
                    self.chorus[channel].mix = 1.0;
                    self.chorus[channel].tick(x)
                }
                FxId::Filter => {
                    self.filter[channel].process(x, 120.0 + self.p[1] * 8000.0, 0.35, SR, self.p[0])
                }
                FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => {
                    let bands = match self.id {
                        FxId::Eq3 => 3,
                        FxId::Eq5 => 5,
                        _ => 8,
                    };
                    let mut last = 0.0;
                    let mut y = 0.0;
                    for band in 0..bands {
                        let low = self.eq[channel][band].tick(x);
                        let split = if band == bands - 1 {
                            x - last
                        } else {
                            low - last
                        };
                        last = low;
                        let t = band as f32 / (bands - 1) as f32;
                        let level = if t < 0.5 {
                            self.p[0] + (self.p[1] - self.p[0]) * (t * 2.0)
                        } else {
                            self.p[1] + (self.p[2] - self.p[1]) * ((t - 0.5) * 2.0)
                        };
                        y += split * (0.25 + level * 1.5);
                    }
                    y
                }
                FxId::Spread => {
                    self.spread[channel].time_samples = (self.p[0] - 0.5).abs() * SR * 0.012;
                    self.spread[channel].fb = 0.0;
                    self.spread[channel].mix = 1.0;
                    self.spread[channel].tick(x)
                }
                FxId::Balance => {
                    let pan = (self.p[0] * 2.0 - 1.0).clamp(-1.0, 1.0);
                    x * if channel == 0 {
                        (1.0 - pan.max(0.0)).sqrt()
                    } else {
                        (1.0 + pan.min(0.0)).sqrt()
                    }
                }
                FxId::Dist => (x * (1.0 + self.p[0] * 8.0)).tanh(),
                FxId::Arp => x,
            };
        }
        if self.id == FxId::Spread {
            let width = self.p[0];
            wet = if width > 0.5 {
                [input[0], wet[1]]
            } else if width < 0.5 {
                let amount = (0.5 - width) * 2.0;
                let middle = (wet[0] + wet[1]) * 0.5;
                input.map(|x| x * (1.0 - amount) + middle * amount)
            } else {
                input
            };
        }
        std::array::from_fn(|channel| input[channel] * (1.0 - self.mix) + wet[channel] * self.mix)
    }
}

fn settings(id: FxId, second: bool) -> ([f32; 4], f32) {
    let p = match id {
        FxId::Gate => {
            if second {
                [0.04, 0.7, 0.0, 0.0]
            } else {
                [0.08, 0.2, 0.0, 0.0]
            }
        }
        FxId::Spread => {
            if second {
                [0.75, 0.0, 0.0, 0.0]
            } else {
                [0.95, 0.0, 0.0, 0.0]
            }
        }
        _ => {
            if second {
                [0.7, 0.4, 0.2, 0.0]
            } else {
                [0.2, 0.85, 0.9, 0.0]
            }
        }
    };
    (p, if second { 0.85 } else { 0.65 })
}

fn signal(frame: usize) -> [f32; 2] {
    let amplitude = if frame % 3072 < 1024 { 0.8 } else { 0.006 };
    [
        (frame as f32 * 0.037).sin() * amplitude,
        (frame as f32 * 0.023).cos() * amplitude * 0.7,
    ]
}

fn assert_frame(actual: [f32; 2], expected: [f32; 2], context: &str, frame: usize) {
    for channel in 0..2 {
        assert!(actual[channel].is_finite());
        assert!(
            (actual[channel] - expected[channel]).abs() < 1e-6,
            "{context}, frame {frame}, channel {channel}: actual={actual:?}, expected={expected:?}"
        );
    }
}

#[test]
fn duplicate_slots_match_independently_constructed_primitive_processors_in_series() {
    for &id in FxId::all() {
        let mut chain = FxChain::new(SR);
        let mut references = Vec::new();
        for second in [false, true] {
            let (p, mix) = settings(id, second);
            let mut slot = FxSlot::new(id, SR);
            slot.p = p;
            slot.mix = mix;
            chain.slots.push(slot);
            references.push(Reference::new(id, p, mix));
        }
        for frame in 0..30_000 {
            let input = signal(frame);
            let expected = references[0].process(input);
            let expected = references[1].process(expected);
            assert_frame(chain.process_stereo(input, SR), expected, id.name(), frame);
        }
    }
}

#[test]
fn mixed_dynamics_eq_and_stereo_slots_keep_independent_histories_in_order() {
    let mut chain = FxChain::new(SR);
    let mut references = Vec::new();
    for (index, id) in [
        FxId::Comp,
        FxId::Gate,
        FxId::Eq3,
        FxId::Eq8,
        FxId::Spread,
        FxId::Filter,
        FxId::Chorus,
        FxId::Balance,
        FxId::Delay,
        FxId::Reverb,
        FxId::Delay,
    ]
    .into_iter()
    .enumerate()
    {
        let (p, mix) = settings(id, index % 2 == 1);
        let mut slot = FxSlot::new(id, SR);
        slot.p = p;
        slot.mix = mix;
        chain.slots.push(slot);
        references.push(Reference::new(id, p, mix));
    }
    for frame in 0..30_000 {
        let input = signal(frame);
        let mut expected = input;
        for reference in &mut references {
            expected = reference.process(expected);
        }
        assert_frame(
            chain.process_stereo(input, SR),
            expected,
            "mixed chain",
            frame,
        );
    }
}

#[test]
fn two_fully_wet_delays_first_echo_at_twice_the_delay_time() {
    for count in [1, 2] {
        let mut chain = FxChain::new(SR);
        for _ in 0..count {
            let mut slot = FxSlot::new(FxId::Delay, SR);
            slot.mix = 1.0;
            slot.p[1] = 0.0;
            chain.slots.push(slot);
        }
        let mut first = None;
        for frame in 0..30_000 {
            let output = chain.tick(if frame == 0 { 1.0 } else { 0.0 }, SR);
            if output.abs() > 1e-6 && first.is_none() {
                first = Some(frame);
            }
            if frame == 12_000 * count {
                assert_eq!(output, 1.0);
            }
        }
        assert_eq!(first, Some(12_000 * count));
    }
}

#[test]
fn parameter_edits_keep_history_and_bypass_freezes_only_the_selected_slot() {
    for id in [
        FxId::Delay,
        FxId::Chorus,
        FxId::Reverb,
        FxId::Comp,
        FxId::Gate,
        FxId::Filter,
        FxId::Eq8,
        FxId::Spread,
    ] {
        let (p, mix) = settings(id, false);
        let mut chain = FxChain::new(SR);
        let mut reference = Reference::new(id, p, mix);
        let mut slot = FxSlot::new(id, SR);
        slot.p = p;
        slot.mix = mix;
        chain.slots.push(slot);
        let (second_p, second_mix) = settings(id, true);
        let mut second = FxSlot::new(id, SR);
        second.p = second_p;
        second.mix = second_mix;
        chain.slots.push(second);
        let mut unaffected = Reference::new(id, second_p, second_mix);
        for frame in 0..18_000 {
            if frame == 1_000 {
                chain.slots[0].on = false;
            }
            if frame == 2_000 {
                chain.slots[0].on = true;
            }
            if frame == 3_000 {
                let (p, mix) = settings(id, true);
                chain.slots[0].p = p;
                chain.slots[0].mix = mix;
                reference.p = p;
                reference.mix = mix;
            }
            let input = signal(frame);
            // The first slot fades for exactly 240 samples at 48 kHz;
            // its primitive history freezes only at the dry endpoint.
            let level = if (1_000..1_240).contains(&frame) {
                1.0 - (frame - 999) as f32 / 240.0
            } else if (1_240..2_000).contains(&frame) {
                0.0
            } else if (2_000..2_240).contains(&frame) {
                (frame - 1_999) as f32 / 240.0
            } else {
                1.0
            };
            let expected = if level == 0.0 {
                input
            } else {
                let wet = reference.process(input);
                std::array::from_fn(|channel| input[channel] * (1.0-level) + wet[channel] * level)
            };
            let expected = unaffected.process(expected);
            assert_frame(chain.process_stereo(input, SR), expected, id.name(), frame);
        }
    }
}

#[test]
fn deleting_a_slot_keeps_survivor_history_and_new_slot_starts_empty() {
    for id in [
        FxId::Delay,
        FxId::Chorus,
        FxId::Reverb,
        FxId::Comp,
        FxId::Gate,
        FxId::Filter,
        FxId::Eq3,
        FxId::Eq5,
        FxId::Eq8,
        FxId::Spread,
    ] {
        let (p, mix) = settings(id, false);
        let mut chain = FxChain::new(SR);
        let mut reference_a = Reference::new(id, p, mix);
        let mut reference_b = Reference::new(id, p, mix);
        for _ in 0..2 {
            let mut slot = FxSlot::new(id, SR);
            slot.p = p;
            slot.mix = mix;
            chain.slots.push(slot);
        }
        for frame in 0..15_000 {
            let input = signal(frame);
            let expected = reference_b.process(reference_a.process(input));
            assert_frame(chain.process_stereo(input, SR), expected, id.name(), frame);
        }
        chain.slots.remove(0);
        for frame in 15_000..16_000 {
            let input = signal(frame);
            assert_frame(
                chain.process_stereo(input, SR),
                reference_b.process(input),
                id.name(),
                frame,
            );
        }
        let mut fresh = FxSlot::new(id, SR);
        fresh.p = p;
        fresh.mix = mix;
        chain.slots.insert(0, fresh);
        let mut fresh_reference = Reference::new(id, p, mix);
        for frame in 16_000..32_000 {
            let input = signal(frame);
            let expected = reference_b.process(fresh_reference.process(input));
            assert_frame(chain.process_stereo(input, SR), expected, id.name(), frame);
        }
    }
}

#[test]
fn stereo_processors_do_not_leak_an_active_channel_into_the_other() {
    for &id in FxId::all() {
        if id == FxId::Spread {
            continue;
        } // Width narrowing deliberately mixes channels.
        let mut chain = FxChain::new(SR);
        let mut slot = FxSlot::new(id, SR);
        slot.mix = 1.0;
        chain.slots.push(slot);
        for frame in 0..14_000 {
            let result = chain.process_stereo([signal(frame)[0], 0.0], SR);
            assert_eq!(
                result[1],
                0.0,
                "{} leaked left into right at {frame}",
                id.name()
            );
        }
    }
}

#[test]
fn empty_chain_is_identity_and_slots_allocate_only_their_processor() {
    let mut empty = FxChain::new(SR);
    assert_eq!(empty.process_stereo([0.4, -0.8], SR), [0.4, -0.8]);
    eprintln!("inline slot bytes: {}", std::mem::size_of::<FxSlot>());
    for &id in FxId::all() {
        let mut slot = None;
        let counts = crate::engine::test_alloc::measure(|| slot = Some(FxSlot::new(id, SR)));
        let expected = match id {
            FxId::Delay => 768_000,
            FxId::Reverb => 115_680,
            FxId::Chorus => 19_200,
            FxId::Spread => 7_680,
            _ => 0,
        };
        assert_eq!(counts.bytes, expected, "{} processor allocation", id.name());
        eprintln!(
            "{}: {} heap bytes in {} allocations",
            id.name(),
            counts.bytes,
            counts.allocations
        );
        std::hint::black_box(slot);
    }
}
