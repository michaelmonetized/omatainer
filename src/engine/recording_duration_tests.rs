use super::*;

fn fixture(playing: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(256);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.selected_track = 1;
    rt.selected_scene = 2;
    rt.apply(Command::ComposeArm { track: 1, scene: 2 });
    if playing {
        rt.apply(Command::LaunchClip { track: 1, scene: 2 });
        rt.apply(Command::Record);
    }
    rt
}

fn render(rt: &mut RtEngine, mut frames: usize) {
    let mut output = [0.0; 512];
    while frames > 0 {
        let count = frames.min(256);
        rt.process(&mut output[..count * 2]);
        frames -= count;
    }
}

fn on(rt: &mut RtEngine, source: u64, ch: u8, note: u8, vel: u8) {
    rt.apply(Command::LiveNoteOn {
        source,
        ch,
        note,
        vel,
    });
}

fn off(rt: &mut RtEngine, source: u64, ch: u8, note: u8) {
    rt.apply(Command::LiveNoteOff { source, ch, note });
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

#[test]
fn recording_short_long_and_overlapping_equal_pitches_keep_each_input_timing() {
    let mut rt = fixture(true);
    on(&mut rt, 1, 0, 60, 31);
    render(&mut rt, 1_200);
    on(&mut rt, 2, 0, 60, 95);
    on(&mut rt, 1, 1, 60, 65);
    render(&mut rt, 600);
    off(&mut rt, 1, 0, 60);
    render(&mut rt, 600);
    on(&mut rt, 1, 0, 60, 87);
    render(&mut rt, 4_800);
    off(&mut rt, 2, 0, 60);
    render(&mut rt, 12_000);
    off(&mut rt, 1, 0, 60);
    off(&mut rt, 1, 1, 60);
    let notes = &rt.tracks[1].clips[2].notes;
    assert_eq!(notes.len(), 4);
    for (note, (start, len, vel)) in notes.iter().zip([
        (0.0, 0.075, 31),
        (0.05, 0.25, 95),
        (0.05, 0.75, 65),
        (0.1, 0.7, 87),
    ]) {
        assert_eq!((note.pitch, note.vel), (60, vel));
        near(note.start, start);
        near(note.len, len);
    }
}

#[test]
fn recording_retrigger_and_zero_velocity_finalize_exact_gate_only_once() {
    let mut rt = fixture(true);
    on(&mut rt, 9, 3, 70, 77);
    render(&mut rt, 9_000);
    on(&mut rt, 9, 3, 70, 123);
    render(&mut rt, 12_000);
    // MIDI's alternative note-off spelling is also accepted directly.
    on(&mut rt, 9, 3, 70, 0);
    render(&mut rt, 24_000);
    off(&mut rt, 9, 3, 70);
    off(&mut rt, 9, 4, 70);
    let notes = &rt.tracks[1].clips[2].notes;
    assert_eq!(notes.len(), 2);
    near(notes[0].start, 0.0);
    near(notes[0].len, 0.375);
    near(notes[1].start, 0.375);
    near(notes[1].len, 0.5);
    assert_eq!((notes[0].vel, notes[1].vel), (77, 123));
}

#[test]
fn recording_release_retains_original_clip_and_pad_pitch_after_control_changes() {
    let mut rt = fixture(true);
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    rt.apply(Command::SamplerPad { pad: 3, on: true });
    let pitch = rt.tracks[1].clips[2].notes[0].pitch;
    on(&mut rt, 4, 0, 81, 99);
    render(&mut rt, 6_000);
    rt.apply(Command::Select { track: 2, scene: 4 });
    rt.apply(Command::SamplerOct(1));
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Pad)));
    render(&mut rt, 12_000);
    rt.apply(Command::SamplerPad { pad: 3, on: false });
    off(&mut rt, 4, 0, 81);
    let notes = &rt.tracks[1].clips[2].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!((notes[0].pitch, notes[0].vel), (pitch, 110));
    assert_eq!((notes[1].pitch, notes[1].vel), (81, 99));
    for note in notes {
        near(note.len, 0.75);
    }
    assert!(rt.tracks[2].clips[4].notes.is_empty());
}

#[test]
fn stopped_compose_records_audio_time_and_integrates_tempo_and_sample_rate_changes() {
    let mut rt = fixture(false);
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    assert_eq!(
        rt.tracks[1].clips[2].notes.len(),
        1,
        "compose onset is visible"
    );
    render(&mut rt, 12_000); // 0.5 beats at 120 BPM / 48 kHz.
    rt.apply(Command::SetBpm(60.0));
    render(&mut rt, 12_000); // 0.25 beats at 60 BPM / 48 kHz.
    rt.set_sample_rate(44_100).unwrap();
    render(&mut rt, 22_050); // 0.5 beats at 60 BPM / 44.1 kHz.
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    assert!(!rt.playing);
    assert_eq!(rt.beat, 0.0);
    let note = &rt.tracks[1].clips[2].notes[0];
    assert_eq!(note.start, 0.0);
    near(note.len, 1.25);
}

#[test]
fn recording_stop_and_take_boundaries_finalize_once_before_later_physical_release() {
    for boundary in [
        Command::Stop,
        Command::Record,
        Command::StopTrack { track: 1 },
        Command::ToggleScene { scene: 2 },
        Command::LaunchClip { track: 1, scene: 0 },
        Command::RestartScene { scene: 2 },
        // Scene 3 has no clip on track 1, which still stops that track.
        Command::LaunchScene { scene: 3 },
    ] {
        let mut rt = fixture(true);
        on(&mut rt, 1, 0, 60, 100);
        render(&mut rt, 12_000);
        rt.apply(boundary);
        near(rt.tracks[1].clips[2].notes[0].len, 0.5);
        render(&mut rt, 12_000);
        off(&mut rt, 1, 0, 60);
        near(rt.tracks[1].clips[2].notes[0].len, 0.5);
    }
}

#[test]
fn recording_note_list_replacement_cancels_only_that_clip_capture() {
    let mut rt = fixture(true);
    on(&mut rt, 1, 0, 60, 100);
    rt.apply(Command::Select { track: 2, scene: 4 });
    on(&mut rt, 2, 0, 62, 90);
    render(&mut rt, 12_000);
    rt.apply(Command::SetNotes {
        track: 1,
        scene: 2,
        notes: vec![MidiNote {
            pitch: 74,
            start: 1.0,
            len: 3.0,
            vel: 45,
        }],
    });
    render(&mut rt, 6_000);
    off(&mut rt, 1, 0, 60);
    off(&mut rt, 2, 0, 62);
    let replacement = &rt.tracks[1].clips[2].notes[0];
    assert_eq!(
        (
            replacement.pitch,
            replacement.start,
            replacement.len,
            replacement.vel
        ),
        (74, 1.0, 3.0, 45)
    );
    near(rt.tracks[2].clips[4].notes[0].len, 0.75);
}

#[test]
fn recording_instantaneous_and_pending_launch_gates_have_explicit_boundaries() {
    let mut rt = fixture(true);
    on(&mut rt, 1, 0, 60, 100);
    off(&mut rt, 1, 0, 60);
    near(rt.tracks[1].clips[2].notes[0].len, 1.0 / 24_000.0);
    render(&mut rt, 6_000);
    rt.quant = 4.0;
    rt.apply(Command::LaunchClip { track: 1, scene: 2 });
    on(&mut rt, 2, 0, 62, 90);
    render(&mut rt, 96_000);
    off(&mut rt, 2, 0, 62);
    assert_eq!(
        rt.tracks[1].clips[2].notes.len(),
        1,
        "pending onset is monitor only, even if release follows launch"
    );
}

#[test]
fn recorded_holds_across_loop_seams_replay_at_the_synthetic_event_times() {
    use midi_schedule::Gate;

    let mut rt = fixture(true);
    on(&mut rt, 1, 0, 60, 40);
    render(&mut rt, 6_000); // 0.25
    on(&mut rt, 2, 0, 60, 90);
    render(&mut rt, 12_000); // 0.75
    off(&mut rt, 1, 0, 60);
    render(&mut rt, 12_000); // 1.25
    off(&mut rt, 2, 0, 60);
    render(&mut rt, 54_000); // 3.5
    on(&mut rt, 1, 0, 67, 75);
    render(&mut rt, 6_000); // 3.75
    on(&mut rt, 1, 0, 64, 103);
    render(&mut rt, 12_000); // 4.25: one seam crossed
    off(&mut rt, 1, 0, 64);
    render(&mut rt, 138_000); // 10: two seams crossed
    off(&mut rt, 1, 0, 67);
    rt.apply(Command::Stop);
    let notes = rt.tracks[1].clips[2].notes.clone();
    assert_eq!(notes.len(), 4);
    for (note, (pitch, start, len, vel)) in notes.iter().zip([
        (60, 0.0, 0.75, 40),
        (60, 0.25, 1.0, 90),
        (67, 3.5, 6.5, 75),
        (64, 3.75, 0.5, 103),
    ]) {
        assert_eq!((note.pitch, note.vel), (pitch, vel));
        near(note.start, start);
        near(note.len, len);
    }

    // Exercise the real renderer/scheduler with the captured data. Expected
    // events come directly from the synthetic physical gate schedule above.
    // Equal-pitch overlap retriggers on each onset and releases on its final
    // off. The 6.5-beat hold overlaps its next four-beat-loop onset.
    for sample_rate in [44_100, 48_000] {
        let mut playback = fixture(true);
        playback.set_sample_rate(sample_rate).unwrap();
        playback.apply(Command::SetNotes {
            track: 1,
            scene: 2,
            notes: notes.clone(),
        });
        playback.tracks[1].midi_schedule.trace = Some(Vec::new());
        let frames_per_beat = sample_rate as f64 / 2.0;
        render(&mut playback, (10.5 * frames_per_beat) as usize + 1);
        let actual: Vec<_> = playback.tracks[1]
            .midi_schedule
            .trace
            .take()
            .unwrap()
            .into_iter()
            .map(|(beat, gate)| ((beat * frames_per_beat - 1.0).round() as usize, gate))
            .collect();
        let expected: Vec<_> = [
            (0.0, Gate::On(60, 40)),
            (0.25, Gate::On(60, 90)),
            (1.25, Gate::Off(60)),
            (3.5, Gate::On(67, 75)),
            (3.75, Gate::On(64, 103)),
            (4.0, Gate::On(60, 40)),
            (4.25, Gate::Off(64)),
            (4.25, Gate::On(60, 90)),
            (5.25, Gate::Off(60)),
            (7.5, Gate::On(67, 75)),
            (7.75, Gate::On(64, 103)),
            (8.0, Gate::On(60, 40)),
            (8.25, Gate::Off(64)),
            (8.25, Gate::On(60, 90)),
            (9.25, Gate::Off(60)),
        ]
        .into_iter()
        .map(|(beat, gate)| {
            (
                ((beat * frames_per_beat) as f64 + 1e-7).floor() as usize,
                gate,
            )
        })
        .collect();
        assert_eq!(actual, expected, "sample rate {sample_rate}");
        assert!(playback.tracks[1]
            .poly
            .voices
            .iter()
            .any(|voice| voice.note() == 67
                && voice.owner == dsp::VoiceOwner::Clip
                && matches!(voice.env.stage, 1..=3)));
    }
}

#[test]
fn recording_single_shot_completion_finalizes_held_capture() {
    let mut rt = fixture(true);
    rt.apply(Command::FireClip {
        track: 1,
        scene: 2,
        looping: false,
    });
    on(&mut rt, 1, 0, 60, 100);
    render(&mut rt, 96_002);
    assert!(rt.tracks[1].playing.is_none());
    let duration = rt.tracks[1].clips[2].notes[0].len;
    assert!((duration - 4.0).abs() <= 1.0 / 24_000.0 + 1e-6);
    render(&mut rt, 12_000);
    off(&mut rt, 1, 0, 60);
    assert_eq!(rt.tracks[1].clips[2].notes[0].len, duration);
}
