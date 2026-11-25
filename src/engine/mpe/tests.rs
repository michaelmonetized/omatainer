use super::*;
use crate::engine::{dsp::Poly, instrument::SynthInstrument, Command, Engine, InputKey, RtEngine};

fn configure(state: &mut State, manager: u8, members: u8) -> Update {
    state.input(&[0xb0 | manager, 101, 0]);
    state.input(&[0xb0 | manager, 100, 6]);
    state.input(&[0xb0 | manager, 6, members])
}
fn range(state: &mut State, channel: u8, semitones: u8) {
    for bytes in [
        [0xb0 | channel, 101, 0],
        [0xb0 | channel, 100, 0],
        [0xb0 | channel, 6, semitones],
    ] {
        state.input(&bytes);
    }
}
fn expression(rt: &mut RtEngine, source: u64, packet: &[u8]) {
    let mut bytes = [0; 3];
    bytes[..packet.len()].copy_from_slice(packet);
    rt.apply(Command::MidiExpression {
        source,
        bytes,
        length: packet.len() as u8,
    });
}
fn zone(rt: &mut RtEngine, source: u64, manager: u8, members: u8) {
    for bytes in [
        [0xb0 | manager, 101, 0],
        [0xb0 | manager, 100, 6],
        [0xb0 | manager, 6, members],
    ] {
        expression(rt, source, &bytes);
    }
}
fn input(source: u64, ch: u8, note: u8) -> InputKey {
    InputKey::Midi { source, ch, note }
}

#[test]
fn expressive_drum_samples_keep_pitch_pressure_timbre_and_releases_source_local() {
    let (engine,mut rt)=Engine::headless_for_test(48000,256);
    rt.selected_track=0;
    for source in [11,22] {zone(&mut rt,source,0,4);}
    expression(&mut rt,11,&[0xe1,0,96]);expression(&mut rt,22,&[0xe1,0,32]);
    for source in [11,22] {rt.apply(Command::LiveNoteOn {source,ch:1,note:36,vel:100});}
    let mut energy=0.0f32;
    assert_eq!(crate::engine::test_alloc::measure(|| {
        for _ in 0..128 {energy+=rt.tick_drums(0).abs();}
        assert_eq!(rt.tracks[0].drum_pos[0].unwrap().position,512.0);
        assert_eq!(rt.tracks[0].drum_pos[1].unwrap().position,32.0);
        expression(&mut rt,11,&[0xd1,91]);expression(&mut rt,11,&[0xb1,74,12]);
        assert_eq!(rt.tracks[0].drum_pos[0].unwrap().expression.unwrap().values,Values {pitch:24.0,pressure:91,timbre:12});
        assert_eq!(rt.tracks[0].drum_pos[1].unwrap().expression.unwrap().values,Values {pitch:-24.0,pressure:0,timbre:64});
        rt.apply(Command::LiveNoteOff {source:11,ch:1,note:36});
        expression(&mut rt,11,&[0xd1,17]);expression(&mut rt,11,&[0xb1,74,120]);
        assert_eq!(rt.tracks[0].drum_pos[0].unwrap().expression.unwrap().values.pressure,91);
        expression(&mut rt,11,&[0xd0,99]);
        assert_eq!(rt.tracks[0].drum_pos[0].unwrap().expression.unwrap().values.pressure,99);
        assert_eq!(rt.tracks[0].drum_pos[1].unwrap().expression.unwrap().values.pressure,0);
    }),crate::engine::test_alloc::Counts::default());
    assert!(energy>0.1);
    engine.cmd.release_midi_source(11);rt.process(&mut []);
    assert!(rt.tracks[0].drum_pos[0].is_none());assert!(rt.tracks[0].drum_pos[1].is_some());
    expression(&mut rt,22,&[0xb0,120,0]);assert!(rt.tracks[0].drum_pos[1].is_none());
}

#[test]
fn zones_latest_configuration_wins_without_expanding_disabled_neighbors() {
    let mut state = State::default();
    assert_eq!(configure(&mut state, 0, 8).reset, 0x01ff);
    assert_eq!(state.role(8), Some((0, true)));
    configure(&mut state, 15, 9);
    assert_eq!((state.lower, state.upper), (5, 9));
    assert_eq!(state.role(5), Some((0, true)));
    assert_eq!(state.role(6), Some((15, true)));
    configure(&mut state, 15, 0);
    assert_eq!((state.lower, state.upper), (5, 0));
    configure(&mut state, 0, 15);
    assert_eq!(state.role(15), Some((0, true)));
    configure(&mut state, 15, 15);
    assert_eq!((state.lower, state.upper), (0, 15));
    assert_eq!(state.role(0), Some((15, true)));
    configure(&mut state, 3, 8);
    assert_eq!((state.lower, state.upper), (0, 15));
    configure(&mut state, 15, 0);
    assert!((0..16).all(|channel| state.role(channel).is_none()));
}

#[test]
fn ranges_and_rpn_selection_respect_zone_and_nrpn_boundaries() {
    let mut state = State::default();
    configure(&mut state, 0, 5);
    configure(&mut state, 15, 5);
    assert_eq!(
        (
            state.channels[0].range,
            state.channels[2].range,
            state.channels[15].range
        ),
        (2, 48, 2)
    );
    range(&mut state, 2, 24);
    assert!((1..=5).all(|channel| state.channels[channel].range == 24));
    assert_eq!(state.channels[10].range, 48);
    range(&mut state, 0, 12);
    assert_eq!(state.channels[0].range, 12);
    assert_eq!(state.channels[1].range, 24);
    state.input(&[0xb0, 99, 0]);
    state.input(&[0xb0, 6, 3]);
    assert_eq!(state.channels[0].range, 12);
    state.input(&[0xb1, 101, 127]);
    state.input(&[0xb1, 100, 127]);
    state.input(&[0xb1, 6, 7]);
    assert_eq!(state.channels[1].range, 24);
    range(&mut state, 1, 97);
    assert_eq!(state.channels[1].range, 24);
    state.input(&[0xe1, 0x2b, 0x41]);
    let note = state.note(1, 1).unwrap();
    assert!((note.member.pitch - 0.500_976_56).abs() < 1e-6);
    assert!(!valid(&[0xd1, 1, 0]));
    assert!(!valid(&[0xe1, 0, 128]));
    assert!(!valid(&[]));
}

#[test]
fn same_pitch_members_have_independent_cached_pitch_pressure_and_timbre() {
    let mut state = State::default();
    configure(&mut state, 0, 15);
    let mut poly = Poly::new(48_000.0, SynthInstrument::Keys, 4);
    state.input(&[0xe1, 0, 96]);
    state.input(&[0xd1, 91]);
    state.input(&[0xb1, 74, 100]);
    poly.note_on_expressive(60, 0.7, input(9, 1, 60), 9, Some(&state));
    poly.note_on_expressive(60, 0.7, input(9, 2, 60), 9, Some(&state));
    let first = poly
        .voices
        .iter()
        .position(|voice| voice.input == Some(input(9, 1, 60)))
        .unwrap();
    let second = poly
        .voices
        .iter()
        .position(|voice| voice.input == Some(input(9, 2, 60)))
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        poly.voices[first].expression_values,
        Values {
            pitch: 24.0,
            pressure: 91,
            timbre: 100
        }
    );
    assert_eq!(poly.voices[second].expression_values, Values::default());
    let update = state.input(&[0xe2, 0, 32]);
    poly.expression_input(9, &state, update);
    assert_eq!(poly.voices[first].expression_values.pitch, 24.0);
    assert_eq!(poly.voices[second].expression_values.pitch, -24.0);
    let counts = crate::engine::test_alloc::measure(|| {
        for _ in 0..4096 {
            assert!(poly.tick(48_000.0).is_finite());
        }
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
}

#[test]
fn sustain_freezes_released_member_while_manager_still_controls_its_tail() {
    let mut state = State::default();
    configure(&mut state, 0, 2);
    let mut poly = Poly::new(48_000.0, SynthInstrument::Keys, 4);
    state.input(&[0xb0, 64, 127]);
    state.input(&[0xe1, 0, 96]);
    poly.note_on_expressive(60, 0.7, input(9, 1, 60), 9, Some(&state));
    poly.note_off_input(input(9, 1, 60));
    let old = poly
        .voices
        .iter()
        .position(|voice| voice.expression.is_some_and(|note| !note.held))
        .unwrap();
    let update = state.input(&[0xe1, 0, 32]);
    poly.expression_input(9, &state, update);
    poly.note_on_expressive(60, 0.7, input(9, 1, 60), 9, Some(&state));
    let new = poly
        .voices
        .iter()
        .position(|voice| voice.input == Some(input(9, 1, 60)))
        .unwrap();
    assert_ne!(old, new);
    assert_eq!(poly.voices[old].expression_values.pitch, 24.0);
    assert_eq!(poly.voices[new].expression_values.pitch, -24.0);
    let update = state.input(&[0xe0, 0, 96]);
    poly.expression_input(9, &state, update);
    assert_eq!(poly.voices[old].expression_values.pitch, 25.0);
    assert_eq!(poly.voices[new].expression_values.pitch, -23.0);
    let update = state.input(&[0xb0, 64, 0]);
    poly.expression_input(9, &state, update);
    assert_eq!(poly.voices[old].env.stage, 4);
    assert!(matches!(poly.voices[new].env.stage, 1..=3));
}

#[test]
fn source_disconnect_retires_sustained_voices_even_when_command_queue_is_full() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.selected_track = 2;
    zone(&mut rt, 9, 0, 2);
    zone(&mut rt, 10, 15, 2);
    expression(&mut rt, 9, &[0xb0, 64, 127]);
    rt.apply(Command::LiveNoteOn {
        source: 9,
        ch: 1,
        note: 60,
        vel: 90,
    });
    rt.apply(Command::LiveNoteOff {
        source: 9,
        ch: 1,
        note: 60,
    });
    rt.apply(Command::LiveNoteOn {
        source: 10,
        ch: 14,
        note: 60,
        vel: 90,
    });
    let mut sounding = [0.0; 1024];
    rt.process(&mut sounding);
    assert!(sounding.iter().any(|sample| sample.abs() > 1e-6));
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.source_origin == 9 && voice.env.level > 0.01));
    let mut full = false;
    for _ in 0..10000 {
        if engine.cmd.send(Command::NudgeBpm(0.001)).is_err() {
            full = true;
            break;
        }
    }
    assert!(full);
    engine.cmd.release_midi_source(9);
    engine.cmd.forget_midi_expression(9);
    assert_eq!(
        crate::engine::test_alloc::measure(|| rt.process(&mut [0.0; 256])),
        crate::engine::test_alloc::Counts::default()
    );
    assert!(rt.expression.get(9).is_none());
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.source_origin == 9 && voice.env.stage == 4));
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.input == Some(input(10, 14, 60)) && matches!(voice.env.stage, 1..=3)));
}

#[test]
fn zone_changes_release_conventional_and_expressive_owners_and_stealing_stays_local() {
    let mut state = State::default();
    let mut poly = Poly::new(48_000.0, SynthInstrument::Keys, 2);
    poly.note_on_expressive(60, 0.7, input(9, 1, 60), 9, Some(&state));
    let update = configure(&mut state, 0, 2);
    poly.expression_input(9, &state, update);
    assert!(poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3)));
    for channel in [1, 2, 1] {
        poly.note_on_expressive(60, 0.7, input(9, channel, 60), 9, Some(&state));
    }
    assert_eq!(poly.voices.len(), 2);
    let update = state.input(&[0xe2, 0, 96]);
    poly.expression_input(9, &state, update);
    for voice in &poly.voices {
        if let Some(note) = voice.expression {
            assert_eq!(
                voice.expression_values.pitch,
                if note.channel == 2 { 24.0 } else { 0.0 }
            );
        }
    }
    poly.expression_retire(9);
    assert!(poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3)));
}

#[test]
fn bounded_source_and_reset_storage_never_allocate_or_leave_an_unreported_release() {
    let mut sources = Sources::default();
    let resets = Resets::default();
    let counts = crate::engine::test_alloc::measure(|| {
        for source in 1..=SOURCES as u64 {
            assert!(sources.input(source, &[0xb0, 101, 0]).is_some());
            resets.request(source);
        }
        assert!(sources.input(257, &[0xb0, 101, 0]).is_none());
        resets.request(257);
        assert!(resets.take_all());
        for index in 0..SOURCES {
            assert_eq!(resets.take(index), index as u64 + 1);
        }
        sources.retire(17);
        assert!(sources.input(257, &[0xb0, 101, 0]).is_some());
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
}

#[test]
fn route_and_learning_releases_preserve_the_native_zone_for_fresh_owned_notes() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    zone(&mut rt, 100, 0, 4);
    expression(&mut rt, 100, &[0xe2, 0, 96]);
    rt.apply(Command::RoutedNoteOn {
        source: 111,
        origin: 100,
        ch: 2,
        note: 60,
        vel: 90,
        track: 2,
        target: None,
    });
    rt.process(&mut [0.0; 256]);
    engine.cmd.release_midi_source(111);
    rt.process(&mut [0.0; 256]);
    assert!(rt.expression.get(100).is_some());
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .all(|voice| voice.input != Some(input(111, 2, 60))));
    rt.apply(Command::RoutedNoteOn {
        source: 222,
        origin: 100,
        ch: 2,
        note: 60,
        vel: 90,
        track: 3,
        target: None,
    });
    let voice = rt.tracks[3]
        .poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(input(222, 2, 60)))
        .unwrap();
    assert_eq!(voice.expression_values.pitch, 24.0);
    engine.cmd.release_midi_source(100);
    rt.process(&mut [0.0; 256]);
    assert!(rt.tracks[3]
        .poly
        .voices
        .iter()
        .all(|voice| voice.input != Some(input(222, 2, 60))));
    assert!(rt.expression.get(100).is_some());
    engine.cmd.forget_midi_expression(100);
    rt.process(&mut [0.0; 256]);
    assert!(rt.expression.get(100).is_none());
}

#[test]
fn private_midi_worker_delivers_spec_packets_and_two_byte_pressure_without_opening_ports() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.selected_track = 2;
    let mut port = engine.midi.open_for_test(
        &engine.cmd,
        9021,
        crate::engine::midi::MidiMap {
            name: "MPE specification fixture".into(),
            matchers: vec![],
            bindings: vec![],
            unmapped_notes: crate::engine::midi::UnmappedNotes::Live,
        },
        "MPE fixture",
        "MPE fixture",
    );
    port.push(&[
        0xb0, 101, 0, 0xb0, 100, 6, 0xb0, 6, 15, 0xe2, 0x2b, 0x41, 0xb2, 74, 64, 0xd2, 0, 0x92, 60,
        56, 0x93, 60, 56, 0xd2, 91,
    ]);
    rt.process(&mut [0.0; 256]);
    let first = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(input(9021, 2, 60)))
        .unwrap();
    let second = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(input(9021, 3, 60)))
        .unwrap();
    assert!((first.expression_values.pitch - 1.001_953_1).abs() < 1e-6);
    assert_eq!(first.expression_values.pressure, 91);
    assert_eq!(second.expression_values, Values::default());
    drop(port);
    rt.process(&mut [0.0; 256]);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3)));
}

pub(crate) fn expressive_clip() -> crate::engine::Clip {
    use crate::{
        engine::{
            midi_data::{Lanes, TickTiming},
            midi_edit::{NoteId, Region},
            Clip, ClipKind, MidiNote,
        },
        midi_file::Message,
    };
    let mut clip = Clip::empty();
    clip.kind = ClipKind::Midi;
    clip.bars = 1.0;
    clip.region = Some(Region {
        start: 0.0,
        end: 4.0,
        loop_start: 0.0,
        loop_end: 4.0,
        loop_enabled: true,
    });
    clip.notes = [2, 3]
        .into_iter()
        .enumerate()
        .map(|(index, channel)| MidiNote {
            variation: None,
            channel,
            release_vel: 64,
            source_timing: Some(TickTiming {
                ppqn: 96,
                start: 96,
                duration: 192,
                start_order: 10 + index as u32,
                end_order: 20 + index as u32,
            }),
            id: NoteId::new(),
            muted: false,
            pitch: 60,
            start: 1.0,
            len: 2.0,
            vel: 90,
        })
        .collect();
    let packets = [
        (0, 0, [0xb0, 101, 0], 3),
        (0, 1, [0xb0, 100, 6], 3),
        (0, 2, [0xb0, 6, 15], 3),
        (48, 3, [0xe2, 0, 96], 3),
        (48, 4, [0xe3, 0, 32], 3),
        (144, 12, [0xd2, 91, 0], 2),
        (144, 13, [0xb3, 74, 100], 3),
    ];
    clip.lanes = Some(
        Lanes::new(
            96,
            384,
            packets
                .into_iter()
                .map(|(tick, order, bytes, length)| Message {
                    tick,
                    order,
                    bytes,
                    length,
                })
                .collect(),
            vec![],
        )
        .unwrap(),
    );
    clip
}

#[test]
fn actual_clip_replay_chases_and_loops_independent_same_pitch_voices_without_ports() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.quant = 0.0;
    rt.bpm = 120.0;
    let clip = expressive_clip();
    let ids = clip.notes.iter().map(|note| note.id).collect::<Vec<_>>();
    rt.tracks[2].clips[0] = clip;
    rt.apply(Command::Play);
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    let mut audible = 0;
    let allocation = crate::engine::test_alloc::measure(|| {
        for _ in 0..310 {
            let mut samples = [0.0; 256];
            rt.process(&mut samples);
            audible += usize::from(samples.iter().any(|sample| sample.abs() > 1e-6));
        }
    });
    assert_eq!(allocation, crate::engine::test_alloc::Counts::default());
    assert!(audible > 100);
    let first = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.clip_note == Some(ids[0]))
        .unwrap();
    let second = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.clip_note == Some(ids[1]))
        .unwrap();
    assert_eq!(
        first.expression_values,
        Values {
            pitch: 24.0,
            pressure: 91,
            timbre: 64
        }
    );
    assert_eq!(
        second.expression_values,
        Values {
            pitch: -24.0,
            pressure: 0,
            timbre: 100
        }
    );
    rt.apply(Command::TimelineSeek(1.0));
    assert_eq!(
        crate::engine::test_alloc::measure(|| rt.process(&mut [0.0; 256])),
        crate::engine::test_alloc::Counts::default()
    );
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.clip_note == Some(ids[0]) && voice.expression_values.pitch == 24.0));
    for _ in 0..800 {
        rt.process(&mut [0.0; 256]);
    }
    assert_eq!(
        rt.tracks[2].clips[0]
            .notes
            .iter()
            .map(|note| note.id)
            .collect::<Vec<_>>(),
        ids
    );
    rt.apply(Command::Stop);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3)));
    drop(engine);
}

#[test]
fn prepared_seek_state_respects_zone_resets_and_complete_parameter_transactions() {
    use crate::{engine::midi_data::Lanes, midi_file::Message};
    let packets = [
        (0, [0xb0, 101, 0]),
        (1, [0xb0, 100, 6]),
        (2, [0xb0, 6, 15]),
        (3, [0xe2, 0, 96]),
        (4, [0xb0, 101, 0]),
        (5, [0xb0, 100, 6]),
        (6, [0xb0, 6, 0]),
        (7, [0xb0, 101, 0]),
        (8, [0xb0, 100, 6]),
        (9, [0xb0, 6, 15]),
        (10, [0xb2, 101, 0]),
        (11, [0xb2, 100, 0]),
        (12, [0xb2, 6, 24]),
        (13, [0xe2, 0, 96]),
    ];
    let lanes = Lanes::new(
        96,
        384,
        packets
            .into_iter()
            .enumerate()
            .map(|(order, (tick, bytes))| Message {
                tick,
                order: order as u32,
                bytes,
                length: 3,
            })
            .collect(),
        vec![],
    )
    .unwrap();
    let index = lanes.expression.as_ref().unwrap();
    let before = index.before(10.0, &lanes.state);
    assert_eq!(before.member(2), Values::default());
    let after = index.before(14.0, &lanes.state);
    assert_eq!(after.member(2).pitch, 12.0);
    let mut wire = State::default();
    after.setup(|bytes, length| {
        wire.input(&bytes[..usize::from(length)]);
    });
    assert_eq!(wire, after);
    assert_eq!(
        crate::engine::test_alloc::measure(|| {
            assert_eq!(index.before(14.0, &lanes.state), after);
        }),
        crate::engine::test_alloc::Counts::default()
    );
}
