use super::model::*;
use super::*;

fn rt() -> RtEngine {
    let (_, receiver) = crossbeam_channel::bounded(4);
    RtEngine::new(48000.0, receiver, Arc::new(Mutex::new(Snapshot::default())))
}
fn captured(rt: &RtEngine) -> Captured {
    let mut frame = capture::Frame::new();
    for _ in 0..4 {
        frame.capture(rt);
        assert!(frame.error.is_none(), "{:?}", frame.error);
        if frame.complete {
            frame.state.deduplicate(&mut frame.media);
            frame.state.validate(&frame.media).unwrap();
            return Captured {
                checkpoint: frame.checkpoint,
                state: frame.state,
                media: frame.media,
                revision: frame.revision,
                playback_receipts: frame.playback_receipts,
            };
        }
        frame.prepare();
    }
    panic!("capture did not complete");
}
fn populated() -> RtEngine {
    let mut rt = rt();
    for t in 0..TRACKS {
        rt.tracks[t].name = format!("track {t} – 演奏");
        rt.tracks[t].gain = 0.1 + t as f32 * 0.1;
        rt.tracks[t].pan = t as f32 * 0.1 - 0.4;
        rt.tracks[t].scene_bus = t;
        rt.tracks[t].fx.slots = fx::FxId::all()
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut slot = fx::FxSlot::new(*id, rt.sr);
                slot.mix = 0.25;
                slot.on = i % 2 == 0;
                slot.p = [0.2, 0.3, 0.4, 0.5];
                slot
            })
            .collect();
        for s in 0..SCENES {
            rt.tracks[t].clips[s] = Clip {
                kind: if s == 7 {
                    ClipKind::Audio
                } else {
                    ClipKind::Midi
                },
                name: format!("clip {t}:{s}"),
                bars: 2.0,
                gain: 0.7,
                notes: vec![MidiNote {
                    pitch: (30 + t + s) as u8,
                    start: 1.125,
                    len: 0.75,
                    vel: 93,
                }],
                audio: (s == 7).then(|| rt.builtin[0].as_ref().unwrap().clone()),
            };
        }
        rt.scene_fx[t]
            .slots
            .push(fx::FxSlot::new(fx::FxId::Delay, rt.sr));
    }
    rt.decks[0].hotcues[3] = HotCue {
        set: true,
        pos: 12345.125,
    };
    rt.decks[0].pos = 1987.5;
    rt.decks[0].cue_pos = 123.75;
    rt.decks[1].loop_on = true;
    rt.decks[1].loop_start = 321.125;
    rt.decks[1].loop_len = 4321.5;
    rt.decks[1].eq_cut[1] = true;
    rt.decks[1].eq_store[1] = 1.75;
    rt.sampler_bank = 2;
    rt.sampler_inst = SamplerInstrument::Synth(SynthInstrument::Pad);
    rt.sampler_oct = 5;
    rt.sampler_poly.select_instrument(SynthInstrument::Pad);
    rt.fx_wet = [0.1, 0.2, 0.3];
    rt.selected_track = 7;
    rt.selected_scene = 6;
    rt.selected_deck = 1;
    rt.view = View::Compose;
    rt.fx_view = 103;
    rt
}

#[test]
fn full_state_roundtrip_preserves_every_attachment_rack_note_and_control() {
    let original = populated();
    let saved = captured(&original);
    let expected = serde_json::to_value(&saved.state).unwrap();
    assert!(saved.media.len() < MAX_MEDIA_REFS);
    let bytes: usize = saved.media.iter().map(|m| m.data.len()).sum();
    let restored = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    let again = captured(&restored.rt);
    assert_eq!(serde_json::to_value(&again.state).unwrap(), expected);
    assert_eq!(
        again.media.iter().map(|m| m.data.len()).sum::<usize>(),
        bytes
    );
    assert!(!restored.rt.playing && restored.rt.decks.iter().all(|d| !d.playing && !d.touching));
    assert!(again
        .playback_receipts
        .iter()
        .all(|r| r.as_ref().unwrap().state() == load_receipt::State::Current));
}

#[test]
fn capture_and_install_boundary_allocate_and_free_nothing() {
    let mut live = populated();
    let mut frame = capture::Frame::new();
    let needed = super::super::test_alloc::measure(|| frame.capture(&live));
    assert_eq!(needed, super::super::test_alloc::Counts::default());
    assert!(!frame.complete);
    frame.prepare();
    let copied = super::super::test_alloc::measure(|| frame.capture(&live));
    assert_eq!(copied, super::super::test_alloc::Counts::default());
    assert!(frame.complete);
    frame.state.deduplicate(&mut frame.media);
    let mut replacement = Prepared::from_state(frame.state, frame.media, 48000).unwrap();
    let swaps = super::super::test_alloc::measure(|| replacement.swap_into(&mut live));
    assert_eq!(swaps, super::super::test_alloc::Counts::default());
    // Replaced media/strings/racks still belong to replacement until this test
    // (the worker equivalent) drops them, never to a temporary on audio.
}

#[test]
fn validation_rejects_new_schema_bad_floats_indices_notes_and_limits() {
    let saved = captured(&rt());
    let mut state = saved.state.clone();
    state.version += 1;
    assert!(state
        .validate(&saved.media)
        .unwrap_err()
        .contains("version"));
    let mut state = saved.state.clone();
    state.bpm = f32::NAN;
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone();
    state.tracks[0].drums[0] = saved.media.len();
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone();
    state.decks[0].hotcues[0] = Some(f64::INFINITY);
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone();
    state.tracks[0].clips[0].notes[0].pitch = 255;
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone();
    state.tracks[0].name = "x".repeat(MAX_TEXT_BYTES + 1);
    assert!(state.validate(&saved.media).is_err());
    let mut json = serde_json::to_value(&saved.state).unwrap();
    json["future_automation"] = serde_json::json!([1, 2]);
    assert!(serde_json::from_value::<State>(json).is_err());
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "worker stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn drive<T: Send + 'static>(rt: &mut RtEngine, work: impl FnOnce() -> T + Send + 'static) -> T {
    let worker = std::thread::spawn(work);
    wait_until(|| {
        rt.process(&mut []);
        worker.is_finished()
    });
    worker.join().unwrap()
}

#[test]
fn real_worker_handoff_captures_and_conflicts_after_older_queued_edits() {
    let (engine, mut live) = Engine::headless_for_test(48000, 256);
    let handle = engine.project.clone();
    let saved = drive(&mut live, move || handle.capture(&AtomicBool::new(false))).unwrap();
    let before = saved.revision;
    let prepared = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    engine
        .send(Command::TrackGain {
            track: 3,
            value: 0.23,
        })
        .unwrap();
    let handle = engine.project.clone();
    let rejected = drive(&mut live, move || {
        handle.install(prepared, before, &AtomicBool::new(false))
    });
    assert!(matches!(rejected, Err(Error::Conflict)));
    assert_eq!(live.tracks[3].gain, 0.23);
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    let applied = drive(&mut live, move || {
        handle.install(
            Prepared::empty(48000).unwrap(),
            revision,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    assert!(applied.revision > revision);
    assert!(live
        .tracks
        .iter()
        .all(|t| t.clips.iter().all(|c| !c.occupied())));
    assert!(live.decks.iter().all(|d| d.audio.is_none()));
}

#[test]
fn cancellation_and_busy_handoffs_cannot_steal_results_or_replace_project() {
    let mut live = rt();
    let handle = live.project.clone();
    let revision = handle.revision();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = cancel.clone();
    let worker_handle = handle.clone();
    let worker = std::thread::spawn(move || {
        worker_handle.install(Prepared::empty(48000).unwrap(), revision, &cancel_worker)
    });
    wait_until(|| !handle.shared.incoming.is_empty());
    assert!(matches!(
        handle.capture(&AtomicBool::new(false)),
        Err(Error::Busy)
    ));
    cancel.store(true, Ordering::Release);
    wait_until(|| worker.is_finished());
    assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
    live.process(&mut []); // Retire the cancelled payload; it must not install.
    assert_eq!(handle.revision(), revision);
    assert!(live.tracks[0].clips[0].occupied());
    let next = handle.clone();
    assert!(drive(&mut live, move || next.capture(&AtomicBool::new(false))).is_ok());
}

#[test]
fn opening_resets_physical_gates_and_resumes_saved_clips_only_on_play() {
    let mut original = rt();
    original.apply(Command::LaunchScene { scene: 0 });
    original.process(&mut [0.0; 1024]);
    let saved = captured(&original);
    let expected = serde_json::to_value(&saved.state).unwrap();
    let mut prepared = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    let mut live = rt();
    live.apply(Command::Select { track: 2, scene: 0 });
    live.apply(Command::LiveNoteOn {
        source: 99,
        ch: 0,
        note: 64,
        vel: 100,
    });
    live.apply(Command::DeckTouch { deck: 0, on: true });
    prepared.swap_into(&mut live);
    let mut out = [0.0; 1024];
    live.process(&mut out);
    assert!(
        out.iter().all(|v| *v == 0.0),
        "opened project emitted audio before Play"
    );
    assert_eq!(
        serde_json::to_value(&captured(&live).state).unwrap(),
        expected
    );
    live.apply(Command::Play);
    live.process(&mut out);
    assert!(out.iter().any(|v| v.abs() > 0.0001));
    assert!(live.tracks.iter().any(|t| t.playing.is_some()));
    live.apply(Command::LiveNoteOff {
        source: 99,
        ch: 0,
        note: 64,
    });
    live.apply(Command::DeckTouch { deck: 0, on: false });
    live.process(&mut out);
    assert!(
        out.iter().any(|v| v.abs() > 0.0001),
        "old physical release silenced restored clip"
    );
}

#[test]
fn canonical_reopen_render_matches_reference_across_block_sizes_and_output_rates() {
    let source = populated();
    for sr in [44100, 48000, 96000] {
        let a = captured(&source);
        let b = captured(&source);
        let mut first = Prepared::from_state(a.state, a.media, sr).unwrap();
        // Exercise the actual serde state boundary, not a clone-only oracle.
        let encoded = serde_json::to_vec(&b.state).unwrap();
        let decoded: State = serde_json::from_slice(&encoded).unwrap();
        let mut second = Prepared::from_state(decoded, b.media, sr).unwrap();
        for live in [&mut first.rt, &mut second.rt] {
            live.apply(Command::LaunchScene { scene: 0 });
            live.decks[0].playing = true;
            live.decks[1].playing = true;
        }
        let mut expected = vec![0.0; 16384];
        let mut actual = vec![0.0; 16384];
        first.rt.process(&mut expected);
        for block in actual.chunks_mut(254) {
            second.rt.process(block);
        }
        assert_eq!(
            expected, actual,
            "reopened reference render changed at {sr} Hz"
        );
    }
}

#[test]
fn dirty_revision_ignores_performance_gates_but_tracks_note_recording_and_controls() {
    let mut live = rt();
    let initial = live.project.revision();
    live.apply(Command::Play);
    live.apply(Command::LiveNoteOn {
        source: 3,
        ch: 0,
        note: 55,
        vel: 90,
    });
    live.apply(Command::LiveNoteOff {
        source: 3,
        ch: 0,
        note: 55,
    });
    live.apply(Command::MidiClock { source: 3 });
    live.apply(Command::Stop);
    assert_eq!(live.project.revision(), initial);
    live.apply(Command::ComposeArm { track: 7, scene: 7 });
    let armed = live.project.revision();
    assert!(armed > initial);
    live.apply(Command::SamplerPad { pad: 3, on: true });
    assert!(live.project.revision() > armed);
    live.process(&mut vec![0.0; 24000]);
    let note = &captured(&live).state.tracks[7].clips[7].notes[0];
    assert!(
        note.len > 0.25,
        "save must capture held duration at its block boundary"
    );
    let held = live.project.revision();
    live.apply(Command::SamplerPad { pad: 3, on: false });
    assert!(live.project.revision() > held);
    let recorded = live.project.revision();
    live.apply(Command::TrackGain {
        track: 2,
        value: 0.33,
    });
    assert!(live.project.revision() > recorded);
}

#[test]
fn stopped_resume_edits_replace_scheduled_notes_and_first_arp_step_chases() {
    for arp in [false, true] {
        let mut source = rt();
        source.tracks[2].clips[0].notes = vec![MidiNote {
            pitch: 60,
            start: 0.0,
            len: 2.0,
            vel: 100,
        }];
        if arp {
            source.tracks[2]
                .fx
                .slots
                .push(fx::FxSlot::new(fx::FxId::Arp, 48000.0));
        }
        source.apply(Command::LaunchClip { track: 2, scene: 0 });
        let saved = captured(&source);
        let mut opened = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
        opened.rt.apply(Command::SetNotes {
            track: 2,
            scene: 0,
            notes: vec![MidiNote {
                pitch: 72,
                start: 0.0,
                len: 2.0,
                vel: 100,
            }],
        });
        opened.rt.apply(Command::Play);
        opened.rt.process(&mut [0.0; 8]);
        let notes: Vec<_> = opened.rt.tracks[2]
            .poly
            .voices
            .iter()
            .filter(|v| v.env.stage > 0)
            .map(|v| v.note())
            .collect();
        assert!(
            notes.contains(&72),
            "new first-step pitch did not sound (arp={arp}): {notes:?}"
        );
        assert!(
            !notes.contains(&60),
            "old saved pitch survived edit (arp={arp}): {notes:?}"
        );
    }
}

#[test]
fn view_edits_and_continuing_held_notes_cannot_appear_saved() {
    let mut live = rt();
    let first = live.project.revision();
    live.apply(Command::SelectDeck(1));
    assert!(live.project.revision() > first);
    live.apply(Command::ComposeArm { track: 7, scene: 7 });
    live.apply(Command::SamplerPad { pad: 3, on: true });
    let saved = captured(&live);
    live.process(&mut [0.0; 256]);
    assert!(
        live.project.revision() > saved.revision,
        "ongoing held recording appeared clean after Save"
    );
    let later = captured(&live);
    assert!(
        later.state.tracks[7].clips[7].notes[0].len > saved.state.tracks[7].clips[7].notes[0].len
    );
}

fn reference_hash(mut prepared: Prepared) -> u64 {
    prepared.rt.apply(Command::LaunchScene { scene: 0 });
    prepared.rt.apply(Command::DeckPlay { deck: 0 });
    prepared.rt.apply(Command::DeckPlay { deck: 1 });
    let mut out = vec![0.0; 8192];
    prepared.rt.process(&mut out);
    assert!(out.iter().any(|v| v.abs() > 0.0001));
    out.into_iter()
        .flat_map(|v| v.to_bits().to_le_bytes())
        .fold(0xcbf29ce484222325, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
}

#[test]
fn fresh_process_reopens_native_project_and_matches_reference_render() {
    const CHILD: &str = "OMATAINER_PROJECT_RENDER_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let bundle = crate::project_file::load::<State>(
            std::path::Path::new(&path),
            &Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let hash = reference_hash(Prepared::from_state(bundle.state, bundle.media, 48000).unwrap());
        assert_eq!(
            hash.to_string(),
            std::env::var("OMATAINER_PROJECT_RENDER_HASH").unwrap()
        );
        return;
    }
    let base = std::env::temp_dir().join(format!(
        "omatainer-project-render-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir(&base).unwrap();
    let path = base.join("full-production.omat");
    let state = captured(&populated());
    let hash = reference_hash(
        Prepared::from_state(state.state.clone(), state.media.clone(), 48000).unwrap(),
    );
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: state.state,
            media: state.media,
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "engine::project::tests::fresh_process_reopens_native_project_and_matches_reference_render", "--nocapture"])
        .env(CHILD, &path).env("OMATAINER_PROJECT_RENDER_HASH", hash.to_string()).output().unwrap();
    assert!(
        child.status.success(),
        "fresh process failed: {}{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn largest_supported_capture_is_bounded_and_has_no_callback_heap_traffic() {
    let mut live = rt();
    for track in &mut live.tracks {
        track.name = "t".repeat(MAX_TEXT_BYTES);
        for clip in &mut track.clips {
            clip.name = "c".repeat(MAX_TEXT_BYTES);
            clip.notes.clear();
        }
        track.clips[0].notes = vec![
            MidiNote {
                pitch: 60,
                start: 0.0,
                len: 1.0,
                vel: 100
            };
            MAX_NOTES_PER_CLIP
        ];
        track.fx.slots = (0..MAX_FX_PER_RACK)
            .map(|_| fx::FxSlot::new(fx::FxId::Comp, live.sr))
            .collect();
    }
    for rack in &mut live.scene_fx {
        rack.slots = (0..MAX_FX_PER_RACK)
            .map(|_| fx::FxSlot::new(fx::FxId::Comp, live.sr))
            .collect();
    }
    for deck in &mut live.decks {
        deck.title = "d".repeat(MAX_TEXT_BYTES);
    }
    live.sampler_banks = (0..MAX_BANKS).map(|_| "b".repeat(MAX_TEXT_BYTES)).collect();
    live.pad_banks = (0..MAX_BANKS).map(|_| live.pad_banks[0].clone()).collect();
    let mut micros = Vec::new();
    for _ in 0..9 {
        let mut frame = capture::Frame::new();
        frame.capture(&live);
        frame.prepare();
        let start = Instant::now();
        let counts = super::super::test_alloc::measure(|| frame.capture(&live));
        micros.push(start.elapsed().as_micros());
        assert_eq!(counts, super::super::test_alloc::Counts::default());
        assert!(frame.complete);
        assert_eq!(
            frame
                .state
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .map(|c| c.notes.len())
                .sum::<usize>(),
            MAX_TOTAL_NOTES
        );
    }
    micros.sort();
    eprintln!("maximum project capture: {} notes, {} rack slots, 16 banks, 4096-byte names; wall us median={} max={}; zero allocation/free (local copy cost, not stream deadline proof)",
        MAX_TOTAL_NOTES, MAX_FX_PER_RACK * (TRACKS + SCENES), micros[4], micros[8]);
    live.tracks[0].clips[1].notes.push(MidiNote {
        pitch: 60,
        start: 0.0,
        len: 1.0,
        vel: 100,
    });
    let mut rejected = capture::Frame::new();
    rejected.capture(&live);
    assert_eq!(rejected.error, Some("project exceeds 65536 notes"));
}

#[test]
fn close_seal_rejects_late_edits_and_drop_reopens_without_blocking_releases() {
    let (engine, mut live) = Engine::headless_for_test(48000, 256);
    let revision = engine.project.revision();
    engine
        .send(Command::TrackGain {
            track: 0,
            value: 0.42,
        })
        .unwrap();
    let handle = engine.project.clone();
    let changed = drive(&mut live, move || {
        handle.seal_for_close(Some(revision), &AtomicBool::new(false))
    });
    assert!(matches!(changed, Err(Error::Conflict)));
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    let guard = drive(&mut live, move || {
        handle.seal_for_close(Some(revision), &AtomicBool::new(false))
    })
    .unwrap();
    assert_eq!(
        engine.send(Command::TrackGain {
            track: 0,
            value: 0.91
        }),
        Err(SubmissionError::ProjectChanging)
    );
    assert!(engine
        .send(Command::LiveNoteOff {
            source: 1,
            ch: 0,
            note: 64
        })
        .is_ok());
    live.process(&mut []);
    assert_eq!(live.tracks[0].gain, 0.42);
    drop(guard);
    live.process(&mut []);
    assert!(engine
        .send(Command::TrackGain {
            track: 0,
            value: 0.91
        })
        .is_ok());
    live.process(&mut []);
    assert_eq!(live.tracks[0].gain, 0.91);
}

#[test]
fn explicit_discard_can_seal_a_held_recording_but_clean_close_cannot() {
    let mut live = rt();
    live.apply(Command::ComposeArm { track: 7, scene: 7 });
    live.apply(Command::SamplerPad { pad: 3, on: true });
    let handle = live.project.clone();
    let revision = handle.revision();
    assert!(matches!(
        drive(&mut live, move || handle
            .seal_for_close(Some(revision), &AtomicBool::new(false))),
        Err(Error::Conflict)
    ));
    let handle = live.project.clone();
    let guard = drive(&mut live, move || {
        handle.seal_for_close(None, &AtomicBool::new(false))
    })
    .unwrap();
    assert!(live.project_sealed);
    drop(guard);
    live.process(&mut []);
    assert!(!live.project_sealed);
}

#[test]
fn deferred_controller_loads_remain_on_old_project_when_install_or_clean_close_conflicts() {
    let (engine, mut live) = Engine::headless_for_test(48000, 256);
    engine
        .ui_requests
        .publish_selection(Some(Arc::new(media_source::Selection {
            source: media_source::LibSource::Builtin(media_source::BuiltinStem::Drums),
            title: "Drums".into(),
        })));
    engine.send(Command::DeckLoadSelected { deck: 1 }).unwrap();
    let handle = engine.project.clone();
    let revision = handle.revision();
    assert!(matches!(
        drive(&mut live, move || handle.install(
            Prepared::empty(48000).unwrap(),
            revision,
            &AtomicBool::new(false)
        )),
        Err(Error::Conflict)
    ));
    assert!(live.tracks[0].clips[0].occupied());
    assert_eq!(engine.cmd.ui_request_stats().pending, 1);
    let handle = engine.project.clone();
    let revision = handle.revision();
    assert!(matches!(
        drive(&mut live, move || handle
            .seal_for_close(Some(revision), &AtomicBool::new(false))),
        Err(Error::Conflict)
    ));
    assert_eq!(engine.cmd.ui_request_stats().pending, 1);
    let requests = engine.ui_requests.take_requests();
    assert!(matches!(&requests[0], Some(ui_requests::Request::Load(request)) if request.deck == 1));
    let handle = engine.project.clone();
    let revision = handle.revision();
    assert!(drive(&mut live, move || handle.install(
        Prepared::empty(48000).unwrap(),
        revision,
        &AtomicBool::new(false)
    ))
    .is_ok());
}
