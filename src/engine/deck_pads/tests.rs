use super::*;
use crate::engine::{
    deck_controls::{Button, Control},
    test_alloc, Command, Engine, RtEngine,
};

fn press(rt: &mut RtEngine, source: u64, key: u32, deck: u8, id: u8, mode: Mode) {
    rt.apply(Command::DeckPadPress(Press {
        source,
        key,
        deck,
        id,
        mode: Some(mode),
        pressure: 0.8,
        shifted: false,
    }));
}
fn release(rt: &mut RtEngine, source: u64, key: u32) {
    rt.apply(Command::DeckPadRelease(Release { source, key }));
}
fn select(rt: &mut RtEngine, deck: u8, mode: Mode) {
    rt.apply(Command::DeckControl {
        source: 81,
        deck,
        control: Control::PadMode { mode: mode.index() },
    });
}
fn direct(rt: &mut RtEngine, source: u64, deck: u8, button: Button, on: bool) {
    rt.apply(Command::DeckControl {
        source,
        deck,
        control: Control::Hold { button, on },
    });
}
fn render(rt: &mut RtEngine, frames: usize) {
    rt.process(&mut vec![0.0; frames * 2]);
}

#[test]
fn deck_pad_mode_change_restores_original_loop_and_preserves_other_deck_and_direct_owners_without_heap_work(
) {
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.apply(Command::DeckSeek {
        deck: 0,
        frac: 0.25,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    let original = (
        rt.decks[0].loop_on,
        rt.decks[0].loop_start,
        rt.decks[0].loop_len,
    );
    let start = rt.decks[0].pos;
    assert_eq!(
        test_alloc::measure(|| press(&mut rt, 11, 31, 0, 4, Mode::Roll)),
        Default::default()
    );
    press(&mut rt, 12, 41, 1, 3, Mode::Slice);
    direct(&mut rt, 11, 0, Button::Reverse, true);
    render(&mut rt, 48);
    assert_eq!(
        test_alloc::measure(|| select(&mut rt, 0, Mode::Sampler)),
        Default::default()
    );
    assert_eq!(
        (
            rt.decks[0].loop_on,
            rt.decks[0].loop_start,
            rt.decks[0].loop_len
        ),
        original
    );
    assert!(rt.decks[0].controls.status().roll.is_none());
    assert!((rt.decks[0].pos - (start + 48.0)).abs() < 0.01);
    assert!(rt.decks[0].controls.status().reverse);
    assert_eq!(rt.decks[1].controls.status().slice, Some(2));
    press(&mut rt, 11, 31, 0, 4, Mode::Roll);
    assert_eq!(
        rt.decks[0].controls.status().pad_mode,
        Mode::Sampler.index()
    );
    release(&mut rt, 11, 31);
    assert!(rt.decks[0].controls.status().reverse);
    release(&mut rt, 12, 41);
    assert!(rt.decks[1].controls.status().slice.is_none());
    direct(&mut rt, 11, 0, Button::Reverse, false);
    press(&mut rt, 11, 31, 0, 4, Mode::Roll);
    assert_eq!(rt.decks[0].controls.status().roll, Some(3));
    release(&mut rt, 11, 31);
}

#[test]
fn deck_pad_mode_cancel_releases_only_keyed_roll_and_preview_owners() {
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 256);
    direct(&mut rt, 41, 0, Button::Roll(1), true);
    press(&mut rt, 41, 99, 0, 2, Mode::Roll);
    select(&mut rt, 0, Mode::HotCue);
    assert_eq!(rt.decks[0].controls.status().roll, Some(1));
    release(&mut rt, 41, 99);
    direct(&mut rt, 41, 0, Button::Roll(1), false);
    rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 0,
        del: false,
    });
    press(&mut rt, 41, 99, 0, 1, Mode::HotCue);
    direct(&mut rt, 41, 0, Button::HotCue(0), true);
    assert!(rt.decks[0].preview_position.is_some());
    select(&mut rt, 0, Mode::Slice);
    assert!(rt.decks[0].preview_position.is_some());
    release(&mut rt, 41, 99);
    direct(&mut rt, 41, 0, Button::HotCue(0), false);
    assert!(rt.decks[0].preview_position.is_none());
}

#[test]
fn deck_pad_sampler_release_keeps_original_voice_bank_and_independent_direct_owner() {
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 256);
    press(&mut rt, 21, 101, 0, 1, Mode::Sampler);
    rt.pad_voices[0].as_mut().unwrap().playback.mode = crate::sampler_bank::PlayMode::Hold;
    let audio = rt.pad_voices[0].as_ref().unwrap().audio.clone();
    rt.apply(Command::SamplerBank(1));
    assert!(std::sync::Arc::ptr_eq(
        &audio,
        &rt.pad_voices[0].as_ref().unwrap().audio
    ));
    rt.surface_sampler(21, 0, true, 0.5);
    rt.pad_voices[0].as_mut().unwrap().playback.mode = crate::sampler_bank::PlayMode::Hold;
    select(&mut rt, 0, Mode::Roll);
    assert!(rt.pad_voices[0].is_some());
    release(&mut rt, 21, 101);
    assert!(rt.pad_voices[0].is_some());
    rt.surface_sampler(21, 0, false, 0.0);
    assert!(rt.pad_voices[0].is_none());
}

#[test]
fn deck_pad_reserved_release_survives_full_admission_and_source_retirement_preserves_another_source(
) {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 32);
    for source in [71, 72] {
        engine
            .cmd
            .send(Command::DeckPadPress(Press {
                source,
                key: 101,
                deck: 0,
                id: 1,
                mode: Some(Mode::Roll),
                pressure: 1.0,
                shifted: false,
            }))
            .unwrap();
    }
    rt.process(&mut []);
    while engine
        .cmd
        .send(Command::DeckControl {
            source: 1,
            deck: 1,
            control: Control::Slip,
        })
        .is_ok()
    {}
    engine.cmd.release_midi_source(71);
    while !rt.cmd_rx.is_empty() {
        rt.process(&mut []);
    }
    assert_eq!(rt.decks[0].controls.status().roll, Some(0));
    engine.cmd.release_midi_source(72);
    while !rt.cmd_rx.is_empty() {
        rt.process(&mut []);
    }
    assert!(rt.decks[0].controls.status().roll.is_none());
    assert_eq!(engine.cmd.queue_pressure().reserved_releases, 0);
}

#[test]
fn deck_pad_parameters_apply_to_current_mode_and_deck_at_ordered_command_boundary() {
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 256);
    select(&mut rt, 0, Mode::Slice);
    let before = rt.decks[0].controls.status();
    rt.apply(Command::DeckPadParameter {
        source: 77,
        deck: 0,
        up: true,
        shifted: false,
    });
    assert_eq!(
        rt.decks[0].controls.status().slice_quant,
        (before.slice_quant + 1).min(3)
    );
    rt.apply(Command::DeckPadParameter {
        source: 77,
        deck: 0,
        up: true,
        shifted: true,
    });
    assert_eq!(
        rt.decks[0].controls.status().slice_domain,
        (before.slice_domain + 1).min(6)
    );
    assert_eq!(
        rt.decks[1].controls.status().slice_domain,
        before.slice_domain
    );
    select(&mut rt, 1, Mode::Roll);
    rt.apply(Command::DeckPadParameter {
        source: 77,
        deck: 1,
        up: false,
        shifted: false,
    });
    assert_eq!(rt.decks[1].controls.status().roll_scale, -1);
    assert_eq!(rt.decks[0].controls.status().roll_scale, 0);
}

#[test]
fn deck_pad_invalid_inputs_never_reserve_or_mutate_and_note_off_keys_do_not_depend_on_assignment() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let valid = Press {
        source: 1,
        key: 8,
        deck: 0,
        id: 1,
        mode: None,
        pressure: 1.0,
        shifted: false,
    };
    for invalid in [
        Press { deck: 2, ..valid },
        Press { id: 0, ..valid },
        Press { id: 9, ..valid },
        Press {
            pressure: f32::NAN,
            ..valid
        },
        Press {
            pressure: 1.1,
            ..valid
        },
    ] {
        assert!(engine.cmd.send(Command::DeckPadPress(invalid)).is_err());
        rt.apply(Command::DeckPadPress(invalid));
    }
    assert_eq!(engine.cmd.queue_pressure().reserved_releases, 0);
    assert_eq!(wire_release(&[0x82, 60, 45], 7).unwrap().key, 2 * 128 + 60);
    assert_eq!(wire_release(&[0x92, 60, 0], 7).unwrap().key, 2 * 128 + 60);
    for bytes in [
        &[][..],
        &[0x82, 60][..],
        &[0x82, 128, 0][..],
        &[0xb2, 60, 0][..],
    ] {
        assert!(wire_release(bytes, 7).is_none());
    }
    assert_eq!(sp1_key(7, 3), sp1_key(9, 0x7b));
    assert_ne!(sp1_key(7, 3), sp1_key(8, 3));
    assert_eq!(Mode::ALL.map(Mode::index), [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn deck_pad_safety_clears_old_ownership_and_allows_fresh_post_recovery_keys() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let onset = Press {
        source: 81,
        key: 201,
        deck: 0,
        id: 1,
        mode: Some(Mode::Roll),
        pressure: 1.0,
        shifted: false,
    };
    engine.cmd.send(Command::DeckPadPress(onset)).unwrap();
    rt.process(&mut []);
    assert_eq!(rt.decks[0].controls.status().roll, Some(0));
    engine
        .cmd
        .send(Command::SafetyStop(
            crate::engine::performance::Safety::Silence,
        ))
        .unwrap();
    rt.process(&mut [0.0; 512]);
    assert!(rt.decks[0].controls.status().roll.is_none());
    engine
        .cmd
        .send(Command::DeckPadRelease(Release {
            source: 81,
            key: 201,
        }))
        .unwrap();
    rt.process(&mut []);
    engine.cmd.send(Command::RecoverPerformance).unwrap();
    rt.process(&mut [0.0; 48000]);
    assert!(!engine.cmd.performance().status().recovery);
    press(&mut rt, 81, 201, 0, 1, Mode::Roll);
    assert_eq!(rt.decks[0].controls.status().roll, Some(0));
    release(&mut rt, 81, 201);
    assert!(rt.decks[0].controls.status().roll.is_none());
}
