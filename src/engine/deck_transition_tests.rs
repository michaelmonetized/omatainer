use super::*;

const FRAMES: usize = 65_536;
const TARGET: usize = 8192;

fn sample(sr: u32, value: impl Fn(usize) -> f32) -> Arc<Sample> {
    Arc::new(Sample {
        name: "marked regions".into(),
        sr,
        ch: 2,
        data: (0..FRAMES)
            .flat_map(|frame| {
                let value = value(frame);
                [value, -value * 0.5]
            })
            .collect(),
        peaks: vec![].into(),
        bpm: 120.0,
        path: String::new(),
    })
}

fn fixture(sr: u32, source_sr: u32, keylock: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: sample(source_sr, |frame| if frame < TARGET { 0.25 } else { -0.5 }),
    });
    rt.apply(Command::DeckGain {
        deck: 0,
        value: 1.0,
    });
    if keylock {
        rt.apply(Command::DeckKeylock { deck: 0 });
    }
    rt.apply(Command::DeckPlay { deck: 0 });
    for _ in 0..700 {
        rt.render_deck(0);
    }
    assert!((rt.decks[0].last_output[0] - 0.25).abs() < 1e-6);
    rt
}

fn assert_grains_reset(rt: &RtEngine, deck: usize) {
    let d = &rt.decks[deck];
    let ratio = d.audio.as_ref().map_or(rt.sr, |audio| audio.sr as f32) as f64 / rt.sr as f64;
    assert_eq!(d.keylock_dsp.phase, 0);
    assert_eq!(d.keylock_dsp.origin, d.pos);
    assert_eq!(d.keylock_dsp.previous, d.pos - d.keylock_dsp.hop as f64 * ratio);
    assert_eq!(d.transition_remaining, (rt.sr as u32).div_ceil(500));
}

fn assert_transition(rt: &mut RtEngine, expected: f32) {
    let old = rt.decks[0].last_output;
    let frames = (rt.sr as u32).div_ceil(500);
    assert_eq!(rt.decks[0].transition_remaining, frames);
    for frame in 0..frames {
        let output = rt.render_deck(0);
        let mix = frame as f32 / (frames - 1) as f32;
        let expected_left = old[0] * (1.0 - mix) + expected * mix;
        let expected_right = old[1] * (1.0 - mix) - expected * 0.5 * mix;
        assert!(
            (output.0 - expected_left).abs() < 1e-6,
            "transition frame {frame}: {} != {expected_left}",
            output.0
        );
        assert!((output.1 - expected_right).abs() < 1e-6);
    }
    assert_eq!(rt.decks[0].transition_remaining, 0);
    for _ in 0..1100 {
        let output = rt.render_deck(0);
        assert!(
            (output.0 - expected).abs() < 1e-6,
            "stale source after transition: {}",
            output.0
        );
        assert!((output.1 + expected * 0.5).abs() < 1e-6);
    }
}

#[test]
fn seek_cue_and_hotcue_use_new_region_after_two_milliseconds() {
    for (sr, source_sr) in [(44_100, 48_000), (48_000, 44_100)] {
        for keylock in [false, true] {
            for action in 0..4 {
                let mut rt = fixture(sr, source_sr, keylock);
                let old_pos = rt.decks[0].pos;
                match action {
                    0 => rt.apply(Command::DeckSeek {
                        deck: 0,
                        frac: TARGET as f32 / FRAMES as f32,
                    }),
                    1 => {
                        rt.decks[0].cue_pos = TARGET as f64;
                        rt.apply(Command::DeckCue { deck: 0 });
                        assert!(!rt.decks[0].playing);
                    }
                    2 => {
                        rt.decks[0].hotcues[0] = HotCue {
                            set: true,
                            pos: TARGET as f64,
                        };
                        rt.apply(Command::DeckHotCue {
                            deck: 0,
                            pad: 0,
                            del: false,
                        });
                    }
                    _ => {
                        rt.decks[0].playing = false;
                        rt.decks[0].pos = 0.0;
                        rt.decks[0].cue_pos = TARGET as f64;
                        rt.apply(Command::DeckPlay { deck: 0 });
                    }
                }
                assert_ne!(rt.decks[0].pos, old_pos);
                assert_eq!(rt.decks[0].pos, TARGET as f64);
                assert_grains_reset(&rt, 0);
                if action == 1 {
                    assert_transition(&mut rt, 0.0);
                    rt.apply(Command::DeckPlay { deck: 0 });
                    assert_grains_reset(&rt, 0);
                }
                assert_transition(&mut rt, -0.5);
            }
        }
    }
}

#[test]
fn replacement_and_unload_retire_old_source_with_bounded_envelope() {
    for keylock in [false, true] {
        let mut rt = fixture(48_000, 44_100, keylock);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: sample(32_000, |_| -0.75),
        });
        assert!(!rt.decks[0].playing);
        assert_grains_reset(&rt, 0);
        assert_transition(&mut rt, 0.0);
        rt.apply(Command::DeckPlay { deck: 0 });
        assert_grains_reset(&rt, 0);
        assert_transition(&mut rt, -0.75);
        rt.apply(Command::DeckUnload { deck: 0 });
        assert_grains_reset(&rt, 0);
        assert_transition(&mut rt, 0.0);
    }
}

#[test]
fn reset_grain_starts_at_target_without_skipping_half_a_window() {
    let mut rt = fixture(44_100, 48_000, true);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: sample(48_000, |frame| {
            if (TARGET..TARGET + 128).contains(&frame) {
                -0.5
            } else {
                0.75
            }
        }),
    });
    rt.apply(Command::DeckSeek {
        deck: 0,
        frac: TARGET as f32 / FRAMES as f32,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    assert_grains_reset(&rt, 0);
    // The narrow target marker ends before the old +512-frame window offset.
    // All samples in the 2 ms transition must still come from this marker.
    for _ in 0..89 {
        rt.render_deck(0);
    }
    assert!((rt.decks[0].last_output[0] + 0.5).abs() < 1e-6);

    rt.set_sample_rate(48_000);
    assert_grains_reset(&rt, 0);
    assert_eq!(rt.decks[0].transition_frames, 96);
}

#[test]
fn keylock_toggles_preserve_playhead_timing_and_restart_at_current_region() {
    let mut rt = fixture(48_000, 44_100, false);
    rt.apply(Command::DeckSeek {
        deck: 0,
        frac: TARGET as f32 / FRAMES as f32,
    });
    assert_transition(&mut rt, -0.5);
    for _ in 0..4 {
        let before = rt.decks[0].pos;
        rt.apply(Command::DeckKeylock { deck: 0 });
        assert_eq!(rt.decks[0].pos, before);
        assert_grains_reset(&rt, 0);
        assert_transition(&mut rt, -0.5);
        let elapsed_frames = 96 + 1100;
        let expected = before + elapsed_frames as f64 * 44_100.0 / 48_000.0;
        assert!(
            (rt.decks[0].pos - expected).abs() < 1e-6,
            "transition changed transport timing"
        );
    }
}

#[test]
fn match_jumps_only_the_unfavored_deck_and_resets_its_grains() {
    let mut rt = fixture(48_000, 48_000, true);
    rt.apply(Command::DeckAudio {
        deck: 1,
        audio: rt.decks[0].audio.clone().unwrap(),
    });
    rt.decks[1].pos = TARGET as f64;
    rt.decks[1].playing = true;
    rt.xfader = 1.0;
    let favored = rt.decks[1].pos;
    rt.apply(Command::DeckMatch);
    assert_eq!(rt.decks[1].pos, favored);
    assert_eq!(rt.decks[0].pos, TARGET as f64);
    assert_grains_reset(&rt, 0);
    assert_transition(&mut rt, -0.5);
}

#[test]
fn pitchlocked_grains_and_wrap_transitions_stay_inside_short_loop() {
    for sr in [44_100, 48_000] {
        let mut rt = fixture(sr, sr, true);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: sample(sr, |frame| {
                if (TARGET..TARGET + 256).contains(&frame) {
                    -0.5
                } else {
                    0.75
                }
            }),
        });
        rt.apply(Command::DeckSeek {
            deck: 0,
            frac: TARGET as f32 / FRAMES as f32,
        });
        rt.apply(Command::DeckPlay { deck: 0 });
        rt.apply(Command::DeckLoop {
            deck: 0,
            beats: 256.0 / (sr as f32 / 2.0),
        });
        rt.decks[0].pitch = 0.0;
        assert_grains_reset(&rt, 0);
        let settle = (sr as usize).div_ceil(500);
        for frame in 0..4000 {
            let output = rt.render_deck(0);
            if frame >= settle {
                assert!(
                    (output.0 + 0.5).abs() < 1e-6,
                    "grain escaped loop at frame {frame}: {}",
                    output.0
                );
            }
            assert!((TARGET as f64..(TARGET + 257) as f64).contains(&rt.decks[0].pos));
        }
        // Interpolation at a loop ending exactly at EOF uses its last sample
        // and loop start, rather than Sample::at's out-of-range zero sentinel.
        rt.decks[0].audio = Some(sample(sr, |_| -0.5));
        rt.decks[0].loop_start = (FRAMES - 128) as f64;
        rt.decks[0].loop_len = 128.0;
        assert_eq!(rt.decks[0].sample_at(FRAMES as f64 - 0.25), (-0.5, 0.25));
    }
}

#[test]
fn loop_edit_commands_reset_grains_through_the_same_transition() {
    let mut rt = fixture(48_000, 48_000, true);
    rt.apply(Command::DeckLoop {
        deck: 0,
        beats: 1.0,
    });
    for command in [
        Command::DeckLoopHalf { deck: 0 },
        Command::DeckLoopDouble { deck: 0 },
        Command::DeckLoopIn { deck: 0 },
        Command::DeckLoopOut { deck: 0 },
        Command::DeckReloop { deck: 0 },
        Command::DeckLoop {
            deck: 0,
            beats: 1.0,
        },
    ] {
        rt.decks[0].keylock_dsp.phase = 99;
        rt.decks[0].keylock_dsp.origin = 42.0;
        rt.apply(command);
        assert_grains_reset(&rt, 0);
    }
}

#[test]
fn scratch_jogs_follow_new_source_immediately_and_release_without_old_grains() {
    let mut rt = fixture(48_000, 48_000, true);
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    let delta = (TARGET as f64 - rt.decks[0].pos) as f32 / 400.0;
    rt.apply(Command::DeckJog { deck: 0, delta });
    assert_eq!(
        rt.decks[0].transition_remaining, 0,
        "jog must not restart a smoothing window"
    );
    assert!(
        (rt.render_deck(0).0 + 0.5).abs() < 1e-6,
        "keylock concealed the jog's new source"
    );
    for _ in 0..16 {
        rt.apply(Command::DeckJog {
            deck: 0,
            delta: -0.01,
        });
        assert_eq!(rt.decks[0].transition_remaining, 0);
        assert!((rt.render_deck(0).0 + 0.5).abs() < 1e-6);
    }
    rt.apply(Command::DeckTouch { deck: 0, on: false });
    assert_grains_reset(&rt, 0);
    assert_transition(&mut rt, -0.5);
}

#[test]
fn transitions_clear_old_channel_filter_history_and_end_of_file_grains() {
    let mut rt = fixture(48_000, 48_000, true);
    for channel in 0..2 {
        rt.decks[0].eq[channel].low.z = 0.5;
        rt.decks[0].eq[channel].high.z = -0.3;
        rt.decks[0].filter[channel].history[0] = 0.2;
        rt.decks[0].filter[channel].history[1] = 0.4;
    }
    rt.apply(Command::DeckSeek {
        deck: 0,
        frac: TARGET as f32 / FRAMES as f32,
    });
    for channel in 0..2 {
        assert_eq!(rt.decks[0].eq[channel].low.z, 0.0);
        assert_eq!(rt.decks[0].eq[channel].high.z, 0.0);
        assert_eq!(rt.decks[0].filter[channel].history[0], 0.0);
        assert_eq!(rt.decks[0].filter[channel].history[1], 0.0);
    }
    rt.apply(Command::DeckSeek { deck: 0, frac: 1.0 });
    rt.render_deck(0);
    assert!(!rt.decks[0].playing);
    assert_eq!(rt.decks[0].pos, 0.0);
    assert_eq!(rt.decks[0].keylock_dsp.origin, 0.0);
    for _ in 0..96 {
        rt.render_deck(0);
    }
    assert_eq!(rt.render_deck(0), (0.0, 0.0));
}
