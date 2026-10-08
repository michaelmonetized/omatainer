use super::*;
use crate::engine::{spindle, test_alloc, Engine, Sample};
use std::{sync::Arc, time::Duration};

fn fixture() -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(48000, 64);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample { spectrum: None,
            name: "NS7 analytical stereo source".into(),
            sr: 48000,
            ch: 2,
            data: (0..48000 * 12)
                .flat_map(|frame| {
                    let x = (std::f64::consts::TAU * 440.0 * f64::from(frame) / 48000.0).sin()
                        as f32
                        * 0.5;
                    [x, -x]
                })
                .collect(),
            peaks: Vec::new().into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.apply(Command::DeckSeek {
        deck: 0,
        frac: 0.25,
    });
    (engine, rt)
}
fn control(rt: &mut RtEngine, source: u64, c: Control) {
    rt.apply(Command::DeckControl {
        source,
        deck: 0,
        control: c,
    });
}
fn hold(rt: &mut RtEngine, source: u64, button: Button, on: bool) {
    control(rt, source, Control::Hold { button, on });
}
fn render(rt: &mut RtEngine, frames: usize) {
    for _ in 0..frames {
        let (l, r) = rt.render_deck(0);
        assert!(l.is_finite() && r.is_finite());
    }
}

#[test]
fn ns7_controls_bleep_keeps_forward_time_reverse_keeps_actual_time_and_owners_release() {
    let (_engine, mut rt) = fixture();
    rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut rt, 500);
    let start = rt.decks[0].pos;
    hold(&mut rt, 1, Button::Bleep, true);
    hold(&mut rt, 2, Button::Bleep, true);
    render(&mut rt, 4800);
    assert!(rt.decks[0].pos < start - 4700.0);
    hold(&mut rt, 1, Button::Bleep, false);
    assert!(rt.decks[0].controls.status().bleep);
    hold(&mut rt, 2, Button::Bleep, false);
    assert!((rt.decks[0].pos - (start + 4800.0)).abs() < 0.01);
    assert!(!rt.decks[0].controls.status().bleep);
    hold(&mut rt, 1, Button::Reverse, true);
    render(&mut rt, 4800);
    let reverse = rt.decks[0].pos;
    hold(&mut rt, 1, Button::Reverse, false);
    assert_eq!(rt.decks[0].pos, reverse);
    hold(&mut rt, 2, Button::BendUp, true);
    hold(&mut rt, 3, Button::BendDown, true);
    assert_eq!(rt.decks[0].controls.multiplier(), 1.0);
    hold(&mut rt, 3, Button::BendDown, false);
    render(&mut rt, 500);
    assert!((rt.decks[0].rate - 1.08).abs() < 0.00001);
    hold(&mut rt, 2, Button::BendUp, false);
    render(&mut rt, 500);
    assert!((rt.decks[0].rate - 1.0).abs() < 0.00001);
    rt.apply(Command::DeckSeek { deck: 0, frac: 0.0 });
    hold(&mut rt, 1, Button::Reverse, true);
    render(&mut rt, 5000);
    assert_eq!(rt.decks[0].pos, 0.0);
    assert!(rt.decks[0]
        .last_output
        .iter()
        .all(|value| value.abs() < 0.0001));
}

#[test]
fn ns7_controls_source_retirement_preserves_another_button_owner_even_when_full() {
    let (engine, mut rt) = fixture();
    for source in [41, 42] {
        engine
            .cmd
            .send(Command::DeckControl {
                source,
                deck: 0,
                control: Control::Hold {
                    button: Button::Reverse,
                    on: true,
                },
            })
            .unwrap();
    }
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].controls.status().reverse);
    for _ in 0..60 {
        let _ = engine.cmd.send(Command::DeckControl {
            source: 9,
            deck: 0,
            control: Control::LoopMode,
        });
    }
    engine.cmd.release_midi_source(41);
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].controls.status().reverse);
    engine.cmd.release_midi_source(42);
    rt.process(&mut [0.0; 256]);
    assert!(!rt.decks[0].controls.status().reverse);
    for (deck, c) in [
        (2, Control::Keylock),
        (0, Control::Strip { value: f32::NAN }),
        (
            0,
            Control::Hold {
                button: Button::HotCue(8),
                on: true,
            },
        ),
    ] {
        assert!(engine
            .cmd
            .send(Command::DeckControl {
                source: 9,
                deck,
                control: c
            })
            .is_err());
    }
}

#[test]
fn ns7_controls_strip_cue_hotcue_preview_delete_and_play_latch_follow_audio() {
    let (_engine, mut rt) = fixture();
    control(&mut rt, 1, Control::Strip { value: 0.5 });
    assert!((rt.decks[0].pos - 288000.0).abs() < 2.0);
    hold(&mut rt, 1, Button::Cue, true);
    let cue = rt.decks[0].cue_pos;
    assert_eq!(cue, rt.decks[0].pos);
    render(&mut rt, 500);
    assert!(rt.decks[0].pos > cue);
    hold(&mut rt, 2, Button::Cue, true);
    hold(&mut rt, 1, Button::Cue, false);
    assert!(rt.decks[0].preview_position.is_some());
    hold(&mut rt, 2, Button::Cue, false);
    assert_eq!(rt.decks[0].pos, cue);
    assert!(!rt.decks[0].playing);
    assert!(rt.decks[0].preview_position.is_none());
    hold(&mut rt, 1, Button::HotCue(0), true);
    hold(&mut rt, 1, Button::HotCue(0), false);
    assert!(rt.decks[0].hotcues[0].set);
    control(&mut rt, 1, Control::Strip { value: 0.7 });
    hold(&mut rt, 1, Button::HotCue(0), true);
    render(&mut rt, 1000);
    assert!(rt.decks[0].preview_position.is_some());
    let preview_position = rt.decks[0].pos;
    rt.apply(Command::DeckPlay { deck: 0 });
    hold(&mut rt, 1, Button::HotCue(0), false);
    assert_eq!(rt.decks[0].pos, preview_position);
    assert!(rt.decks[0].playing);
    assert!(rt.decks[0].preview_position.is_none());
    hold(&mut rt, 1, Button::Delete, true);
    hold(&mut rt, 1, Button::HotCue(0), true);
    assert!(!rt.decks[0].hotcues[0].set);
    hold(&mut rt, 1, Button::HotCue(0), false);
    hold(&mut rt, 1, Button::Delete, false);
    assert!(!rt.decks[0].controls.status().delete);
}

#[test]
fn ns7_controls_manual_auto_loop_banks_scaling_shift_and_edge_edit_are_bounded() {
    let (_engine, mut rt) = fixture();
    rt.quantize = false;
    control(&mut rt, 1, Control::LoopButton { index: 0 });
    rt.decks[0].pos += 24000.0;
    control(&mut rt, 1, Control::LoopButton { index: 1 });
    assert_eq!(rt.decks[0].loop_len, 24000.0);
    assert!(rt.decks[0].loop_on);
    control(&mut rt, 1, Control::LoopToggle);
    assert!(!rt.decks[0].loop_on);
    control(&mut rt, 1, Control::LoopScale { double: true });
    assert_eq!(rt.decks[0].loop_len, 48000.0);
    assert!(!rt.decks[0].loop_on);
    control(&mut rt, 1, Control::Reloop);
    assert!(rt.decks[0].loop_on);
    assert_eq!(rt.decks[0].pos, rt.decks[0].loop_start);
    let start = rt.decks[0].loop_start;
    control(&mut rt, 1, Control::LoopShift { forward: true });
    assert_eq!(rt.decks[0].loop_start, start + 48000.0);
    control(&mut rt, 1, Control::LoopShift { forward: false });
    assert_eq!(rt.decks[0].loop_start, start);
    control(&mut rt, 1, Control::LoopSelect);
    assert_eq!(rt.decks[0].controls.status().loop_slot, 1);
    assert!(!rt.decks[0].loop_on);
    control(&mut rt, 1, Control::LoopMode);
    for button in 0..4 {
        control(&mut rt, 1, Control::LoopButton { index: button });
        assert_eq!(rt.decks[0].loop_len, 24000.0 * f64::from(1 << button));
        assert!(rt.decks[0].loop_on);
    }
    control(&mut rt, 1, Control::LoopButton { index: 3 });
    assert!(!rt.decks[0].loop_on);
    assert_eq!(rt.decks[0].loop_len, 0.0);
    control(&mut rt, 1, Control::LoopButton { index: 3 });
    assert!(rt.decks[0].loop_on);
    control(&mut rt, 1, Control::LoopMode);
    control(&mut rt, 1, Control::LoopButton { index: 0 });
    rt.decks[0].vinyl = false;
    assert!(rt.controller_loop_edit(0, 100));
    assert!(rt.controller_loop_edit(0, 110));
    assert_eq!(rt.decks[0].loop_start, start + 240.0);
    control(&mut rt, 1, Control::LoopButton { index: 0 });
    assert!(!rt.controller_loop_edit(0, 120));
    for _ in 0..7 {
        control(&mut rt, 1, Control::LoopSelect);
    }
    assert_eq!(rt.decks[0].controls.status().loop_slot, 0);
    assert_eq!(rt.decks[0].loop_len, 48000.0);
    control(&mut rt, 1, Control::Reloop);
    rt.decks[0].playing = true;
    hold(&mut rt, 1, Button::Reverse, true);
    render(&mut rt, 100000);
    assert!(
        rt.decks[0].pos >= rt.decks[0].loop_start
            && rt.decks[0].pos < rt.decks[0].loop_start + rt.decks[0].loop_len
    );
}

#[test]
fn ns7_controls_transport_ramps_fader_start_reverse_and_warm_audio_do_not_allocate() {
    let (_engine, mut rt) = fixture();
    control(&mut rt, 1, Control::StartTime { value: 0.025 });
    control(&mut rt, 1, Control::StopTime { value: 0.025 });
    rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut rt, 2400);
    assert!(rt.decks[0].rate > 0.45 && rt.decks[0].rate < 0.51);
    render(&mut rt, 3000);
    assert!((rt.decks[0].rate - 1.0).abs() < 0.0001);
    rt.apply(Command::DeckPlay { deck: 0 });
    assert!(rt.decks[0].controls.braking);
    render(&mut rt, 6000);
    assert!(!rt.decks[0].controls.braking);
    assert!(!rt.decks[0].playing);
    assert!(rt.decks[0]
        .last_output
        .iter()
        .all(|value| value.abs() < 0.0001));
    rt.apply(Command::FaderStart { deck: 0, on: true });
    rt.apply(Command::Xfader(1.0));
    rt.apply(Command::Xfader(0.99));
    assert!(rt.decks[0].playing);
    rt.apply(Command::Xfader(1.0));
    assert!(!rt.decks[0].playing);
    assert_eq!(rt.decks[0].pos, 0.0);
    rt.apply(Command::XfaderReverse(true));
    assert_eq!(rt.crossfader_position(), 0.0);
    rt.apply(Command::FaderStart { deck: 0, on: false });
    control(&mut rt, 1, Control::StartTime { value: 0.0 });
    rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut rt, 1000);
    let counts = test_alloc::measure(|| {
        hold(&mut rt, 1, Button::Bleep, true);
        render(&mut rt, 1024);
        hold(&mut rt, 1, Button::Bleep, false);
        hold(&mut rt, 1, Button::Reverse, true);
        render(&mut rt, 1024);
        hold(&mut rt, 1, Button::Reverse, false);
        hold(&mut rt, 1, Button::BendUp, true);
        render(&mut rt, 1024);
        hold(&mut rt, 1, Button::BendUp, false);
    });
    assert_eq!(counts, test_alloc::Counts::default());
}

#[test]
fn ns7_controls_keylock_keeps_pitch_during_free_rotation_and_bypasses_for_scratching() {
    let (_engine, mut rt) = fixture();
    rt.decks[0].pitch_range = 1;
    rt.decks[0].pitch = 1.0;
    rt.apply(Command::DeckPlay { deck: 0 });
    control(&mut rt, 1, Control::Keylock);
    let at = Instant::now();
    let mut sine = 0.0;
    let mut cosine = 0.0;
    for frame in 0..72000 {
        if frame % 240 == 0 {
            let now = at + Duration::from_secs_f64(f64::from(frame) / 48000.0);
            rt.apply(Command::DeckSpindle {
                source: 1,
                deck: 0,
                motion: spindle::Motion {
                    ticks: i64::from(frame) / 24,
                    rate: 0.94,
                    at: now,
                    hold: 0.02,
                },
            });
            rt.decks[0].spindle.as_mut().unwrap().begin(now);
        }
        let (l, r) = rt.render_deck(0);
        assert!((l + r).abs() < 1e-6);
        if frame >= 48000 {
            let phase = std::f64::consts::TAU * 440.0 * f64::from(frame) / 48000.0;
            sine += f64::from(l) * phase.sin();
            cosine += f64::from(l) * phase.cos();
        }
    }
    assert_eq!(
        rt.decks[0].keylock_mode(),
        crate::engine::keylock::Mode::Locked
    );
    assert!(2.0 * sine.hypot(cosine) / 24000.0 > 0.2);
    for rate in [0.0, -1.0, 0.5, 1.5] {
        rt.apply(Command::DeckSpindle {
            source: 1,
            deck: 0,
            motion: spindle::Motion {
                ticks: 3000,
                rate,
                at: Instant::now(),
                hold: 0.02,
            },
        });
        render(&mut rt, 256);
        assert_ne!(
            rt.decks[0].keylock_mode(),
            crate::engine::keylock::Mode::Locked
        );
    }
}

#[test]
fn ns7_controls_cue_preview_plays_with_a_stopped_physical_spindle() {
    let (_engine, mut rt) = fixture();
    rt.apply(Command::DeckSpindle {
        source: 42,
        deck: 0,
        motion: spindle::Motion {
            ticks: 0,
            rate: 0.0,
            at: Instant::now(),
            hold: 0.008,
        },
    });
    hold(&mut rt, 1, Button::Cue, true);
    let mut energy = 0.0;
    for _ in 0..2000 {
        let (l, r) = rt.render_deck(0);
        energy += l * l + r * r;
    }
    assert!(energy > 20.0);
    assert!(rt.decks[0].pos > rt.decks[0].cue_pos + 1800.0);
    hold(&mut rt, 1, Button::Cue, false);
    render(&mut rt, 1000);
    assert_eq!(rt.decks[0].last_output, [0.0; 2]);
}

#[test]
fn ns7_controls_tap_updates_preparation_tempo_without_overriding_a_locked_grid() {
    let (_engine, mut rt) = fixture();
    rt.decks[0].controls.taps[0] = Some(Instant::now() - Duration::from_secs(1));
    rt.decks[0].controls.tap_cursor = 1;
    control(&mut rt, 1, Control::Tap);
    assert!((rt.decks[0].musical_bpm() - 60.0).abs() < 0.1);
    let grid = crate::engine::beatgrid::Grid::new(0.0, 120.0).unwrap();
    let receipt = crate::engine::load_receipt::Receipt::new();
    receipt.set_grid_protection(true, Some(grid));
    rt.decks[0].load_receipt = Some(receipt);
    rt.decks[0].grid = Some(grid);
    control(&mut rt, 1, Control::Tap);
    assert_eq!(rt.decks[0].grid, Some(grid));
}

#[test]
fn ns7_controls_loop_undo_restores_inactive_length_and_selected_bank() {
    let (_engine, mut rt) = fixture();
    rt.quantize = false;
    control(&mut rt, 1, Control::LoopMode);
    control(&mut rt, 1, Control::LoopButton { index: 2 });
    control(&mut rt, 1, Control::LoopToggle);
    let length = rt.decks[0].loop_len;
    control(&mut rt, 1, Control::LoopScale { double: true });
    assert_eq!(rt.decks[0].loop_len, length * 2.0);
    assert!(!rt.decks[0].loop_on);
    rt.apply(Command::Undo);
    assert_eq!(rt.decks[0].loop_len, length);
    assert!(!rt.decks[0].loop_on);
    rt.apply(Command::Redo);
    assert_eq!(rt.decks[0].loop_len, length * 2.0);
    assert!(!rt.decks[0].loop_on);
    control(&mut rt, 1, Control::LoopSelect);
    assert_eq!(rt.decks[0].controls.status().loop_slot, 1);
    rt.apply(Command::Undo);
    assert_eq!(rt.decks[0].controls.status().loop_slot, 0);
    assert_eq!(rt.decks[0].loop_len, length * 2.0);
}

#[test]
fn ns7_controls_bleep_return_uses_the_real_free_spindle_speed() {
    let (_engine, mut rt) = fixture();
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.apply(Command::DeckSpindle {
        source: 42,
        deck: 0,
        motion: spindle::Motion {
            ticks: 0,
            rate: 0.94,
            at: Instant::now(),
            hold: 0.05,
        },
    });
    render(&mut rt, 500);
    let start = rt.decks[0].pos;
    hold(&mut rt, 1, Button::Bleep, true);
    render(&mut rt, 1200);
    assert!(rt.decks[0].pos < start - 1000.0);
    hold(&mut rt, 1, Button::Bleep, false);
    assert!((rt.decks[0].pos - start - f64::from(0.94_f32) * 1200.0).abs() < 0.01);
}
