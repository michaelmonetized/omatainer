use super::*;
use crate::engine::{beatgrid::Grid, test_alloc, Engine, Sample};
use std::sync::Arc;

fn fixture(source_rate: u32, output_rate: u32, grid: Option<Grid>) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(output_rate, 64);
    let audio = Arc::new(Sample {
        spectrum: None, name: "Beat jump reference".into(), sr: source_rate, ch: 2,
        data: (0..source_rate * 16).flat_map(|frame| {
            let time = f64::from(frame) / f64::from(source_rate);
            [(time * 711.0 + time * time * 19.0).sin() as f32 * 0.3,
                (time * 913.0 + time * time * 23.0).cos() as f32 * 0.2]
        }).collect(), peaks: Vec::new().into(), bpm: 120.0, path: String::new(),
    });
    rt.apply(Command::DeckAudio { deck: 0, audio });
    rt.decks[0].grid = grid;
    (engine, rt)
}
fn command(control: Control) -> Command { Command::DeckControl { source: 73, deck: 0, control } }

#[test]
fn constant_and_variable_jumps_null_match_independent_source_positions_at_every_rate() {
    for source_rate in [44_100, 48_000, 96_000] {
        for output_rate in [44_100, 48_000, 96_000] {
            for variable in [false, true] {
                let grid = Grid::new(0.25, 120.0).unwrap();
                let grid = if variable { grid.with_anchor(4.0, 2.25).unwrap().with_anchor(8.0, 5.25).unwrap() } else { grid };
                let seconds = |beat: f64| if variable && beat >= 4.0 { 2.25 + (beat - 4.0) * 0.75 } else { 0.25 + beat * 0.5 };
                for keylock in [false, true] {
                    let (_engine, mut jumped) = fixture(source_rate, output_rate, Some(grid));
                    let (_reference_engine, mut reference) = fixture(source_rate, output_rate, Some(grid));
                    for rt in [&mut jumped, &mut reference] {
                        rt.decks[0].pos = seconds(3.375) * f64::from(source_rate);
                        rt.decks[0].pitch = 0.625;
                        if keylock { rt.apply(Command::DeckKeylock { deck: 0 }); }
                        rt.apply(Command::DeckPlay { deck: 0 });
                    }
                    let playing = jumped.decks[0].playing;
                    let cue = jumped.decks[0].cue_pos;
                    let expected = seconds(7.375) * f64::from(source_rate);
                    assert_eq!(test_alloc::measure(|| jumped.apply(command(Control::BeatJump { forward: true }))), test_alloc::Counts::default());
                    assert!((jumped.decks[0].pos - expected).abs() < 1.0e-7);
                    assert_eq!(jumped.decks[0].playing, playing);
                    assert_eq!(jumped.decks[0].cue_pos, cue);
                    reference.decks[0].transition_to(expected, output_rate as f32, DeckTransition::Jump);
                    let mut peak_error = 0.0f32;
                    let mut energy = 0.0f64;
                    let counts = test_alloc::measure(|| for _ in 0..2048 {
                        let actual = jumped.render_deck(0); let expected = reference.render_deck(0);
                        for (a, b) in [(actual.0, expected.0), (actual.1, expected.1)] {
                            assert!(a.is_finite()); peak_error = peak_error.max((a - b).abs()); energy += f64::from(a).powi(2);
                        }
                    });
                    assert_eq!(counts, test_alloc::Counts::default());
                    assert!(peak_error < 0.000001, "{source_rate}/{output_rate}, variable={variable}, keylock={keylock}: {peak_error}");
                    assert!(energy > 0.01);
                    jumped.decks[0].playing = false;
                    jumped.decks[0].pos = expected;
                    jumped.apply(command(Control::BeatJump { forward: false }));
                    assert!((jumped.decks[0].pos - seconds(3.375) * f64::from(source_rate)).abs() < 1.0e-7);
                    assert!(!jumped.decks[0].playing);
                }
            }
        }
    }
}

#[test]
fn sizes_bounds_fallback_phase_and_loop_wrap_keep_preparation_and_transport_unchanged() {
    let (engine, mut rt) = fixture(48_000, 48_000, None);
    for (index, beats) in BEAT_JUMP_SIZES.into_iter().enumerate() {
        rt.decks[0].pos = 96_750.0;
        rt.apply(command(Control::BeatJumpSize { index: index as u8 }));
        rt.apply(command(Control::BeatJump { forward: true }));
        let expected = (96_750.0 + f64::from(beats) * 24_000.0).min(768_000.0);
        assert_eq!(rt.decks[0].pos, expected);
        assert!(!rt.decks[0].playing);
        assert_eq!(rt.decks[0].controls.status().beat_jump_size, index as u8);
    }
    rt.apply(command(Control::BeatJumpScale { up: true })); assert_eq!(rt.decks[0].controls.beat_jump_size, 9);
    for _ in 0..20 { rt.apply(command(Control::BeatJumpScale { up: false })); }
    assert_eq!(rt.decks[0].controls.beat_jump_size, 0);
    for size in 10..=255 { assert!(engine.cmd.send(command(Control::BeatJumpSize { index: size })).is_err()); }
    rt.decks[0].pos = 10.0; rt.apply(command(Control::BeatJump { forward: false })); assert_eq!(rt.decks[0].pos, 0.0);
    let grid = Grid::new(0.25, 120.0).unwrap().with_anchor(4.0, 2.25).unwrap().with_anchor(8.0, 5.25).unwrap();
    rt.decks[0].grid = Some(grid);
    rt.decks[0].loop_start = 2.25 * 48_000.0; rt.decks[0].loop_len = 3.0 * 48_000.0; rt.decks[0].loop_on = true;
    rt.decks[0].pos = 4.40625 * 48_000.0;
    rt.apply(command(Control::BeatJumpSize { index: 3 }));
    let history = engine.undo.checkpoint();
    assert_eq!(test_alloc::measure(|| rt.apply(command(Control::BeatJump { forward: true }))), test_alloc::Counts::default());
    assert!((rt.decks[0].pos - 5.15625 * 48_000.0).abs() < 1.0e-7);
    rt.apply(command(Control::BeatJump { forward: true }));
    assert!((rt.decks[0].pos - 2.90625 * 48_000.0).abs() < 1.0e-7);
    rt.apply(command(Control::BeatJump { forward: false }));
    assert!((rt.decks[0].pos - 5.15625 * 48_000.0).abs() < 1.0e-7);
    assert_eq!(rt.decks[0].loop_start, 108_000.0); assert_eq!(rt.decks[0].loop_len, 144_000.0); assert!(rt.decks[0].loop_on);
    assert_eq!(engine.undo.checkpoint(), history);
    rt.decks[0].loop_on = false;
    rt.decks[0].controls.saved_loop = Some((true, 0.0, f64::INFINITY));
    rt.decks[0].controls.performance_forward = Some(192_000.0);
    let before = rt.decks[0].pos;
    rt.apply(command(Control::BeatJump { forward: true }));
    assert_eq!(rt.decks[0].pos, before); assert_eq!(rt.decks[0].controls.performance_forward, Some(192_000.0));
    rt.decks[0].controls.release(); rt.decks[0].controls.media_changed(); assert_eq!(rt.decks[0].controls.beat_jump_size, 3);
}

#[test]
fn held_bleep_roll_and_slip_clocks_keep_the_jump_after_release() {
    let (_engine, mut rt) = fixture(48_000, 48_000, None);
    rt.decks[0].pos = 144_000.0; rt.decks[0].playing = true;
    rt.apply(command(Control::Hold { button: Button::Bleep, on: true }));
    rt.decks[0].pos = 120_000.0;
    rt.apply(command(Control::BeatJump { forward: true }));
    assert_eq!(rt.decks[0].pos, 216_000.0);
    rt.apply(command(Control::Hold { button: Button::Bleep, on: false })); assert_eq!(rt.decks[0].pos, 240_000.0);
    rt.apply(command(Control::Hold { button: Button::Roll(4), on: true }));
    let loop_bounds = (rt.decks[0].loop_start, rt.decks[0].loop_len);
    rt.apply(command(Control::BeatJump { forward: true }));
    assert_eq!((rt.decks[0].loop_start, rt.decks[0].loop_len), loop_bounds);
    rt.apply(command(Control::Hold { button: Button::Roll(4), on: false })); assert_eq!(rt.decks[0].pos, 336_000.0);
    rt.apply(command(Control::Slip)); rt.decks[0].controls.slip_forward = Some(360_000.0);
    rt.apply(command(Control::BeatJump { forward: false })); rt.deck_surface_tick(0);
    assert_eq!(rt.decks[0].pos, 264_000.0);
}
