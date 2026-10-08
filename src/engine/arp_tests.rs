use super::*;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn note(pitch: u8, start: f32, len: f32) -> MidiNote {
    MidiNote { variation: None,
        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
        pitch,
        start,
        len,
        vel: 100,
    }
}

fn fixture(notes: Vec<MidiNote>) -> RtEngine {
    let mut rt = engine();
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.tracks[2].clips[0].bars = 1.0;
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes,
    });
    rt.tracks[2]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    rt
}

fn render_at(rt: &mut RtEngine, beat: f64) -> Option<u8> {
    rt.beat = beat + 2.0 * midi_schedule::BEAT_EPSILON;
    rt.render_track(2, false);
    rt.tracks[2].arp_note
}

fn assert_released(rt: &RtEngine, pitch: u8) {
    assert!(
        rt.tracks[2]
            .poly
            .voices
            .iter()
            .filter(|v| v.note() == pitch)
            .all(|v| v.env.stage == 0 || v.env.stage == 4),
        "pitch {pitch} stayed held"
    );
}

#[test]
fn explicit_region_arp_intro_and_muted_notes_do_not_return_in_repeating_chord() {
    let mut muted = note(70, 4.0, 4.0); muted.muted = true;
    let mut rt = fixture(vec![note(60, 1.0, 7.0), note(64, 4.0, 4.0), muted]);
    rt.apply(Command::StopTrack { track: 2 });
    rt.tracks[2].clips[0].region = Some(midi_edit::Region {
        start: 2.0, end: 8.0, loop_start: 4.0, loop_end: 6.0, loop_enabled: true,
    });
    rt.tracks[2].clips[0].bars = 2.0;
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    assert_eq!(render_at(&mut rt, 0.0), Some(60));
    render_at(&mut rt, 2.0);
    assert!(rt.tracks[2].arp_cache.contains(60));
    assert!(rt.tracks[2].arp_cache.contains(64));
    assert!(!rt.tracks[2].arp_cache.contains(70));
    render_at(&mut rt, 3.75);
    assert_eq!(render_at(&mut rt, 4.0), Some(64));
    assert!(!rt.tracks[2].arp_cache.contains(60));
    assert_released(&rt, 60);
    assert_eq!(render_at(&mut rt, 6.0), Some(64));
}

#[test]
fn arp_chord_cache_rebuilds_at_membership_boundaries_not_steps() {
    let mut rt = fixture(vec![
        note(67, 0.0, 2.0),
        note(60, 0.0, 2.0),
        note(64, 0.0, 2.0),
    ]);
    assert_eq!(render_at(&mut rt, 0.0), Some(60));
    for frame in 1..48_000 {
        rt.beat = frame as f64 / 24_000.0;
        rt.render_track(2, false);
    }
    assert_eq!(
        rt.tracks[2].arp_cache.rebuilds, 1,
        "held chord was rebuilt between boundaries"
    );
    assert_eq!(render_at(&mut rt, 2.0), None);
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 2);
    assert_eq!(render_at(&mut rt, 3.5), None);
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 2);
    assert_eq!(render_at(&mut rt, 4.0), Some(60));
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 3);
}

#[test]
fn arp_emitted_sequence_deduplicates_and_releases_rests_across_loops() {
    let mut rt = fixture(vec![
        note(67, 0.0, 1.0),
        note(60, 0.0, 1.0),
        note(64, 0.0, 1.0),
        note(60, 0.0, 0.6),
        note(69, 2.0, 0.3),
        note(62, 2.0, 0.3),
    ]);
    for cycle in 0..3 {
        let offset = cycle as f64 * 4.0;
        for (beat, expected) in [
            (0.0, Some(60)),
            (0.25, Some(64)),
            (0.5, Some(67)),
            (0.75, Some(60)),
            (1.0, None),
            (1.5, None),
            (2.0, Some(62)),
            (2.25, Some(69)),
            (2.31, None),
            (3.75, None),
        ] {
            assert_eq!(
                render_at(&mut rt, offset + beat),
                expected,
                "at beat {}",
                offset + beat
            );
            if let Some(pitch) = expected {
                assert!(rt.tracks[2]
                    .poly
                    .voices
                    .iter()
                    .any(|v| v.note() == pitch && v.env.stage > 0 && v.env.stage < 4));
            } else {
                for pitch in [60, 62, 64, 67, 69] {
                    assert_released(&rt, pitch);
                }
            }
        }
    }
}

#[test]
fn arp_existing_chord_fixture_matches_reference_steps_for_three_loops() {
    let mut rt = engine();
    rt.tracks[2]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    for step in 0..96 {
        // The built-in fixture holds C/E/G at 0..0.45 and D/F/A at 4..4.45.
        let expected = match step % 32 {
            0 => Some(60),
            1 => Some(64),
            16 => Some(65),
            17 => Some(69),
            _ => None,
        };
        assert_eq!(
            render_at(&mut rt, step as f64 / 4.0),
            expected,
            "at step {step}"
        );
    }
}

#[test]
fn arp_set_notes_between_steps_invalidates_only_the_playing_scene() {
    let mut rt = fixture(vec![
        note(60, 0.0, 4.0),
        note(64, 0.0, 4.0),
        note(67, 0.0, 4.0),
    ]);
    assert_eq!(render_at(&mut rt, 0.0), Some(60));
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 1,
        notes: vec![note(80, 0.0, 4.0)],
    });
    assert_eq!(render_at(&mut rt, 0.05), Some(60));
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 1);
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: vec![note(69, 0.0, 4.0), note(62, 0.0, 4.0), note(65, 0.0, 4.0)],
    });
    let mut emitted = Some(0);
    let counts = super::test_alloc::measure(|| emitted = render_at(&mut rt, 0.1));
    assert_eq!(counts, super::test_alloc::Counts::default());
    assert_eq!(
        emitted, None,
        "edit must release a removed pitch without a mid-step retrigger"
    );
    assert_released(&rt, 60);
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 2);
    assert_eq!(render_at(&mut rt, 0.25), Some(65));
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: vec![],
    });
    assert_eq!(render_at(&mut rt, 0.3), None);
    assert_released(&rt, 65);
}

#[test]
fn arp_record_and_compose_additions_preserve_existing_chord_while_monitored() {
    for compose in [false, true] {
        let mut rt = fixture(vec![
            note(60, 0.0, 4.0),
            note(64, 0.0, 4.0),
            note(67, 0.0, 4.0),
        ]);
        render_at(&mut rt, 0.0);
        rt.beat = 0.1;
        if compose {
            rt.apply(Command::ComposeArm { track: rt.selected_track, scene: rt.selected_scene });
            // Sample pad zero writes pitch 36, before the held chord's notes.
            rt.apply(Command::SamplerPad { pad: 0, on: true });
        } else {
            rt.recording = true;
            rt.apply(Command::LiveNoteOn {
                source: 0,
                ch: 0,
                note: 59,
                vel: 100,
            });
        }
        assert_eq!(render_at(&mut rt, 0.10001), Some(60));
        assert_eq!(rt.tracks[2].arp_cache.rebuilds, 2);
        assert!(
            rt.tracks[2].poly.voices.iter().any(|voice| {
                voice.note() == 60
                    && voice.owner == dsp::VoiceOwner::Clip
                    && matches!(voice.env.stage, 1..=3)
            }),
            "adding a note released the current arpeggiator gate"
        );
        assert_eq!(
            render_at(&mut rt, 0.25),
            Some(64),
            "captured input must not become a second arpeggiated monitor"
        );
        // Unrelated chord notes continue their normal step order.
        assert_eq!(render_at(&mut rt, 0.5), Some(67));
    }
}

#[test]
fn arp_toggle_releases_current_gate_and_refreshes_on_reenable() {
    let mut rt = fixture(vec![note(60, 0.0, 4.0), note(64, 0.0, 4.0)]);
    assert_eq!(render_at(&mut rt, 0.0), Some(60));
    render_at(&mut rt, 0.05);
    rt.fx_view = 2;
    rt.apply(Command::FxToggle(0));
    assert_eq!(render_at(&mut rt, 0.1), None);
    // Returning to ordinary playback restores the currently held chord.
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.note() == 60 && matches!(v.env.stage, 1..=3)));
    rt.apply(Command::FxToggle(0));
    assert_eq!(render_at(&mut rt, 0.15), None);
    assert_eq!(rt.tracks[2].arp_cache.rebuilds, 2);
    assert_eq!(render_at(&mut rt, 0.25), Some(64));
}

#[test]
fn arp_mute_and_solo_across_wrap_refresh_the_current_chord() {
    for mute in [false, true] {
        for starts_with_rest in [false, true] {
            let start = if starts_with_rest { 0.0 } else { 2.0 };
            let mut rt = fixture(vec![note(60, start, 1.0)]);
            assert_eq!(render_at(&mut rt, 2.5), (!starts_with_rest).then_some(60));
            rt.tracks[2].mute = mute;
            for beat in [3.5, 4.25] {
                rt.beat = beat;
                rt.render_track(2, !mute);
            }
            rt.tracks[2].mute = false;
            assert_eq!(render_at(&mut rt, 4.5), starts_with_rest.then_some(60));
            if !starts_with_rest {
                assert_released(&rt, 60);
            }
        }
    }
}

#[test]
fn arp_renderer_and_full_process_block_allocate_nothing() {
    let control = super::test_alloc::measure(|| drop(std::hint::black_box(vec![0u8; 17])));
    assert_eq!(
        control,
        super::test_alloc::Counts {
            allocations: 1,
            frees: 1,
            bytes: 17
        }
    );
    let mut rt = engine();
    rt.bpm = 120.0;
    rt.tracks[2]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    let mut out = [0.0; 2048];
    rt.process(&mut out);
    // This matches the 1024-frame audit fixture. Snapshot publication, which
    // separately allocates at 8 Hz, does not fall inside this block.
    assert!(rt.frames_done + 1024 < rt.sr as u64 / 8);
    let block = super::test_alloc::measure(|| rt.process(&mut out));
    assert_eq!(block, super::test_alloc::Counts::default());

    let before = rt.tracks[2].arp_cache.rebuilds;
    let started = Instant::now();
    // Three complete eight-beat loops through the real MIDI/poly/EQ/FX path,
    // including chord starts, rests and wraparounds, without UI publication.
    let render = super::test_alloc::measure(|| {
        for _ in 0..576_000 {
            rt.beat += 1.0 / 24_000.0;
            let _ = std::hint::black_box(rt.render_track(2, false));
        }
    });
    let elapsed = started.elapsed();
    assert_eq!(render, super::test_alloc::Counts::default());
    assert!(
        rt.tracks[2].arp_cache.rebuilds > before + 10,
        "measurement did not cross note and loop boundaries"
    );
    println!("arp 1024-frame process: {block:?}; 576000-frame renderer: {render:?}; local elapsed {elapsed:?}");
}
