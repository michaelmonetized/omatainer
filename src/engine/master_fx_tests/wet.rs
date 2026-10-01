//! Issue #62 closes the old third-wet gap with a mapped endpoint regression.
//! The independent three-slot renderer itself was implemented by issue #55.
use super::*;

#[test]
fn mapped_third_wet_endpoints_match_dry_and_default_filter_audio() {
    for sr in [44_100, 48_000, 96_000] {
        let mut endpoints = Vec::new();
        for value in [0, 127] {
            let (mut rt, commands, hub) = engine(sr);
            assert_eq!(rt.fx_kind, [FxKind::Echo, FxKind::Reverb, FxKind::Filter]);
            // Exact existing legacy NS7FX factory address: channel 1, CC 0x32.
            // Exercise its MIDI callback/worker, command admission and renderer.
            message(&hub, &commands, &[0xb0, 0x32, value]);
            rt.process(&mut []);
            let wet = value as f32 / 127.0;
            assert_eq!(rt.fx_wet, [0.0, 0.0, wet]);
            rt.publish_for_test();
            {
                let snapshot = rt.snap.lock();
                assert_eq!(
                    snapshot.fx_kind,
                    [FxKind::Echo, FxKind::Reverb, FxKind::Filter]
                );
                assert_eq!(snapshot.fx_wet, [0.0, 0.0, wet]);
            }

            let frames = sr as usize / 4;
            signal(&mut rt, frames);
            let mut output = vec![0.0; frames * 2];
            rt.process(&mut output);
            let mut eq = [ThreeBand::new(sr as f32); 2];
            let mut filter = [[dsp::OnePole::lpf(sr as f32, 1000.0); 2]; 2];
            for (frame, actual) in output.chunks_exact(2).enumerate() {
                let phase = std::f32::consts::TAU * frame as f32 / sr as f32;
                let source = [
                    (phase * 311.0).sin() * 0.2 + if frame == 0 { 0.15 } else { 0.0 },
                    (phase * 5011.0).sin() * 0.13,
                ];
                for channel in 0..2 {
                    let dry = eq[channel].tick(source[channel]);
                    let [first, second] = &mut filter[channel];
                    let filtered = second.tick(first.tick(dry));
                    let expected = (dry * (1.0 - wet) + filtered * wet).tanh();
                    assert!(
                        (actual[channel] - expected).abs() < 2e-6,
                        "sr{sr} wet{wet} frame{frame} channel{channel}: {} vs {expected}",
                        actual[channel]
                    );
                }
            }
            endpoints.push(output);
        }
        let energy = |output: &[f32], channel: usize| -> f32 {
            output
                .chunks_exact(2)
                .map(|frame| frame[channel].powi(2))
                .sum()
        };
        let dry = &endpoints[0];
        let wet = &endpoints[1];
        for channel in 0..2 {
            assert!(energy(dry, channel) > 1.0);
            assert!(energy(wet, channel) > 0.001);
        }
        assert!(
            energy(wet, 1) < energy(dry, 1) * 0.01,
            "high band was not filtered"
        );
        assert!(
            energy(wet, 0) > energy(dry, 0) * 0.5,
            "filter muted its pass band"
        );
        let difference: f32 = wet
            .iter()
            .zip(dry)
            .map(|(wet, dry)| (wet - dry).abs())
            .sum();
        assert!(
            difference > sr as f32 * 0.005,
            "slot 2 wet was audibly inert"
        );
    }
}

#[test]
fn every_factory_fx_binding_targets_a_supported_observable_slot() {
    let maps = midi::builtin_maps().unwrap();
    let mut wet_controls = 0;
    let mut selectors = 0;
    let mut third_controls = 0;
    for map in maps {
        let bindings: Vec<_> = map
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.action, midi::Action::FxWet | midi::Action::FxSelect)
            })
            .collect();
        if bindings.is_empty() {
            continue;
        }
        let (mut rt, commands, hub) = engine(48_000);
        let device = &map.matchers[0];
        for binding in bindings {
            assert!(
                binding.extra < 3,
                "{} has unsupported FX slot {}",
                map.name,
                binding.extra
            );
            let slot = binding.extra as usize;
            let channel = if binding.ch == 0xff { 0 } else { binding.ch };
            match binding.action {
                midi::Action::FxWet => {
                    assert_eq!(binding.kind, midi::MsgKind::Cc);
                    for value in [127, 0] {
                        let mut expected = rt.fx_wet;
                        expected[slot] = value as f32 / 127.0;
                        hub.receive_for_test(
                            &commands,
                            620,
                            device,
                            &[0xb0 | channel, binding.data, value],
                        );
                        rt.process(&mut []);
                        rt.publish_for_test();
                        assert_eq!(rt.fx_wet, expected, "{} CC{}", map.name, binding.data);
                        assert_eq!(rt.snap.lock().fx_wet, expected);
                        assert_eq!(rt.snap.lock().fx_kind, rt.fx_kind);
                    }
                    wet_controls += 1;
                    third_controls += usize::from(slot == 2);
                }
                midi::Action::FxSelect => {
                    assert_eq!(binding.kind, midi::MsgKind::Note);
                    let mut expected = rt.fx_kind;
                    expected[slot] = expected[slot].next();
                    hub.receive_for_test(
                        &commands,
                        620,
                        device,
                        &[0x90 | channel, binding.data, 127],
                    );
                    rt.process(&mut []);
                    rt.publish_for_test();
                    assert_eq!(rt.fx_kind, expected, "{} note{}", map.name, binding.data);
                    assert_eq!(rt.snap.lock().fx_kind, expected);
                    selectors += 1;
                }
                _ => unreachable!(),
            }
        }
    }
    assert!(wet_controls > 0 && selectors > 0);
    assert!(
        third_controls > 0,
        "the original slot-2 factory mappings disappeared"
    );
    println!("validated {wet_controls} wet bindings ({third_controls} third-slot) and {selectors} selectors");
}
