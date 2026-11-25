use super::*;
use std::sync::Arc;
use crate::engine::test_alloc;

fn configured(capacity: u32) -> (Arc<Shared>, Instant) {
    let shared = Arc::new(Shared::default());
    let start = Instant::now();
    shared
        .configure(
            Config {
                enabled: true,
                seconds: 120,
                events: capacity,
            },
            start,
        )
        .unwrap();
    (shared, start)
}
fn send(shared: &Shared, start: Instant, millis: u64, track: u8, source: u64, bytes: &[u8]) {
    let allocation = super::super::test_alloc::measure(|| {
        assert!(shared.observe(track, source, start + Duration::from_millis(millis), bytes))
    });
    assert_eq!((allocation.allocations, allocation.frees), (0, 0));
}
fn choice() -> Choice {
    Choice {
        track: 2,
        source: 17,
        from_seconds_ago: 1.0,
        to_seconds_ago: 0.0,
        tempo: 120.0,
        loop_beats: 4.0,
    }
}

#[test]
fn mpe_setup_before_opt_in_survives_capture_replay_and_actual_smf_export() {
    use crate::engine::{midi_data::Lanes, midi_interchange, mpe::{State,Values}, project, Command, Engine};
    let (shared,start)=configured(256);
    let mut state=State::default();
    for bytes in [[0xb0,101,0],[0xb0,100,6],[0xb0,6,4]] {state.input(&bytes);}
    state.input(&[0xe1,0,96]);state.input(&[0xe2,0,32]);
    let trace=[(50,&[0x91,60,100][..]),(75,&[0x92,60,80][..]),(100,&[0xd1,91][..]),(125,&[0xb2,74,24][..]),(700,&[0x81,60,37][..]),(750,&[0x82,60,21][..])];
    for (millis,bytes) in trace {assert!(shared.observe_with_setup(2,17,start+Duration::from_millis(millis),bytes,Some(&state)));state.input(bytes);}
    let snapshot=shared.snapshot(start+Duration::from_secs(1));
    let captured=prepare(&snapshot,choice(),&AtomicBool::new(false)).unwrap();
    let lanes=Lanes::new(captured.content.ppqn,captured.content.end_tick,captured.content.messages.clone(),captured.content.meta.clone()).unwrap();
    let index=lanes.expression.as_ref().unwrap();
    assert_eq!(index.before(300.0,&lanes.state).member(1),Values {pitch:24.0,pressure:91,timbre:64});
    assert_eq!(index.before(300.0,&lanes.state).member(2),Values {pitch:-24.0,pressure:0,timbre:24});
    let (_,mut rt)=Engine::headless_for_test(48000,256);
    let clip=&mut rt.tracks[2].clips[0];clip.kind=crate::engine::ClipKind::Midi;clip.bars=1.0;clip.notes=captured.content.notes.clone();clip.lanes=Some(lanes.clone());
    rt.quant=0.0;rt.bpm=120.0;rt.apply(Command::LaunchClip {track:2,scene:0});
    let mut energy=0.0f32;
    assert_eq!(test_alloc::measure(||for _ in 0..100 {let mut block=[0.0;256];rt.process(&mut block);energy+=block.iter().map(|value|value.abs()).sum::<f32>();}),Default::default());
    assert!(energy>1.0);
    for (id,expected) in captured.content.notes.iter().map(|note|(note.id,if note.channel==1 {24.0} else {-24.0})) {
        assert_eq!(rt.tracks[2].poly.voices.iter().find(|voice|voice.clip_note==Some(id)).unwrap().expression_values.pitch,expected);
    }
    let (mut saved,media)=project::State::empty().unwrap();let clip=&mut saved.tracks[2].clips[0];clip.kind=crate::engine::ClipKind::Midi;clip.bars=1.0;clip.notes=captured.content.notes;clip.lanes=Some(lanes);
    saved.validate(&media).unwrap();
    let file=midi_interchange::export(&saved,&[(2,0)],midi_interchange::ExportOptions {ppqn:PPQN,single_track:true,include_muted:true,allow_rounding:false,session_conductor:false}).unwrap();
    let roundtrip=crate::midi_file::decode(&crate::midi_file::encode(&file,false).unwrap()).unwrap();
    assert_eq!(roundtrip.tracks[0].messages.iter().map(|m|(m.tick,m.bytes,m.length)).collect::<Vec<_>>(),file.tracks[0].messages.iter().map(|m|(m.tick,m.bytes,m.length)).collect::<Vec<_>>());
}

#[test]
fn aged_mpe_zone_and_range_are_preserved_at_the_retained_history_boundary() {
    let (shared,start)=configured(256);
    shared.configure(Config {enabled:true,seconds:1,events:256},start).unwrap();
    for bytes in [[0xb0,101,0],[0xb0,100,6],[0xb0,6,4],[0xb1,101,0],[0xb1,100,0],[0xb1,6,12]] {send(&shared,start,20,2,17,&bytes);}
    for millis in (200..=2000).step_by(100) {send(&shared,start,millis,2,17,&[0xb1,1,10]);}
    send(&shared,start,2100,2,17,&[0x91,60,100]);
    send(&shared,start,2150,2,17,&[0xe1,0,96]);
    send(&shared,start,2200,2,17,&[0x81,60,20]);
    let snapshot=shared.snapshot(start+Duration::from_millis(2250));
    let prepared=prepare(&snapshot,choice(),&AtomicBool::new(false)).unwrap();
    let lanes=crate::engine::midi_data::Lanes::new(PPQN,prepared.content.end_tick,prepared.content.messages,prepared.content.meta).unwrap();
    let state=lanes.expression.as_ref().unwrap().before(1800.0,&lanes.state);
    assert_eq!(state.role(1),Some((0,true)));
    assert_eq!(state.range(1),12);
    assert_eq!(state.member(1).pitch,6.0);
}
#[test]
fn timestamped_overlapping_trace_retains_original_channels_release_velocity_expression_and_wire_order(
) {
    let (shared, start) = configured(256);
    send(&shared, start, 50, 2, 17, &[0x91, 60, 100]);
    send(&shared, start, 75, 2, 17, &[0x92, 60, 80]);
    send(&shared, start, 100, 2, 17, &[0xe1, 0, 96]);
    send(&shared, start, 125, 2, 17, &[0xe2, 0, 32]);
    send(&shared, start, 200, 2, 17, &[0xb2, 74, 111]);
    send(&shared, start, 250, 2, 17, &[0xd1, 90]);
    send(&shared, start, 300, 2, 17, &[0xa2, 60, 78]);
    send(&shared, start, 500, 2, 17, &[0x81, 60, 37]);
    send(&shared, start, 750, 2, 17, &[0x82, 60, 21]);
    let snapshot = shared.snapshot(start + Duration::from_secs(1));
    let prepared = prepare(&snapshot, choice(), &AtomicBool::new(false)).unwrap();
    assert_eq!(prepared.content.notes.len(), 2);
    let a = &prepared.content.notes[0];
    let b = &prepared.content.notes[1];
    assert_ne!(a.id, b.id);
    assert_eq!((a.channel, a.pitch, a.vel, a.release_vel), (1, 60, 100, 37));
    assert_eq!((b.channel, b.pitch, b.vel, b.release_vel), (2, 60, 80, 21));
    assert_eq!(
        (
            a.source_timing.unwrap().start,
            a.source_timing.unwrap().duration
        ),
        (96, 864)
    );
    assert_eq!(
        (
            b.source_timing.unwrap().start,
            b.source_timing.unwrap().duration
        ),
        (144, 1296)
    );
    assert_eq!(
        prepared
            .content
            .messages
            .iter()
            .map(|m| (m.tick, m.order, m.bytes, m.length))
            .collect::<Vec<_>>(),
        vec![
            (192, 3, [0xe1, 0, 96], 3),
            (240, 4, [0xe2, 0, 32], 3),
            (384, 5, [0xb2, 74, 111], 3),
            (480, 6, [0xd1, 90, 0], 2),
            (576, 7, [0xa2, 60, 78], 3)
        ]
    );
    assert_eq!(
        (
            prepared.clipped_at_start,
            prepared.held_at_end,
            prepared.short_gates_extended
        ),
        (0, 0, 0)
    );
    assert_eq!(snapshot.events.len(), 9);
    eprintln!("MIDI_RETROSPECTIVE_TRACE {{\"original_timestamps\":true,\"overlapping_same_pitch_channels\":true,\"independent_wire_expression\":true,\"release_velocity\":true,\"callback_allocations\":0,\"callback_frees\":0,\"physical_devices_opened\":false}}");
}
#[test]
fn range_clipping_preserves_rpn_order_and_reports_held_boundaries_without_record_arming() {
    let (shared, start) = configured(256);
    for (ms, bytes) in [
        (10, [0xb1, 101, 0]),
        (20, [0xb1, 100, 0]),
        (30, [0xb1, 6, 48]),
        (40, [0xb1, 38, 0]),
        (50, [0x91, 64, 95]),
        (600, [0xe1, 12, 70]),
    ] {
        send(&shared, start, ms, 2, 17, &bytes);
    }
    let snapshot = shared.snapshot(start + Duration::from_secs(1));
    let mut range = choice();
    range.from_seconds_ago = 0.5;
    let prepared = prepare(&snapshot, range, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        prepared.content.messages[..4]
            .iter()
            .map(|m| m.bytes)
            .collect::<Vec<_>>(),
        vec![[0xb1, 101, 0], [0xb1, 100, 0], [0xb1, 6, 48], [0xb1, 38, 0]]
    );
    assert!(prepared.content.messages[..4].iter().all(|m| m.tick == 0));
    assert_eq!(prepared.clipped_at_start, 1);
    assert_eq!(prepared.held_at_end, 1);
    let note = &prepared.content.notes[0];
    assert_eq!(
        (
            note.source_timing.unwrap().start,
            note.source_timing.unwrap().duration
        ),
        (0, 960)
    );
    assert_eq!(note.release_vel, 0);
    assert!(note.variation.is_none());
}
#[test]
fn disabled_private_history_clear_limits_overwrites_disconnect_and_invalid_frames_are_explicit() {
    let shared = Shared::default();
    let start = Instant::now();
    assert!(!shared.observe(2, 17, start, &[0x91, 60, 100]));
    assert!(shared.snapshot(start).events.is_empty());
    shared
        .configure(
            Config {
                enabled: true,
                seconds: 120,
                events: 256,
            },
            start,
        )
        .unwrap();
    assert!(!shared.observe(2, 17, start, &[0x91, 60, 128]));
    assert!(!shared.observe(2, 17, start, &[0xf0, 1, 0xf7]));
    for index in 0..300 {
        send(
            &shared,
            start,
            index,
            2,
            17,
            &[0xb1, 7, (index % 128) as u8],
        );
    }
    let review = shared.snapshot(start + Duration::from_secs(1));
    assert_eq!((review.events.len(), review.dropped), (256, 44));
    assert!(prepare(&review, choice(), &AtomicBool::new(false))
        .unwrap_err()
        .contains("discarded"));
    let epoch = shared.clear(start + Duration::from_secs(2));
    assert_ne!(epoch, review.epoch);
    assert!(shared
        .snapshot(start + Duration::from_secs(2))
        .events
        .is_empty());
    assert!(!shared.observe(2, 17, start + Duration::from_secs(1), &[0x91, 60, 100]));
    send(&shared, start, 2050, 2, 17, &[0x91, 60, 100]);
    shared.disconnect(17, start + Duration::from_millis(2500));
    let disconnected = shared.snapshot(start + Duration::from_secs(3));
    assert!(prepare(&disconnected, choice(), &AtomicBool::new(false))
        .unwrap_err()
        .contains("disconnect"));
    assert!(shared
        .configure(
            Config {
                enabled: true,
                seconds: 601,
                events: 256
            },
            start
        )
        .is_err());
    assert!(shared
        .configure(
            Config {
                enabled: true,
                seconds: 120,
                events: 65537
            },
            start
        )
        .is_err());
    shared
        .configure(Config::default(), start + Duration::from_secs(4))
        .unwrap();
    assert!(shared
        .snapshot(start + Duration::from_secs(4))
        .events
        .is_empty());
}
#[test]
fn original_input_timestamps_sort_delayed_sources_and_cancellation_overlap_and_short_loops_refuse_whole_review(
) {
    let (shared, start) = configured(256);
    send(&shared, start, 500, 2, 17, &[0x81, 60, 9]);
    send(&shared, start, 50, 2, 17, &[0x91, 60, 100]);
    send(&shared, start, 10, 3, 99, &[0x91, 70, 111]);
    let snapshot = shared.snapshot(start + Duration::from_secs(1));
    assert!(snapshot.events.windows(2).all(|p| p[0].at <= p[1].at));
    let prepared = prepare(&snapshot, choice(), &AtomicBool::new(false)).unwrap();
    assert_eq!(prepared.content.notes.len(), 1);
    assert_eq!(prepared.content.notes[0].pitch, 60);
    assert!(prepare(&snapshot, choice(), &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    let mut short = choice();
    short.loop_beats = 1.0;
    assert!(prepare(&snapshot, short, &AtomicBool::new(false))
        .unwrap_err()
        .contains("shorter"));
    send(&shared, start, 60, 2, 17, &[0x91, 60, 100]);
    assert!(prepare(
        &shared.snapshot(start + Duration::from_secs(1)),
        choice(),
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("paired safely"));
}
