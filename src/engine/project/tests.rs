use super::model::*;
use super::*;
mod sampler_tests;
mod gain_tests;
mod dependency_tests;
mod template_tests;
#[test]
fn channel_effects_native_archive_reopen_rate_change_and_legacy_migration_preserve_both_decks() {
    let mut original = rt();
    original.decks[0].channel_effect = channel_fx::Kind::Echo;
    original.decks[0].filter_amt = 0.18;
    original.decks[1].channel_effect = channel_fx::Kind::Room;
    original.decks[1].filter_amt = 0.83;
    let saved = captured(&original);
    let directory = std::env::temp_dir().join(format!("omatainer-channel-effects-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("two-decks.omat");
    let cancelled = AtomicBool::new(false);
    let outcome = crate::project_file::save(&path, &crate::project_file::Bundle { state: saved.state.clone(), media: saved.media.clone() }, crate::project_file::Overwrite::Never, &Default::default(), &cancelled).unwrap();
    assert_eq!(outcome, crate::project_file::SaveOutcome::Durable);
    let loaded = crate::project_file::load::<State>(&path, &Default::default(), &cancelled).unwrap();
    for rate in [44100, 48000, 96000] {
        let reopened = Prepared::from_state(loaded.state.clone(), loaded.media.clone(), rate).unwrap();
        for deck in 0..2 { assert_eq!(reopened.rt.decks[deck].channel_effect, original.decks[deck].channel_effect); assert_eq!(reopened.rt.decks[deck].filter_amt, original.decks[deck].filter_amt); }
    }
    let mut legacy = serde_json::to_value(captured(&rt()).state).unwrap();
    legacy["version"] = 34.into();
    assert!(legacy["decks"].as_array().unwrap().iter().all(|deck| deck.get("channel_effect").is_none()));
    let migrated: State = serde_json::from_value(legacy.clone()).unwrap();
    assert!(migrated.decks.iter().all(|deck| deck.channel_effect == channel_fx::Kind::Filter));
    for field in [serde_json::Value::Null, serde_json::json!("filter"), serde_json::json!("echo")] { let mut invalid = legacy.clone(); invalid["decks"][0]["channel_effect"] = field; assert!(serde_json::from_value::<State>(invalid).is_err()); }
    legacy["version"] = STATE_VERSION.into(); legacy["decks"][0]["channel_effect"] = "unknown".into();
    assert!(serde_json::from_value::<State>(legacy).is_err());
    println!("CHANNEL_EFFECT_PROJECT {{\"native_archive\":true,\"three_output_rates\":true,\"legacy_filter_migration\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn named_controller_lanes_roundtrip_in_schema_fifteen_and_reject_legacy_or_invalid_labels() {
    let mut original = rt();
    let lanes = midi_data::Lanes::named(960, 15360, vec![crate::midi_file::Message { tick: 120, order: 1, bytes: [0xbf,74,99], length: 3 }], vec![], vec![midi_data::Label { channel: 15, control: midi_data::ControlKind::Cc { controller: 74 }, name: "Filter cutoff".into() }]).unwrap();
    original.tracks[2].clips[7].kind = ClipKind::Midi; original.tracks[2].clips[7].lanes = Some(lanes.clone());
    let saved = captured(&original);
    let json = serde_json::to_value(&saved.state).unwrap();
    assert_eq!(json["version"], STATE_VERSION);
    assert!(json["tracks"][2]["clips"][7]["lanes"].get("state").is_none());
    let decoded: State = serde_json::from_value(json.clone()).unwrap();
    let prepared = Prepared::from_state(decoded, saved.media.clone(), 48000).unwrap();
    assert_eq!(prepared.rt.tracks[2].clips[7].lanes, Some(lanes));
    assert!(prepared.rt.tracks[2].clips[7].lanes.as_ref().unwrap().prepared());
    for labels in [serde_json::json!([]), serde_json::Value::Null] {
        let mut old = json.clone(); old["version"] = 14.into(); old["tracks"][2]["clips"][7]["lanes"]["labels"] = labels;
        assert!(serde_json::from_value::<State>(old).is_err());
    }
    let mut invalid = saved.state.clone(); let source = invalid.tracks[2].clips[7].lanes.as_mut().unwrap();
    Arc::make_mut(source).labels[0].channel = 16;
    assert!(invalid.validate(&saved.media).is_err());
}
#[test]
fn prepared_controller_edit_is_saveable_allocation_free_and_rejects_concurrent_project_changes() {
    let (engine, mut live) = Engine::headless_for_test(48000, 256);
    let baseline = midi_edit::Document::capture(captured(&live), 2, 7).unwrap();
    let lanes = midi_data::Lanes::new(960,15360,vec![crate::midi_file::Message { tick: 960, order: 1, bytes: [0xb2,74,88], length: 3 }],vec![]).unwrap();
    let create = |live: &RtEngine| {
        let (request, ack, _) = midi_edit::Request::with_lanes(baseline.clone(), "Filter".into(), midi_edit::Region::full(4.0), vec![], Some(lanes.clone())).unwrap();
        (request.guard_metadata(captured(live), &AtomicBool::new(false)).unwrap(), ack)
    };
    let (request, rejected) = create(&live);
    engine.send(Command::TrackGain { track: 3, value: 0.25 }).unwrap(); live.process(&mut []);
    engine.send(Command::MidiEdit(request)).unwrap(); live.process(&mut []);
    assert_eq!(rejected.state(), midi_edit::Outcome::Rejected);
    assert!(live.tracks[2].clips[7].lanes.is_none());
    let (request, applied) = create(&live); engine.send(Command::MidiEdit(request)).unwrap();
    assert_eq!(super::super::test_alloc::measure(|| live.process(&mut [])), super::super::test_alloc::Counts::default());
    assert_eq!(applied.state(), midi_edit::Outcome::Applied);
    let saved = captured(&live); saved.state.validate(&saved.media).unwrap();
    let bytes = serde_json::to_vec(&saved.state).unwrap();
    let state: State = serde_json::from_slice(&bytes).unwrap();
    let reopened = Prepared::from_state(state,saved.media,48000).unwrap();
    assert_eq!(reopened.rt.tracks[2].clips[7].lanes,Some(lanes));
}
pub(super) mod session_tests;

fn legacy_midi_fields(state: &mut serde_json::Value) {
    state.as_object_mut().unwrap().remove("timeline_seconds");
    state.as_object_mut().unwrap().remove("conductor");
    state.as_object_mut().unwrap().remove("session");
    if state["fx_view"].as_i64().is_some_and(|v| v >= session::SCENE_FX_BASE as i64) { state["fx_view"] = (state["fx_view"].as_i64().unwrap() - (session::SCENE_FX_BASE as i64 - 100)).into(); }
    for track in state["tracks"].as_array_mut().unwrap() {
        for clip in track["clips"].as_array_mut().unwrap() {
            clip.as_object_mut().unwrap().remove("region");
            clip.as_object_mut().unwrap().remove("lanes");
            for note in clip["notes"].as_array_mut().unwrap() {
                let note = note.as_object_mut().unwrap();
                note.remove("id"); note.remove("muted");
                note.remove("channel"); note.remove("release_vel"); note.remove("source_timing");
            }
        }
    }
}

#[test]
fn version_four_notes_migrate_deterministic_identity_and_new_fields_are_not_legacy_data() {
    let original = rt();
    let saved = captured(&original);
    let mut json = serde_json::to_value(&saved.state).unwrap();
    json["version"] = 4.into();
    legacy_midi_fields(&mut json);
    let legacy: State = serde_json::from_value(json.clone()).unwrap();
    legacy.validate(&saved.media).unwrap();
    let first = Prepared::from_state(legacy.clone(), saved.media.clone(), 48_000).unwrap();
    let second = Prepared::from_state(legacy, saved.media.clone(), 48_000).unwrap();
    for (a, b) in first.rt.tracks.iter().zip(&second.rt.tracks) {
        for (a, b) in a.clips.iter().zip(&b.clips) {
            assert_eq!(a.notes, b.notes);
            assert!(a.notes.iter().all(|note| note.id.valid() && !note.muted));
        }
    }
    let recaptured = captured(&first.rt);
    assert_eq!(recaptured.state.version, STATE_VERSION);
    for field in ["id", "muted"] {
        let mut invalid = json.clone();
        invalid["tracks"][0]["clips"][0]["notes"][0][field] = serde_json::Value::Null;
        assert!(serde_json::from_value::<State>(invalid).is_err());
    }
    json["tracks"][0]["clips"][0]["region"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<State>(json).is_err());
}

#[test]
fn current_note_identity_and_regions_validate_before_installation() {
    let original = rt();
    let saved = captured(&original);
    let mut invalid = saved.state.clone();
    invalid.tracks[0].clips[0].notes[0].id = midi_edit::NoteId::default();
    assert!(invalid.validate(&saved.media).is_err());
    let mut invalid = saved.state.clone();
    invalid.tracks[0].clips[0].notes[1].id = invalid.tracks[0].clips[0].notes[0].id;
    assert!(invalid.validate(&saved.media).is_err());
    let mut valid = saved.state.clone();
    let clip = &mut valid.tracks[0].clips[0];
    let mut region = midi_edit::Region::full(clip.bars);
    region.start = 0.125; region.loop_start = 1.0;
    clip.region = Some(region); clip.notes[0].muted = true;
    valid.validate(&saved.media).unwrap();
    let prepared = Prepared::from_state(valid, saved.media.clone(), 48_000).unwrap();
    assert_eq!(prepared.rt.tracks[0].clips[0].region, Some(region));
    assert!(prepared.rt.tracks[0].clips[0].notes[0].muted);
    let mut invalid = saved.state;
    invalid.tracks[0].clips[0].region = Some(midi_edit::Region {loop_end:f64::NAN,..region});
    assert!(invalid.validate(&saved.media).is_err());
}

#[test]
fn explicit_region_clock_cursor_survives_capture_install_and_output_rate_change() {
    use super::super::{midi_schedule::Gate, test_alloc};
    let mut live = rt();
    live.bpm = 120.0;
    live.quant = 0.0;
    live.beat = 1000.0;
    live.sync_midi_clock();
    // Make the two clocks observably different. An immediate region launch
    // must capture its own clock origin, while saved state uses transport beat.
    live.midi_beat += 0.125;
    live.apply(Command::SetNotes {track:2, scene:7, notes:vec![
        MidiNote { variation: None, channel:0,release_vel:64,source_timing:None, id:midi_edit::NoteId::new(), muted:false, pitch:60, start:0.0, len:2.0, vel:80 },
        MidiNote { variation: None, channel:0,release_vel:64,source_timing:None, id:midi_edit::NoteId::new(), muted:false, pitch:62, start:4.0, len:1.0, vel:90 },
    ]});
    live.tracks[2].clips[7].bars = 16.0;
    live.tracks[2].clips[7].region = Some(midi_edit::Region::full(16.0));
    live.apply(Command::LaunchClip {track:2, scene:7});
    live.process(&mut vec![0.0; 24_000 * 2]);
    let launch = live.tracks[2].playing.unwrap();
    let cursor = live.precise_midi_beat() - launch.midi_start_beat;
    assert!((cursor - 1.0).abs() < 1e-10);
    let saved = captured(&live);
    let saved_launch = saved.state.tracks[2].launch.unwrap();
    assert!((saved.state.beat - saved_launch.start_beat - cursor).abs() < 1e-10);
    let mut prepared = Prepared::from_state(saved.state, saved.media, 48_000).unwrap();
    let mut opened = rt();
    opened.beat = 37.0;
    opened.sync_midi_clock();
    opened.midi_beat += 0.5;
    assert_eq!(test_alloc::measure(|| prepared.swap_into(&mut opened)), test_alloc::Counts::default());
    opened.set_sample_rate(96_000).unwrap();
    let resumed = opened.tracks[2].project_resume.unwrap();
    assert!((opened.precise_midi_beat() - resumed.midi_start_beat - cursor).abs() < 1e-10);
    opened.tracks[2].midi_schedule.trace = Some(Vec::with_capacity(16));
    opened.apply(Command::Play);
    let mut output = vec![0.0; 512 * 2];
    for _ in 0..376 { opened.process(&mut output); }
    let trace = opened.tracks[2].midi_schedule.trace.take().unwrap();
    let events:Vec<_> = trace.into_iter()
        .filter(|(_,gate)| *gate != Gate::On(60,80)) // initial resume chase
        .map(|(elapsed,gate)| (((elapsed-cursor)*48_000.0-1.0).round() as usize,gate))
        .collect();
    assert_eq!(events,vec![(48_000,Gate::Off(60)),(144_000,Gate::On(62,90)),(192_000,Gate::Off(62))]);
}

#[test]
fn schema_six_note_channels_and_ticks_roundtrip_and_cannot_impersonate_schema_five() {
    let original=rt();let mut saved=captured(&original);
    let raw=crate::midi_file::Note {channel:12,pitch:64,velocity:80,release_velocity:21,
        start_tick:1,duration_ticks:719,start_order:1,end_order:2};
    saved.state.tracks[2].clips[7].kind=ClipKind::Midi;
    saved.state.tracks[2].clips[7].notes=vec![MidiNote::from_smf(&raw,960).unwrap()];
    let exact=saved.state.tracks[2].clips[7].notes.clone();
    let restored=Prepared::from_state(saved.state.clone(),saved.media.clone(),48000).unwrap();
    assert_eq!(captured(&restored.rt).state.tracks[2].clips[7].notes,exact);
    let mut legacy=serde_json::to_value(&saved.state).unwrap();legacy["version"]=5.into();
    legacy.as_object_mut().unwrap().remove("timeline_seconds");
    legacy.as_object_mut().unwrap().remove("conductor");
    legacy.as_object_mut().unwrap().remove("session");
    for track in legacy["tracks"].as_array_mut().unwrap() {for clip in track["clips"].as_array_mut().unwrap() {
        clip.as_object_mut().unwrap().remove("lanes");
        for note in clip["notes"].as_array_mut().unwrap() {for field in ["channel","release_vel","source_timing"] {note.as_object_mut().unwrap().remove(field);}}
    }}
    let decoded:State=serde_json::from_value(legacy.clone()).unwrap();decoded.validate(&saved.media).unwrap();
    for field in ["channel","release_vel","source_timing"] {
        let mut bad=legacy.clone();bad["tracks"][2]["clips"][7]["notes"][0][field]=serde_json::Value::Null;
        assert!(serde_json::from_value::<State>(bad).is_err());
    }
    let mut invalid=saved.state;invalid.tracks[2].clips[7].notes[0].source_timing.as_mut().unwrap().duration+=1;
    assert!(invalid.validate(&saved.media).is_err());
}

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
            rt.tracks[t].clips[s] = Clip { variation: None,
                properties: Default::default(),
                audio_region: None, lanes: None,
                region: None,
                kind: if s == 7 {
                    ClipKind::Audio
                } else {
                    ClipKind::Midi
                },
                name: format!("clip {t}:{s}"),
                bars: 2.0,
                gain: 0.7,
                notes: vec![MidiNote { variation: None,
                    channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
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
    rt.decks[0].cue_styles[3] = crate::engine::cue_metadata::Style {
        name: crate::engine::cue_metadata::Name::new("Drop • 演奏").unwrap(),
        color: Some([240, 32, 90]),
    };
    rt.decks[0].grid = Some(crate::engine::beatgrid::Grid::new(0.5, 127.0).unwrap());
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
    rt.fx_view = session::SCENE_FX_BASE + 3;
    rt
}

#[test]
fn tempo_anchor_state_preserves_media_and_old_versions_reject_anchor_fields() {
    use super::super::beatgrid::Grid;
    let mut original = populated();
    let grid = Grid::new(0.1, 120.0).unwrap().with_anchor(4.0, 2.1).unwrap().with_anchor(8.0, 3.6).unwrap();
    original.decks[0].grid = Some(grid);
    let saved = captured(&original);
    let value = serde_json::to_value(&saved.state).unwrap();
    let state: State = serde_json::from_value(value.clone()).unwrap();
    state.validate(&saved.media).unwrap();
    let prepared = Prepared::from_state(state, saved.media.clone(), 96_000).unwrap();
    assert_eq!(prepared.rt.decks[0].grid, Some(grid));
    assert!(Arc::ptr_eq(original.decks[0].audio.as_ref().unwrap(), prepared.rt.decks[0].audio.as_ref().unwrap()));
    assert_eq!(prepared.rt.decks[0].pos, original.decks[0].pos);
    let mut legacy = value;
    legacy["version"] = 12.into();
    assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    legacy["decks"][0]["grid"]["anchors"] = serde_json::json!([]);
    assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    legacy["decks"][0]["grid"].as_object_mut().unwrap().remove("anchors");
    let state: State = serde_json::from_value(legacy).unwrap();
    state.validate(&saved.media).unwrap();
    assert!(Prepared::from_state(state, saved.media, 48_000).unwrap().rt.decks[0].grid.unwrap().anchors().is_empty());
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
        source.tracks[2].clips[0].notes = vec![MidiNote { variation: None,
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
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
            notes: vec![MidiNote { variation: None,
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
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
            MidiNote { variation: None,
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
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
    live.sampler_banks = (0..MAX_BANKS).map(|_| sampler::test_bank(&live, "b".repeat(MAX_TEXT_BYTES), live.sampler_banks[0].data.audio.clone())).collect();
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
    live.tracks[0].clips[1].notes.push(MidiNote { variation: None,
        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
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
            title: "Drums".into(), fingerprint: None, })));
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

#[test]
fn close_acknowledgement_waits_for_preceding_first_frame_playback_history() {
    for clean in [true, false] {
        let (engine, mut live) = Engine::headless_for_test(48_000, 256);
        let receipt = engine.initial_playback[0].clone().unwrap();
        live.apply(Command::DeckPlay { deck: 0 });
        assert!(receipt.last_play().is_none());
        let handle = engine.project.clone();
        let expected = clean.then(|| handle.revision());
        let watched = receipt.clone();
        let (done, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let guard = handle.seal_for_close(expected, &AtomicBool::new(false)).unwrap();
            done.send((guard, watched.last_play())).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while live.project.shared.incoming.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        // Exercise the exact pre-render project boundary with no timing race.
        let counts = test_alloc::measure(|| live.project_tick());
        assert_eq!((counts.allocations, counts.frees), (0, 0));
        assert!(live.project_sealed);
        assert!(received.try_recv().is_err(), "close guard escaped before source playback");
        assert!(receipt.last_play().is_none());
        let counts = test_alloc::measure(|| live.process(&mut [0.0; 2]));
        assert_eq!((counts.allocations, counts.frees), (0, 0));
        let (guard, observed) = received.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(observed.is_some(), "close authorization includes the final first-play stamp");
        assert_eq!(observed, receipt.last_play());
        worker.join().unwrap();
        drop(guard);
        live.process(&mut []);
        assert!(engine.send(Command::Master(0.44)).is_ok());
    }
}

#[test]
fn version_one_projects_migrate_empty_cue_names_and_colors_and_unknown_versions_fail() {
    let original = populated();
    let captured = captured(&original);
    let mut json = serde_json::to_value(&captured.state).unwrap();
    json["version"] = serde_json::json!(1);
    json.as_object_mut().unwrap().remove("session");
    legacy_midi_fields(&mut json);
    for bank in json["banks"].as_array_mut().unwrap() { let bank = bank.as_object_mut().unwrap(); bank.remove("instance"); bank.remove("settings"); }
    for deck in json["decks"].as_array_mut().unwrap() {
        deck.as_object_mut().unwrap().remove("cue_styles");
        deck.as_object_mut().unwrap().remove("grid");
    }
    let legacy: State = serde_json::from_value(json.clone()).unwrap();
    legacy.validate(&captured.media).unwrap();
    assert!(legacy.decks.iter().all(|d| d.grid.is_none()));
    assert!(legacy.decks.iter().all(|d| d.cue_styles == [crate::engine::cue_metadata::Style::default(); HOTCUES]));
    let restored = Prepared::from_state(legacy, captured.media.clone(), 48000).unwrap();
    assert!(restored.rt.decks[0].hotcues[3].set);
    assert_eq!(restored.rt.decks[0].hotcues[3].pos, original.decks[0].hotcues[3].pos);
    json["version"] = serde_json::json!(STATE_VERSION + 1);
    let future: State = serde_json::from_value(json).unwrap();
    assert!(future.validate(&captured.media).is_err());
}

#[test]
fn version_two_projects_keep_cue_metadata_with_no_invented_manual_grid() {
    let original = populated();
    let captured = captured(&original);
    let mut json = serde_json::to_value(&captured.state).unwrap();
    json["version"] = serde_json::json!(2);
    json.as_object_mut().unwrap().remove("session");
    legacy_midi_fields(&mut json);
    for bank in json["banks"].as_array_mut().unwrap() { let bank = bank.as_object_mut().unwrap(); bank.remove("instance"); bank.remove("settings"); }
    for deck in json["decks"].as_array_mut().unwrap() { deck.as_object_mut().unwrap().remove("grid"); }
    let legacy: State = serde_json::from_value(json).unwrap();
    legacy.validate(&captured.media).unwrap();
    assert!(legacy.decks.iter().all(|deck| deck.grid.is_none()));
    assert_eq!(legacy.decks[0].cue_styles, original.decks[0].cue_styles);
    let restored = Prepared::from_state(legacy, captured.media, 48000).unwrap();
    assert_eq!(restored.rt.decks[0].cue_styles, original.decks[0].cue_styles);
    assert_eq!(restored.rt.decks[0].hotcues[3].pos, original.decks[0].hotcues[3].pos);
}

#[test]
fn prepared_project_history_uses_qualified_receipt_keys_and_empty_decks_have_no_load() {
    let source = populated();
    let saved = captured(&source);
    let prepared = Prepared::from_state(saved.state, saved.media, 48_000).unwrap();
    for deck in &prepared.rt.decks {
        assert!(deck.audio.is_some());
        assert_eq!(deck.history_key, deck.load_receipt.as_ref().unwrap().history_key());
    }
    let handle = prepared.rt.history_measurement.as_ref().unwrap().handle();
    let request = handle.submit(super::super::history_measurement::control::Action::Start(1)).unwrap();
    let mut callback = super::super::audio::OutputCallback::new(*prepared.rt, 2);
    callback.render(&mut [0.0_f32; 256]);
    let ack = handle.poll(request).unwrap().unwrap();
    assert_eq!(ack.current, callback.renderer_for_test().decks.each_ref().map(|d|d.load_receipt.as_ref().unwrap().history_key()));
    let empty = Prepared::empty(48_000).unwrap();
    assert!(empty.rt.decks.iter().all(|d|d.audio.is_none() && d.history_key == 0));
}

mod structure_tests;

#[test]
fn reinstalling_the_same_project_rejects_old_musical_jobs_before_playback_resumes() {
    use crate::engine::remote;
    for epoch in [0, u64::MAX] {
        let (engine, mut live) = Engine::headless_for_test(48000, 256);
        live.transport_epoch = epoch;
        live.apply(Command::Play);
        live.process(&mut []);
        let handle = engine.project.clone();
        let saved = drive(&mut live, move || handle.capture(&AtomicBool::new(false))).unwrap();
        let namespace = live.session.namespace;
        let target = live.session.reference(session::Axis::Track, 2).unwrap();
        let before = live.tracks[2].gain;
        let at = live.beat + 4.0;
        let ack = midi_edit::Ack::new();
        engine
            .send(Command::Remote(remote::Request {
                namespace,
                transport_epoch: live.transport_epoch,
                safety_epoch: live.performance.safety_epoch(),
                at: Some(at),
                action: remote::Action::Gain {
                    slot: 2,
                    target,
                    value: 0.17,
                },
                ack: ack.clone(),
            }))
            .unwrap();
        live.process(&mut []);
        assert_eq!(ack.state(), midi_edit::Outcome::Pending);
        let prepared = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
        let handle = engine.project.clone();
        let revision = handle.revision();
        let worker =
            std::thread::spawn(move || handle.install(prepared, revision, &AtomicBool::new(false)));
        wait_until(|| {
            assert_eq!(
                crate::engine::test_alloc::measure(|| live.process(&mut [])),
                crate::engine::test_alloc::Counts::default()
            );
            worker.is_finished()
        });
        worker.join().unwrap().unwrap();
        assert_eq!(live.session.namespace, namespace);
        assert_eq!(
            live.session.reference(session::Axis::Track, 2).unwrap(),
            target
        );
        assert_ne!(live.transport_epoch, epoch);
        assert_eq!(ack.state(), midi_edit::Outcome::Rejected);
        assert!(!live.playing);
        live.apply(Command::Play);
        live.beat = at + 1.0;
        assert_eq!(
            crate::engine::test_alloc::measure(|| live.process(&mut [0.0; 128])),
            crate::engine::test_alloc::Counts::default()
        );
        assert_eq!(live.tracks[2].gain, before);
    }
}

#[test]
fn sample_based_position_roundtrips_and_legacy_headers_cannot_hide_it() {
    let mut original=rt();original.apply(Command::Play);original.process(&mut vec![0.0;96000]);
    original.apply(Command::SetBpm(200.0));original.process(&mut vec![0.0;48000]);
    let saved=captured(&original);assert_eq!(saved.state.timeline_seconds,1.5);
    let prepared=Prepared::from_state(saved.state.clone(),saved.media.clone(),48000).unwrap();
    assert_eq!(prepared.rt.timeline_seconds(),1.5);
    let mut wire=serde_json::to_value(&saved.state).unwrap();wire["version"]=9.into();
    assert!(serde_json::from_value::<State>(wire.clone()).is_err());
    wire.as_object_mut().unwrap().remove("timeline_seconds");
    let legacy:State=serde_json::from_value(wire).unwrap();
    assert_eq!(legacy.timeline_seconds,saved.state.beat*60.0/f64::from(saved.state.bpm));
}

mod input_monitor_tests;

#[test]
fn key_shift_offsets_roundtrip_native_project_pcm_and_reject_older_headers_or_invalid_ranges() {
    use crate::engine::key_shift;
    let mut live=Box::new(rt());live.publish_for_test();
    for (deck,offset,lock) in [(0,3,true),(1,-2,false)] {
        let request={let snapshot=live.snap.lock();key_shift::Request::new(deck,&snapshot.decks[usize::from(deck)],offset,lock).unwrap()};
        live.apply(Command::DeckKeyShift(request));
    }
    let saved=captured(&live);let audio=saved.media.clone();let wire=serde_json::to_value(&saved.state).unwrap();assert_eq!(wire["version"],STATE_VERSION);assert_eq!(wire["decks"][0]["key_shift"],3);assert_eq!(wire["decks"][1]["key_shift"],-2);
    let path=std::env::temp_dir().join(format!("omatainer-key-shift-project-{}.omat",std::process::id()));
    struct Remove(std::path::PathBuf);impl Drop for Remove {fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}let _remove=Remove(path.clone());
    let cancel=AtomicBool::new(false);let bundle=crate::project_file::Bundle {state:saved.state.clone(),media:saved.media.clone()};
    assert_eq!(crate::project_file::save(&path,&bundle,crate::project_file::Overwrite::Never,&crate::project_file::Limits::default(),&cancel).unwrap(),crate::project_file::SaveOutcome::Durable);
    let decoded=crate::project_file::load::<State>(&path,&crate::project_file::Limits::default(),&cancel).unwrap();assert_eq!(decoded.media.len(),audio.len());for (actual,original) in decoded.media.iter().zip(&audio){assert_eq!(actual.data,original.data);}
    let reopened=Prepared::from_state(decoded.state,decoded.media,44100).unwrap();assert_eq!(reopened.rt.decks[0].key_shift,3);assert!(reopened.rt.decks[0].keylock);assert_eq!(reopened.rt.decks[1].key_shift,-2);assert!(!reopened.rt.decks[1].keylock);assert!(reopened.rt.decks.iter().all(|deck|!deck.playing));
    for value in [serde_json::json!(0),serde_json::Value::Null] {let mut legacy=wire.clone();legacy["version"]=27.into();for deck in legacy["decks"].as_array_mut().unwrap(){deck.as_object_mut().unwrap().remove("key_shift");}legacy["decks"][0]["key_shift"]=value;assert!(serde_json::from_value::<State>(legacy).is_err());}
    let mut legacy=serde_json::to_value(State::blank()).unwrap();legacy["version"]=27.into();let legacy:State=serde_json::from_value(legacy).unwrap();assert!(legacy.decks.iter().all(|deck|deck.key_shift==0));
    for offset in [-7,7] {let mut invalid=saved.state.clone();invalid.decks[0].key_shift=offset;assert!(invalid.validate(&saved.media).is_err());}
    let mut invalid=saved.state;invalid.version=27;assert!(invalid.validate(&saved.media).is_err());
    println!("KEY_SHIFT_PROJECT_RECEIPT {}",serde_json::json!({"project_state_version":STATE_VERSION,"real_native_file_roundtrip":true,"embedded_pcm_preserved":true,"output_sample_rate":44100,"offsets":[3,-2],"reopen_starts_stopped":true,"legacy_zero_migrated":true,"legacy_field_injection_refused":true,"invalid_offsets_refused":true,"physical_devices_opened":false}));
}
