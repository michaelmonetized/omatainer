use super::*;
use crate::engine::{Engine, Sample, test_alloc};
use std::sync::Arc;

fn fixture(capacity: usize) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(48_000, capacity);
    let audio = Arc::new(Sample { spectrum: None, name: "Cue audition stereo reference".into(), sr: 48_000, ch: 2,
        data: (0..48_000 * 4).flat_map(|f| [(f as f64 * 0.011).sin() as f32 * 0.4, (f as f64 * 0.019).cos() as f32 * 0.3]).collect(),
        peaks: Vec::new().into(), bpm: 120.0, path: String::new() });
    for deck in 0..2 { rt.apply(Command::DeckAudio { deck, audio: audio.clone() }); rt.apply(Command::DeckSeek { deck, frac: 0.25 }); }
    (engine, rt)
}
fn hold(source: u64, deck: u8, button: Button, on: bool) -> Command { Command::DeckControl { source, deck, control: Control::Hold { button, on } } }
fn render(rt: &mut RtEngine, frames: usize) -> f64 {
    let mut energy = 0.0;
    for _ in 0..frames { let (left, right) = rt.render_deck(0); assert!(left.is_finite() && right.is_finite()); energy += f64::from(left).powi(2) + f64::from(right).powi(2); }
    energy
}

#[test]
fn cue_hold_audio_matches_ordinary_source_playback_and_taps_return_without_callback_heap_work() {
    let (_engine, mut actual) = fixture(64);
    let (_reference_engine, mut reference) = fixture(64);
    reference.apply(Command::DeckCue { deck: 0 }); reference.apply(Command::DeckPlay { deck: 0 });
    actual.apply(hold(41, 0, Button::Cue, true));
    let cue = actual.decks[0].cue_pos;
    let other = actual.decks[1].pos;
    let mut energy = [0.0; 2]; let mut error = 0.0f32;
    assert_eq!(test_alloc::measure(|| for _ in 0..4096 {
        let a = actual.render_deck(0); let b = reference.render_deck(0);
        for (index, (a, b)) in [(a.0, b.0), (a.1, b.1)].into_iter().enumerate() { error = error.max((a-b).abs()); energy[index] += f64::from(a).powi(2); }
    }), test_alloc::Counts::default());
    assert!(energy.iter().all(|e| *e > 1.0)); assert!(error < 0.000001);
    assert_eq!(test_alloc::measure(|| actual.apply(hold(41, 0, Button::Cue, false))), test_alloc::Counts::default());
    assert_eq!(actual.decks[0].pos, cue); assert!(!actual.decks[0].playing); assert_eq!(actual.decks[1].pos, other);
    for _ in 0..16 {
        assert_eq!(test_alloc::measure(|| actual.apply(hold(41, 0, Button::Cue, true))), test_alloc::Counts::default());
        assert!(render(&mut actual, 256) > 0.01);
        actual.apply(hold(41, 0, Button::Cue, false)); assert_eq!(actual.decks[0].pos, cue);
    }
    render(&mut actual, 256); assert_eq!(actual.decks[0].last_output, [0.0; 2]);
}

#[test]
fn every_cue_and_play_order_has_explicit_latch_stop_and_return_behavior() {
    for (events, playing, returned) in [
        (vec![0, 1], false, true), (vec![0, 2, 1], true, false),
        (vec![2, 0, 1], false, true), (vec![0, 1, 2], true, true),
        (vec![0, 2, 2, 1], false, false), (vec![2, 0, 2, 1], true, true),
    ] {
        let (_engine, mut rt) = fixture(64); rt.apply(Command::DeckCue { deck: 0 });
        let cue = rt.decks[0].cue_pos;
        for event in &events {
            rt.apply(match event { 0 => hold(41, 0, Button::Cue, true), 1 => hold(41, 0, Button::Cue, false), _ => Command::DeckPlay { deck: 0 } });
            if *event != 1 { render(&mut rt, 512); }
        }
        assert_eq!(rt.decks[0].playing, playing, "{events:?}");
        if returned && !playing { assert_eq!(rt.decks[0].pos, cue, "{events:?}"); }
        assert!(rt.decks[0].preview_position.is_none()); assert!(rt.decks[0].controls.preview.is_none());
        let energy = render(&mut rt, 1024);
        if playing { assert!(energy > 0.1); } else { assert_eq!(rt.decks[0].last_output, [0.0; 2]); }
        assert!(!rt.decks[1].playing);
    }
}

#[test]
fn independent_cue_owners_survive_overload_and_retire_on_source_project_and_safety_boundaries() {
    let (engine, mut rt) = fixture(64);
    engine.send(hold(41, 0, Button::Cue, true)).unwrap(); engine.send(hold(42, 0, Button::Cue, true)).unwrap();
    rt.process(&mut []); let cue = rt.decks[0].cue_pos; render(&mut rt, 512);
    while engine.send(Command::DeckPlay { deck: 1 }).is_ok() {}
    engine.send(hold(41, 0, Button::Cue, false)).unwrap(); engine.send(hold(42, 0, Button::Cue, false)).unwrap();
    assert_eq!(test_alloc::measure(|| for _ in 0..3 {
        rt.process(&mut []); assert!(rt.command_stats.received_last_block <= crate::engine::control::COMMANDS_PER_BLOCK);
    }), test_alloc::Counts::default());
    assert!(rt.cmd_rx.is_empty()); assert_eq!(rt.decks[0].pos, cue); assert!(!rt.decks[0].controls.held(Button::Cue));
    engine.send(hold(41, 0, Button::Cue, true)).unwrap(); engine.send(hold(42, 0, Button::Cue, true)).unwrap(); rt.process(&mut []);
    engine.cmd.release_midi_source(41); rt.process(&mut []); assert!(rt.decks[0].preview_position.is_some());
    engine.cmd.release_midi_source(42); rt.process(&mut []); assert!(rt.decks[0].preview_position.is_none());
    rt.apply(hold(43, 0, Button::Cue, true)); rt.decks[0].stop_preview(48_000.0);
    assert!(rt.decks[0].controls.preview.is_none()); rt.apply(hold(43, 0, Button::Cue, false));
    for boundary in 0..3 {
        let (_engine, mut rt) = fixture(64);
        rt.apply(hold(41, 0, Button::Cue, true));
        match boundary { 0 => rt.apply(Command::DeckUnload { deck: 0 }), 1 => { crate::engine::project::Prepared::empty(48_000).unwrap().swap_into(&mut rt); }, _ => rt.apply(Command::SafetyStop(crate::engine::performance::Safety::Stop)) }
        rt.apply(hold(41, 0, Button::Cue, false));
        assert!(!rt.decks[0].playing); assert!(rt.decks[0].preview_position.is_none()); assert!(!rt.decks[0].controls.held(Button::Cue));
    }
    rt.apply(Command::DeckUnload { deck: 0 }); rt.apply(hold(44, 0, Button::Cue, true)); assert!(rt.decks[0].preview_position.is_none());
}
