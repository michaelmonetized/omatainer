use super::*;
use crate::engine::{midi_data::Lanes, test_alloc, MidiNote};

#[test]
fn arrangement_seek_emits_original_program_pending_banks_and_owned_notes_without_devices(){
    use crate::engine::{arrangement::{Model,Source,Instance},Command,Engine};
    use std::sync::atomic::{AtomicBool,Ordering::Release};
    let(engine,rt)=Engine::headless_for_test(48000,256);let mut rt=Box::new(rt);rt.bpm=120.0;
    let handle=engine.project.clone();let capture=std::thread::spawn(move||handle.capture(&AtomicBool::new(false)).unwrap());while !capture.is_finished(){rt.process(&mut []);std::thread::sleep(std::time::Duration::from_millis(1));}let captured=capture.join().unwrap();
    let clip=controller_clip(&[(0,&[0xb3,0,2]),(0,&[0xb3,32,3]),(0,&[0xc3,5]),(960,&[0xb3,0,9]),(960,&[0xb3,32,10]),(1920,&[0xb3,74,88]),(2880,&[0xe3,9,70]),(3840,&[0xd3,12]),(5760,&[0xb3,74,99])]);
    let mut saved=captured.state.tracks[2].clips[7].clone();saved.kind=clip.kind;saved.name="owned song MIDI".into();saved.bars=clip.bars;saved.region=clip.region;saved.notes=clip.notes;saved.lanes=clip.lanes;
    let track=captured.state.session.as_ref().unwrap().reference(crate::engine::session::Axis::Track,2).unwrap();let model=Model{enabled:true,next_id:3,sources:vec![Source{id:1,clip:saved}],instances:vec![Instance{id:2,source:1,track,start:0.0,offset:0.0,duration:16.0,repeating:false,gain:1.0}]};let(request,ack)=crate::engine::arrangement::edit::Request::prepare(captured,model,&AtomicBool::new(false)).unwrap();assert_eq!(test_alloc::measure(||rt.apply(Command::ArrangementEdit(request))),Default::default());assert_eq!(ack.state(),crate::engine::midi_edit::Outcome::Applied);
    let shared=engine.cmd.midi_routing();let events=shared.receiver.lock().take().unwrap();shared.bind_identity(&super::super::Routing{enabled:true,routes:vec![super::super::Route{track:2,inputs:vec![],output:None,output_channel:None,monitor:false,thru:false,filter:Default::default()}]});shared.mask.store(1<<2,Release);shared.alive.store(true,Release);
    let mut packets=Vec::with_capacity(16);assert_eq!(test_alloc::measure(||{rt.apply(Command::Play);rt.apply(Command::TimelineSeek(2.5));rt.process(&mut[0.0;4]);while let Ok(event)=events.try_recv(){if event.clear.is_none(){packets.push(event);}}}),Default::default());
    assert_eq!(packets.iter().map(|p|p.packet.bytes()).collect::<Vec<_>>(),vec![&[0xb3,0,2][..],&[0xb3,32,3],&[0xc3,5],&[0xb3,0,9],&[0xb3,32,10],&[0xb3,74,88],&[0xe3,9,70],&[0xd3,12],&[0x93,60,100]]);assert!(matches!(packets.last().unwrap().owner,Owner::Clip{track:2,..}));shared.alive.store(false,Release);
}

fn controller_clip(messages: &[(u64, &[u8])]) -> Clip {
    let mut clip = Clip::empty(); clip.kind = crate::engine::ClipKind::Midi; clip.bars = 4.0;
    clip.region = Some(Region { loop_enabled: false, ..Region::full(4.0) });
    clip.notes = vec![MidiNote { id: crate::engine::midi_edit::NoteId::new(), channel: 3, release_vel: 64, pitch: 60, start: 0.0, len: 10.0, vel: 100, muted: false, source_timing: None }];
    clip.lanes = Some(Lanes::new(960, 15360, messages.iter().enumerate().map(|(i, (tick, bytes))| { let mut saved = [0; 3]; saved[..bytes.len()].copy_from_slice(bytes); crate::midi_file::Message { tick: *tick, order: i as u32, bytes: saved, length: bytes.len() as u8 } }).collect(), vec![]).unwrap());
    clip
}
#[test]
fn controller_chase_selects_the_original_patch_then_restores_pending_banks_before_notes() {
    let clip = controller_clip(&[(0, &[0xb3, 0, 2]), (0, &[0xb3, 32, 3]), (0, &[0xc3, 5]), (960, &[0xb3, 0, 9]), (960, &[0xb3, 32, 10]), (1920, &[0xb3, 74, 88]), (2880, &[0xe3, 9, 70]), (3840, &[0xd3, 12]), (5760, &[0xb3, 74, 99])]);
    let mut playback = Playback::default();
    assert_eq!(test_alloc::measure(|| playback.rebuild(&clip, 5.0, false, &[])), test_alloc::Counts::default());
    let mut trace = Vec::with_capacity(16);
    assert_eq!(test_alloc::measure(|| while let Some((packet, _, _)) = playback.next(2, &clip, 5.0001) { trace.push(packet); }), test_alloc::Counts::default());
    assert_eq!(trace.iter().map(|p| p.bytes()).collect::<Vec<_>>(), vec![&[0xb3,0,2][..], &[0xb3,32,3][..], &[0xc3,5][..], &[0xb3,0,9][..], &[0xb3,32,10][..], &[0xb3,74,88][..], &[0xe3,9,70][..], &[0xd3,12][..], &[0x93,60,100][..]]);
    assert_eq!(playback.next(2, &clip, 6.0001).unwrap().0.bytes(), &[0xb3,74,99]);
}
#[test]
fn controller_chase_honors_reset_boundaries_and_does_not_guess_parameter_transactions() {
    let clip = controller_clip(&[(0,&[0xb3,7,90]), (0,&[0xb3,1,70]), (0,&[0xe3,0,70]), (960,&[0xb3,101,0]), (960,&[0xb3,100,1]), (960,&[0xb3,6,5]), (1920,&[0xb3,121,0]), (2880,&[0xd3,33])]);
    let mut playback = Playback::default(); playback.rebuild(&clip, 4.0, false, &[]);
    let mut trace = vec![]; while let Some((packet,_,_)) = playback.next(2,&clip,4.0001) { trace.push(packet.bytes().to_vec()); }
    assert_eq!(trace, vec![vec![0xb3,121,0],vec![0xb3,7,90],vec![0xd3,33],vec![0x93,60,100]]);
}
#[test]
fn controller_chase_after_a_loop_uses_the_previous_pass_until_a_new_point_arrives() {
    let mut clip = controller_clip(&[(0,&[0xb3,74,10]), (2880,&[0xb3,74,99])]);
    clip.region = Some(Region { loop_enabled: true, start: 0.0, end: 4.0, loop_start: 1.0, loop_end: 4.0 });
    let mut playback = Playback::default(); playback.rebuild(&clip, 4.5, true, &[]);
    assert_eq!(playback.next(2,&clip,4.5001).unwrap().0.bytes(), &[0xb3,74,99]);
}
#[test]
fn controller_points_at_launch_and_seek_arrive_before_new_or_chased_native_notes() {
    let clip = controller_clip(&[(0,&[0xb3,0,2]), (0,&[0xb3,32,3]), (0,&[0xc3,5]), (1920,&[0xb3,74,99])]);
    let mut playback = Playback::default(); playback.rebuild(&clip,0.0,false,&[]);
    let mut start = vec![]; while let Some((p,_,_)) = playback.next(2,&clip,0.0001) { start.push(p.bytes().to_vec()); }
    assert_eq!(start, vec![vec![0xb3,0,2],vec![0xb3,32,3],vec![0xc3,5],vec![0x93,60,100]]);
    playback.rebuild(&clip,2.0,false,&[]);
    let mut seek = vec![]; while let Some((p,_,_)) = playback.next(2,&clip,2.0001) { seek.push(p.bytes().to_vec()); }
    assert_eq!(&seek[seek.len()-2..], &[vec![0xb3,74,99],vec![0x93,60,100]]);
}

#[test]
fn actual_ramp_controller_lane_emission_matches_an_independent_sample_timestamp_oracle() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    use std::sync::atomic::Ordering::Release;
    let map = Conductor::native(960, vec![Tempo::new(0, 120.0, true).unwrap(), Tempo::new(8160, 180.0, false).unwrap()], vec![
        Meter { tick: 0, numerator: 7, denominator_power: 3, clocks: 12, thirty_seconds: 8 },
        Meter { tick: 3360, numerator: 5, denominator_power: 2, clocks: 24, thirty_seconds: 8 },
        Meter { tick: 8160, numerator: 4, denominator_power: 2, clocks: 24, thirty_seconds: 8 },
    ], TimingSettings::default()).unwrap();
    let b = 60000000.0 / f64::from(map.tempos[1].micros);
    let timestamp = |beat: f64| {
        let n = 1000; let length = beat.min(8.5); let h = length / f64::from(n);
        let f = |x: f64| 60.0 / (120.0 + (b - 120.0) * x / 8.5);
        let mut sum = f(0.0) + f(length);
        for i in 1..n { sum += f(f64::from(i) * h) * if i % 2 == 0 { 2.0 } else { 4.0 }; }
        sum * h / 3.0 + (beat - 8.5).max(0.0) * 60.0 / b
    };
    let starts = [0.0, 0.5, 3.5, 5.0, 8.5, 12.5];
    for rate in [44100, 48000, 96000] {
        let (engine, mut rt) = crate::engine::Engine::headless_for_test(rate, 48);
        let shared = engine.cmd.midi_routing();
        let events = shared.receiver.lock().take().unwrap();
        shared.bind_identity(&super::super::Routing { enabled: true, routes: vec![super::super::Route { track: 2, inputs: vec![], output: None, output_channel: None, monitor: false, thru: false, filter: Default::default() }] });
        shared.mask.store(1 << 2, Release); shared.alive.store(true, Release);
        let mut clip = Clip::empty(); clip.kind = crate::engine::ClipKind::Midi;
        clip.region = Some(Region::full(16.0)); clip.bars = 4.0;
        clip.lanes = Some(Lanes::new(960, 15360, starts.iter().enumerate().map(|(i, &beat)| crate::midi_file::Message { tick: (beat * 960.0) as u64, order: i as u32, bytes: [0xb3, 74, (20 + i) as u8], length: 3 }).collect(), vec![]).unwrap());
        rt.tracks[2].clips[0] = clip; rt.conductor = Some(map.clone()); rt.quant = 0.0;
        rt.tracks[2].midi_output.trace = Some(Vec::with_capacity(16));
        engine.cmd.send(crate::engine::Command::FireClip { track: 2, scene: 0, looping: false }).unwrap(); rt.process(&mut []);
        let frames = (timestamp(12.6) * f64::from(rate)).ceil() as usize;
        let mut output = [0.0; 514]; let mut packets = Vec::with_capacity(16);
        assert_eq!(test_alloc::measure(|| { for begin in (0..frames).step_by(257) {
            rt.process(&mut output[..(frames - begin).min(257) * 2]);
            while let Ok(event) = events.try_recv() { if event.clear.is_none() { packets.push(event.packet); } }
        } }), test_alloc::Counts::default());
        let trace = rt.tracks[2].midi_output.trace.take().unwrap(); assert_eq!(trace.len(), starts.len());
        assert_eq!(packets, trace.iter().map(|(_, p)| *p).collect::<Vec<_>>());
        for (i, ((elapsed, packet), beat)) in trace.into_iter().zip(&starts).enumerate() {
            let frame = (timestamp(elapsed) * f64::from(rate) - 1.0).round().max(0.0) as u64;
            let expected = (timestamp(*beat) * f64::from(rate)).floor() as u64;
            assert!(frame.abs_diff(expected) <= 1, "{rate} beat{beat}: {frame} vs{expected}");
            assert_eq!(packet, Packet::new(&[0xb3, 74, (20 + i) as u8]).unwrap());
        }
        shared.alive.store(false, Release);
    }
}

fn fixture() -> (crate::midi_file::File, Clip) {
    let file = crate::midi_file::decode(include_bytes!(
        "../../../../../tests/fixtures/midi/sixteen-bars-ppqn960.mid"
    ))
    .unwrap();
    let track = &file.tracks[1];
    let mut clip = Clip::empty();
    clip.kind = crate::engine::ClipKind::Midi;
    clip.bars = 16.0;
    clip.region = Some(Region {
        loop_enabled: false,
        ..Region::full(16.0)
    });
    clip.notes = track
        .notes
        .iter()
        .map(|n| MidiNote::from_smf(n, file.ppqn).unwrap())
        .collect();
    clip.lanes = Some(
        Lanes::new(
            file.ppqn,
            track.end_tick,
            track.messages.clone(),
            track.meta.clone(),
        )
        .unwrap(),
    );
    (file, clip)
}
#[test]
fn external_sixteen_bars_keeps_every_channel_velocity_controller_and_frame() {
    let (file, clip) = fixture();
    for rate in [48000u32, 96000] {
        let mut playback = Playback::default();
        assert_eq!(
            test_alloc::measure(|| playback.rebuild(&clip, 0.0, false, &[])),
            test_alloc::Counts::default()
        );
        let mut trace = Vec::with_capacity(256);
        let count = ((16.0 + 32.0 * 666667.0 / 1_000_000.0) * f64::from(rate)).ceil() as u64 + 2;
        let counts = test_alloc::measure(|| {
            for frame in 0..count {
                let seconds = (frame + 1) as f64 / f64::from(rate);
                let elapsed = if seconds <= 16.0 {
                    seconds * 2.0
                } else {
                    32.0 + (seconds - 16.0) * 1_000_000.0 / 666667.0
                };
                while let Some((packet, _, weight)) = playback.next(2, &clip, elapsed) {
                    assert_eq!(weight, 1);
                    trace.push((frame, packet));
                }
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        let frame = |tick: u64| {
            ((tick.min(30720) as f64 * 500000.0 + tick.saturating_sub(30720) as f64 * 666667.0)
                * f64::from(rate)
                / (960.0 * 1_000_000.0)
                + 1e-7)
                .floor() as u64
        };
        let mut expected = Vec::new();
        for note in &file.tracks[1].notes {
            expected.push((
                frame(note.start_tick),
                note.start_order,
                Packet::new(&[0x90 | note.channel, note.pitch, note.velocity]).unwrap(),
            ));
            expected.push((
                frame(note.start_tick + note.duration_ticks),
                note.end_order,
                Packet::new(&[0x80 | note.channel, note.pitch, note.release_velocity]).unwrap(),
            ));
        }
        for message in &file.tracks[1].messages {
            expected.push((
                frame(message.tick),
                message.order,
                Packet::new(&message.bytes[..usize::from(message.length)]).unwrap(),
            ));
        }
        expected.sort_by_key(|(frame, order, _)| (*frame, *order));
        assert_eq!(
            trace,
            expected
                .into_iter()
                .map(|(f, _, p)| (f, p))
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn seek_capacity_and_loop_boundary_order_are_prepared_without_callback_growth() {
    let (_, mut clip) = fixture();
    clip.region = None;
    clip.bars = 1.0;
    let prototype = clip.notes[0].clone();
    clip.notes = (0..crate::engine::project::MAX_NOTES_PER_CLIP)
        .map(|i| MidiNote {
            id: crate::engine::midi_edit::NoteId::new(),
            pitch: (60 + i % 12) as u8,
            start: 0.0,
            len: 16.0,
            source_timing: None,
            ..prototype.clone()
        })
        .collect();
    clip.lanes = None;
    let mut playback = Playback::default();
    assert_eq!(
        test_alloc::measure(|| playback.rebuild(&clip, 14.0, true, &[])),
        test_alloc::Counts::default()
    );
    assert_eq!(
        playback.events.len(),
        3 * crate::engine::project::MAX_NOTES_PER_CLIP
    );
    assert_eq!(playback.next(2, &clip, 14.0001).unwrap().2, 4);
    let mut clip = Clip::empty();
    clip.kind = crate::engine::ClipKind::Midi;
    clip.bars = 1.0;
    clip.notes = vec![MidiNote {
        id: crate::engine::midi_edit::NoteId::new(),
        channel: 3,
        release_vel: 27,
        pitch: 60,
        start: 0.0,
        len: 4.0,
        vel: 90,
        muted: false,
        source_timing: None,
    }];
    playback.rebuild(&clip, 0.0, true, &[]);
    assert_eq!(
        playback.next(2, &clip, 0.0001).unwrap().0.bytes(),
        &[0x93, 60, 90]
    );
    assert_eq!(
        playback.next(2, &clip, 4.0001).unwrap().0.bytes(),
        &[0x83, 60, 27]
    );
    assert_eq!(
        playback.next(2, &clip, 4.0001).unwrap().0.bytes(),
        &[0x93, 60, 90]
    );
}
#[test]
fn lane_final_boundary_is_included() {
    let (_, mut clip) = fixture();
    let lane = crate::midi_file::Message {
        tick: 61440,
        order: 2000,
        bytes: [0xb2, 1, 99],
        length: 3,
    };
    clip.lanes = Some(Lanes::new(960, 61440, vec![lane], vec![]).unwrap());
    let mut playback = Playback::default();
    playback.rebuild(&clip, 63.9, false, &[]);
    let mut end = Vec::new();
    while let Some((p, _, _)) = playback.next(2, &clip, 64.0001) {
        end.push(p);
    }
    assert!(end.iter().any(|p| p.bytes() == [0xb2, 1, 99]));
}

#[test]
fn actual_renderer_emits_all_source_messages_at_exact_frames_with_conductor() {
    use std::sync::atomic::Ordering::*;
    for rate in [48000, 96000] {
        let (file, clip) = fixture();
        let (engine, mut rt) = crate::engine::Engine::headless_for_test(rate, 256);
        let shared = engine.cmd.midi_routing();
        let events = shared.receiver.lock().take().unwrap();
        shared.bind_identity(&super::super::Routing { enabled:true, routes:vec![super::super::Route {track:2,inputs:vec![],output:None,output_channel:None,monitor:false,thru:false,filter:Default::default()}] });
        shared.mask.store(1 << 2, Release);
        shared.alive.store(true, Release);
        rt.conductor = Some(
            crate::engine::midi_data::Conductor::from_meta(
                file.ppqn,
                file.tracks.iter().flat_map(|t| t.meta.clone()),
            )
            .unwrap(),
        );
        rt.tracks[2].clips[0] = clip;
        rt.tracks[2].midi_output.trace = Some(Vec::with_capacity(256));
        rt.quant = 0.0;
        let conductor = rt.conductor.clone().unwrap();
        engine
            .cmd
            .send(crate::engine::Command::FireClip {
                track: 2,
                scene: 0,
                looping: false,
            })
            .unwrap();
        rt.process(&mut []);
        let frames = (conductor.seconds_at(65.0) * f64::from(rate)).ceil() as usize;
        let mut output = [0.0; 514];
        let mut packets = Vec::with_capacity(256);
        assert_eq!(
            test_alloc::measure(|| for _ in 0..frames.div_ceil(257) {
                rt.process(&mut output);
                while let Ok(event) = events.try_recv() {
                    if event.clear.is_none() {
                        packets.push(event.packet);
                    }
                }
            }),
            test_alloc::Counts::default()
        );
        let trace = rt.tracks[2].midi_output.trace.take().unwrap();
        assert_eq!(packets, trace.iter().map(|(_, p)| *p).collect::<Vec<_>>());
        let actual = trace
            .into_iter()
            .map(|(elapsed, packet)| {
                (
                    (conductor.seconds_at(elapsed) * f64::from(rate) - 1.0)
                        .round()
                        .max(0.0) as u64,
                    packet,
                )
            })
            .collect::<Vec<_>>();
        let frame = |tick: u64| {
            ((tick.min(30720) as f64 * 500000.0 + tick.saturating_sub(30720) as f64 * 666667.0)
                * f64::from(rate)
                / (960.0 * 1_000_000.0)
                + 1e-7)
                .floor() as u64
        };
        let mut expected = Vec::new();
        for note in &file.tracks[1].notes {
            expected.push((
                frame(note.start_tick),
                note.start_order,
                Packet::new(&[0x90 | note.channel, note.pitch, note.velocity]).unwrap(),
            ));
            expected.push((
                frame(note.start_tick + note.duration_ticks),
                note.end_order,
                Packet::new(&[0x80 | note.channel, note.pitch, note.release_velocity]).unwrap(),
            ));
        }
        for message in &file.tracks[1].messages {
            expected.push((
                frame(message.tick),
                message.order,
                Packet::new(&message.bytes[..usize::from(message.length)]).unwrap(),
            ));
        }
        expected.sort_by_key(|(frame, order, _)| (*frame, *order));
        assert_eq!(
            actual,
            expected
                .into_iter()
                .map(|(f, _, p)| (f, p))
                .collect::<Vec<_>>(),
            "actual renderer at {rate} Hz"
        );
        assert_eq!(shared.activity().0[2].overruns, 0);
        shared.alive.store(false, Release);
    }
}

#[test]
fn recording_first_pass_and_unrelated_cell_edits_do_not_retrigger_external_notes() {
    use std::sync::atomic::Ordering::*;
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 256);
    let shared = engine.cmd.midi_routing();
    let events = shared.receiver.lock().take().unwrap();
    shared.alive.store(true, Release);
    shared.bind_identity(&super::super::Routing { enabled:true, routes:vec![super::super::Route {track:2,inputs:vec![],output:None,output_channel:None,monitor:false,thru:false,filter:Default::default()}] });
        shared.mask.store(1 << 2, Release);
    rt.quant = 0.0;
    rt.selected_scene = 0;
    rt.tracks[2].clips[0] = Clip::empty();
    rt.tracks[2].clips[0].kind = crate::engine::ClipKind::Midi;
    rt.tracks[2].clips[0].bars = 1.0;
    engine
        .cmd
        .send(crate::engine::Command::FireClip {
            track: 2,
            scene: 0,
            looping: true,
        })
        .unwrap();
    engine.cmd.send(crate::engine::Command::Record).unwrap();
    engine
        .cmd
        .send(crate::engine::Command::RoutedNoteOn {
            target: None,
            source: 888,
            ch: 3,
            note: 60,
            vel: 95,
            track: 2,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    assert_eq!(rt.tracks[2].clips[0].notes.len(), 1);
    assert!(rt.tracks[2].recorded_playback[0].is_some());
    assert_eq!(rt.tracks[2].clips[0].notes[0].channel, 3);
    for _ in 0..10 {
        rt.process(&mut [0.0; 256]);
    }
    assert!(
        !events.try_iter().any(|event| event.clear.is_none()),
        "physical first pass was duplicated into clip output"
    );
    engine
        .cmd
        .send(crate::engine::Command::LiveNoteOff {
            source: 888,
            ch: 3,
            note: 60,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    for _ in 0..400 {
        rt.process(&mut [0.0; 512]);
    }
    let packets = events
        .try_iter()
        .filter(|event| event.clear.is_none())
        .map(|event| event.packet)
        .collect::<Vec<_>>();
    assert!(
        packets
            .iter()
            .any(|packet| packet.bytes() == [0x93, 60, 95]),
        "released recording failed to play on its next loop"
    );
    rt.tracks[2].midi_output.dirty = false;
    rt.tracks[2].midi_output.clear = false;
    let midi_beat = rt.precise_midi_beat();
    rt.tracks[2].clip_notes_changed(1, rt.beat, midi_beat);
    assert!(
        !rt.tracks[2].midi_output.dirty && !rt.tracks[2].midi_output.clear,
        "editing another cell interrupted active output"
    );
    shared.alive.store(false, Release);
}

#[test]
fn publication_and_reset_refuse_old_renderer_packets_until_relaunch() {
    use std::sync::atomic::Ordering::*;
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 256);
    let shared = engine.cmd.midi_routing();
    let events = shared.receiver.lock().take().unwrap();
    shared.alive.store(true, Release);
    shared.bind_identity(&super::super::Routing { enabled:true, routes:vec![super::super::Route {track:2,inputs:vec![],output:None,output_channel:None,monitor:false,thru:false,filter:Default::default()}] });
        shared.mask.store(1 << 2, Release);
    let (_, clip) = fixture();
    rt.tracks[2].clips[0] = clip;
    rt.quant = 0.0;
    engine
        .cmd
        .send(crate::engine::Command::FireClip {
            track: 2,
            scene: 0,
            looping: true,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    events.try_iter().for_each(drop);
    let old = (
        rt.tracks[2].midi_output.generation,
        rt.tracks[2].midi_output.epoch,
    );
    let packet = Packet::new(&[0x90, 60, 100]).unwrap();
    shared.reset_outputs();
    assert!(!shared.emit_weighted(2, packet, Owner::ClipLane(2), 1, old));
    rt.process(&mut [0.0; 128]);
    assert!(rt.tracks[2].midi_output.refused);
    assert!(shared.activity().0[2].clip_refused);
    assert!(events.try_recv().is_err());
    engine
        .cmd
        .send(crate::engine::Command::FireClip {
            track: 2,
            scene: 0,
            looping: true,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    assert!(!rt.tracks[2].midi_output.refused);
    events.try_iter().for_each(drop);
    let prior = (
        rt.tracks[2].midi_output.generation,
        rt.tracks[2].midi_output.epoch,
    );
    shared.generation.store(2, Release);
    shared.reset_outputs();
    assert!(!shared.emit_weighted(2, packet, Owner::ClipLane(2), 1, prior));
    rt.process(&mut [0.0; 128]);
    assert_eq!(rt.tracks[2].midi_output.generation, 2);
    assert!(!rt.tracks[2].midi_output.refused);
    shared.alive.store(false, Release);
}
