use super::*;
use crate::engine::{
    audio::OutputCallback, beatgrid::Grid, test_alloc, Command, Engine, RtEngine, Sample,
};
use std::sync::Arc;

fn fixture(source_sr: u32, output_sr: u32) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(output_sr, 64);
    let source = Arc::new(Sample {
        name: "Original stereo slip reference".into(),
        path: String::new(),
        sr: source_sr,
        ch: 2,
        bpm: 120.0,
        peaks: vec![].into(),
        spectrum: None,
        data: (0..source_sr * 16)
            .flat_map(|frame| {
                let t = f64::from(frame) / f64::from(source_sr);
                [
                    (t * std::f64::consts::TAU * 337.0).sin() as f32 * 0.3,
                    (t * std::f64::consts::TAU * 491.0).sin() as f32 * 0.2,
                ]
            })
            .collect(),
    });
    for deck in 0..2 {
        rt.apply(Command::DeckAudio {
            deck,
            audio: source.clone(),
        });
        rt.apply(Command::DeckSeek { deck, frac: 0.25 });
        rt.apply(Command::DeckPlay { deck });
        rt.decks[usize::from(deck)].rate = 1.0;
    }
    (engine, Box::new(rt))
}
fn command(deck: u8, control: super::super::Control) -> Command {
    Command::DeckControl {
        source: 73,
        deck,
        control,
    }
}
fn options(rt: &mut RtEngine, division: Option<u8>) {
    rt.apply(command(
        0,
        super::super::Control::SlipSettings {
            enabled: true,
            division,
        },
    ));
}
fn hold(rt: &mut RtEngine, button: Button, on: bool) {
    rt.apply(command(0, super::super::Control::Hold { button, on }));
}
fn render(rt: &mut RtEngine, frames: usize) {
    for _ in 0..frames {
        let (l, r) = rt.render_deck(0);
        assert!(l.is_finite() && r.is_finite());
    }
}

#[test]
fn nested_cue_scratch_and_roll_share_one_source_timeline_and_return_after_the_last_release() {
    let mut cases = 0;
    let mut max_error = 0.0_f64;
    for source_sr in [44100, 48000, 96000] {
        for output_sr in [44100, 48000, 96000] {
            for scratch_last in [false, true] {
                let (_engine, mut rt) = fixture(source_sr, output_sr);
                options(&mut rt, None);
                rt.decks[0].cue_pos = f64::from(source_sr);
                let start = rt.decks[0].pos;
                let other = rt.decks[1].pos;
                hold(&mut rt, Button::Cue, true);
                assert!(rt.decks[0].playing);
                assert_eq!(rt.decks[0].pos, f64::from(source_sr));
                rt.apply(Command::DeckTouch { deck: 0, on: true });
                render(&mut rt, 512);
                hold(&mut rt, Button::Roll(3), true);
                render(&mut rt, 512);
                hold(&mut rt, Button::Cue, false);
                if scratch_last {
                    hold(&mut rt, Button::Roll(3), false);
                } else {
                    rt.apply(Command::DeckTouch { deck: 0, on: false });
                }
                render(&mut rt, 512);
                assert!(rt.decks[0].controls.slip_forward.is_some());
                if scratch_last {
                    rt.apply(Command::DeckTouch { deck: 0, on: false });
                } else {
                    hold(&mut rt, Button::Roll(3), false);
                }
                rt.deck_surface_tick(0);
                let expected = start + 1536.0 * f64::from(source_sr) / f64::from(output_sr);
                let error = (rt.decks[0].pos - expected).abs();
                assert!(
                    error < 1e-6,
                    "{source_sr}/{output_sr} scratch_last={scratch_last}: {error}"
                );
                max_error = max_error.max(error);
                assert!(rt.decks[0].controls.slip_forward.is_none());
                assert!(!rt.decks[0].loop_on);
                assert!(rt.decks[0].playing);
                assert_eq!(rt.decks[1].pos, other);
                cases += 1;
            }
        }
    }
    println!(
        "SLIP_NESTED_RECEIPT {}",
        serde_json::json!({"cases":cases,"max_source_frame_error":max_error,"gesture_frames":1536,"nested_gestures":["cue","touch","roll"],"release_orders":2,"physical_devices_opened":false})
    );
}
#[test]
fn quantized_returns_follow_the_shadow_through_variable_grids_tempo_changes_and_original_loops() {
    let mut cases = 0;
    let mut max_frames = 0;
    for source_sr in [44100, 48000, 96000] {
        for output_sr in [44100, 48000, 96000] {
            for synced in [false, true] {
                for looped in [false, true] {
                    let (_engine, mut rt) = fixture(source_sr, output_sr);
                    options(&mut rt, Some(3));
                    let d = &mut rt.decks[0];
                    d.grid = Some(
                        Grid::new(0.125, 120.0)
                            .unwrap()
                            .with_anchor(4.0, 2.125)
                            .unwrap()
                            .with_anchor(8.0, 5.125)
                            .unwrap(),
                    );
                    d.sync = synced;
                    d.sync_bpm = 100.0;
                    d.pitch_range = 2;
                    d.pitch = 0.5;
                    d.pos = (if looped { 5.10 } else { 3.4 }) * f64::from(source_sr);
                    if looped {
                        d.loop_on = true;
                        d.loop_start = 2.125 * f64::from(source_sr);
                        d.loop_len = 3.0 * f64::from(source_sr);
                    }
                    rt.apply(Command::DeckTouch { deck: 0, on: true });
                    let mut expected = if looped { 5.10 } else { 3.4 };
                    let mut unwrapped = 4.0 + (expected - 2.125) / 0.75;
                    for frame in 0..2048 {
                        if frame == 1024 {
                            rt.apply(Command::DeckPitch {
                                deck: 0,
                                value: 0.75,
                            });
                            rt.decks[0].sync_bpm = 137.0;
                        }
                        let beat = if expected < 2.125 {
                            (expected - 0.125) * 2.0
                        } else {
                            4.0 + (expected - 2.125) / 0.75
                        };
                        if synced {
                            let delta = if frame < 1024 { 100.0 } else { 137.0 }
                                / (60.0 * f64::from(output_sr));
                            unwrapped += delta;
                            let next = beat + delta;
                            expected = if next < 4.0 {
                                0.125 + next * 0.5
                            } else {
                                2.125 + (next - 4.0) * 0.75
                            };
                        } else {
                            let delta =
                                if frame < 1024 { 1.0 } else { 1.25 } / f64::from(output_sr);
                            expected += delta;
                            unwrapped += delta / 0.75;
                        }
                        if looped && expected >= 5.125 {
                            expected = 2.125 + (expected - 5.125);
                        }
                        rt.render_deck(0);
                        assert!(
                            (rt.decks[0].controls.slip_forward.unwrap() / f64::from(source_sr)
                                - expected)
                                .abs()
                                < 1e-8
                        );
                    }
                    rt.apply(Command::DeckTouch { deck: 0, on: false });
                    rt.deck_surface_tick(0);
                    let due = unwrapped.ceil();
                    assert!((rt.decks[0].controls.slip_due.unwrap() - due).abs() < 1e-7);
                    let return_beat = if looped {
                        4.0 + (due - 4.0).rem_euclid(4.0)
                    } else {
                        due
                    };
                    let expected_return = 2.125 + (return_beat - 4.0) * 0.75;
                    assert!((rt.decks[0].controls.slip_return.unwrap()/f64::from(source_sr)-expected_return).abs()<1e-7,"src={source_sr} output={output_sr} sync={synced} loop={looped} due={due} unwrapped={unwrapped} shadow={expected} marker={} wanted={expected_return} state={:?}",rt.decks[0].controls.slip_return.unwrap()/f64::from(source_sr),rt.decks[0].controls.status());
                    let mut frames = 0;
                    while rt.decks[0].controls.slip_forward.is_some() {
                        rt.deck_surface_tick(0);
                        frames += 1;
                        assert!(frames < output_sr as usize);
                    }
                    assert!(
                        (rt.decks[0].pos / f64::from(source_sr) - expected_return).abs()
                            < 2.0 / f64::from(output_sr)
                    );
                    assert_eq!(rt.decks[0].loop_on, looped);
                    max_frames = max_frames.max(frames);
                    cases += 1;
                }
            }
        }
    }
    println!(
        "SLIP_QUANTIZED_RECEIPT {}",
        serde_json::json!({"cases":cases,"source_rates":[44100,48000,96000],"output_rates":[44100,48000,96000],"tempo_change_during_hold":true,"variable_grid":true,"original_loop_preserved":true,"max_wait_output_frames":max_frames,"physical_devices_opened":false})
    );
}
#[test]
fn a_new_gesture_cancels_the_pending_return_but_retains_the_original_root_and_settings_are_bounded()
{
    let (engine, mut rt) = fixture(48000, 48000);
    options(&mut rt, Some(5));
    let start = rt.decks[0].pos;
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    render(&mut rt, 1024);
    rt.apply(Command::DeckTouch { deck: 0, on: false });
    rt.deck_surface_tick(0);
    assert!(rt.decks[0].controls.slip_due.is_some());
    let before = rt.decks[0].controls.slip_forward.unwrap();
    hold(&mut rt, Button::Bleep, true);
    assert_eq!(rt.decks[0].controls.slip_forward, Some(before));
    assert!(rt.decks[0].controls.slip_due.is_none());
    render(&mut rt, 512);
    assert!(rt.decks[0].controls.slip_forward.unwrap() > start + 1535.0);
    hold(&mut rt, Button::Bleep, false);
    options(&mut rt, None);
    rt.deck_surface_tick(0);
    assert!(rt.decks[0].controls.slip_forward.is_none());
    assert!(rt.decks[0].pos > start + 1535.0);
    for index in [6, 255] {
        assert!(engine
            .send(command(
                0,
                super::super::Control::SlipSettings {
                    enabled: true,
                    division: Some(index)
                }
            ))
            .is_err());
    }
    assert!(engine
        .send(command(
            2,
            super::super::Control::SlipSettings {
                enabled: true,
                division: None
            }
        ))
        .is_err());
}
#[test]
fn pause_seek_source_project_and_safety_boundaries_clear_shadow_without_a_delayed_jump() {
    for boundary in 0..6 {
        let (_engine, mut rt) = fixture(48000, 48000);
        options(&mut rt, Some(5));
        hold(&mut rt, Button::HotCue(0), true);
        rt.apply(Command::DeckTouch { deck: 0, on: true });
        render(&mut rt, 256);
        assert!(rt.decks[0].controls.slip_forward.is_some());
        match boundary {
            0 => rt.apply(Command::DeckPlay { deck: 0 }),
            1 => rt.apply(Command::DeckSeek { deck: 0, frac: 0.5 }),
            2 => rt.apply(Command::DeckUnload { deck: 0 }),
            3 => crate::engine::project::Prepared::empty(48000)
                .unwrap()
                .swap_into(&mut rt),
            4 => rt.apply(Command::SafetyStop(
                crate::engine::performance::Safety::Stop,
            )),
            _ => rt.apply(command(
                0,
                super::super::Control::SlipSettings {
                    enabled: false,
                    division: None,
                },
            )),
        };
        assert!(rt.decks[0].controls.slip_forward.is_none());
        assert!(rt.decks[0].controls.slip_due.is_none());
        assert!(rt.decks[0].controls.slip_return.is_none());
    }
}
#[test]
fn actual_callback_nested_gestures_and_quantized_return_allocate_nothing_and_keep_the_other_decks_pcm_exact(
) {
    let (engine, mut rt) = fixture(48000, 48000);
    let (_other, mut baseline) = fixture(48000, 48000);
    for r in [&mut *rt, &mut *baseline] {
        r.apply(Command::Xfader(1.0));
        for _ in 0..64 {
            r.process(&mut [0.0; 256]);
        }
    }
    options(&mut rt, Some(2));
    let mut actual = Box::new(OutputCallback::new(*rt, 2));
    let mut reference = Box::new(OutputCallback::new(*baseline, 2));
    let mut frames = 0;
    let mut energy = 0.0;
    for block in 0..256 {
        let control = match block {
            0 => Some(Command::DeckTouch { deck: 0, on: true }),
            4 => Some(command(
                0,
                super::super::Control::Hold {
                    button: Button::Roll(3),
                    on: true,
                },
            )),
            12 => Some(Command::DeckTouch { deck: 0, on: false }),
            16 => Some(command(
                0,
                super::super::Control::Hold {
                    button: Button::Roll(3),
                    on: false,
                },
            )),
            _ => None,
        };
        if let Some(control) = control {
            engine.send(control).unwrap();
        }
        let mut a = [0.0; 256];
        let mut b = [0.0; 256];
        assert_eq!(
            test_alloc::measure(|| actual.render(&mut a)),
            Default::default()
        );
        reference.render(&mut b);
        assert_eq!(a, b);
        energy += a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        frames += 128;
    }
    assert!(energy > 1.0);
    assert!(actual.renderer_for_test().decks[0]
        .controls
        .slip_forward
        .is_none());
    println!(
        "SLIP_CALLBACK_RECEIPT {}",
        serde_json::json!({"compared_other_deck_frames":frames,"other_deck_max_error":0,"callback_allocations":0,"callback_frees":0,"energy":energy,"physical_devices_opened":false})
    );
}
#[test]
fn held_input_owners_release_independently_and_new_instances_start_with_slip_disabled() {
    let (engine, mut rt) = fixture(48000, 48000);
    assert!(!rt.decks[0].controls.status().slip);
    options(&mut rt, None);
    rt.decks[0].cue_pos = 48000.0;
    for source in [41, 42] {
        engine
            .send(Command::DeckControl {
                source,
                deck: 0,
                control: super::super::Control::Hold {
                    button: Button::Cue,
                    on: true,
                },
            })
            .unwrap();
    }
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].playing);
    assert!(rt.decks[0].controls.slip_forward.is_some());
    engine.cmd.release_midi_source(41);
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].controls.slip_forward.is_some());
    engine.cmd.release_midi_source(42);
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].controls.slip_forward.is_none());
    assert!(rt.decks[0].playing);
}

#[test]
fn the_first_rendered_sample_after_immediate_release_matches_the_predicted_forward_position() {
    for (source_sr, output_sr) in [(44100, 48000), (48000, 44100), (96000, 96000)] {
        let (_engine, mut rt) = fixture(source_sr, output_sr);
        options(&mut rt, None);
        let before = rt.decks[0].pos;
        rt.apply(Command::DeckTouch { deck: 0, on: true });
        render(&mut rt, 128);
        rt.apply(Command::DeckTouch { deck: 0, on: false });
        rt.render_deck(0);
        let expected = before + 129.0 * f64::from(source_sr) / f64::from(output_sr);
        assert!(
            (rt.decks[0].pos - expected).abs() < 1e-6,
            "{source_sr}/{output_sr}: actual {} expected {expected}",
            rt.decks[0].pos
        );
    }
}

#[test]
fn seeking_during_a_held_gesture_does_not_rearm_a_cancelled_shadow_until_a_new_press() {
    let (_engine, mut rt) = fixture(48000, 48000);
    options(&mut rt, Some(5));
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    render(&mut rt, 128);
    rt.apply(Command::DeckSeek { deck: 0, frac: 0.5 });
    render(&mut rt, 128);
    assert!(rt.decks[0].controls.slip_forward.is_none());
    rt.apply(Command::DeckTouch { deck: 0, on: false });
    render(&mut rt, 128);
    assert!(rt.decks[0].controls.slip_due.is_none());
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    render(&mut rt, 128);
    assert!(rt.decks[0].controls.slip_forward.is_some());
}
