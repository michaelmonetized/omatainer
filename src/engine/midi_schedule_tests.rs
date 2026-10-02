use super::*;
use midi_schedule::{Gate, MidiSchedule};

fn note(pitch: u8, start: f32, len: f32) -> MidiNote {
    MidiNote {
        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
        pitch,
        start,
        len,
        vel: 100,
    }
}

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(256);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.decks.iter_mut().for_each(|deck| deck.audio = None);
    rt
}

fn fixture(notes: Vec<MidiNote>) -> RtEngine {
    let mut rt = engine();
    rt.tracks[2].clips[0].bars = 1.0;
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes,
    });
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    rt
}

fn render(rt: &mut RtEngine, frames: usize, block: usize) {
    let mut output = vec![0.0; block * 2];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(block);
        rt.process(&mut output[..count * 2]);
        remaining -= count;
    }
}

#[test]
fn explicit_midi_region_chases_pickup_plays_intro_once_and_closes_loop_gates() {
    let mut muted = note(70, 4.5, 0.5);
    muted.muted = true;
    for block in [1, 127, 512] {
        let mut rt = engine();
        rt.apply(Command::SetNotes { track: 2, scene: 0, notes: vec![
            note(59, 0.0, 0.5), note(60, 1.0, 2.0), note(62, 3.0, 0.5),
            note(64, 4.0, 3.0), note(67, 6.5, 0.5), muted.clone(),
        ] });
        rt.tracks[2].clips[0].bars = 2.0;
        rt.tracks[2].clips[0].region = Some(midi_edit::Region {
            start: 2.0, end: 8.0, loop_start: 4.0, loop_end: 6.0, loop_enabled: true,
        });
        rt.tracks[2].midi_schedule.trace = Some(Vec::with_capacity(32));
        rt.apply(Command::LaunchClip { track: 2, scene: 0 });
        render(&mut rt, 144_001, block);
        assert_eq!(trace(&mut rt, 2), vec![
            (0, Gate::On(60, 100)), (24_000, Gate::Off(60)),
            (24_000, Gate::On(62, 100)), (36_000, Gate::Off(62)),
            (48_000, Gate::On(64, 100)), (96_000, Gate::Off(64)),
            (96_000, Gate::On(64, 100)), (144_000, Gate::Off(64)),
            (144_000, Gate::On(64, 100)),
        ], "block size {block}");
    }
}

#[test]
fn explicit_disabled_loop_plays_after_loop_end_and_stops_at_clip_end() {
    let mut rt = engine();
    rt.apply(Command::SetNotes { track: 2, scene: 0, notes: vec![
        note(60, 1.0, 2.0), note(64, 4.0, 3.0), note(67, 6.5, 3.0),
    ] });
    rt.tracks[2].clips[0].bars = 2.0;
    rt.tracks[2].clips[0].region = Some(midi_edit::Region {
        start: 2.0, end: 8.0, loop_start: 4.0, loop_end: 6.0, loop_enabled: false,
    });
    rt.tracks[2].midi_schedule.trace = Some(Vec::with_capacity(16));
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    render(&mut rt, 144_001, 257);
    assert_eq!(trace(&mut rt, 2), vec![
        (0, Gate::On(60, 100)), (24_000, Gate::Off(60)),
        (48_000, Gate::On(64, 100)), (108_000, Gate::On(67, 100)),
        (120_000, Gate::Off(64)), (144_000, Gate::Off(67)),
    ]);
    assert!(rt.tracks[2].playing.is_none());
}

#[test]
fn explicit_region_quantized_launch_retains_shared_transport_grid_with_distinct_clocks() {
    let mut rt = engine();
    rt.beat = 10.25;
    rt.sync_midi_clock();
    rt.midi_beat += 0.125;
    rt.playing = true;
    rt.quant = 1.0;
    rt.apply(Command::SetNotes {track:2, scene:7, notes:vec![note(60,0.0,0.5)]});
    rt.tracks[2].clips[7].region = Some(midi_edit::Region::full(1.0));
    rt.tracks[2].midi_schedule.trace = Some(Vec::with_capacity(8));
    rt.apply(Command::LaunchClip {track:2, scene:7});
    assert_eq!(rt.tracks[2].playing.unwrap().start_beat,11.0);
    assert_eq!(rt.recording_position(2,7),None);
    render(&mut rt,18_000,257);
    assert!(rt.tracks[2].midi_schedule.trace.as_ref().unwrap().is_empty());
    render(&mut rt,1,1);
    assert_eq!(trace(&mut rt,2),vec![(0,Gate::On(60,100))]);
}

fn reference(notes: &[MidiNote], loop_beats: f64, frames: usize) -> Vec<(usize, Gate)> {
    let mut events = Vec::new();
    for cycle in 0..=(frames as f64 / 24_000.0 / loop_beats).ceil() as usize {
        for (index, note) in notes.iter().enumerate() {
            let start = (note.start as f64).rem_euclid(loop_beats) + cycle as f64 * loop_beats;
            events.push((start, 1u8, index, Gate::On(note.pitch, note.vel)));
            events.push((start + note.len as f64, 0u8, index, Gate::Off(note.pitch)));
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut held = [0usize; 256];
    let mut trace = Vec::new();
    for (beat, _, _, gate) in events {
        let frame = (beat * 24_000.0 + 1e-7).floor() as usize;
        if frame >= frames {
            break;
        }
        match gate {
            Gate::On(pitch, _) => held[pitch as usize] += 1,
            Gate::Off(pitch) => {
                held[pitch as usize] -= 1;
                if held[pitch as usize] > 0 {
                    continue;
                }
            }
        }
        trace.push((frame, gate));
    }
    trace
}

fn trace(rt: &mut RtEngine, track: usize) -> Vec<(usize, Gate)> {
    rt.tracks[track]
        .midi_schedule
        .trace
        .take()
        .unwrap()
        .into_iter()
        .map(|(beat, gate)| ((beat * 24_000.0 - 1.0).round() as usize, gate))
        .collect()
}

#[test]
fn midi_default_scene_events_match_reference_at_multiple_block_sizes() {
    for block in [1, 127, 1024] {
        let mut rt = engine();
        rt.apply(Command::LaunchScene { scene: 0 });
        let frames = 216_001;
        let expected: Vec<_> = (0..4)
            .map(|track| {
                let clip = &rt.tracks[track].clips[0];
                reference(&clip.notes, clip.bars as f64 * 4.0, frames)
            })
            .collect();
        for track in &mut rt.tracks[..4] {
            track.midi_schedule.trace = Some(Vec::new());
        }
        render(&mut rt, frames, block);
        for (track, expected) in expected.into_iter().enumerate() {
            assert_eq!(
                trace(&mut rt, track),
                expected,
                "track {track}, block {block}"
            );
        }
    }
}

#[test]
fn midi_sparse_dense_overlapping_and_cross_loop_notes_match_reference() {
    let sparse = vec![note(60, 0.125, 0.1), note(64, 3.875, 0.25)];
    let dense: Vec<_> = (0..256)
        .map(|i| {
            note(
                36 + (i % 48) as u8,
                (i % 64) as f32 / 16.0,
                0.1 + (i % 8) as f32 * 0.071,
            )
        })
        .collect();
    let overlaps = vec![
        note(60, 0.0, 6.5),
        note(60, 1.0, 0.5),
        note(64, 0.5, 0.5),
        note(64, 1.0, 0.5),
    ];
    for notes in [sparse, dense, overlaps] {
        let frames = 240_001;
        let expected = reference(&notes, 4.0, frames);
        for block in [1, 257, 1024] {
            let mut rt = fixture(notes.clone());
            rt.tracks[2].midi_schedule.trace = Some(Vec::new());
            render(&mut rt, frames, block);
            assert_eq!(
                trace(&mut rt, 2),
                expected,
                "{} notes, block {block}",
                notes.len()
            );
        }
    }
}

#[test]
fn midi_same_pitch_overlap_holds_until_final_off_and_orders_off_before_on() {
    let mut rt = fixture(vec![
        note(60, 0.0, 1.0),
        note(60, 0.5, 1.0),
        note(60, 1.5, 0.5),
    ]);
    rt.tracks[2].midi_schedule.trace = Some(Vec::new());
    render(&mut rt, 24_001, 512);
    assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.note() == 60
        && voice.owner == dsp::VoiceOwner::Clip
        && matches!(voice.env.stage, 1..=3)));
    render(&mut rt, 24_000, 127);
    assert_eq!(
        trace(&mut rt, 2),
        vec![
            (0, Gate::On(60, 100)),
            (12_000, Gate::On(60, 100)),
            (36_000, Gate::Off(60)),
            (36_000, Gate::On(60, 100)),
            (48_000, Gate::Off(60)),
        ]
    );
}

#[test]
fn midi_edits_chase_active_notes_once_and_note_additions_keep_existing_gates() {
    let mut rt = fixture(vec![note(60, 0.0, 3.0)]);
    render(&mut rt, 12_000, 512);
    rt.tracks[2].midi_schedule.trace = Some(Vec::with_capacity(16));
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: vec![note(62, 0.0, 1.0), note(64, 0.75, 0.5)],
    });
    let mut one = [0.0; 2];
    let counts = test_alloc::measure(|| rt.process(&mut one));
    assert_eq!(
        counts,
        test_alloc::Counts::default(),
        "first post-edit render allocated"
    );
    render(&mut rt, 18_000, 257);
    assert_eq!(
        trace(&mut rt, 2),
        vec![
            (12_000, Gate::On(62, 100)),
            (18_000, Gate::On(64, 100)),
            (24_000, Gate::Off(62)),
            (30_000, Gate::Off(64)),
        ]
    );

    for compose in [false, true] {
        let mut rt = fixture(vec![note(60, 0.0, 3.0)]);
        render(&mut rt, 2400, 256);
        rt.tracks[2].midi_schedule.trace = Some(Vec::new());
        if compose {
            rt.apply(Command::ComposeArm { track: rt.selected_track, scene: rt.selected_scene });
            rt.apply(Command::SamplerPad { pad: 0, on: true });
        } else {
            rt.recording = true;
            rt.apply(Command::LiveNoteOn {
                source: 0,
                ch: 0,
                note: 65,
                vel: 100,
            });
        }
        render(&mut rt, 7000, 512);
        assert!(
            trace(&mut rt, 2).is_empty(),
            "capture created clip monitoring gates"
        );
        assert!(
            rt.tracks[2]
                .poly
                .voices
                .iter()
                .any(|voice| voice.owner == dsp::VoiceOwner::Clip
                    && voice.note() == 60
                    && matches!(voice.env.stage, 1..=3))
        );
        // Ordinary edits still chase their active notes; recording suppression
        // remains attached only to the captured index through later rebuilds.
        rt.tracks[2].midi_schedule.trace = Some(Vec::new());
        rt.tracks[2].clips[0].notes.push(note(73, 0.0, 1.0));
        let midi_beat = rt.precise_midi_beat();
        rt.tracks[2].clip_notes_changed(0, rt.beat, midi_beat);
        render(&mut rt, 1, 1);
        assert_eq!(trace(&mut rt, 2), [(9400, Gate::On(73, 100))]);
    }
}

#[test]
fn midi_multiple_edits_at_an_exact_boundary_preserve_pending_gates() {
    let mut rt = fixture(vec![note(60, 0.0, 0.5), note(60, 0.5, 0.5)]);
    render(&mut rt, 12_000, 256);
    rt.beat = 0.5;
    rt.tracks[2].midi_schedule.trace = Some(Vec::new());
    for pitch in [65, 67] {
        rt.tracks[2].clips[0].notes.push(note(pitch, 0.5, 0.25));
        let midi_beat = rt.precise_midi_beat();
        rt.tracks[2].clip_notes_changed(0, rt.beat, midi_beat);
    }
    render(&mut rt, 1, 1);
    assert_eq!(
        trace(&mut rt, 2),
        vec![
            (12_000, Gate::Off(60)),
            (12_000, Gate::On(60, 100)),
            (12_000, Gate::On(65, 100)),
            (12_000, Gate::On(67, 100)),
        ]
    );
}

#[test]
fn midi_pending_launch_waits_for_grid_and_one_shot_keeps_full_length() {
    let mut rt = engine();
    rt.tracks[2].clips[0].bars = 0.25;
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: vec![note(60, 0.0, 2.0), note(64, 0.5, 0.25)],
    });
    rt.playing = true;
    rt.beat = 0.5;
    rt.quant = 1.0;
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: false,
    });
    rt.tracks[2].midi_schedule.trace = Some(Vec::new());
    render(&mut rt, 12_000, 127);
    assert!(
        rt.tracks[2]
            .midi_schedule
            .trace
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert!(rt.tracks[2].playing.unwrap().last_beat < 0.0);
    render(&mut rt, 24_000, 257);
    assert!(
        rt.tracks[2].playing.is_some(),
        "one-shot ended before its full beat"
    );
    render(&mut rt, 1, 1);
    assert!(rt.tracks[2].playing.is_none());
    assert!(
        rt.tracks[2]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.owner == dsp::VoiceOwner::Clip)
            .all(|voice| voice.env.stage == 0 || voice.env.stage == 4)
    );
    let on_count = rt.tracks[2]
        .midi_schedule
        .trace
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(_, gate)| matches!(gate, Gate::On(..)))
        .count();
    assert_eq!(on_count, 2, "one-shot restarted at its loop boundary");
}

#[test]
fn midi_invalid_ranges_do_not_schedule_or_spin() {
    let mut schedule = MidiSchedule::default();
    schedule.rebuild(
        &[
            note(60, 0.0, 0.0),
            note(61, 0.0, -1.0),
            note(62, f32::NAN, 1.0),
            note(63, f32::INFINITY, 1.0),
            note(64, 0.0, f32::INFINITY),
            note(65, 0.0, f32::NAN),
        ],
        4.0,
        None,
        true,
        &[],
    );
    assert_eq!(schedule.next_due(100.0, true), None);
    assert_eq!(schedule.events_visited, 0);
    schedule.rebuild(&[note(255, -1.0, 0.25)], 4.0, None, true, &[]);
    assert_eq!(schedule.next_due(f64::INFINITY, true), None);
    assert_eq!(schedule.next_due(f64::NAN, true), None);
    assert_eq!(schedule.next_due(3.001, true), Some(Gate::On(255, 100)));
    assert_eq!(schedule.next_due(3.251, true), Some(Gate::Off(255)));
}

#[test]
fn midi_warmed_renderer_allocates_nothing_and_does_no_note_work_between_events() {
    let mut rt = engine();
    rt.apply(Command::LaunchScene { scene: 0 });
    let mut output = [0.0; 2048];
    rt.process(&mut output);
    let counts = test_alloc::measure(|| rt.process(&mut output));
    assert_eq!(counts, test_alloc::Counts::default());
    println!("four-clip 1024-frame callback without snapshot: {counts:?}");
    for note_count in [1, 4096] {
        let mut rt = fixture(
            (0..note_count)
                .map(|i| note(36 + (i % 48) as u8, 3.0, 0.25))
                .collect(),
        );
        let rebuilds = rt.tracks[2].midi_schedule.rebuilds;
        let start = Instant::now();
        let counts = test_alloc::measure(|| {
            for _ in 0..24_000 {
                rt.beat += 1.0 / 24_000.0;
                std::hint::black_box(rt.render_track(2, false));
            }
        });
        let elapsed = start.elapsed();
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(rt.tracks[2].midi_schedule.rebuilds, rebuilds);
        assert_eq!(rt.tracks[2].midi_schedule.events_visited, 0);
        println!("{note_count} notes between boundaries: {counts:?}, local elapsed {elapsed:?}");
        let counts = test_alloc::measure(|| {
            for _ in 0..192_001 {
                rt.beat += 1.0 / 24_000.0;
                std::hint::black_box(rt.render_track(2, false));
            }
        });
        assert_eq!(
            counts,
            test_alloc::Counts::default(),
            "loop or dense boundaries allocated"
        );
        assert!(rt.tracks[2].midi_schedule.events_visited > note_count * 2);
    }
}
