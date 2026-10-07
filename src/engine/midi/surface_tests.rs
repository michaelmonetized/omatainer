use super::*;
use crate::engine::{surface_controls, Engine, RtEngine, Sample};

fn fixture(map: MidiMap, source: u64) -> (Engine, RtEngine, TestInput) {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let hub = MidiHub::without_devices();
    let input = hub.open_for_test(
        &engine.cmd,
        source,
        map,
        "qualified surface fixture",
        "fixture:surface",
    );
    (engine, rt, input)
}
#[test]
fn clip_launch_apc_release_keeps_its_original_target_after_bank_and_selection_changes(){
    let(_engine,mut rt,mut input)=fixture(akai_apc40_mk2(),83);
    rt.quant=0.0;
    rt.tracks[0].clips[0].properties.launch=crate::engine::clip_launch::Policy{mode:crate::engine::clip_launch::Mode::Gate,grid:crate::engine::clip_launch::Grid::Immediate,legato:false};
    send(&mut input,&mut rt,&[0x90,0x20,127]);assert_eq!(rt.tracks[0].playing.unwrap().scene,0);
    rt.surface.status.scene_offset=1;rt.apply(Command::Select{track:1,scene:1});rt.apply(Command::LaunchClip{track:1,scene:0});
    send(&mut input,&mut rt,&[0x80,0x20,64]);rt.process(&mut[0.0;2]);
    assert!(rt.tracks[0].playing.is_none());assert_eq!(rt.tracks[1].playing.unwrap().scene,0);
}
#[test]
fn clip_launch_controller_disconnect_reserves_release_at_full_queue_and_preserves_other_owners(){
    let(engine,mut rt,mut input)=fixture(akai_apc40_mk2(),83);
    let hub=MidiHub::without_devices();let mut other=hub.open_for_test(&engine.cmd,84,akai_apc40_mk2(),"other held clip","fixture:clip");
    rt.quant=0.0;
    for scene in 0..2{rt.tracks[0].clips[scene].properties.launch=crate::engine::clip_launch::Policy{mode:crate::engine::clip_launch::Mode::Gate,grid:crate::engine::clip_launch::Grid::Immediate,legato:false};}
    send(&mut input,&mut rt,&[0x90,0x20,127]);send(&mut other,&mut rt,&[0x90,0x18,127]);
    drop(input);rt.process(&mut[0.0;2]);assert_eq!(rt.tracks[0].playing.unwrap().scene,1);
    while engine.cmd.send(Command::NudgeBpm(0.0)).is_ok(){}
    drop(other);for _ in 0..16{rt.process(&mut[0.0;2]);}
    assert!(rt.tracks[0].playing.is_none());
}
fn send(input: &mut TestInput, rt: &mut RtEngine, bytes: &[u8]) {
    input.push(bytes);
    rt.process(&mut []);
}
fn tap(input: &mut TestInput, rt: &mut RtEngine, bytes: [u8;3]) {
    send(input,rt,&bytes);
    send(input,rt,&[bytes[0]&15|0x80,bytes[1],0]);
}
fn deck(rt: &mut RtEngine, index: u8) {
    rt.apply(Command::DeckAudio {
        deck: index,
        audio: Arc::new(Sample { spectrum: None,
            name: "Surface stereo qualification".into(),
            sr: 48000,
            ch: 2,
            data: (0..48000 * 12)
                .flat_map(|frame| {
                    let value = (frame as f32 * 0.057).sin() * 0.25;
                    [value, -value]
                })
                .collect(),
            peaks: Vec::new().into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.apply(Command::DeckSeek {
        deck: index,
        frac: 0.25,
    });
}

#[test]
fn surfaces_apc_track_buttons_and_faders_target_each_track_without_release_toggles() {
    let (_engine, mut rt, mut input) = fixture(akai_apc40_mk2(), 81);
    for track in 0..8 {
        send(&mut input, &mut rt, &[0xb0 + track, 7, 100]);
        assert!((rt.tracks[usize::from(track)].gain - 100.0 / 127.0).abs() < 0.00001);
        for note in [0x30, 0x31, 0x32] {
            send(&mut input, &mut rt, &[0x90 + track, note, 127]);
            send(&mut input, &mut rt, &[0x80 + track, note, 127]);
        }
        assert!(
            rt.tracks[usize::from(track)].armed
                && rt.tracks[usize::from(track)].solo
                && rt.tracks[usize::from(track)].mute
        );
        send(&mut input, &mut rt, &[0x90 + track, 0x33, 127]);
        assert_eq!(rt.selected_track, usize::from(track));
    }
    send(&mut input, &mut rt, &[0x90, 0x5b, 127]);
    assert!(rt.playing);
    send(&mut input, &mut rt, &[0x90, 0x5c, 127]);
    assert!(!rt.playing);
    send(&mut input, &mut rt, &[0x90, 0x5a, 127]);
    assert!(rt.surface_status().shift == false);
    send(&mut input, &mut rt, &[0xb0, 0x0f, 0]);
    assert_eq!(rt.xfader, 0.0);
    send(&mut input, &mut rt, &[0xb0, 0x0e, 32]);
    assert_eq!(rt.master, 32.0 / 127.0);
    send(&mut input, &mut rt, &[0xb0, 0x0d, 127]);
    assert_eq!(rt.bpm, 123.0);
}

#[test]
fn surfaces_apc_knob_modes_assignments_and_master_device_controls_change_real_values() {
    let (_engine, mut rt, mut input) = fixture(akai_apc40_mk2(), 82);
    send(&mut input, &mut rt, &[0xb0, 0x30, 127]);
    assert_eq!(rt.tracks[0].pan, 1.0);
    send(&mut input, &mut rt, &[0x90, 0x58, 127]);
    send(&mut input, &mut rt, &[0xb0, 0x31, 100]);
    assert_eq!(rt.surface_status().sends[1][0], 100.0 / 127.0);
    send(&mut input, &mut rt, &[0x90, 0x58, 127]);
    send(&mut input, &mut rt, &[0xb0, 0x31, 64]);
    assert_eq!(rt.surface_status().sends[1][1], 64.0 / 127.0);
    send(&mut input, &mut rt, &[0x90, 0x42, 127]);
    assert_eq!(rt.surface_status().assignments[0], 1);
    rt.surface.prepare(48000.0, 1.0, 1.0);
    for _ in 0..240 {
        rt.surface.track(0, [0.0; 2]);
    }
    let dry = rt.surface.track(0, [0.2, -0.3]);
    assert_eq!(dry, [0.0, 0.0]);
    send(&mut input, &mut rt, &[0x90, 0x42, 127]);
    assert_eq!(rt.surface_status().assignments[0], 2);
    rt.surface.prepare(48000.0, 1.0, 1.0);
    for _ in 0..240 {
        rt.surface.track(0, [0.0; 2]);
    }
    let dry = rt.surface.track(0, [0.2, -0.3]);
    assert_eq!(dry, [0.2, -0.3]);
    send(&mut input, &mut rt, &[0x90, 0x50, 127]);
    send(&mut input, &mut rt, &[0xb0, 0x10, 80]);
    assert_eq!(rt.fx_wet[0], 80.0 / 127.0);
    send(&mut input, &mut rt, &[0x90, 0x3e, 127]);
    assert_eq!(rt.fx_wet[0], 0.0);
    send(&mut input, &mut rt, &[0x90, 0x3e, 127]);
    assert_eq!(rt.fx_wet[0], 80.0 / 127.0);
    assert_eq!(rt.fx_wet[1], 0.0);
}

#[test]
fn surfaces_apc_window_shift_selection_bank_lock_and_device_lock_follow_session_state() {
    let (_engine, mut rt, mut input) = fixture(akai_apc40_mk2(), 83);
    for index in 8..16 {
        let mut track = rt.tracks[0].as_ref().clone();
        track.name = format!("Surface {index}");
        rt.tracks.push(Box::new(track));
    }
    rt.session = crate::engine::session::Layout::fresh(
        rt.tracks.iter().map(|track| track.name.clone()),
        rt.scene_fx.len(),
    );
    send(&mut input, &mut rt, &[0x90, 0x62, 127]);
    send(&mut input, &mut rt, &[0x90, 0x60, 127]);
    assert_eq!(rt.surface_status().track_offset, 8);
    send(&mut input, &mut rt, &[0x90, 0x5f, 127]);
    assert_eq!(rt.surface_status().scene_offset, 3);
    send(&mut input, &mut rt, &[0x90, 32, 127]);
    assert_eq!((rt.selected_track, rt.selected_scene), (8, 3));
    send(&mut input, &mut rt, &[0x80, 0x62, 0]);
    send(&mut input, &mut rt, &[0xb0, 7, 21]);
    assert_eq!(rt.tracks[8].gain, 21.0 / 127.0);
    assert_eq!(rt.tracks[0].gain, 0.8);
    send(&mut input, &mut rt, &[0x90, 0x67, 127]);
    send(&mut input, &mut rt, &[0x90, 0x61, 127]);
    assert_eq!(rt.surface_status().track_offset, 8);
    send(&mut input, &mut rt, &[0x90, 0x3f, 127]);
    assert_eq!(rt.surface_status().device_lock, Some(8));
    send(&mut input, &mut rt, &[0x91, 0x33, 127]);
    assert_eq!(rt.selected_track, 9);
    assert_eq!(rt.surface_status().device_lock, Some(8));
}

#[test]
fn surfaces_apc_grid_rows_launch_and_select_the_matching_visible_scenes() {
    let (_engine, mut rt, mut input) = fixture(akai_apc40_mk2(), 86);
    for index in 8..16 {
        let mut track = rt.tracks[0].as_ref().clone();
        track.name = format!("Grid {index}");
        rt.tracks.push(Box::new(track));
    }
    rt.session = crate::engine::session::Layout::fresh(
        rt.tracks.iter().map(|track| track.name.clone()),
        rt.scene_fx.len(),
    );
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            clip.kind = crate::engine::ClipKind::Midi;
        }
    }
    let rows = [
        [32, 33, 34, 35, 36, 37, 38, 39],
        [24, 25, 26, 27, 28, 29, 30, 31],
        [16, 17, 18, 19, 20, 21, 22, 23],
        [8, 9, 10, 11, 12, 13, 14, 15],
        [0, 1, 2, 3, 4, 5, 6, 7],
    ];
    let map = akai_apc40_mk2();
    for (row, notes) in rows.iter().enumerate() {
        for (column, note) in notes.iter().enumerate() {
            let binding = map.bindings.iter().find(|binding| {
                binding.kind == MsgKind::Note && binding.ch == 0xff && binding.data == *note
            }).unwrap();
            assert_eq!((binding.action, binding.deck, binding.extra), (Action::Clip, column as u8, row as u16));
        }
    }
    for track_offset in [0, 8] {
        for scene_offset in [0, 3] {
            rt.surface.status.track_offset = track_offset;
            rt.surface.status.scene_offset = scene_offset;
            for shift in [false, true] {
                send(&mut input, &mut rt, &[0x90, 0x62, if shift { 127 } else { 0 }]);
                for (row, notes) in rows.iter().enumerate() {
                    for (column, note) in notes.iter().enumerate() {
                        let track = track_offset + column;
                        let scene = scene_offset + row;
                        rt.apply(Command::Stop);
                        send(&mut input, &mut rt, &[0x90, *note, 127]);
                        send(&mut input, &mut rt, &[0x80, *note, 0]);
                        assert_eq!((rt.selected_track, rt.selected_scene), (track, scene));
                        if shift {
                            assert!(rt.tracks.iter().all(|track| track.playing.is_none()));
                            assert!(!rt.playing);
                        } else {
                            assert_eq!(rt.tracks[track].playing.as_ref().unwrap().scene as usize, scene);
                            assert!(rt.playing);
                        }
                    }
                }
            }
        }
    }
    rt.surface.status.track_offset = 15;
    rt.surface.status.scene_offset = 7;
    send(&mut input, &mut rt, &[0x90, 0x62, 127]);
    send(&mut input, &mut rt, &[0x90, 32, 127]);
    assert_eq!((rt.selected_track, rt.selected_scene), (15, 7));
    for note in [33, 24, 40] {
        send(&mut input, &mut rt, &[0x90, note, 127]);
        assert_eq!((rt.selected_track, rt.selected_scene), (15, 7));
    }
}

#[test]
fn surfaces_sp1_roll_and_slicer_restore_forward_time_and_release_on_disconnect() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 84);
    deck(&mut rt, 0);
    deck(&mut rt, 1);
    rt.apply(Command::DeckPlay { deck: 0 });
    for _ in 0..1000 {
        rt.render_deck(0);
    }
    for address in [0x10, 0x23] {
        let start = rt.decks[0].pos;
        let other = rt.decks[1].pos;
        send(&mut input, &mut rt, &[0x97, address, 127]);
        for _ in 0..4800 {
            rt.render_deck(0);
        }
        assert!(rt.decks[0].loop_on);
        send(&mut input, &mut rt, &[0x87, address, 127]);
        assert!(!rt.decks[0].loop_on);
        assert!((rt.decks[0].pos - start - 4800.0).abs() < 0.01);
        assert_eq!(rt.decks[1].pos, other);
    }
    send(&mut input, &mut rt, &[0x97, 0x12, 127]);
    drop(input);
    rt.process(&mut []);
    assert!(!rt.decks[0].loop_on && rt.decks[0].controls.status().roll.is_none());
}

#[test]
fn surfaces_sp1_effect_precision_bank_isolation_buttons_and_assignments_are_independent() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 85);
    for (channel, msb, lsb) in [(0xb4, 127, 127), (0xb5, 32, 17)] {
        send(&mut input, &mut rt, &[channel, 2, msb]);
        send(&mut input, &mut rt, &[channel, 0x22, lsb]);
    }
    assert_eq!(rt.surface_status().fx[0].wet[0], 1.0);
    assert_eq!(rt.surface_status().fx[1].wet[0], 4113.0 / 16383.0);
    send(&mut input, &mut rt, &[0x94, 0x47, 127]);
    assert!(rt.surface_status().fx[0].on[0]);
    assert!(!rt.surface_status().fx[1].on[0]);
    let kind = rt.surface_status().fx[0].kinds[0];
    send(&mut input, &mut rt, &[0x94, 0x63, 127]);
    assert_ne!(rt.surface_status().fx[0].kinds[0], kind);
    send(&mut input, &mut rt, &[0xb5, 0x12, 127]);
    send(&mut input, &mut rt, &[0xb5, 0x32, 127]);
    assert_eq!(rt.surface_status().fx[1].parameter[0], 1.0);
    send(&mut input, &mut rt, &[0x96, 0x4d, 127]);
    assert!(rt.surface_status().fx[0].assigned[1]);
}

#[test]
fn surfaces_sp1_hotloops_all_pad_modes_and_sampler_volume_have_real_engine_targets() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 86);
    deck(&mut rt, 0);
    for (note, mode) in [
        (0x1b, 0),
        (0x1e, 1),
        (0x20, 2),
        (0x22, 3),
        (0x69, 4),
        (0x6b, 5),
        (0x6d, 6),
        (0x6f, 7),
    ] {
        send(&mut input, &mut rt, &[0x90, note, 127]);
        assert_eq!(rt.decks[0].controls.status().pad_mode, mode);
    }
    tap(&mut input, &mut rt, [0x97, 0x40, 127]);
    assert!(rt.decks[0].controls.status().hotloops[0]);
    tap(&mut input, &mut rt, [0x97, 0x48, 127]);
    assert!(!rt.decks[0].controls.status().hotloops[0]);
    send(&mut input, &mut rt, &[0xb6, 3, 64]);
    send(&mut input, &mut rt, &[0xb6, 0x23, 0]);
    assert_eq!(rt.surface_status().sampler_volume, 8192.0 / 16383.0);
    send(&mut input, &mut rt, &[0x90, 0x40, 127]);
    assert!(rt.decks[0].controls.status().slip);
}

#[test]
fn surfaces_mpd_captured_preset_maps_all_banks_and_preserves_musical_pads() {
    let bytes = include_bytes!("../../../tests/fixtures/mpd232-livelite.syx");
    let map = surface::mpd232::parse(bytes).unwrap();
    assert_eq!(map.name, "Akai MPD232 (LiveLite)");
    assert_eq!(map.bindings.len(), 75);
    let (_engine, mut rt, mut input) = fixture(map, 87);
    for bank in 0..3u8 {
        for index in 0..8u8 {
            send(&mut input, &mut rt, &[0xb0 + bank, 12 + index, 30 + index]);
            assert_eq!(
                rt.tracks[usize::from(index)].gain,
                f32::from(30 + index) / 127.0
            );
            send(&mut input, &mut rt, &[0xb0 + bank, 22 + index, 110]);
            match bank {
                0 => assert!(
                    (rt.tracks[usize::from(index)].pan - (110.0 / 127.0 * 2.0 - 1.0)).abs()
                        < 0.00001
                ),
                _ => assert_eq!(
                    rt.surface_status().sends[usize::from(index)][usize::from(bank - 1)],
                    110.0 / 127.0
                ),
            }
            send(&mut input, &mut rt, &[0xb0 + bank, 32 + index, 127]);
            send(&mut input, &mut rt, &[0xb0 + bank, 32 + index, 0]);
        }
    }
    assert!(rt
        .tracks
        .iter()
        .all(|track| track.mute && track.solo && track.armed));
    let map = surface::mpd232::parse(bytes).unwrap();
    for pad in bytes[30..734].chunks_exact(11) {
        for velocity in [23, 117] {
            let commands =
                super::profile_tests::observe(&map, &[0x90 + pad[1] - 1, pad[2], velocity]);
            assert!(
                matches!(commands.as_slice(), [Command::LiveNoteOn { ch, note, vel, .. }] if *ch == pad[1]-1 && *note == pad[2] && *vel == velocity)
            );
        }
    }
}

#[test]
fn surfaces_mpd_mmc_is_idempotent_and_remains_specific_to_the_mpd() {
    let (_engine, mut rt, mut input) = fixture(akai_mpd232(), 88);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 2, 0xf7]);
    assert!(rt.playing);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 6, 0xf7]);
    assert!(rt.recording);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 6, 0xf7]);
    assert!(rt.recording);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 7, 0xf7]);
    assert!(!rt.recording);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 1, 0xf7]);
    assert!(!rt.playing);
}

#[test]
fn surfaces_mpd_cc_transport_is_idempotent_on_every_common_channel() {
    let map = surface::mpd232::parse(include_bytes!("../../../tests/fixtures/mpd232-livelite.syx")).unwrap();
    let (_engine, mut rt, mut input) = fixture(map, 101);
    for channel in 0..16 {
        for _ in 0..2 {
            send(&mut input, &mut rt, &[0xb0 + channel, 118, 127]);
            send(&mut input, &mut rt, &[0xb0 + channel, 118, 0]);
            assert!(rt.playing);
            send(&mut input, &mut rt, &[0xb0 + channel, 119, 127]);
            send(&mut input, &mut rt, &[0xb0 + channel, 119, 0]);
            assert!(rt.recording);
        }
        send(&mut input, &mut rt, &[0xb0 + channel, 117, 127]);
        assert!(!rt.playing);
        send(&mut input, &mut rt, &[0xb0 + channel, 118, 127]);
        send(&mut input, &mut rt, &[0xb0 + channel, 117, 0]);
        assert!(rt.playing, "Stop release cannot stop playback");
    }
}

#[test]
fn surfaces_controls_render_without_callback_allocations_and_send_stereo_effect_tails() {
    let (_engine, mut rt, mut input) = fixture(akai_apc40_mk2(), 89);
    send(&mut input, &mut rt, &[0x90, 0x50, 127]);
    input.push(&[0xb0, 0x11, 98]);
    let measurement = crate::engine::test_alloc::measure(|| rt.process(&mut [0.0; 512]));
    assert_eq!((measurement.allocations, measurement.frees), (0, 0));
    rt.apply(Command::Surface(surface_controls::Input::TrackSend {
        track: 0,
        send: 0,
        value: 1.0,
    }));
    rt.surface.prepare(48000.0, 0.5, 0.0);
    rt.surface.track(0, [1.0, -0.5]);
    let mut energy = [0.0; 2];
    for _ in 0..8000 {
        let output = rt.surface.render_sends(24000.0);
        for channel in 0..2 {
            energy[channel] += output[channel].abs();
        }
    }
    assert!(energy[0] > 0.1 && energy[1] > 0.05 && energy[0] > energy[1] * 1.9);
}

#[test]
fn surfaces_safety_stop_restores_the_original_loop_without_reviving_held_pads() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 90);
    deck(&mut rt, 0);
    rt.apply(Command::DeckLoop {
        deck: 0,
        beats: 8.0,
    });
    let length = rt.decks[0].loop_len;
    send(&mut input, &mut rt, &[0x97, 0x10, 127]);
    assert_ne!(rt.decks[0].loop_len, length);
    rt.apply(Command::SafetyStop(
        crate::engine::performance::Safety::Stop,
    ));
    assert!(!rt.decks[0].playing && rt.decks[0].controls.status().roll.is_none());
    assert_eq!(rt.decks[0].loop_len, length);
    assert!(rt.decks[0].loop_on);
}

#[test]
fn surfaces_source_owned_sampler_and_shift_release_when_the_queue_is_full() {
    let (engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 91);
    let hub = MidiHub::without_devices();
    let mut second = hub.open_for_test(
        &engine.cmd,
        92,
        surface::pioneer_sp1(),
        "second SP1",
        "fixture:second",
    );
    rt.sampler_inst = crate::engine::SamplerInstrument::Synth(crate::engine::SynthInstrument::Keys);
    send(&mut input, &mut rt, &[0x97, 0x70, 64]);
    send(&mut second, &mut rt, &[0x97, 0x70, 100]);
    send(&mut input, &mut rt, &[0x87, 0x70, 0]);
    assert!(rt.surface_status().sampler_playing[0]);
    while engine
        .cmd
        .send(Command::Tap(std::time::Instant::now()))
        .is_ok()
    {}
    drop(second);
    for _ in 0..8 {
        rt.process(&mut []);
    }
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3)));
    let mut apc = hub.open_for_test(&engine.cmd, 93, akai_apc40_mk2(), "APC", "fixture:apc");
    send(&mut apc, &mut rt, &[0x90, 0x62, 127]);
    assert!(rt.surface_status().shift);
    while engine
        .cmd
        .send(Command::Tap(std::time::Instant::now()))
        .is_ok()
    {}
    drop(apc);
    for _ in 0..8 {
        rt.process(&mut []);
    }
    assert!(!rt.surface_status().shift);
}

#[test]
fn surfaces_mpd_rejects_corrupt_conflicting_and_non_usb_a_presets() {
    let original = include_bytes!("../../../tests/fixtures/mpd232-livelite.syx");
    for (offset, value) in [
        (0, 0),
        (5, 0),
        (734, 3),
        (735, 0),
        (735, 17),
        (736, 117),
        (737, 1),
        (738, 64),
        (1095, 2),
        (1096, 60),
    ] {
        let mut bytes = original.to_vec();
        bytes[offset] = value;
        if offset == 1095 {
            bytes[1094] = 1;
            bytes[1096] = 60;
        }
        if offset == 1096 {
            bytes[1094] = 1;
            bytes[1095] = 2;
        }
        assert!(
            surface::mpd232::parse(&bytes).is_err(),
            "offset {offset} value {value}"
        );
    }
    let mut bytes = original.to_vec();
    bytes[736] = 12;
    assert!(surface::mpd232::parse(&bytes).is_err());
    assert!(surface::mpd232::parse(&original[..3482]).is_err());
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 94);
    send(&mut input, &mut rt, &[0xf0, 0x7f, 0x7f, 6, 2, 0xf7]);
    assert!(!rt.playing, "MPD MMC must not control another surface");
}

#[test]
fn surfaces_sp1_effect_rendering_keeps_banks_decks_and_stereo_histories_separate() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 95);
    send(&mut input, &mut rt, &[0x94, 0x49, 127]);
    send(&mut input, &mut rt, &[0xb4, 6, 127]);
    send(&mut input, &mut rt, &[0xb4, 0x26, 127]);
    let first = rt.surface.deck(0, [1.0, 0.0], 24000.0);
    assert!(first[0] > 0.0 && first[0] < 1.0 && first[1] == 0.0);
    assert_eq!(rt.surface.deck(1, [0.7, -0.2], 24000.0), [0.7, -0.2]);
    send(&mut input, &mut rt, &[0x96, 0x4d, 127]);
    assert_eq!(rt.surface.deck(1, [0.0; 2], 24000.0), [0.0; 2]);
}

#[test]
fn surfaces_sp1_auto_loop_encoder_selects_a_length_before_activation() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 96);
    deck(&mut rt, 0);
    send(&mut input, &mut rt, &[0xb0, 0x17, 1]);
    assert!(!rt.decks[0].loop_on && rt.decks[0].loop_len > 1.0);
    let length = rt.decks[0].loop_len;
    send(&mut input, &mut rt, &[0x90, 0x55, 127]);
    assert!(rt.decks[0].loop_on && rt.decks[0].loop_len == length);
}

#[test]
fn surfaces_mpd_physical_bank_a_capture_replays_through_the_production_worker() {
    let map = surface::mpd232::parse(include_bytes!(
        "../../../tests/fixtures/mpd232-livelite.syx"
    ))
    .unwrap();
    let (_engine, mut rt, mut input) = fixture(map, 99);
    let bytes = include_bytes!("../../../tests/fixtures/mpd232-bank-a.midi");
    let mut last = [None; 128];
    for message in bytes.chunks_exact(3) {
        last[usize::from(message[1])] = Some(message[2]);
        send(&mut input, &mut rt, message);
    }
    for index in 0..8 {
        assert_eq!(
            rt.tracks[index].gain,
            f32::from(last[12 + index].unwrap()) / 127.0
        );
        assert_eq!(
            rt.tracks[index].pan,
            f32::from(last[22 + index].unwrap()) / 127.0 * 2.0 - 1.0
        );
        assert!(rt.tracks[index].mute && !rt.tracks[index].solo && !rt.tracks[index].armed);
    }
}

#[test]
fn surfaces_sp1_auto_loop_pads_replace_lengths_and_parameter_buttons_change_the_range() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 100);
    deck(&mut rt, 0);
    tap(&mut input, &mut rt, [0x97, 0x55, 127]);
    let first = rt.decks[0].loop_len;
    tap(&mut input, &mut rt, [0x97, 0x56, 127]);
    assert!(rt.decks[0].loop_on && (rt.decks[0].loop_len - first * 2.0).abs() < 0.01);
    send(&mut input, &mut rt, &[0x90, 0x31, 127]);
    tap(&mut input, &mut rt, [0x97, 0x56, 127]);
    assert!(rt.decks[0].loop_on && (rt.decks[0].loop_len - first * 4.0).abs() < 0.01);
    tap(&mut input, &mut rt, [0x97, 0x56, 127]);
    assert!(!rt.decks[0].loop_on);
}

#[test]
fn surfaces_sp1_sync_off_is_idempotent_and_manual_pads_save_and_select_real_loops() {
    let (_engine, mut rt, mut input) = fixture(surface::pioneer_sp1(), 101);
    deck(&mut rt, 0);
    deck(&mut rt, 1);
    rt.apply(Command::DeckSync { deck: 0 });
    assert!(rt.decks[0].sync);
    send(&mut input, &mut rt, &[0x90, 0x5c, 127]);
    assert!(!rt.decks[0].sync);
    send(&mut input, &mut rt, &[0x90, 0x5c, 127]);
    assert!(!rt.decks[0].sync);
    rt.apply(Command::DeckLoop {
        deck: 0,
        beats: 4.0,
    });
    let length = rt.decks[0].loop_len;
    send(&mut input, &mut rt, &[0x97, 0x62, 127]);
    assert!(rt.decks[0].controls.status().hotloops[0]);
    send(&mut input, &mut rt, &[0x97, 0x63, 127]);
    assert_eq!(rt.decks[0].controls.status().loop_slot, 7);
    assert!(!rt.decks[0].loop_on);
    send(&mut input, &mut rt, &[0x97, 0x67, 127]);
    assert_eq!(rt.decks[0].controls.status().loop_slot, 0);
    assert_eq!(rt.decks[0].loop_len, length);
    send(&mut input, &mut rt, &[0x97, 0x61, 127]);
    assert!(rt.decks[0].loop_on);
    assert!(!rt.decks[1].loop_on);
}
