use super::*;

fn close(actual: [f32; 2], expected: [f32; 2], id: FxId, mix: f32, frame: usize) {
    for channel in 0..2 {
        assert!(actual[channel].is_finite());
        assert!(
            (actual[channel] - expected[channel]).abs() <= 1e-6,
            "{} mix={mix} frame={frame}: {actual:?} != {expected:?}",
            id.name()
        );
    }
}

#[test]
fn time_effect_impulses_have_single_linear_dry_and_wet_coefficients() {
    for sr in [32_000.0, 44_100.0, 48_000.0, 96_000.0] {
        for id in [FxId::Delay, FxId::Reverb, FxId::Chorus] {
            let mut reference = FxSlot::new(id, sr);
            reference.mix = 1.0;
            let mut levels: Vec<_> = [0.0, 0.25, 0.5, 1.0]
                .map(|mix| {
                    let mut slot = FxSlot::new(id, sr);
                    slot.mix = mix;
                    slot
                })
                .into();
            let mut saw_wet = false;
            for frame in 0..(sr * 0.45) as usize {
                let input = if frame == 0 { [0.8, -0.4] } else { [0.0; 2] };
                let wet = reference.tick_stereo(input, sr);
                if frame == 0 {
                    assert_eq!(
                        wet, [0.0; 2],
                        "fully wet must contain no immediate dry impulse"
                    );
                } else {
                    saw_wet |= wet.iter().any(|x| x.abs() > 1e-7);
                }
                for slot in &mut levels {
                    let mix = slot.mix;
                    let expected =
                        std::array::from_fn(|ch| input[ch] * (1.0 - mix) + wet[ch] * mix);
                    close(slot.tick_stereo(input, sr), expected, id, mix, frame);
                }
            }
            assert!(
                saw_wet,
                "{} at {sr} must have a nonempty wet window",
                id.name()
            );
        }
    }
}

#[test]
fn moving_mix_does_not_scale_stored_time_effect_history_twice() {
    for id in [FxId::Delay, FxId::Reverb, FxId::Chorus] {
        let sr = 48_000.0;
        let mut wet = FxSlot::new(id, sr);
        wet.mix = 1.0;
        let mut actual = FxSlot::new(id, sr);
        for frame in 0..48_000 {
            let mix = [0.0, 0.25, 0.5, 1.0][(frame / 149) % 4];
            actual.mix = mix;
            let input = if frame < 8_000 {
                [
                    (frame as f32 * 0.071).sin() * 0.4,
                    (frame as f32 * 0.039).cos() * 0.25,
                ]
            } else {
                [0.0; 2]
            };
            let reference = wet.tick_stereo(input, sr);
            let expected = std::array::from_fn(|ch| input[ch] * (1.0 - mix) + reference[ch] * mix);
            close(actual.tick_stereo(input, sr), expected, id, mix, frame);
        }
    }
}

#[test]
fn time_effects_share_the_existing_linear_law_of_other_processed_slots() {
    for id in [
        FxId::Comp,
        FxId::Gate,
        FxId::Dist,
        FxId::Filter,
        FxId::Eq3,
        FxId::Eq5,
        FxId::Eq8,
        FxId::Arp,
    ] {
        let sr = 48_000.0;
        let mut wet = FxSlot::new(id, sr);
        wet.p = [0.25, 0.75, 0.2, 0.0];
        wet.mix = 1.0;
        let mut levels: Vec<_> = [0.0, 0.25, 0.5, 1.0]
            .map(|mix| {
                let mut slot = wet.clone();
                slot.mix = mix;
                slot
            })
            .into();
        for frame in 0..4096 {
            let input = [
                (frame as f32 * 0.047).sin() * 0.8,
                (frame as f32 * 0.019).cos() * 0.3,
            ];
            let reference = wet.tick_stereo(input, sr);
            for slot in &mut levels {
                let mix = slot.mix;
                let expected =
                    std::array::from_fn(|ch| input[ch] * (1.0 - mix) + reference[ch] * mix);
                close(slot.tick_stereo(input, sr), expected, id, mix, frame);
            }
        }
    }
}
