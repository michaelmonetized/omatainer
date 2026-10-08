use super::*;
use crate::engine::{
    deck_controls::Control,
    deck_pads::{Mode, Press, Release},
    dsp::Sample,
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::Arc;

fn tone(rate: u32) -> Arc<Sample> {
    Arc::new(Sample {
        name: "Original chromatic cue stereo tone".into(),
        path: String::new(),
        sr: rate,
        ch: 2,
        data: (0..rate * 4)
            .flat_map(|frame| {
                let value = (std::f64::consts::TAU * 440.0 * f64::from(frame) / f64::from(rate))
                    .sin() as f32
                    * 0.5;
                [value, -value]
            })
            .collect(),
        peaks: Vec::new().into(),
        spectrum: None,
        bpm: 120.0,
    })
}

fn rig(source_rate: u32, output_rate: u32, tempo: f32) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(output_rate, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: tone(source_rate),
    });
    let d = &mut rt.decks[0];
    d.sync = false;
    d.gain = 1.0;
    d.keylock = false;
    d.key_shift = 3;
    d.pitch_range = 2;
    d.pitch = tempo - 0.5;
    d.rate = tempo;
    d.target_rate = tempo;
    d.hotcues[2].set = true;
    d.hotcues[2].pos = f64::from(source_rate) / 4.0;
    rt.publish_for_test();
    rt.apply(Command::DeckControl {
        source: 247,
        deck: 0,
        control: Control::PitchPads {
            media_key: rt.decks[0].history_key,
            cue: 2,
            range: 1,
        },
    });
    (engine, Box::new(rt))
}

fn press(rt: &mut RtEngine, source: u64, key: u32, pad: u8) {
    rt.apply(Command::DeckPadPress(Press {
        source,
        key,
        deck: 0,
        id: pad + 1,
        mode: Some(Mode::PitchCue),
        pressure: 1.0,
        shifted: false,
    }));
}

fn release(rt: &mut RtEngine, source: u64, key: u32) {
    rt.apply(Command::DeckPadRelease(Release { source, key }));
}

#[test]
fn all_chromatic_ranges_retain_original_key_and_legacy_mode_numbers() {
    assert_eq!(Mode::ALL.map(Mode::index), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(Mode::PitchCue.index(), 8);
    assert_eq!(Mode::from_index(8), Some(Mode::PitchCue));
    assert!(Mode::from_index(9).is_none());
    assert!(Control::PadMode { mode: 8 }.valid());
    assert!(!Control::PadMode { mode: 9 }.valid());
    assert!(!Control::PitchPads {
        media_key: 1,
        cue: 8,
        range: 0
    }
    .valid());
    assert!(!Control::PitchPads {
        media_key: 1,
        cue: 0,
        range: 3
    }
    .valid());
    assert!(!Control::PitchPads {
        media_key: 0,
        cue: 0,
        range: 0
    }
    .valid());
    for (range, expected) in [
        [-6, -5, -4, -3, -2, -1, 0, 1],
        [-3, -2, -1, 0, 1, 2, 3, 4],
        [-1, 0, 1, 2, 3, 4, 5, 6],
    ]
    .into_iter()
    .enumerate()
    {
        for (pad, expected) in expected.into_iter().enumerate() {
            assert_eq!(semitones(range as u8, pad as u8), Some(expected));
        }
    }
    assert!(semitones(3, 0).is_none());
    assert!(semitones(0, 8).is_none());
}

#[test]
fn actual_chromatic_cue_pcm_has_declared_pitch_at_independent_tempos_and_source_clocks() {
    let mut groups = 0;
    let mut minimum_amplitude = f64::INFINITY;
    let mut maximum_source_error = 0.0_f64;
    for (source_rate, output_rate) in [(44100, 48000), (48000, 44100), (96000, 96000)] {
        for tempo in [0.84, 1.0] {
            for range in 0..3 {
                for pad in 0..8 {
                    for playing in [false, true] {
                        let (_engine, mut rt) = rig(source_rate, output_rate, tempo);
                        let media_key = rt.decks[0].history_key;
                        rt.apply(Command::DeckControl {
                            source: 247,
                            deck: 0,
                            control: Control::PitchPads {
                                media_key,
                                cue: 2,
                                range,
                            },
                        });
                        rt.apply(Command::DeckControl {
                            source: 247,
                            deck: 0,
                            control: Control::Quantize {
                                enabled: true,
                                division: 3,
                            },
                        });
                        if playing {
                            rt.apply(Command::DeckPlay { deck: 0 });
                        }
                        let onset = rt.decks[0].hotcues[2].pos;
                        assert_eq!(
                            test_alloc::measure(|| press(&mut rt, 247, 41, pad)),
                            Default::default()
                        );
                        assert_eq!(rt.decks[0].pos, onset);
                        assert!(rt.decks[0].controls.status().pending.is_none());
                        assert!(rt.decks[0].controls.status().quantize);
                        assert_eq!(rt.decks[0].playing, playing);
                        assert_eq!(rt.decks[0].preview_position.is_some(), !playing);
                        assert_eq!(rt.decks[0].key_shift, 3);
                        assert!(!rt.decks[0].keylock);
                        let warm = output_rate / 10;
                        for _ in 0..warm {
                            rt.render_deck(0);
                        }
                        let count = output_rate / 4;
                        let expected_hz = 440.0
                            * crate::engine::key_shift::factor(semitones(range, pad).unwrap());
                        let (mut sin, mut cos) = (0.0_f64, 0.0_f64);
                        assert_eq!(
                            test_alloc::measure(|| {
                                for frame in 0..count {
                                    let (left, right) = rt.render_deck(0);
                                    assert!(left.is_finite());
                                    assert_eq!(left, -right);
                                    let phase =
                                        std::f64::consts::TAU * expected_hz * f64::from(frame)
                                            / f64::from(output_rate);
                                    sin += f64::from(left) * phase.sin();
                                    cos += f64::from(left) * phase.cos();
                                }
                            }),
                            Default::default()
                        );
                        let amplitude = 2.0 * sin.hypot(cos) / f64::from(count);
                        assert!(
                        (0.43..0.57).contains(&amplitude),
                        "src{source_rate} out{output_rate} tempo{tempo} range{range} pad{pad} pitch{expected_hz} amplitude{amplitude}"
                    );
                        let expected_source = onset
                            + f64::from(warm + count) * f64::from(source_rate)
                                / f64::from(output_rate)
                                * f64::from(tempo);
                        let error = (rt.decks[0].pos - expected_source).abs();
                        assert!(error < 1e-5, "Cue source clock moved by {error} frames");
                        minimum_amplitude = minimum_amplitude.min(amplitude);
                        maximum_source_error = maximum_source_error.max(error);
                        assert_eq!(
                            test_alloc::measure(|| release(&mut rt, 247, 41)),
                            Default::default()
                        );
                        assert!(rt.decks[0].controls.status().pitch_pad.is_none());
                        assert_eq!(rt.decks[0].effective_key_shift(), 3);
                        assert_eq!(rt.decks[0].preview_position, None);
                        if playing {
                            assert!((rt.decks[0].pos - expected_source).abs() < 1e-5);
                        } else {
                            assert_eq!(rt.decks[0].pos, onset);
                        }
                        assert_eq!(rt.decks[0].playing, playing);
                        groups += 1;
                    }
                }
            }
        }
    }
    println!(
        "CHROMATIC_CUE_PCM {}",
        serde_json::json!({"groups": groups, "minimum_fundamental_amplitude": minimum_amplitude, "maximum_source_frame_error": maximum_source_error, "callback_allocations": 0, "physical_devices_opened": false})
    );
}

#[test]
fn latest_original_pitch_owner_survives_slot_reuse_and_mode_change_without_changing_saved_key() {
    let (_engine, mut rt) = rig(48000, 48000, 1.0);
    let other = (
        rt.decks[1].pos,
        rt.decks[1].key_shift,
        rt.decks[1].controls.status().pitch_pad,
    );
    press(&mut rt, 11, 1, 0);
    press(&mut rt, 12, 2, 5);
    assert_eq!(rt.decks[0].effective_key_shift(), 2);
    release(&mut rt, 11, 1);
    press(&mut rt, 13, 3, 7);
    assert_eq!(rt.decks[0].effective_key_shift(), 4);
    release(&mut rt, 13, 3);
    assert_eq!(rt.decks[0].effective_key_shift(), 2);
    assert_eq!(rt.decks[0].controls.status().pitch_pad, Some(5));
    rt.apply(Command::DeckControl {
        source: 247,
        deck: 0,
        control: Control::PadMode {
            mode: Mode::Roll.index(),
        },
    });
    assert_eq!(rt.decks[0].effective_key_shift(), 3);
    assert_eq!(rt.decks[0].key_shift, 3);
    assert!(rt.decks[0].preview_position.is_none());
    release(&mut rt, 12, 2);
    assert_eq!(rt.decks[0].effective_key_shift(), 3);
    assert_eq!(
        (
            rt.decks[1].pos,
            rt.decks[1].key_shift,
            rt.decks[1].controls.status().pitch_pad
        ),
        other
    );
}

#[test]
fn source_qualified_cue_changes_and_original_key_reset_retire_old_notes_and_are_undoable() {
    let (engine, mut rt) = rig(48000, 48000, 1.0);
    let media_key = rt.decks[0].history_key;
    rt.decks[0].hotcues[4].set = true;
    rt.decks[0].hotcues[4].pos = 24000.0;
    press(&mut rt, 11, 1, 0);
    rt.apply(Command::DeckControl {
        source: 247,
        deck: 0,
        control: Control::PitchPads {
            media_key,
            cue: 4,
            range: 2,
        },
    });
    assert_eq!(rt.decks[0].effective_key_shift(), 3);
    assert!(rt.decks[0].controls.status().pitch_pad.is_none());
    assert_eq!(rt.decks[0].controls.status().pitch_cue, 4);
    release(&mut rt, 11, 1);
    press(&mut rt, 11, 1, 7);
    assert_eq!(rt.decks[0].pos, 24000.0);
    assert_eq!(rt.decks[0].effective_key_shift(), 6);
    rt.apply(Command::DeckControl {
        source: 247,
        deck: 0,
        control: Control::PitchReset { media_key },
    });
    rt.process(&mut []);
    assert_eq!(rt.decks[0].key_shift, 0);
    assert_eq!(rt.decks[0].effective_key_shift(), 0);
    assert!(rt.decks[0].controls.status().pitch_pad.is_none());
    let reset_cursor = engine.undo.view().cursor;
    assert!(reset_cursor > 0);
    rt.apply(Command::Undo);
    assert_eq!(rt.decks[0].key_shift, 3);
    assert!(rt.decks[0].controls.status().pitch_pad.is_none());
    rt.apply(Command::Redo);
    assert_eq!(rt.decks[0].key_shift, 0);
    assert_eq!(engine.undo.view().cursor, reset_cursor);
    release(&mut rt, 11, 1);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: tone(48000),
    });
    rt.apply(Command::DeckControl {
        source: 247,
        deck: 0,
        control: Control::PitchPads {
            media_key,
            cue: 4,
            range: 0,
        },
    });
    assert_eq!(rt.decks[0].controls.status().pitch_cue, 0);
    assert!(rt.decks[0].controls.status().pitch_semitones.is_none());
    assert!(rt.decks[0].hotcues.iter().all(|cue| !cue.set));
}

#[test]
fn actual_output_callback_emits_pitched_cue_pcm_without_changing_master_or_other_deck() {
    use crate::engine::audio::OutputCallback;
    let (engine, mut rt) = rig(48000, 48000, 0.84);
    rt.decks[0].gain = 1.0;
    rt.apply(Command::Master(1.0));
    rt.apply(Command::Xfader(0.0));
    for _ in 0..64 {
        rt.process(&mut [0.0; 256]);
    }
    let mixer = (rt.master, rt.xfader);
    let other = (
        rt.decks[1].history_key,
        rt.decks[1].pos,
        rt.decks[1].key_shift,
        rt.decks[1].gain,
    );
    let media = rt.decks[0].history_key;
    let mut callback = Box::new(OutputCallback::new(*rt, 2));
    let mut samples = 0;
    let mut minimum_amplitude = f64::INFINITY;
    for (range, pad, interval) in [(0, 0, -6), (1, 0, -3), (1, 3, 0), (1, 6, 3), (2, 7, 6)] {
        engine
            .send(Command::DeckControl {
                source: 247,
                deck: 0,
                control: Control::PitchPads {
                    media_key: media,
                    cue: 2,
                    range,
                },
            })
            .unwrap();
        engine
            .send(Command::DeckPadPress(Press {
                source: 247,
                key: 41,
                deck: 0,
                id: pad + 1,
                mode: Some(Mode::PitchCue),
                pressure: 1.0,
                shifted: false,
            }))
            .unwrap();
        for _ in 0..48 {
            assert_eq!(
                test_alloc::measure(|| callback.render(&mut [0.0; 256])),
                Default::default()
            );
        }
        let hz = 440.0 * crate::engine::key_shift::factor(interval);
        let (mut sin, mut cos) = (0.0_f64, 0.0_f64);
        let mut frames = 0;
        for _ in 0..96 {
            let mut block = [0.0; 256];
            assert_eq!(
                test_alloc::measure(|| callback.render(&mut block)),
                Default::default()
            );
            for stereo in block.chunks_exact(2) {
                assert_eq!(stereo[0], -stereo[1]);
                let phase = std::f64::consts::TAU * hz * frames as f64 / 48000.0;
                sin += f64::from(stereo[0]) * phase.sin();
                cos += f64::from(stereo[0]) * phase.cos();
                frames += 1;
            }
        }
        let amplitude = 2.0 * sin.hypot(cos) / frames as f64;
        assert!(
            (0.43..0.57).contains(&amplitude),
            "Actual callback {interval} st amplitude {amplitude}"
        );
        minimum_amplitude = minimum_amplitude.min(amplitude);
        samples += frames;
        engine
            .send(Command::DeckPadRelease(Release {
                source: 247,
                key: 41,
            }))
            .unwrap();
        assert_eq!(
            test_alloc::measure(|| callback.render(&mut [0.0; 256])),
            Default::default()
        );
        let renderer = callback.renderer_for_test();
        assert_eq!((renderer.master, renderer.xfader), mixer);
        assert_eq!(
            (
                renderer.decks[1].history_key,
                renderer.decks[1].pos,
                renderer.decks[1].key_shift,
                renderer.decks[1].gain
            ),
            other
        );
        assert!(renderer.decks[0].controls.status().pitch_pad.is_none());
        assert!(!renderer.decks[0].playing);
        assert_eq!(renderer.decks[0].key_shift, 3);
    }
    println!(
        "CHROMATIC_CUE_CALLBACK {}",
        serde_json::json!({"semitones":[-6,-3,0,3,6], "compared_frames":samples,"minimum_fundamental_amplitude":minimum_amplitude,"callback_allocations":0,"callback_frees":0,"other_deck_and_master_unchanged":true,"physical_devices_opened":false})
    );
}

#[test]
fn releasing_chromatic_pad_preserves_direct_cue_owner_and_selected_range_parameter_ownership() {
    use crate::engine::deck_controls::Button;
    let (_engine, mut rt) = rig(48000, 48000, 1.0);
    rt.apply(Command::DeckControl {
        source: 99,
        deck: 0,
        control: Control::Hold {
            button: Button::HotCue(2),
            on: true,
        },
    });
    press(&mut rt, 247, 41, 7);
    assert_eq!(rt.decks[0].effective_key_shift(), 4);
    release(&mut rt, 247, 41);
    assert_eq!(rt.decks[0].effective_key_shift(), 3);
    assert!(rt.decks[0].controls.held(Button::HotCue(2)));
    assert!(rt.decks[0].preview_position.is_some());
    rt.apply(Command::DeckPadParameter {
        source: 247,
        deck: 0,
        up: true,
        shifted: false,
    });
    assert_eq!(rt.decks[0].controls.status().pitch_range, 2);
    assert!(rt.decks[0].controls.held(Button::HotCue(2)));
    rt.apply(Command::DeckControl {
        source: 99,
        deck: 0,
        control: Control::Hold {
            button: Button::HotCue(2),
            on: false,
        },
    });
    assert!(rt.decks[0].preview_position.is_none());
}
