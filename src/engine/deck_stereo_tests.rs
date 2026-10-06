use super::*;

fn engine(sr: u32) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn gains(eq: &ThreeBand) -> [f32; 3] {
    [eq.low_g, eq.mid_g, eq.high_g]
}

fn assert_gains(rt: &mut RtEngine, deck: usize, expected: [f32; 3]) {
    for channel in &rt.decks[deck].eq {
        assert_eq!(gains(channel), expected);
    }
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().decks[deck].eq, expected);
}

fn sweep(sr: u32) -> Vec<f32> {
    let frames = 8192;
    let mut phase = 0.0;
    (0..frames)
        .map(|frame| {
            let hz = 20.0 * (sr as f64 * 0.45 / 20.0).powf(frame as f64 / frames as f64);
            phase += std::f64::consts::TAU * hz / sr as f64;
            phase.sin() as f32 * 0.5
        })
        .collect()
}

fn load_fixture(rt: &mut RtEngine, deck: usize, samples: &[[f32; 2]], mono: bool) {
    let channels = if mono { 1 } else { 2 };
    // render_deck advances before reading. Pad both ends so every requested
    // frame is valid for Sample::at's interpolation window.
    let mut data = vec![0.0; channels];
    for frame in samples {
        data.extend_from_slice(&frame[..channels]);
    }
    data.extend_from_slice(&[0.0, 0.0][..channels]);
    rt.apply(Command::DeckAudio {
        deck: deck as u8,
        audio: Arc::new(Sample { spectrum: None,
            name: "stereo regression".into(),
            sr: rt.sr as u32,
            ch: channels as u16,
            data,
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.decks[deck].playing = true;
    rt.decks[deck].rate = 1.0;
    // These references isolate the EQ/filter response after the source
    // transition. The transition envelope has its own marked-region tests.
    rt.decks[deck].transition_remaining = 0;
}

fn configure(rt: &mut RtEngine, deck: usize, eq: [f32; 3], filter: f32) {
    rt.decks[deck] = DeckRt::new(rt.sr);
    rt.apply(Command::DeckGain {
        deck: deck as u8,
        value: 1.0,
    });
    for (band, value) in eq.into_iter().enumerate() {
        rt.apply(Command::DeckEq {
            deck: deck as u8,
            band: band as u8,
            value,
        });
    }
    rt.apply(Command::DeckFilter {
        deck: deck as u8,
        value: filter,
    });
}

fn render_and_compare(
    rt: &mut RtEngine,
    deck: usize,
    samples: &[[f32; 2]],
    mono: bool,
) -> Vec<[f32; 2]> {
    let sr = rt.sr;
    let gain = rt.decks[deck].gain;
    let eq_gains = gains(&rt.decks[deck].eq[0]);
    let mut eq = [ThreeBand::new(sr); 2];
    for channel in &mut eq {
        channel.low_g = eq_gains[0];
        channel.mid_g = eq_gains[1];
        channel.high_g = eq_gains[2];
    }
    let mut filters = [deck_filter::ChannelFilter::default(); 2];
    let mut position = rt.decks[deck].filter_position;
    let filter = rt.decks[deck].filter_amt;
    load_fixture(rt, deck, samples, mono);
    let mut output = Vec::with_capacity(samples.len());
    for (frame, input) in samples.iter().enumerate() {
        position = deck_filter::slew(position, filter, sr);
        let curve = deck_filter::Curve::at(position, sr);
        let actual = rt.render_deck(deck);
        let actual = [actual.0, actual.1];
        for channel in 0..2 {
            let source = input[if mono { 0 } else { channel }];
            let mut expected = eq[channel].tick(source * gain);
            expected = filters[channel].process(expected, curve);
            assert!(actual[channel].is_finite());
            assert!(
                (actual[channel] - expected).abs() < 1e-6,
                "deck {deck} channel {channel} frame {frame} at {sr} Hz: {} != {expected}",
                actual[channel]
            );
        }
        output.push(actual);
    }
    output
}

#[test]
fn deck_eq_and_filter_keep_impulses_and_sweeps_in_their_channel() {
    // EQ alone, LP alone, HP alone, and the EQ+filter serial path.
    let configurations = [
        ([0.2, 0.7, 0.9], 0.5),
        ([0.5; 3], 0.2),
        ([0.5; 3], 0.8),
        ([0.2, 0.7, 0.9], 0.2),
    ];
    for sr in [44_100, 48_000] {
        let mut rt = engine(sr);
        let mut impulse = vec![0.0; 2048];
        impulse[0] = 0.5;
        for signal in [impulse, sweep(sr)] {
            for active_channel in 0..2 {
                let samples: Vec<_> = signal
                    .iter()
                    .map(|&sample| {
                        let mut frame = [0.0; 2];
                        frame[active_channel] = sample;
                        frame
                    })
                    .collect();
                for deck in 0..DECKS {
                    for (eq, filter) in configurations {
                        configure(&mut rt, deck, eq, filter);
                        let output = render_and_compare(&mut rt, deck, &samples, false);
                        assert!(
                            output.iter().all(|frame| frame[1 - active_channel] == 0.0),
                            "crosstalk on deck {deck} at {sr} Hz with filter {filter}"
                        );
                        let energy: f32 = output
                            .iter()
                            .map(|frame| frame[active_channel].powi(2))
                            .sum();
                        assert!(energy > 1e-4, "fixture must exercise audible processing");
                    }
                }
            }
        }
    }
}

#[test]
fn deck_stereo_and_dual_mono_match_independent_mono_frequency_response() {
    for sr in [44_100, 48_000] {
        let mut rt = engine(sr);
        let signal = sweep(sr);
        let dual_mono: Vec<_> = signal.iter().map(|&sample| [sample; 2]).collect();
        let stereo: Vec<_> = signal
            .iter()
            .zip(signal.iter().rev())
            .map(|(&l, &r)| [l, r])
            .collect();
        for deck in 0..DECKS {
            for filter in [0.2, 0.5, 0.8] {
                configure(&mut rt, deck, [0.2, 0.7, 0.9], filter);
                render_and_compare(&mut rt, deck, &stereo, false);
                configure(&mut rt, deck, [0.2, 0.7, 0.9], filter);
                let dual = render_and_compare(&mut rt, deck, &dual_mono, false);
                configure(&mut rt, deck, [0.2, 0.7, 0.9], filter);
                let mono = render_and_compare(&mut rt, deck, &dual_mono, true);
                assert_eq!(dual, mono, "stereo duplication differs from a mono file");
                assert!(dual.iter().all(|frame| frame[0] == frame[1]));
            }
        }
    }
}

#[test]
fn deck_eq_gain_cut_and_solo_commands_update_both_channels() {
    let mut rt = engine(48_000);
    for deck in 0..DECKS {
        let other = gains(&rt.decks[1 - deck].eq[0]);
        let mut expected = [1.0; 3];
        for band in 0..3 {
            rt.apply(Command::DeckEq {
                deck: deck as u8,
                band: band as u8,
                value: 0.2 + band as f32 * 0.2,
            });
            expected[band] = eq_gain(0.2 + band as f32 * 0.2);
            assert_gains(&mut rt, deck, expected);
        }
        for band in 0..3 {
            rt.apply(Command::DeckEqCut {
                deck: deck as u8,
                band: band as u8,
            });
            expected[band] = 0.0;
            assert_gains(&mut rt, deck, expected);
            rt.apply(Command::DeckEq {
                deck: deck as u8,
                band: band as u8,
                value: 0.7,
            });
            assert_gains(&mut rt, deck, expected);
            rt.apply(Command::DeckEqCut {
                deck: deck as u8,
                band: band as u8,
            });
            expected[band] = eq_gain(0.7);
            assert_gains(&mut rt, deck, expected);
        }
        for band in 0..3 {
            rt.apply(Command::DeckEqSolo {
                deck: deck as u8,
                band: band as u8,
            });
            let mut solo = [0.0; 3];
            solo[band] = expected[band];
            assert_gains(&mut rt, deck, solo);
            let muted_band = (band + 1) % 3;
            rt.apply(Command::DeckEq {
                deck: deck as u8,
                band: muted_band as u8,
                value: 0.3,
            });
            expected[muted_band] = eq_gain(0.3);
            assert_gains(&mut rt, deck, solo);
            rt.apply(Command::DeckEqSolo {
                deck: deck as u8,
                band: band as u8,
            });
            assert_gains(&mut rt, deck, expected);
        }
        rt.apply(Command::DeckEqSolo {
            deck: deck as u8,
            band: 3,
        });
        assert_gains(&mut rt, deck, expected);
        rt.apply(Command::DeckEqSolo {
            deck: deck as u8,
            band: 3,
        });
        assert_gains(&mut rt, deck, expected);
        rt.apply(Command::DeckGain {
            deck: deck as u8,
            value: 1.2,
        });
        rt.apply(Command::DeckEqCut {
            deck: deck as u8,
            band: 3,
        });
        assert_eq!(rt.decks[deck].gain, 0.0);
        rt.apply(Command::DeckGain {
            deck: deck as u8,
            value: 1.3,
        });
        assert_eq!(rt.decks[deck].gain, 0.0);
        rt.apply(Command::DeckEqCut {
            deck: deck as u8,
            band: 3,
        });
        assert_eq!(rt.decks[deck].gain, 1.3);
        assert_gains(&mut rt, deck, expected);
        assert_gains(&mut rt, 1 - deck, other);
    }
}

#[test]
fn deck_sample_rate_change_resets_both_histories_and_preserves_controls() {
    let mut rt = engine(48_000);
    for deck in 0..DECKS {
        configure(&mut rt, deck, [0.2, 0.7, 0.9], 0.2);
        rt.apply(Command::DeckEqCut {
            deck: deck as u8,
            band: 1,
        });
    }
    for sr in [44_100, 48_000] {
        let before = gains(&rt.decks[0].eq[0]);
        for deck in &mut rt.decks {
            for channel in 0..2 {
                deck.eq[channel].tick(0.4 + channel as f32 * 0.2);
                deck.filter[channel].process(0.3, deck_filter::Curve::at(0.2, rt.sr));
                assert!(deck.eq[channel].low.z != 0.0);
                assert!(deck.filter[channel].history[0] != 0.0);
            }
        }
        rt.set_sample_rate(sr).unwrap();
        for deck in 0..DECKS {
            assert_gains(&mut rt, deck, before);
            assert!(rt.decks[deck].eq_cut[1]);
            assert_eq!(rt.decks[deck].filter_amt, 0.2);
            for channel in 0..2 {
                let eq = &rt.decks[deck].eq[channel];
                let fresh = ThreeBand::new(sr as f32);
                assert_eq!((eq.low.z, eq.high.z), (0.0, 0.0));
                assert_eq!((eq.low.a, eq.high.a), (fresh.low.a, fresh.high.a));
                let filter = &rt.decks[deck].filter[channel];
                assert_eq!((filter.history[0], filter.history[1]), (0.0, 0.0));
            }
            let samples: Vec<_> = sweep(sr)
                .into_iter()
                .map(|sample| [sample, -sample])
                .collect();
            render_and_compare(&mut rt, deck, &samples, false);
        }
    }
}
