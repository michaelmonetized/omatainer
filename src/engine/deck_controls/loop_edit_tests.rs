use super::*;
use crate::engine::{beatgrid::Grid, test_alloc, Engine, Sample};
use std::sync::Arc;

fn fixture(source_rate: u32, output_rate: u32) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(output_rate, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample {
            spectrum: None,
            name: "Loop editing reference".into(),
            sr: source_rate,
            ch: 2,
            data: (0..source_rate * 48)
                .flat_map(|frame| {
                    let time = f64::from(frame) / f64::from(source_rate);
                    [
                        (time * 67.0).sin() as f32 * 0.3,
                        (time * 89.0).cos() as f32 * 0.2,
                    ]
                })
                .collect(),
            peaks: Vec::new().into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.decks[0].grid = Some(
        Grid::new(0.25, 120.0)
            .unwrap()
            .with_anchor(4.0, 2.25)
            .unwrap()
            .with_anchor(8.0, 5.25)
            .unwrap(),
    );
    (engine, rt)
}
fn command(control: Control) -> Command {
    Command::DeckControl {
        source: 77,
        deck: 0,
        control,
    }
}

#[test]
fn loop_media_keys_roundtrip_without_json_number_precision_loss() {
    for media_key in [1, (1_u64 << 63) + 1, u64::MAX] {
        let control = Control::LoopMove {
            media_key,
            beats: 0.125,
        };
        let json = serde_json::to_value(control).unwrap();
        assert_eq!(json["media_key"], media_key.to_string());
        assert!(
            matches!(serde_json::from_value::<Control>(json).unwrap(), Control::LoopMove { media_key: key, .. } if key==media_key)
        );
    }
}
fn region(rt: &RtEngine) -> (bool, f64, f64) {
    let d = &rt.decks[0];
    (d.loop_on, d.loop_start, d.loop_len)
}

#[test]
fn musical_moves_cross_tempo_anchors_preserve_phase_and_match_independent_stereo_rendering() {
    for source_rate in [44_100, 48_000, 96_000] {
        for output_rate in [44_100, 48_000, 96_000] {
            let (_engine, mut actual) = fixture(source_rate, output_rate);
            let (_reference_engine, mut reference) = fixture(source_rate, output_rate);
            for rt in [&mut actual, &mut reference] {
                let d = &mut rt.decks[0];
                d.loop_start = 1.25 * f64::from(source_rate);
                d.loop_len = 2.5 * f64::from(source_rate);
                d.loop_on = true;
                d.pos = 1.5 * f64::from(source_rate);
                d.playing = true;
            }
            let key = actual.decks[0].history_key;
            assert_eq!(
                test_alloc::measure(|| actual.apply(command(Control::LoopMove {
                    media_key: key,
                    beats: 4.0
                }))),
                test_alloc::Counts::default()
            );
            assert!((actual.decks[0].loop_start / f64::from(source_rate) - 3.75).abs() < 1e-10);
            assert!((actual.decks[0].loop_len / f64::from(source_rate) - 3.0).abs() < 1e-10);
            assert!((actual.decks[0].pos / f64::from(source_rate) - 4.125).abs() < 1e-10);
            let d = &mut reference.decks[0];
            d.loop_start = 3.75 * f64::from(source_rate);
            d.loop_len = 3.0 * f64::from(source_rate);
            d.transition_to(
                4.125 * f64::from(source_rate),
                output_rate as f32,
                DeckTransition::Jump,
            );
            let counts = test_alloc::measure(|| {
                for _ in 0..1024 {
                    let (al, ar) = actual.render_deck(0);
                    let (bl, br) = reference.render_deck(0);
                    assert!((al - bl).abs() < 0.000001 && (ar - br).abs() < 0.000001);
                }
            });
            assert_eq!(counts, test_alloc::Counts::default());
            assert!(actual.decks[0].playing);
            let before = actual.decks[0].cue_pos;
            actual.apply(command(Control::LoopMove {
                media_key: key,
                beats: -4.0,
            }));
            assert!((actual.decks[0].loop_start / f64::from(source_rate) - 1.25).abs() < 1e-10);
            assert!((actual.decks[0].loop_len / f64::from(source_rate) - 2.5).abs() < 1e-10);
            assert_eq!(actual.decks[0].cue_pos, before);
        }
    }
}

#[test]
fn exact_edges_all_lengths_file_limits_stale_media_and_performance_owners_are_atomic() {
    let (engine, mut rt) = fixture(48_000, 44_100);
    let key = rt.decks[0].history_key;
    let other = rt.decks[1].pos;
    let cue = rt.decks[0].cue_pos;
    for beats in BEAT_JUMP_SIZES {
        rt.decks[0].loop_len = 0.0;
        rt.decks[0].pos = 47.9 * 48_000.0;
        rt.apply(command(Control::LoopLength {
            media_key: key,
            beats: f64::from(beats),
        }));
        let d = &rt.decks[0];
        assert!(d.loop_start >= 0.0 && d.loop_start + d.loop_len <= 48.0 * 48_000.0);
        assert!(
            (d.grid_beats_between(d.loop_start, d.loop_start + d.loop_len, rt.sr, rt.bpm)
                - f64::from(beats))
            .abs()
                < 1e-10
        );
        assert!(!d.playing && !d.loop_on);
        assert_eq!(d.pos, 47.9 * 48_000.0);
    }
    rt.apply(command(Control::LoopBounds {
        media_key: key,
        start_seconds: 0.1,
        end_seconds: 0.1 + 64.0 / 48_000.0,
    }));
    assert!((rt.decks[0].loop_len - 64.0).abs() < 1e-8);
    for control in [
        Control::LoopBounds {
            media_key: key,
            start_seconds: 0.1,
            end_seconds: 0.1 + 63.0 / 48_000.0,
        },
        Control::LoopBounds {
            media_key: key,
            start_seconds: 1.0,
            end_seconds: 100.0,
        },
        Control::LoopLength {
            media_key: key + 1,
            beats: 4.0,
        },
    ] {
        let old = region(&rt);
        rt.apply(command(control));
        assert_eq!(region(&rt), old);
    }
    for control in [
        Control::LoopMove {
            media_key: key,
            beats: f64::NAN,
        },
        Control::LoopLength {
            media_key: 0,
            beats: 4.0,
        },
        Control::LoopBounds {
            media_key: key,
            start_seconds: 2.0,
            end_seconds: 1.0,
        },
    ] {
        assert!(engine.cmd.send(command(control)).is_err());
    }
    rt.apply(command(Control::Hold {
        button: Button::Roll(4),
        on: true,
    }));
    let old = region(&rt);
    rt.apply(command(Control::LoopLength {
        media_key: key,
        beats: 4.0,
    }));
    assert_eq!(region(&rt), old);
    rt.apply(command(Control::Hold {
        button: Button::Roll(4),
        on: false,
    }));
    assert_eq!(rt.decks[1].pos, other);
    assert_eq!(rt.decks[0].cue_pos, cue);
    rt.decks[0].grid = None;
    rt.decks[0].loop_len = 0.0;
    rt.decks[0].pos = 24_000.0;
    rt.apply(command(Control::LoopLength {
        media_key: key,
        beats: 4.0,
    }));
    assert_eq!(rt.decks[0].loop_len, 96_000.0);
    rt.apply(command(Control::LoopMove {
        media_key: key,
        beats: -64.0,
    }));
    assert_eq!(rt.decks[0].loop_start, 0.0);
}

#[test]
fn repeated_edges_and_moves_keep_transition_continuity_undo_and_preparation_roundtrips() {
    let (_engine, mut rt) = fixture(48_000, 48_000);
    let key = rt.decks[0].history_key;
    rt.apply(command(Control::LoopBounds {
        media_key: key,
        start_seconds: 1.25,
        end_seconds: 3.75,
    }));
    rt.decks[0].loop_on = true;
    rt.decks[0].pos = 1.5 * 48_000.0;
    rt.decks[0].playing = true;
    for n in 0..80 {
        for _ in 0..128 {
            rt.render_deck(0);
        }
        let before = rt.decks[0].last_output;
        let start = if n % 2 == 0 { 1.251 } else { 1.25 };
        assert_eq!(
            test_alloc::measure(|| {
                rt.apply(command(Control::LoopBounds {
                    media_key: key,
                    start_seconds: start,
                    end_seconds: 3.75,
                }));
                let (left, right) = rt.render_deck(0);
                assert!((left - before[0]).abs() < 0.001 && (right - before[1]).abs() < 0.001);
                for _ in 0..128 {
                    let (l, r) = rt.render_deck(0);
                    assert!(l.is_finite() && r.is_finite());
                }
            }),
            test_alloc::Counts::default()
        );
    }
    rt.decks[0].playing = false;
    let before = region(&rt);
    rt.apply(command(Control::LoopMove {
        media_key: key,
        beats: 4.0,
    }));
    let moved = region(&rt);
    assert_ne!(before, moved);
    rt.apply(Command::Undo);
    assert_eq!(region(&rt), before);
    rt.apply(Command::Redo);
    assert_eq!(region(&rt), moved);
    let preparation = rt.decks[0].preparation().unwrap();
    let (_reopened_engine, mut reopened) = fixture(44_100, 96_000);
    reopened.decks[0].restore_preparation(preparation);
    assert!(
        (reopened.decks[0].loop_start / 44_100.0 - rt.decks[0].loop_start / 48_000.0).abs() < 1e-10
    );
    assert!(
        (reopened.decks[0].loop_len / 44_100.0 - rt.decks[0].loop_len / 48_000.0).abs() < 1e-10
    );
}
