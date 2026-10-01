use super::*;

fn input(frame: usize) -> [f32; 2] {
    let amplitude = [0.02, 0.18, 0.38, 0.58, 0.78, 0.98][(frame / 8000).min(5)];
    let noise = ((frame as u32)
        .wrapping_mul(1664525)
        .wrapping_add(1013904223)
        >> 8) as f32
        / 16777216.0
        - 0.5;
    [
        amplitude * (0.7 + 0.15 * (frame as f32 * 0.37).sin() + 0.15 * noise),
        amplitude * (-0.5 + 0.2 * (frame as f32 * 0.081).cos() + 0.3 * noise),
    ]
}

#[test]
fn every_exposed_control_has_a_measurable_sweep_and_matches_an_independent_dsp_reference() {
    let sr = 48000.0;
    let mut parameters = 0;
    let mut wet_controls = 0;
    for &id in FxId::all() {
        for &control in id.controls() {
            assert!(
                !control.name.is_empty()
                    && !control.unit.is_empty()
                    && !control.consumer.is_empty()
            );
            assert!(control.max > control.min);
            if control.parameter.is_some() {
                parameters += 1;
            } else {
                wet_controls += 1;
            }
            let mut previous: Option<Vec<[f32; 2]>> = None;
            for value in [0.0, 0.25, 0.5, 0.75, 1.0] {
                assert!((control.normalized(control.display(value)) - value).abs() < 1e-6);
                let mut slot = FxSlot::new(id, sr);
                slot.mix = 1.0;
                if control.parameter.is_none() {
                    match id {
                        FxId::Spread | FxId::Balance => slot.p[0] = 0.9,
                        FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => slot.p = [0.2, 0.8, 0.3, 0.5],
                        _ => {}
                    }
                }
                assert!(slot.set_control(control.parameter, value));
                let mut reference = super::tests::Reference::new(id, slot.p, slot.mix);
                let mut output = Vec::with_capacity(48000);
                let mut difference = 0.0f64;
                for frame in 0..48000 {
                    let source = input(frame);
                    let actual = slot.tick_stereo(source, sr);
                    let expected = reference.process(source);
                    for channel in 0..2 {
                        assert!(
                            (actual[channel] - expected[channel]).abs() < 1e-6,
                            "{id:?} {}={value} frame{frame}",
                            control.name
                        );
                        if let Some(previous) = &previous {
                            difference += (actual[channel] - previous[frame][channel]).abs() as f64;
                        }
                    }
                    output.push(actual);
                }
                if previous.is_some() {
                    assert!(
                        difference > 0.01,
                        "inert sweep: {id:?} {}={value}",
                        control.name
                    );
                }
                previous = Some(output);
            }
        }
    }
    assert_eq!((parameters, wet_controls), (18, 12));
}

#[test]
fn fixed_delay_time_p0_endpoints_remain_identical_and_feedback_controls_repeat_level() {
    for sr in [44100.0, 48000.0, 96000.0] {
        let mut endpoints = [FxSlot::new(FxId::Delay, sr), FxSlot::new(FxId::Delay, sr)];
        endpoints[0].p[0] = 0.0;
        endpoints[1].p[0] = 1.0;
        for slot in &mut endpoints {
            slot.mix = 1.0;
        }
        assert!(!FxId::Delay
            .controls()
            .iter()
            .any(|c| c.parameter == Some(0)));
        let first = (sr * 0.25) as usize;
        let mut observed = None;
        for frame in 0..first * 3 {
            let source = if frame == 0 { [1.0, -0.5] } else { [0.0; 2] };
            let a = endpoints[0].tick_stereo(source, sr);
            let b = endpoints[1].tick_stereo(source, sr);
            assert_eq!(a, b);
            if observed.is_none() && a[0] != 0.0 {
                observed = Some(frame);
            }
        }
        assert_eq!(observed, Some(first));
        for feedback in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let mut slot = FxSlot::new(FxId::Delay, sr);
            slot.mix = 1.0;
            assert!(slot.set_control(Some(1), feedback));
            for frame in 0..=first * 2 {
                let output = slot.tick_stereo(if frame == 0 { [1.0, 0.0] } else { [0.0; 2] }, sr);
                if frame == first {
                    assert_eq!(output, [1.0, 0.0]);
                }
                if frame == first * 2 {
                    assert_eq!(output, [feedback, 0.0]);
                }
            }
        }
    }
}

#[test]
fn unsupported_indices_nonfinite_values_and_arp_mix_cannot_be_edited_as_parameters() {
    for &id in FxId::all() {
        let mut slot = FxSlot::new(id, 48000.0);
        for index in 0..=u8::MAX {
            let before = slot.p;
            let supported = id
                .controls()
                .iter()
                .any(|control| control.parameter == Some(index));
            assert_eq!(slot.set_control(Some(index), 0.9), supported);
            if !supported {
                assert_eq!(slot.p, before);
            }
        }
        for control in id.controls() {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let before = (slot.mix, slot.p);
                assert!(!slot.set_control(control.parameter, value));
                assert_eq!((slot.mix, slot.p), before);
            }
        }
    }
    assert!(FxId::Arp.controls().is_empty());
    let mut arp = FxSlot::new(FxId::Arp, 48000.0);
    assert!(!arp.set_control(None, 0.9));
    assert!(!FxId::Arp.supports_scene());
}
