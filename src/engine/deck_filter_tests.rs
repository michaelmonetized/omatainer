use super::*;

const PERIOD: usize = 4096;

fn engine(sr: u32) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn load(rt: &mut RtEngine, deck: usize, period: &[[f32; 2]]) {
    let mut data = vec![0.0; 2];
    for _ in 0..4 {
        for frame in period {
            data.extend_from_slice(frame);
        }
    }
    data.extend_from_slice(&[0.0; 2]);
    rt.apply(Command::DeckAudio {
        deck: deck as u8,
        audio: Arc::new(Sample {
            name: "filter broadband".into(),
            sr: rt.sr as u32,
            ch: 2,
            data,
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.apply(Command::DeckGain {
        deck: deck as u8,
        value: 1.0,
    });
    rt.decks[deck].playing = true;
    rt.decks[deck].transition_remaining = 0;
}

fn broadband(sr: u32) -> (Vec<[f32; 2]>, Vec<usize>) {
    let bins: Vec<_> = [
        65.0, 135.0, 275.0, 550.0, 1200.0, 2800.0, 5000.0, 7500.0, 11000.0, 16000.0,
    ]
    .map(|hz| (hz * PERIOD as f32 / sr as f32).round() as usize)
    .into();
    let frames = (0..PERIOD)
        .map(|frame| {
            let mut result = [0.0; 2];
            for (index, &bin) in bins.iter().enumerate() {
                let phase = std::f64::consts::TAU * bin as f64 * frame as f64 / PERIOD as f64;
                result[0] += 0.025 * phase.sin() as f32;
                result[1] += 0.02 * (phase + index as f64 * 0.31).cos() as f32;
            }
            result
        })
        .collect();
    (frames, bins)
}

fn response(rt: &mut RtEngine, deck: usize, signal: &[[f32; 2]], position: f32) -> Vec<[f32; 2]> {
    load(rt, deck, signal);
    rt.apply(Command::DeckFilter {
        deck: deck as u8,
        value: position,
    });
    // Discard the bounded control slew and full source/filter warm-up period.
    for _ in 0..PERIOD {
        rt.render_deck(deck);
    }
    (0..PERIOD)
        .map(|_| {
            let (l, r) = rt.render_deck(deck);
            [l, r]
        })
        .collect()
}

fn spectral_energy(output: &[[f32; 2]], bins: &[usize], channel: usize) -> f64 {
    bins.iter()
        .map(|&bin| {
            let (mut real, mut imag) = (0.0, 0.0);
            for (frame, value) in output.iter().enumerate() {
                let phase = std::f64::consts::TAU * bin as f64 * frame as f64 / PERIOD as f64;
                real += value[channel] as f64 * phase.cos();
                imag += value[channel] as f64 * phase.sin();
            }
            real * real + imag * imag
        })
        .sum()
}

#[test]
fn actual_deck_broadband_sweep_attenuates_each_half_monotonically() {
    for sr in [44_100, 48_000, 96_000] {
        let mut rt = engine(sr);
        let (signal, bins) = broadband(sr);
        for deck in 0..DECKS {
            let dry = response(&mut rt, deck, &signal, 0.5);
            for (low_pass, measured_bins) in [(true, &bins[6..]), (false, &bins[..4])] {
                let dry_energy =
                    [0, 1].map(|channel| spectral_energy(&dry, measured_bins, channel));
                let mut previous = [1.0; 2];
                for step in 0..=40 {
                    let distance = step as f32 / 40.0;
                    let position = if low_pass {
                        0.47 * (1.0 - distance)
                    } else {
                        0.53 + 0.47 * distance
                    };
                    let out = response(&mut rt, deck, &signal, position);
                    for channel in 0..2 {
                        let ratio =
                            spectral_energy(&out, measured_bins, channel) / dry_energy[channel];
                        assert!(ratio <= previous[channel] + 2e-6,
                            "nonmonotonic sr={sr} deck={deck} LP={low_pass} step={step} ch={channel}: {ratio} > {}", previous[channel]);
                        previous[channel] = ratio;
                    }
                }
                assert!(
                    previous.iter().all(|ratio| *ratio < 0.0001),
                    "endpoint must remove the rejected band: {previous:?}"
                );
            }
        }
    }
}

#[test]
fn bypass_and_both_thresholds_are_continuous_for_broadband_audio() {
    for sr in [44_100, 48_000, 96_000] {
        let mut rt = engine(sr);
        let (signal, _) = broadband(sr);
        for deck in 0..DECKS {
            let dry = response(&mut rt, deck, &signal, 0.5);
            for position in [0.47, 0.48, 0.5, 0.52, 0.53] {
                assert_eq!(response(&mut rt, deck, &signal, position), dry);
            }
            for position in [0.47 - 1e-5, 0.47 + 1e-5, 0.53 - 1e-5, 0.53 + 1e-5] {
                let out = response(&mut rt, deck, &signal, position);
                let error = out
                    .iter()
                    .zip(&dry)
                    .flat_map(|(a, b)| [(a[0] - b[0]).abs(), (a[1] - b[1]).abs()])
                    .fold(0.0f32, f32::max);
                assert!(error < 2e-6, "threshold {position} at {sr}: {error}");
            }
        }
    }
}

#[test]
fn control_slew_keeps_center_crossing_bounded_and_clears_old_history() {
    for sr in [44_100, 48_000, 96_000] {
        let mut rt = engine(sr);
        let signal = vec![[0.25, -0.2]; PERIOD];
        load(&mut rt, 0, &signal);
        let frames = (0.005 * sr as f32).ceil() as usize + 2;
        for target in [0.0, 0.5, 1.0, 0.5, 0.469, 0.531, 0.5] {
            let previous = rt.decks[0].filter_position;
            rt.apply(Command::DeckFilter {
                deck: 0,
                value: target,
            });
            assert_eq!(
                rt.decks[0].filter_position, previous,
                "command cannot jump the rendered control"
            );
            let mut last = rt.render_deck(0);
            let mut largest_change = 0.0f32;
            for _ in 1..frames {
                let now = rt.render_deck(0);
                largest_change = largest_change
                    .max((now.0 - last.0).abs())
                    .max((now.1 - last.1).abs());
                last = now;
            }
            assert_eq!(rt.decks[0].filter_position, target);
            assert!(
                largest_change < 0.025,
                "sr={sr} target={target}: {largest_change}"
            );
            if target == 0.5 {
                assert!((last.0 - 0.25).abs() < 1e-7 && (last.1 + 0.2).abs() < 1e-7);
                assert!(rt.decks[0]
                    .filter
                    .iter()
                    .all(|filter| filter.history == [0.0; 2]));
            }
        }
    }
}

#[test]
fn deck_filter_identity_and_independent_scalar_reference_are_exact() {
    // Pseudorandom stereo plus impulses: the processor itself must be exactly
    // transparent in the deadband, before any existing EQ arithmetic.
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for position in [0.0, 0.2, 0.46999, 0.47, 0.5, 0.53, 0.53001, 0.8, 1.0] {
            let mut filters = [deck_filter::ChannelFilter::default(); 2];
            let mut reference = [[0.0f32; 2]; 2];
            let mut seed = 91u32;
            let distance = (((position - 0.5f32).abs() - 0.03).max(0.0) / 0.47).min(1.0);
            let t = (distance * 5.0).min(1.0);
            let activation = t * t * (3.0 - 2.0 * t);
            for frame in 0..8192 {
                for channel in 0..2 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let x = if frame == 0 {
                        1.0
                    } else {
                        (seed as i32 as f32 / i32::MAX as f32) * 0.5
                    };
                    let mut expected = x;
                    if distance > f32::EPSILON {
                        if position < 0.5 {
                            let retention = activation
                                * (-std::f32::consts::TAU
                                    * (18000.0 * (60.0f32 / 18000.0).powf(distance))
                                    / sr)
                                    .exp();
                            for state in &mut reference[channel] {
                                *state = expected + retention * (*state - expected);
                                expected = *state;
                            }
                        } else {
                            let amount = 1.0
                                - (-std::f32::consts::TAU
                                    * (20.0 * (8000.0f32 / 20.0).powf(distance))
                                    / sr)
                                    .exp();
                            for state in &mut reference[channel] {
                                *state += amount * (expected - *state);
                                expected -= activation * *state;
                            }
                        }
                    }
                    let actual = filters[channel].process(x, deck_filter::Curve::at(position, sr));
                    assert_eq!(actual, expected);
                    if distance <= f32::EPSILON {
                        assert_eq!(actual.to_bits(), x.to_bits());
                    }
                }
            }
        }
    }
}
