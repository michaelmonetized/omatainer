use super::*;
use crate::engine::{
    audio::OutputCallback,
    beatgrid::Grid,
    deck_controls::Control,
    deck_pads::{Mode, Press, Release},
    test_alloc, Command, Engine, RtEngine, Sample,
};
use std::sync::Arc;

fn seconds(beat: f64) -> f64 {
    if beat < 4.0 {
        0.125 + beat * 0.5
    } else {
        2.125 + (beat - 4.0) * 0.75
    }
}
fn beats(seconds: f64) -> f64 {
    if seconds < 2.125 {
        (seconds - 0.125) * 2.0
    } else {
        4.0 + (seconds - 2.125) / 0.75
    }
}
fn fixture(source_sr: u32, output_sr: u32) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(output_sr, 128);
    let mut data = vec![0.0; source_sr as usize * 50 * 2];
    for eighth in 0..512 {
        let start = (seconds(f64::from(eighth) / 8.0) * f64::from(source_sr)).round() as usize;
        for offset in 0..16 {
            if (start + offset) * 2 + 1 < data.len() {
                let value = (eighth % 16 + 1) as f32 / 32.0;
                data[(start + offset) * 2] = value;
                data[(start + offset) * 2 + 1] = -value;
            }
        }
    }
    let source = Arc::new(Sample {
        name: "Numbered eighth-beat stereo transients".into(),
        path: String::new(),
        sr: source_sr,
        ch: 2,
        bpm: 120.0,
        peaks: vec![].into(),
        spectrum: None,
        data,
    });
    for deck in 0..2 {
        rt.apply(Command::DeckAudio {
            deck,
            audio: source.clone(),
        });
        rt.apply(Command::DeckPlay { deck });
        let d = &mut rt.decks[usize::from(deck)];
        d.grid = Some(
            Grid::new(0.125, 120.0)
                .unwrap()
                .with_anchor(4.0, 2.125)
                .unwrap()
                .with_anchor(8.0, 5.125)
                .unwrap(),
        );
        d.pos = seconds(2.5) * f64::from(source_sr);
        d.rate = 1.0;
    }
    (engine, Box::new(rt))
}
fn control(rt: &mut RtEngine, c: Control) {
    rt.apply(Command::DeckControl {
        source: 71,
        deck: 0,
        control: c,
    });
}
fn settings(rt: &mut RtEngine, repeating: bool, domain: u8, repeat: u8, division: Option<u8>) {
    control(
        rt,
        Control::SlicerSettings {
            repeating,
            domain,
            repeat,
            division,
        },
    );
    control(rt, Control::PadMode { mode: 2 });
}
fn press(rt: &mut RtEngine, key: u32, pad: u8) {
    rt.apply(Command::DeckPadPress(Press {
        source: 71,
        key,
        deck: 0,
        id: pad + 1,
        mode: Some(Mode::Slice),
        pressure: 0.8,
        shifted: false,
    }));
}
fn release(rt: &mut RtEngine, key: u32) {
    rt.apply(Command::DeckPadRelease(Release { source: 71, key }));
}
fn render(rt: &mut RtEngine, frames: usize) {
    for _ in 0..frames {
        let (l, r) = rt.render_deck(0);
        assert!(l.is_finite() && r.is_finite());
    }
}

#[test]
fn numbered_transient_domains_and_repeat_subdivisions_use_exact_source_bounds_at_mixed_rates() {
    let mut cases = 0;
    let mut maximum = 0.0_f64;
    for source_sr in [44100, 48000, 96000] {
        let (_engine, mut rt) = fixture(source_sr, 48000);
        for domain in 1..=6 {
            for repeat in 0..=3 {
                settings(&mut rt, false, domain, repeat, None);
                rt.decks[0].pos = seconds(2.5) * f64::from(source_sr);
                for pad in 0..8 {
                    press(&mut rt, 100 + u32::from(pad), pad);
                    let span = 2_f64.powi(i32::from(domain));
                    let start = (2.5 / span).floor() * span + f64::from(pad) * span / 8.0;
                    let expected = seconds(start) * f64::from(source_sr);
                    let length = (seconds(start + span / 8.0 / 2_f64.powi(i32::from(repeat)))
                        - seconds(start))
                        * f64::from(source_sr);
                    let error = (rt.decks[0].loop_start - expected)
                        .abs()
                        .max((rt.decks[0].loop_len - length).abs());
                    maximum = maximum.max(error);
                    assert!(
                        error < 1e-6,
                        "{source_sr} domain{domain} repeat{repeat} pad{pad}: {error}"
                    );
                    assert_eq!(rt.decks[0].pos, rt.decks[0].loop_start);
                    let boundary = rt.decks[0].controls.status().slicer.bounds.unwrap();
                    for (i, p) in boundary.iter().enumerate() {
                        assert!(
                            (*p - seconds((2.5 / span).floor() * span + i as f64 * span / 8.0)
                                * f64::from(source_sr))
                            .abs()
                                < 1e-6
                        );
                    }
                    assert!(
                        rt.decks[0].audio.as_ref().unwrap().data[(expected.round() as usize) * 2]
                            > 0.0
                    );
                    render(&mut rt, 128);
                    release(&mut rt, 100 + u32::from(pad));
                    rt.decks[0].pos = seconds(2.5) * f64::from(source_sr);
                    cases += 1;
                }
            }
        }
    }
    println!(
        "SLICER_DOMAIN_RECEIPT {}",
        serde_json::json!({"cases":cases,"max_source_frame_error":maximum,"source_rates":[44100,48000,96000],"domains_beats":[2,4,8,16,32,64],"repeat_divisors":[1,2,4,8],"numbered_stereo_transients":true,"variable_grid":true,"physical_devices_opened":false})
    );
}
#[test]
fn quantized_triggers_follow_the_background_during_audible_slice_loops_and_tempo_changes() {
    let mut cases = 0;
    let mut max_delay = 0;
    for source_sr in [44100, 48000, 96000] {
        for output_sr in [44100, 48000, 96000] {
            for synced in [false, true] {
                let (_engine, mut rt) = fixture(source_sr, output_sr);
                settings(&mut rt, false, 3, 2, Some(3));
                rt.decks[0].pos = seconds(4.0) * f64::from(source_sr);
                press(&mut rt, 11, 0);
                assert_eq!(rt.decks[0].controls.status().slicer.active, Some(0));
                render(&mut rt, 512);
                press(&mut rt, 12, 7);
                assert_eq!(rt.decks[0].controls.status().slicer.pending, Some(7));
                assert_eq!(rt.decks[0].controls.status().slicer.active, Some(0));
                rt.decks[0].sync = synced;
                rt.decks[0].sync_bpm = 137.0;
                rt.decks[0].pitch_range = 2;
                rt.decks[0].pitch = 0.75;
                let mut expected =
                    beats(rt.decks[0].controls.performance_forward.unwrap() / f64::from(source_sr));
                let due = expected.ceil();
                let mut waiting = 0;
                while rt.decks[0].controls.status().slicer.pending.is_some() {
                    let before = expected;
                    rt.render_deck(0);
                    let delta = if synced {
                        137.0 / (60.0 * f64::from(output_sr))
                    } else {
                        1.25 / (0.75 * f64::from(output_sr))
                    };
                    expected += delta;
                    waiting += 1;
                    if rt.decks[0].controls.status().slicer.pending.is_none() {
                        assert!(
                            (before - due).abs() <= delta * 2.0,
                            "{source_sr}/{output_sr} sync{synced}: {before} != {due}"
                        );
                    }
                    assert!(waiting < output_sr as usize);
                }
                assert_eq!(rt.decks[0].controls.status().slicer.active, Some(7));
                assert!(
                    (rt.decks[0].loop_start - seconds(7.0) * f64::from(source_sr)).abs() < 1e-6
                );
                assert!(
                    (rt.decks[0].controls.performance_forward.unwrap() / f64::from(source_sr)
                        - seconds(expected))
                    .abs()
                        < 1e-7
                );
                release(&mut rt, 11);
                assert_eq!(rt.decks[0].controls.status().slicer.active, Some(7));
                release(&mut rt, 12);
                assert!(rt.decks[0].controls.performance_forward.is_none());
                max_delay = max_delay.max(waiting);
                cases += 1;
            }
        }
    }
    println!(
        "SLICER_TRIGGER_RECEIPT {}",
        serde_json::json!({"cases":cases,"max_wait_output_frames":max_delay,"tempo_change_during_hold":true,"audible_loop_independent":true,"source_rates":[44100,48000,96000],"output_rates":[44100,48000,96000],"physical_devices_opened":false})
    );
}
#[test]
fn moving_domains_follow_the_background_and_repeating_domains_restore_the_original_loop() {
    for repeating in [false, true] {
        let (_engine, mut rt) = fixture(48000, 48000);
        let d = &mut rt.decks[0];
        d.loop_on = true;
        d.loop_start = 0.0;
        d.loop_len = 48000.0 * 20.0;
        d.pos = seconds(7.99) * 48000.0;
        let original = (d.loop_on, d.loop_start, d.loop_len);
        settings(&mut rt, repeating, 3, 0, None);
        render(&mut rt, 1);
        press(&mut rt, 23, 2);
        render(&mut rt, 1024);
        let state = rt.decks[0].controls.status().slicer;
        assert_eq!(
            state.bounds.unwrap()[0],
            seconds(if repeating { 0.0 } else { 8.0 }) * 48000.0
        );
        assert_eq!(state.active, Some(2));
        if !repeating {
            assert!((rt.decks[0].loop_start - seconds(10.0) * 48000.0).abs() < 1e-6);
        }
        release(&mut rt, 23);
        if repeating {
            assert_eq!(rt.decks[0].loop_start, seconds(0.0) * 48000.0);
            assert_eq!(
                rt.decks[0].loop_len,
                (seconds(8.0) - seconds(0.0)) * 48000.0
            );
        }
        control(&mut rt, Control::PadMode { mode: 0 });
        assert_eq!(
            (
                rt.decks[0].loop_on,
                rt.decks[0].loop_start,
                rt.decks[0].loop_len
            ),
            original
        );
    }
}
#[test]
fn source_owned_pending_requests_cancel_on_release_seek_settings_source_and_safety_boundaries() {
    for boundary in 0..6 {
        let (engine, mut rt) = fixture(48000, 48000);
        settings(&mut rt, true, 3, 0, Some(5));
        render(&mut rt, 1);
        press(&mut rt, 31, 4);
        assert!(rt.decks[0].controls.status().slicer.pending.is_some());
        match boundary {
            0 => release(&mut rt, 31),
            1 => rt.apply(Command::DeckSeek { deck: 0, frac: 0.4 }),
            2 => control(&mut rt, Control::PadMode { mode: 1 }),
            3 => settings(&mut rt, false, 2, 1, None),
            4 => rt.apply(Command::DeckUnload { deck: 0 }),
            _ => {
                engine
                    .cmd
                    .send(Command::SafetyStop(
                        crate::engine::performance::Safety::Silence,
                    ))
                    .unwrap();
                rt.process(&mut [0.0; 128]);
            }
        }
        assert!(rt.decks[0].controls.status().slicer.pending.is_none());
        let position = rt.decks[0].pos;
        release(&mut rt, 31);
        assert_eq!(rt.decks[0].pos, position);
        assert!(rt.decks[0].controls.performance_forward.is_none());
    }
    let (engine, mut rt) = fixture(48000, 48000);
    for c in [
        Control::SlicerSettings {
            repeating: false,
            domain: 0,
            repeat: 0,
            division: None,
        },
        Control::SlicerSettings {
            repeating: false,
            domain: 7,
            repeat: 0,
            division: None,
        },
        Control::SlicerSettings {
            repeating: false,
            domain: 3,
            repeat: 4,
            division: None,
        },
        Control::SlicerSettings {
            repeating: false,
            domain: 3,
            repeat: 0,
            division: Some(6),
        },
    ] {
        assert!(engine
            .cmd
            .send(Command::DeckControl {
                source: 71,
                deck: 0,
                control: c
            })
            .is_err());
        control(&mut rt, c);
    }
    assert_eq!(rt.decks[0].controls.slice_domain, 3);
}
#[test]
fn actual_private_midi_packets_and_independent_owners_keep_pending_onsets_and_original_releases() {
    let (engine, mut rt) = fixture(48000, 48000);
    settings(&mut rt, false, 3, 0, Some(3));
    let map = crate::engine::midi::builtin_maps()
        .unwrap()
        .into_iter()
        .find(|map| map.name == "Pioneer DDJ-SP1")
        .unwrap();
    let mut left = engine.midi.open_for_test(
        &engine.cmd,
        41,
        map.clone(),
        "Pioneer DDJ-SP1",
        "fixture:slicer41",
    );
    let mut right =
        engine
            .midi
            .open_for_test(&engine.cmd, 42, map, "Pioneer DDJ-SP1", "fixture:slicer42");
    left.push(&[0x97, 0x20, 100]);
    rt.process(&mut []);
    right.push(&[0x97, 0x20, 90]);
    rt.process(&mut []);
    assert!(rt.decks[0].controls.status().slicer.pending.is_some());
    left.push(&[0x89, 0x28, 0]);
    rt.process(&mut []);
    assert!(rt.decks[0].controls.status().slicer.pending.is_some());
    assert!(rt.decks[0].controls.status().slice.is_some());
    right.push(&[0x87, 0x20, 0]);
    rt.process(&mut []);
    assert!(rt.decks[0].controls.status().slicer.pending.is_none());
    assert!(rt.decks[0].controls.status().slice.is_none());
    assert_eq!(Mode::ALL.map(Mode::index), [0, 1, 2, 3, 4, 5, 6, 7]);
    println!(
        "SLICER_INPUT_RECEIPT {}",
        serde_json::json!({"actual_private_midi_worker":true,"sources":2,"sp1_layer_release_identity":true,"physical_devices_opened":false})
    );
}
#[test]
fn actual_callback_slicing_and_mode_changes_are_heap_free_and_keep_the_other_decks_pcm_exact() {
    let (engine, mut rt) = fixture(48000, 48000);
    let (_baseline, mut base) = fixture(48000, 48000);
    for r in [&mut *rt, &mut *base] {
        r.apply(Command::Xfader(1.0));
        r.process(&mut [0.0; 256]);
    }
    settings(&mut rt, true, 3, 2, Some(2));
    let mut actual = Box::new(OutputCallback::new(*rt, 2));
    let mut reference = Box::new(OutputCallback::new(*base, 2));
    let mut frames = 0;
    let mut energy = 0.0;
    for block in 0..256 {
        if block == 0 || block == 32 {
            engine
                .cmd
                .send(Command::DeckPadPress(Press {
                    source: 71,
                    key: block,
                    deck: 0,
                    id: if block == 0 { 3 } else { 8 },
                    mode: Some(Mode::Slice),
                    pressure: 1.0,
                    shifted: false,
                }))
                .unwrap();
        }
        if block == 64 || block == 96 {
            engine
                .cmd
                .send(Command::DeckPadRelease(Release {
                    source: 71,
                    key: if block == 64 { 0 } else { 32 },
                }))
                .unwrap();
        }
        if block == 128 {
            engine
                .cmd
                .send(Command::DeckControl {
                    source: 71,
                    deck: 0,
                    control: Control::PadMode { mode: 0 },
                })
                .unwrap();
        }
        let mut a = [0.0; 256];
        let mut b = [0.0; 256];
        assert_eq!(
            test_alloc::measure(|| actual.render(&mut a)),
            Default::default()
        );
        reference.render(&mut b);
        assert_eq!(a, b);
        frames += 128;
        energy += a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
    }
    assert!(energy > 0.01);
    println!(
        "SLICER_CALLBACK_RECEIPT {}",
        serde_json::json!({"compared_other_deck_frames":frames,"other_deck_max_error":0.0,"energy":energy,"callback_allocations":0,"callback_frees":0,"physical_devices_opened":false})
    );
}
#[test]
fn clipped_first_and_last_domains_never_create_out_of_source_loops_and_slip_keeps_one_root() {
    let (_engine, mut rt) = fixture(44100, 48000);
    settings(&mut rt, false, 6, 0, None);
    rt.decks[0].pos = 0.0;
    press(&mut rt, 51, 0);
    assert!(rt.decks[0].loop_start >= 0.0);
    release(&mut rt, 51);
    rt.decks[0].pos = rt.decks[0].audio.as_ref().unwrap().frames() as f64 - 1.0;
    press(&mut rt, 52, 7);
    assert!(
        rt.decks[0].loop_start + rt.decks[0].loop_len
            <= rt.decks[0].audio.as_ref().unwrap().frames() as f64
    );
    release(&mut rt, 52);
    settings(&mut rt, false, 3, 0, None);
    rt.decks[0].pos = seconds(2.5) * 44100.0;
    control(
        &mut rt,
        Control::SlipSettings {
            enabled: true,
            division: None,
        },
    );
    let start = rt.decks[0].pos;
    press(&mut rt, 53, 3);
    render(&mut rt, 512);
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    render(&mut rt, 512);
    release(&mut rt, 53);
    render(&mut rt, 512);
    assert!(rt.decks[0].controls.slip_forward.is_some());
    rt.apply(Command::DeckTouch { deck: 0, on: false });
    rt.deck_surface_tick(0);
    assert!((rt.decks[0].pos - start - 1536.0 * 44100.0 / 48000.0).abs() < 1e-6);
    assert!(!rt.decks[0].loop_on);
}

#[test]
fn synced_background_wrap_retains_musical_phase_at_a_variable_tempo_repeating_domain() {
    let mut cases = 0;
    let mut maximum = 0.0_f64;
    for source_sr in [44100, 48000, 96000] {
        for output_sr in [44100, 48000, 96000] {
            let (_engine, mut rt) = fixture(source_sr, output_sr);
            rt.decks[0].pos = seconds(7.999) * f64::from(source_sr);
            rt.decks[0].sync = true;
            rt.decks[0].sync_bpm = 137.0;
            settings(&mut rt, true, 3, 0, None);
            render(&mut rt, 1);
            let mut expected = beats(rt.decks[0].pos / f64::from(source_sr));
            press(&mut rt, 61, 3);
            for _ in 0..2048 {
                expected = (expected + 137.0 / (60.0 * f64::from(output_sr))).rem_euclid(8.0);
                rt.render_deck(0);
                let actual =
                    rt.decks[0].controls.performance_forward.unwrap() / f64::from(source_sr);
                let error = (actual - seconds(expected)).abs() * f64::from(source_sr);
                maximum = maximum.max(error);
                assert!(
                    error < 1e-5,
                    "{source_sr}/{output_sr}: {actual} != {} error{error}",
                    seconds(expected)
                );
            }
            release(&mut rt, 61);
            cases += 1;
        }
    }
    println!(
        "SLICER_WRAP_RECEIPT {}",
        serde_json::json!({"cases":cases,"max_source_frame_error":maximum,"synced_variable_grid_wrap":true,"physical_devices_opened":false})
    );
}

#[test]
fn repeating_phrase_mode_settings_and_parameter_changes_cannot_revive_the_old_slip_loop() {
    let mut cases = 0;
    for source_sr in [44100, 48000, 96000] {
        for boundary in 0..3 {
            let (_engine, mut rt) = fixture(source_sr, 48000);
            let original = (
                rt.decks[0].loop_on,
                rt.decks[0].loop_start,
                rt.decks[0].loop_len,
            );
            settings(&mut rt, true, 3, 0, None);
            render(&mut rt, 1);
            control(
                &mut rt,
                Control::SlipSettings {
                    enabled: true,
                    division: None,
                },
            );
            let position = rt.decks[0].pos;
            press(&mut rt, 91, 3);
            render(&mut rt, 512);
            assert!(rt.decks[0].controls.slip_loop.unwrap().0);
            match boundary {
                0 => control(&mut rt, Control::PadMode { mode: 0 }),
                1 => settings(&mut rt, false, 2, 1, None),
                _ => control(
                    &mut rt,
                    Control::Parameter {
                        mode: 2,
                        up: true,
                        shifted: true,
                    },
                ),
            }
            assert_eq!(
                (
                    rt.decks[0].loop_on,
                    rt.decks[0].loop_start,
                    rt.decks[0].loop_len
                ),
                original
            );
            assert!(rt.decks[0].controls.slip_forward.is_none());
            let background = position + 512.0 * f64::from(source_sr) / 48000.0;
            assert!((rt.decks[0].pos - background).abs() < 1e-6);
            render(&mut rt, 128);
            assert!(rt.decks[0].controls.slip_forward.is_none());
            assert!(
                (rt.decks[0].pos - background - 128.0 * f64::from(source_sr) / 48000.0).abs()
                    < 1e-6
            );
            if boundary < 2 {
                assert_eq!(rt.decks[0].loop_on, original.0);
            } else {
                assert_eq!(
                    rt.decks[0].loop_len,
                    (seconds(16.0) - seconds(0.0)) * f64::from(source_sr)
                );
            }
            release(&mut rt, 91);
            assert!(rt.decks[0].controls.slip_forward.is_none());
            cases += 1;
        }
    }
    println!(
        "SLICER_SLIP_BOUNDARY_RECEIPT {}",
        serde_json::json!({"cases":cases,"retired_old_fixed_slip_root":true,"original_loop_restored_before_new_domain":true,"background_preserved":true,"physical_devices_opened":false})
    );
}

#[test]
fn preparing_slicer_settings_in_another_mode_preserves_its_held_roll_and_slip_timeline() {
    let mut cases = 0;
    for source_sr in [44100, 48000, 96000] {
        for parameter in [false, true] {
            let (_engine, mut rt) = fixture(source_sr, 48000);
            control(&mut rt, Control::PadMode { mode: 1 });
            control(
                &mut rt,
                Control::SlipSettings {
                    enabled: true,
                    division: None,
                },
            );
            let start = rt.decks[0].pos;
            rt.apply(Command::DeckPadPress(Press {
                source: 71,
                key: 111,
                deck: 0,
                id: 4,
                mode: Some(Mode::Roll),
                pressure: 1.0,
                shifted: false,
            }));
            render(&mut rt, 512);
            let before = (
                rt.decks[0].loop_on,
                rt.decks[0].loop_start,
                rt.decks[0].loop_len,
                rt.decks[0].controls.slip_forward,
                rt.decks[0].controls.performance_forward,
            );
            if parameter {
                control(
                    &mut rt,
                    Control::Parameter {
                        mode: 2,
                        up: true,
                        shifted: true,
                    },
                );
            } else {
                control(
                    &mut rt,
                    Control::SlicerSettings {
                        repeating: true,
                        domain: 4,
                        repeat: 1,
                        division: Some(2),
                    },
                );
            }
            assert_eq!(rt.decks[0].controls.status().pad_mode, 1);
            assert_eq!(rt.decks[0].controls.status().roll, Some(3));
            assert_eq!(
                (
                    rt.decks[0].loop_on,
                    rt.decks[0].loop_start,
                    rt.decks[0].loop_len,
                    rt.decks[0].controls.slip_forward,
                    rt.decks[0].controls.performance_forward
                ),
                before
            );
            render(&mut rt, 512);
            release(&mut rt, 111);
            rt.deck_surface_tick(0);
            assert!(
                (rt.decks[0].pos - start - 1024.0 * f64::from(source_sr) / 48000.0).abs() < 1e-6
            );
            assert!(!rt.decks[0].loop_on);
            assert!(rt.decks[0].controls.slip_forward.is_none());
            cases += 1;
        }
    }
    println!(
        "SLICER_PREPARATION_RECEIPT {}",
        serde_json::json!({"cases":cases,"unrelated_held_roll_preserved":true,"unrelated_slip_root_preserved":true,"physical_devices_opened":false})
    );
}
