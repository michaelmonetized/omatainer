use super::*;
use crate::engine::{deck_filter::{ChannelFilter, Curve}, test_alloc};

#[test]
fn actual_deck_renderer_sweep_type_changes_and_center_return_are_bounded_and_heap_free() {
    use crate::engine::{Command, Engine, dsp::Sample};
    let (engine, mut actual) = Engine::headless_for_test(48000, 256);
    let (_, mut reference) = Engine::headless_for_test(48000, 256);
    let audio = std::sync::Arc::new(Sample { spectrum: None, name: "Stereo channel qualification".into(), sr: 48000, ch: 2, data: (0..480000).flat_map(|frame| { let phase = frame as f32 * std::f32::consts::TAU / 48000.0; [(phase * 311.0).sin() * 0.1, (phase * 5011.0).sin() * 0.07] }).collect(), peaks: vec![].into(), bpm: 120.0, path: String::new() });
    for rt in [&mut actual, &mut reference] { rt.apply(Command::DeckAudio { deck: 0, audio: audio.clone() }); rt.decks[0].playing = true; rt.decks[0].rate = 1.0; rt.decks[1].playing = false; rt.xfader = 0.0; rt.master = 1.0; }
    let mut changed = 0; let mut peak = 0.0_f32;
    for kind in [Kind::Echo, Kind::Room, Kind::Filter, Kind::Room, Kind::Echo] {
        engine.send(Command::DeckChannelEffect { deck: 0, effect: kind }).unwrap();
        for value in [0.0, 0.2, 0.5, 0.8, 1.0] {
            engine.send(Command::DeckFilter { deck: 0, value }).unwrap();
            for _ in 0..8 { let mut output = [0.0; 512]; let mut dry = [0.0; 512]; assert_eq!(test_alloc::measure(|| actual.process(&mut output)), test_alloc::Counts::default()); reference.process(&mut dry); assert!(output.iter().all(|value| value.is_finite())); peak = output.iter().fold(peak, |peak, value| peak.max(value.abs())); if output != dry { changed += 1; } }
        }
    }
    assert!(changed > 100); assert!(peak < 0.3, "channel sweep peak {peak}");
    engine.send(Command::DeckFilter { deck: 0, value: 0.5 }).unwrap();
    let mut exact = 0;
    for block in 0..20 { let mut output = [0.0; 512]; let mut dry = [0.0; 512]; actual.process(&mut output); reference.process(&mut dry); if block >= 2 { assert_eq!(output, dry); assert!(output.iter().any(|value| *value != 0.0)); exact += 1; } }
    println!("CHANNEL_EFFECT_RENDERED {{\"sweep_blocks\":200,\"changed_blocks\":{changed},\"exact_dry_center_blocks\":{exact},\"peak\":{peak},\"callback_heap_work\":false,\"physical_devices_opened\":false}}");
}

#[test]
fn neutral_detent_type_reversals_and_stereo_isolation_are_sample_exact_without_heap_work() {
    for rate in [8000.0, 44100.0, 48000.0, 96000.0, 192000.0] {
        let mut channel = Channel::new(rate).unwrap();
        for kind in Kind::ALL {
            for position in [0.47, 0.5, 0.53] {
                for frame in 0..(rate * 0.006) as usize {
                    let input = [0.1 * (frame as f32 * 0.07).sin(), -0.2 * (frame as f32 * 0.03).cos()];
                    assert_eq!(channel.process(input, input, kind, position, f64::from(rate) * 0.5, rate), input);
                }
            }
        }
        for kind in [Kind::Echo, Kind::Room] {
            for position in [0.0, 1.0] {
                for impulse_channel in 0..2 {
                    let mut channel = Channel::new(rate).unwrap();
                    for frame in 0..(rate * 0.5) as usize {
                        let mut input = [0.0; 2];
                        if frame == 0 { input[impulse_channel] = 0.25; }
                        let mut output = [0.0; 2];
                        let counts = test_alloc::measure(|| output = channel.process(input, input, kind, position, f64::from(rate) * 0.5, rate));
                        assert_eq!(counts, test_alloc::Counts::default());
                        assert_eq!(output[1 - impulse_channel], 0.0);
                        assert!(output.iter().all(|sample| sample.is_finite() && sample.abs() <= 0.5));
                    }
                }
            }
        }
    }
}

#[test]
fn echo_sides_follow_the_musical_clock_and_center_retires_owned_history() {
    let rate = 8000.0;
    for (position, delay) in [(0.0, 1000), (1.0, 3000)] {
        let mut channel = Channel::new(rate).unwrap();
        for _ in 0..40 { channel.process([0.0; 2], [0.0; 2], Kind::Echo, position, 4000.0, rate); }
        let mut echoes = Vec::new();
        for frame in 0..=delay * 2 {
            let input = if frame == 0 { [0.25, -0.125] } else { [0.0; 2] };
            let output = channel.process(input, input, Kind::Echo, position, 4000.0, rate);
            if output != [0.0; 2] { echoes.push((frame, output)); }
        }
        assert_eq!(echoes.len(), 3);
        assert_eq!(echoes.iter().map(|(frame, _)| *frame).collect::<Vec<_>>(), [0, delay, delay * 2]);
        for ((_, frame), expected) in echoes.iter().zip([[0.1, -0.05], [0.15, -0.075], [0.0825, -0.04125]]) {
            for (actual, expected) in frame.iter().zip(expected) { assert!((actual - expected).abs() < 1e-6); }
        }
        channel.process([0.0; 2], [0.0; 2], Kind::Echo, 0.5, 4000.0, rate);
        for _ in 0..delay * 3 { assert_eq!(channel.process([0.0; 2], [0.0; 2], Kind::Echo, position, 4000.0, rate), [0.0; 2]); }
    }
}

#[test]
fn nonneutral_type_changes_complete_in_five_milliseconds_and_rapid_changes_keep_convex_gains() {
    let mut max_jump = 0.0_f32;
    for rate in [8000.0, 44100.0, 48000.0, 96000.0, 192000.0] {
        let mut channel = Channel::new(rate).unwrap();
        let input = [0.1, -0.1];
        let filtered = [0.07, -0.07];
        let mut last = filtered;
        for kind in [Kind::Echo, Kind::Room, Kind::Filter, Kind::Room, Kind::Echo] {
            let length = (rate * 0.005).ceil() as usize;
            for _ in 0..length {
                let output = channel.process(input, filtered, kind, 0.0, f64::from(rate) * 0.5, rate);
                max_jump = max_jump.max((output[0] - last[0]).abs());
                assert!((output[0] + output[1]).abs() < 1e-7);
                assert!(output[0].abs() <= 0.3);
                assert!((channel.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                assert!(channel.weights.iter().all(|weight| (0.0..=1.0).contains(weight)));
                last = output;
            }
            assert_eq!(channel.remaining, 0);
            assert_eq!(channel.weights[kind.index()], 1.0);
        }
        for frame in 0..1000 {
            let kind = Kind::ALL[frame % 3];
            let output = channel.process(input, filtered, kind, 0.0, f64::from(rate) * 0.5, rate);
            max_jump = max_jump.max((output[0] - last[0]).abs());
            assert!(channel.weights.iter().all(|weight| (0.0..=1.0).contains(weight)));
            last = output;
        }
    }
    assert!(max_jump <= 0.003, "type change jump {max_jump}");
    println!("CHANNEL_EFFECT_TRANSITION {{\"five_ms_exact\":true,\"max_step\":{max_jump},\"physical_devices_opened\":false}}");
}

#[test]
fn selecting_filter_after_wet_processing_preserves_the_legacy_sweep_and_retires_previous_effects() {
    let mut channel = Channel::new(48000.0).unwrap();
    let mut filters = [ChannelFilter::default(); 2];
    for _ in 0..1000 { channel.process([0.1, -0.1], [0.02, -0.02], Kind::Echo, 0.0, 24000.0, 48000.0); }
    for frame in 0..2000 {
        let input = [(frame as f32 * 0.01).sin() * 0.1, (frame as f32 * 0.07).cos() * 0.1];
        let filtered = std::array::from_fn(|i| filters[i].process(input[i], Curve::at(0.2, 48000.0)));
        let output = channel.process(input, filtered, Kind::Filter, 0.2, 24000.0, 48000.0);
        if frame >= 240 { assert_eq!(output, filtered); }
    }
    assert_eq!(channel.sides, [0; 2]);
    for _ in 0..240 { channel.process([0.0; 2], [0.0; 2], Kind::Echo, 0.0, 24000.0, 48000.0); }
    for _ in 0..12000 { assert_eq!(channel.process([0.0; 2], [0.0; 2], Kind::Echo, 0.0, 24000.0, 48000.0), [0.0; 2]); }
}
