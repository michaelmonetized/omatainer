use super::*;

fn engine(sr: u32, input: &[[f32; 2]]) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    let mut rt = RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    // Render quiet preroll before the measured signal so any deck-load fade
    // has settled. Deck rendering advances before reading its source frame.
    const PREROLL: usize = 256;
    let mut data = vec![0.0; (PREROLL + 1) * 2];
    data.extend(input.iter().flatten().copied());
    data.extend_from_slice(&[0.0; 2]);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample {
            name: "master stereo fixture".into(),
            sr,
            ch: 2,
            data,
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.decks[0].playing = true;
    rt.decks[0].gain = 1.0;
    rt.decks[1].audio = None;
    rt.master = 1.0;
    rt.xfader = 0.0;
    rt.bpm = 120.0;
    rt.process(&mut [0.0; PREROLL * 2]);
    rt
}

fn render(rt: &mut RtEngine, frames: usize, block_frames: usize) -> Vec<[f32; 2]> {
    let mut output = vec![0.0; frames * 2];
    for block in output.chunks_mut(block_frames * 2) {
        rt.process(block);
    }
    output
        .chunks_exact(2)
        .map(|frame| [frame[0], frame[1]])
        .collect()
}

#[test]
fn master_echo_arrives_in_audio_frames_at_each_tempo_and_sample_rate() {
    for sr in [44_100, 48_000] {
        for (bpm, block_frames) in [(90.0, 64), (120.0, 511), (180.0, 1024)] {
            let delay_frames = sr as f64 * 60.0 / bpm as f64 * 0.75;
            let frames = delay_frames.ceil() as usize + 4;
            for channel in 0..2 {
                let mut input = vec![[0.0; 2]; frames];
                input[0][channel] = 0.5;
                let mut rt = engine(sr, &input);
                rt.apply(Command::SetBpm(bpm));
                rt.apply(Command::FxWet {
                    slot: 0,
                    value: 1.0,
                });
                let output = render(&mut rt, frames, block_frames);
                let first = output
                    .iter()
                    .position(|sample| sample[channel].abs() > 1e-6);
                assert_eq!(
                    first,
                    Some(delay_frames.floor() as usize),
                    "{sr} Hz, {bpm} BPM, channel {channel}"
                );
                assert!(
                    output.iter().all(|sample| sample[1 - channel] == 0.0),
                    "echo crossed channels"
                );
                assert_eq!(rt.master_fx[0].echo[0].time_samples, delay_frames as f32);
                assert_eq!(rt.master_fx[0].echo[1].time_samples, delay_frames as f32);
                if delay_frames.fract() == 0.5 {
                    let first = first.unwrap();
                    assert!(
                        (output[first][channel] - output[first + 1][channel]).abs() < 1e-6,
                        "fractional frame delay must interpolate both adjacent frames equally"
                    );
                }
                if sr == 48_000 && bpm == 120.0 {
                    assert_eq!(
                        first,
                        Some(18_000),
                        "original audit echo arrived twice too early"
                    );
                }
            }
        }
    }
}

#[test]
fn master_reverb_preserves_arrival_seconds_and_channel_isolation() {
    for sr in [44_100, 48_000] {
        let first_frame = (887.0 * sr as f64 / 48_000.0).round() as usize;
        for channel in 0..2 {
            let mut input = vec![[0.0; 2]; 8192];
            input[0][channel] = 0.5;
            let mut rt = engine(sr, &input);
            rt.apply(Command::FxWet {
                slot: 1,
                value: 1.0,
            });
            let output = render(&mut rt, input.len(), 257);
            let first = output.iter().position(|frame| frame[channel].abs() > 1e-6);
            assert_eq!(first, Some(first_frame));
            assert!(output.iter().all(|frame| frame[1 - channel] == 0.0));
            assert!((first_frame as f64 / sr as f64 - 887.0 / 48_000.0).abs() <= 0.5 / sr as f64);
            assert!(
                (output[first_frame][channel] - (-0.125f32).tanh()).abs() < 1e-6,
                "first negative comb reflection changed gain or sign"
            );
        }
    }
}

#[test]
fn master_stereo_effects_match_independent_mono_processors_on_sines() {
    for sr in [44_100, 48_000] {
        for dual_mono in [false, true] {
            let input: Vec<_> = (0..30_000)
                .map(|frame| {
                    let phase = frame as f32 * std::f32::consts::TAU / sr as f32;
                    let left = (phase * 233.0).sin() * 0.2;
                    let right = if dual_mono {
                        left
                    } else {
                        (phase * 997.0).sin() * 0.13
                    };
                    [left, right]
                })
                .collect();
            for (echo_mix, reverb_mix) in
                [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.35, 0.65), (1.0, 1.0)]
            {
                let mut rt = engine(sr, &input);
                rt.apply(Command::FxWet {
                    slot: 0,
                    value: echo_mix,
                });
                rt.apply(Command::FxWet {
                    slot: 1,
                    value: reverb_mix,
                });
                let output = render(&mut rt, input.len(), 509);
                let mut eq = [ThreeBand::new(sr as f32); 2];
                let mut delays: [Delay; 2] = std::array::from_fn(|_| {
                    let mut delay = Delay::new(sr as usize * 2);
                    delay.time_samples = sr as f32 * 0.375;
                    delay.mix = echo_mix;
                    delay
                });
                let mut reverbs: [Reverb; 2] = std::array::from_fn(|_| {
                    let mut reverb = Reverb::at_sample_rate(sr as f32);
                    reverb.mix = reverb_mix;
                    reverb
                });
                for (frame, (input, actual)) in input.iter().zip(&output).enumerate() {
                    for channel in 0..2 {
                        let source = eq[channel].tick(input[channel]);
                        let expected = reverbs[channel].tick(delays[channel].tick(source)).tanh();
                        assert!((actual[channel] - expected).abs() < 1e-6,
                            "{sr} Hz, frame {frame}, channel {channel}, mix {echo_mix}/{reverb_mix}: {} vs {expected}", actual[channel]);
                    }
                    if dual_mono {
                        assert_eq!(
                            actual[0], actual[1],
                            "master effects changed dual-mono width"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn master_controls_apply_to_both_channels_and_rate_changes_reset_both_tails() {
    let input = vec![[0.0; 2]; 100_000];
    let mut rt = engine(48_000, &input);
    for sr in [44_100, 48_000] {
        for channel in 0..2 {
            rt.master_fx[0].echo[channel].tick(0.5);
            rt.master_fx[1].reverb[channel].tick(0.5);
        }
        rt.set_sample_rate(sr);
        for (bpm, echo_mix, reverb_mix) in
            [(90.0, 0.25, 0.75), (120.0, 1.0, 1.0), (180.0, 0.0, 0.0)]
        {
            rt.apply(Command::SetBpm(bpm));
            rt.apply(Command::FxWet {
                slot: 0,
                value: echo_mix,
            });
            rt.apply(Command::FxWet {
                slot: 1,
                value: reverb_mix,
            });
            let output = render(&mut rt, 25_000, 512);
            assert!(
                output.iter().flatten().all(|sample| *sample == 0.0),
                "rate reset retained master effect tails"
            );
            for channel in 0..2 {
                assert_eq!(rt.master_fx[0].echo[channel].mix, echo_mix);
                assert_eq!(rt.master_fx[1].reverb[channel].mix, reverb_mix);
                assert_eq!(
                    rt.master_fx[0].echo[channel].time_samples,
                    sr as f32 * 60.0 / bpm * 0.75
                );
            }
        }
    }
}
