use super::*;
use crate::engine::{beatgrid::Grid, test_alloc, Engine, Sample};
use std::sync::Arc;

fn fixture(source_rate: u32, output_rate: u32) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(output_rate, 64);
    let audio = Arc::new(Sample {
        spectrum: None,
        name: "Quantized stereo click reference".into(),
        sr: source_rate,
        ch: 2,
        data: (0..source_rate * 12)
            .flat_map(|frame| {
                let seconds = f64::from(frame) / f64::from(source_rate);
                let click = ((seconds - 1.125).abs() < 0.002) as u8 as f32;
                [click * 0.4, click * -0.3]
            })
            .collect(),
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    });
    for deck in 0..2 {
        rt.apply(Command::DeckAudio {
            deck,
            audio: audio.clone(),
        });
        rt.decks[usize::from(deck)].grid = Some(
            Grid::new(0.125, 120.0)
                .unwrap()
                .with_anchor(4.0, 2.125)
                .unwrap()
                .with_anchor(8.0, 5.125)
                .unwrap(),
        );
    }
    (engine, rt)
}
fn control(deck: u8, control: Control) -> Command {
    Command::DeckControl {
        source: 71,
        deck,
        control,
    }
}
fn enable(rt: &mut RtEngine, deck: u8, division: u8) {
    rt.apply(control(
        deck,
        Control::Quantize {
            enabled: true,
            division,
        },
    ));
}
fn hold(source: u64, pad: u8, on: bool) -> Command {
    Command::DeckControl {
        source,
        deck: 0,
        control: Control::Hold {
            button: Button::HotCue(pad),
            on,
        },
    }
}
fn beat(seconds: f64) -> f64 {
    if seconds < 2.125 {
        (seconds - 0.125) * 2.0
    } else {
        4.0 + (seconds - 2.125) / 0.75
    }
}
fn seconds(beat: f64) -> f64 {
    if beat < 4.0 {
        0.125 + beat * 0.5
    } else {
        2.125 + (beat - 4.0) * 0.75
    }
}

#[test]
fn queued_click_onsets_match_independent_anchor_timing_at_every_source_and_output_rate() {
    for source_rate in [44_100, 48_000, 96_000] {
        for output_rate in [44_100, 48_000, 96_000] {
            for (start, synced) in [
                (0.18, false),
                (2.02, false),
                (2.18, false),
                (5.18, false),
                (2.02, true),
                (5.18, true),
            ] {
                let (_engine, mut rt) = fixture(source_rate, output_rate);
                enable(&mut rt, 0, 2);
                let d = &mut rt.decks[0];
                d.pos = start * f64::from(source_rate);
                d.playing = true;
                d.rate = 1.0;
                d.hotcues[0] = crate::engine::HotCue {
                    set: true,
                    pos: 1.125 * f64::from(source_rate),
                };
                d.sync = synced;
                d.sync_bpm = 100.0;
                let target_beat = (beat(start) / 0.5).ceil() * 0.5;
                let target = seconds(target_beat);
                let elapsed = if synced {
                    (target_beat - beat(start)) * 0.6
                } else {
                    target - start
                };
                let frames = (elapsed * f64::from(output_rate)).ceil() as usize;
                assert_eq!(
                    test_alloc::measure(|| rt.apply(Command::DeckHotCue {
                        deck: 0,
                        pad: 0,
                        del: false
                    })),
                    test_alloc::Counts::default()
                );
                let pending = rt.decks[0].controls.status().pending.unwrap();
                assert!((pending.source_seconds - target).abs() < 1e-10);
                let mut dispatched = None;
                let mut energy = [0.0; 2];
                assert_eq!(
                    test_alloc::measure(|| {
                        for frame in 1..=frames + 256 {
                            let (l, r) = rt.render_deck(0);
                            if dispatched.is_none()
                                && rt.decks[0].controls.status().pending.is_none()
                            {
                                dispatched = Some(frame);
                            }
                            energy[0] += f64::from(l).powi(2);
                            energy[1] += f64::from(r).powi(2);
                        }
                    }),
                    test_alloc::Counts::default()
                );
                assert!(dispatched.unwrap().abs_diff(frames) <= 1, "{source_rate}/{output_rate}, start={start}, sync={synced}: {dispatched:?} != {frames}");
                assert!(
                    energy.iter().all(|energy| *energy > 0.01),
                    "Both click channels must be rendered"
                );
                assert!(!rt.decks[1].playing);
            }
        }
    }
}

#[test]
fn cue_creation_and_manual_loop_edges_use_selected_divisions_without_rounding_precise_edits() {
    let (_engine, mut rt) = fixture(48_000, 48_000);
    for division in 0..6 {
        enable(&mut rt, 0, division);
        let size = QUANTIZE_DIVISIONS[usize::from(division)];
        for position in [0.18, 2.18, 5.18] {
            let expected = seconds((beat(position) / size).round() * size).max(0.0) * 48_000.0;
            rt.decks[0].pos = position * 48_000.0;
            rt.apply(Command::DeckCue { deck: 0 });
            assert!((rt.decks[0].cue_pos - expected).abs() < 1e-6);
            rt.apply(Command::DeckHotCue {
                deck: 0,
                pad: 0,
                del: true,
            });
            rt.apply(Command::DeckHotCue {
                deck: 0,
                pad: 0,
                del: false,
            });
            assert!((rt.decks[0].hotcues[0].pos - expected).abs() < 1e-6);
            rt.apply(Command::DeckLoopIn { deck: 0 });
            assert!((rt.decks[0].loop_start - expected).abs() < 1e-6);
            rt.decks[0].pos += 24_000.0;
            rt.apply(Command::DeckLoopOut { deck: 0 });
            let endpoint =
                seconds((beat(position + 0.5) / size).round() * size).max(0.0) * 48_000.0;
            assert!((rt.decks[0].loop_len - (endpoint - expected).abs().max(64.0)).abs() < 1e-6);
            rt.decks[0].loop_on = false;
        }
    }
    let key = rt.decks[0].history_key;
    rt.apply(control(
        0,
        Control::LoopBounds {
            media_key: key,
            start_seconds: 0.187,
            end_seconds: 0.519,
        },
    ));
    assert!((rt.decks[0].loop_start - 0.187 * 48_000.0).abs() < 1e-6);
    assert!((rt.decks[0].loop_len - 0.332 * 48_000.0).abs() < 1e-6);
    rt.apply(control(
        0,
        Control::Quantize {
            enabled: false,
            division: 3,
        },
    ));
    rt.decks[0].pos = 0.191 * 48_000.0;
    rt.apply(Command::DeckCue { deck: 0 });
    assert_eq!(rt.decks[0].cue_pos, rt.decks[0].pos);
    rt.apply(Command::DeckLoopIn { deck: 0 });
    assert_eq!(rt.decks[0].loop_start, rt.decks[0].pos);
    assert!(!rt.decks[1].controls.status().quantize);
    assert!(!Control::Quantize {
        enabled: true,
        division: 6
    }
    .valid());
}

#[test]
fn rapid_retriggers_and_independent_held_owners_replace_or_cancel_exactly_one_onset() {
    let (_engine, mut rt) = fixture(48_000, 48_000);
    enable(&mut rt, 0, 3);
    let d = &mut rt.decks[0];
    d.playing = true;
    d.pos = 0.18 * 48_000.0;
    for pad in 0..8 {
        d.hotcues[pad] = crate::engine::HotCue {
            set: true,
            pos: (1.125 + pad as f64 * 0.5) * 48_000.0,
        };
    }
    for pad in 0..8 {
        rt.apply(Command::DeckHotCue {
            deck: 0,
            pad,
            del: false,
        });
    }
    assert_eq!(
        rt.decks[0].controls.status().pending.unwrap().action,
        QuantizedAction::HotCue { pad: 7 }
    );
    rt.apply(hold(11, 1, true));
    rt.apply(hold(12, 2, true));
    rt.apply(hold(11, 1, false));
    assert!(rt.decks[0].controls.status().pending.is_some());
    rt.apply(hold(12, 2, false));
    assert!(rt.decks[0].controls.status().pending.is_none());
    rt.apply(hold(13, 3, true));
    for _ in 0..24_000 {
        rt.render_deck(0);
    }
    assert!(rt.decks[0].controls.status().pending.is_none());
    assert!(rt.decks[0].pos > 2.625 * 48_000.0 && rt.decks[0].pos < 3.0 * 48_000.0);
    rt.apply(hold(13, 3, false));
    assert!(rt.decks[0].playing);
}

#[test]
fn pending_work_retires_on_input_reset_overload_grid_undo_stop_scratch_and_media_replacement() {
    let (engine, mut rt) = fixture(48_000, 48_000);
    enable(&mut rt, 0, 3);
    for event in 0..11 {
        let d = &mut rt.decks[0];
        d.playing = true;
        d.touching = false;
        d.pos = 0.18 * 48_000.0;
        d.rate = 1.0;
        d.loop_on = false;
        d.hotcues[0] = crate::engine::HotCue {
            set: true,
            pos: 1.125 * 48_000.0,
        };
        rt.apply(Command::DeckHotCue {
            deck: 0,
            pad: 0,
            del: false,
        });
        assert!(
            rt.decks[0].controls.status().pending.is_some(),
            "event {event}"
        );
        match event {
            0 => {
                rt.midi_learning.retire_pending();
            }
            1 => {
                rt.decks[0]
                    .controls
                    .restore_loops(rt.decks[0].controls.loop_history());
            }
            2 => rt.apply(Command::DeckPlay { deck: 0 }),
            3 => rt.apply(Command::DeckTouch { deck: 0, on: true }),
            4 => rt.apply(control(
                0,
                Control::Hold {
                    button: Button::Reverse,
                    on: true,
                },
            )),
            5 => {
                rt.decks[0].controls.media_changed();
            }
            6 => {
                rt.apply(Command::DeckSeek {
                    deck: 0,
                    frac: 0.02,
                });
            }
            7 => {
                rt.apply(control(
                    0,
                    Control::Quantize {
                        enabled: false,
                        division: 3,
                    },
                ));
            }
            8 => {
                while engine.send(Command::Metronome).is_ok() {}
                engine.cmd.release_midi_source(71);
            }
            9 => {
                use crate::engine::midi::{
                    learn::{Config, Endpoint, Mapping},
                    Action, Binding, MsgKind,
                };
                rt.midi_learning.configure(Config::default()).unwrap();
                assert!(rt.decks[0].controls.status().pending.is_some());
                rt.midi_learning
                    .configure(Config {
                        mappings: vec![Mapping {
                            endpoint: Endpoint {
                                name: "Quantize fixture".into(),
                                id: "fixture:0".into(),
                            },
                            binding: Binding {
                                kind: MsgKind::Note,
                                ch: 0,
                                data: 60,
                                action: Action::DeckPlay,
                                deck: 0,
                                extra: 0,
                                relative: None,
                            },
                        }],
                    })
                    .unwrap();
            }
            _ => {
                rt.midi_routing
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
        }
        rt.quantized_deck_maintain();
        assert!(
            rt.decks[0].controls.status().pending.is_none(),
            "event {event}"
        );
        rt.decks[0].controls.release();
        enable(&mut rt, 0, 3);
    }
    rt.process(&mut []);
    rt.decks[0].playing = false;
    rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 0,
        del: false,
    });
    assert!(rt.decks[0].controls.status().pending.is_none());
    rt.decks[0].playing = true;
    rt.decks[0].touching = true;
    rt.decks[0].pos = 0.19 * 48_000.0;
    let before = rt.decks[0].pos;
    rt.apply(Command::DeckJog {
        deck: 0,
        delta: -0.25,
    });
    assert_eq!(rt.decks[0].pos, before - 100.0);
    assert!(rt.decks[0].controls.status().pending.is_none());
}

#[test]
fn real_grid_edits_undo_safety_and_project_swaps_cannot_revive_pending_cues() {
    use crate::engine::beatgrid::{GridEditAck, GridEditState};
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 128);
    let receipt = rt.decks[0].load_receipt.clone().unwrap();
    let grid = Grid::new(0.125, 120.0).unwrap();
    rt.apply(Command::DeckGrid {
        deck: 0,
        grid: Some(grid),
        receipt: receipt.clone(),
        ack: GridEditAck::new(),
    });
    rt.clear_undo_for_test();
    enable(&mut rt, 0, 3);
    for event in 0..3 {
        rt.decks[0].playing = true;
        rt.decks[0].pos = 0.18 * 48_000.0;
        rt.decks[0].rate = 1.0;
        rt.decks[0].hotcues[0] = crate::engine::HotCue {
            set: true,
            pos: 1.125 * 48_000.0,
        };
        rt.apply(Command::DeckHotCue {
            deck: 0,
            pad: 0,
            del: false,
        });
        assert!(
            rt.decks[0].controls.status().pending.is_some(),
            "event {event}"
        );
        let ack = GridEditAck::new();
        match event {
            0 => {
                rt.apply(Command::DeckGrid {
                    deck: 0,
                    grid: Some(Grid::new(0.1, 110.0).unwrap()),
                    receipt: receipt.clone(),
                    ack: ack.clone(),
                });
                assert_eq!(ack.state(), GridEditState::Applied);
            }
            1 => {
                rt.apply(Command::Undo);
                assert_eq!(rt.decks[0].grid, Some(grid));
            }
            2 => rt.apply(Command::SafetyStop(
                crate::engine::performance::Safety::Stop,
            )),
            _ => unreachable!(),
        }
        assert!(
            rt.decks[0].controls.status().pending.is_none(),
            "event {event}"
        );
    }
    for swap_project in [false, true] {
        let (_engine, mut rt) = Engine::headless_for_test(48_000, 128);
        enable(&mut rt, 0, 3);
        rt.decks[0].playing = true;
        rt.decks[0].pos = 0.18 * 48_000.0;
        rt.decks[0].hotcues[0] = crate::engine::HotCue {
            set: true,
            pos: 1.0 * 48_000.0,
        };
        rt.apply(Command::DeckHotCue {
            deck: 0,
            pad: 0,
            del: false,
        });
        assert!(rt.decks[0].controls.status().pending.is_some());
        if swap_project {
            crate::engine::project::Prepared::empty(48_000)
                .unwrap()
                .swap_into(&mut rt);
        } else {
            rt.apply(Command::DeckUnload { deck: 0 });
        }
        assert!(rt.decks[0].controls.status().pending.is_none());
    }
}

#[test]
fn loop_onsets_queue_at_mapped_boundaries_and_cue_stop_remains_immediate() {
    let (_engine, mut rt) = fixture(48_000, 48_000);
    enable(&mut rt, 0, 2);
    rt.decks[0].playing = true;
    rt.decks[0].rate = 1.0;
    rt.decks[0].pos = 2.18 * 48_000.0;
    rt.apply(Command::DeckLoopIn { deck: 0 });
    assert_eq!(
        rt.decks[0].controls.status().pending.unwrap().action,
        QuantizedAction::LoopIn
    );
    for _ in 0..15_360 {
        rt.render_deck(0);
    }
    assert!((rt.decks[0].loop_start - 2.5 * 48_000.0).abs() < 1e-6);
    rt.decks[0].pos = 3.05 * 48_000.0;
    rt.apply(Command::DeckLoopOut { deck: 0 });
    for _ in 0..9600 {
        rt.render_deck(0);
    }
    assert!((rt.decks[0].loop_len - 0.75 * 48_000.0).abs() < 1e-6);
    rt.decks[0].loop_on = false;
    rt.decks[0].pos = 3.05 * 48_000.0;
    rt.apply(control(0, Control::Reloop));
    assert!(rt.decks[0].controls.status().pending.is_some());
    for _ in 0..9600 {
        rt.render_deck(0);
    }
    assert!(rt.decks[0].loop_on && rt.decks[0].pos >= rt.decks[0].loop_start);
    rt.decks[0].loop_on = false;
    rt.decks[0].pos = 3.05 * 48_000.0;
    rt.apply(Command::DeckReloop { deck: 0 });
    assert!(rt.decks[0].controls.status().pending.is_some());
    let cue = rt.decks[0].cue_quantized_position(rt.decks[0].cue_pos, rt.sr, rt.bpm);
    rt.apply(Command::DeckCue { deck: 0 });
    assert!(!rt.decks[0].playing);
    assert_eq!(rt.decks[0].pos, cue);
    assert!(rt.decks[0].controls.status().pending.is_none());
}
