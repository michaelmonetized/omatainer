use super::*;

#[test]
fn exported_ramp_roundtrips_midi_ticks_and_agrees_with_independent_timestamp_integration() {
    use crate::engine::midi_data::{Meter, Tempo, TimingSettings};
    let map = Conductor::native(960, vec![Tempo::new(0, 120.0, true).unwrap(), Tempo::new(8160, 180.0, false).unwrap()], vec![Meter { tick: 0, numerator: 7, denominator_power: 3, clocks: 12, thirty_seconds: 8 }], TimingSettings::default()).unwrap();
    let b = 60000000.0 / f64::from(map.tempos[1].micros);
    for ppqn in [480, 960, 1920] {
        let mut meta = export_conductor(&map, ppqn, u64::from(ppqn) * 16, &mut || false).unwrap();
        meta.sort_by_key(|m| (m.tick, m.order));
        let file = smf::File { format: smf::Format::Single, ppqn, tracks: vec![smf::Track { end_tick: u64::from(ppqn) * 16, meta, ..Default::default() }], warnings: vec![] };
        let bytes = smf::encode(&file, false).unwrap();
        let decoded = smf::decode(&bytes).unwrap();
        assert_eq!(musical(decoded.clone()), musical(file));
        let tempos: Vec<_> = decoded.tracks[0].meta.iter().filter_map(|m| if let MetaValue::Tempo(micros) = m.value { Some((m.tick, micros)) } else { None }).collect();
        let exported_seconds = |beat: f64| {
            let target = beat * f64::from(ppqn); let mut seconds = 0.0; let mut tick = 0.0; let mut micros = 500000;
            for &(next, value) in &tempos {
                if next as f64 > target { break; }
                seconds += (next as f64 - tick) * f64::from(micros) / (f64::from(ppqn) * 1_000_000.0);
                tick = next as f64; micros = value;
            }
            seconds + (target - tick) * f64::from(micros) / (f64::from(ppqn) * 1_000_000.0)
        };
        for beat in [0.5_f64, 3.5, 5.0, 8.5, 12.5, 16.0] {
            let n = 20000; let length = beat.min(8.5); let h = length / f64::from(n);
            let f = |x: f64| 60.0 / (120.0 + (b - 120.0) * x / 8.5);
            let mut sum = f(0.0) + f(length);
            for i in 1..n { sum += f(f64::from(i) * h) * if i % 2 == 0 { 2.0 } else { 4.0 }; }
            let expected = sum * h / 3.0 + (beat - 8.5).max(0.0) * 60.0 / b;
            assert!((exported_seconds(beat) - expected).abs() < 1.0 / 96000.0, "{ppqn} beat{beat}: {} vs{expected}", exported_seconds(beat));
        }
    }
    assert!(export_conductor(&map, 960, 15360, &mut || true).is_err());
    let huge = Conductor::native(960, vec![Tempo::new(0, 40.0, true).unwrap(), Tempo::new(960 * 100, 240.0, false).unwrap()], map.meters.clone(), TimingSettings::default()).unwrap();
    assert!(export_conductor(&huge, 32767, 32767 * 100, &mut || false).is_err());
}
use crate::engine::{midi_edit::Outcome, test_alloc};
use std::{sync::atomic::AtomicBool, time::Instant};

fn capture(engine: &Engine, rt: &mut RtEngine) -> project::Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.join().unwrap()
}
fn fixture() -> smf::File {
    smf::decode(include_bytes!(
        "../../../tests/fixtures/midi/sixteen-bars-ppqn960.mid"
    ))
    .unwrap()
}
fn mapping(file: &smf::File) -> Vec<Mapping> {
    sources(file, false)
        .into_iter()
        .enumerate()
        .map(|(i, source)| Mapping {
            source,
            destination: Some((2, (6 + i) as u16)),
        })
        .collect()
}
fn request(engine: &Engine, rt: &mut RtEngine, tempo: TempoChoice) -> (Request, Ack) {
    let file = fixture();
    Request::prepare(
        capture(engine, rt),
        &file,
        &mapping(&file),
        false,
        false,
        tempo,
        false,
        false,
    )
    .unwrap()
}
fn musical(mut file: smf::File) -> smf::File {
    for track in &mut file.tracks {
        for note in &mut track.notes {
            note.start_order = 0;
            note.end_order = 0;
        }
        for message in &mut track.messages {
            message.order = 0;
        }
        for meta in &mut track.meta {
            meta.order = 0;
        }
        track.meta.sort_by_key(|m| m.tick);
    }
    file
}
fn options() -> ExportOptions {
    ExportOptions {
        ppqn: 960,
        single_track: false,
        include_muted: true,
        allow_rounding: false,
        session_conductor: false,
    }
}

#[test]
fn imported_lanes_ticks_and_conductor_roundtrip_with_one_undo_and_no_callback_heap() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let before = capture(&engine, &mut rt);
    let (request, ack) = request(&engine, &mut rt, TempoChoice::File(None));
    engine
        .send(Command::TrackGain {
            track: 5,
            value: 0.42,
        })
        .unwrap();
    rt.process(&mut []);
    let previous = engine.undo.view().cursor;
    engine.send(Command::MidiImport(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    let imported = capture(&engine, &mut rt);
    assert_eq!(engine.undo.view().cursor, previous + 1);
    assert_eq!(engine.undo.view().items[previous].unwrap().patches, 3);
    assert_eq!(imported.state.tracks[5].gain, 0.42);
    assert_eq!(
        imported.state.conductor.as_ref().unwrap().tempos[1].micros,
        666667
    );
    assert_eq!(
        imported.state.tracks[2].clips[7]
            .lanes
            .as_ref()
            .unwrap()
            .messages
            .len(),
        11
    );
    let cells = [(2, 6), (2, 7)];
    assert_eq!(
        musical(
            smf::decode(
                &smf::encode(&export(&imported.state, &cells, options()).unwrap(), false).unwrap()
            )
            .unwrap()
        ),
        musical(fixture())
    );
    let value = serde_json::to_vec(&imported.state).unwrap();
    let state: project::State = serde_json::from_slice(&value).unwrap();
    let prepared = project::Prepared::from_state(state, imported.media.clone(), 96000).unwrap();
    let (reopened_engine, mut reopened_rt) = Engine::headless_for_test(96000, 256);
    let handle = reopened_engine.project.clone();
    let revision = handle.revision();
    let job = std::thread::spawn(move || {
        handle
            .install(prepared, revision, &AtomicBool::new(false))
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !job.is_finished() {
        reopened_rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.join().unwrap();
    assert_eq!(
        musical(
            smf::decode(
                &smf::encode(
                    &export(
                        &capture(&reopened_engine, &mut reopened_rt).state,
                        &cells,
                        options()
                    )
                    .unwrap(),
                    false
                )
                .unwrap()
            )
            .unwrap()
        ),
        musical(fixture())
    );
    engine.send(Command::Undo).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    let undone = capture(&engine, &mut rt);
    assert_eq!(
        undone.state.tracks[2].clips[7].notes,
        before.state.tracks[2].clips[7].notes
    );
    assert!(undone.state.conductor.is_none());
    assert_eq!(undone.state.tracks[5].gain, 0.42);
    engine.send(Command::Redo).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(
        export(&capture(&engine, &mut rt).state, &cells, options()).unwrap(),
        export(&imported.state, &cells, options()).unwrap()
    );
}

#[test]
fn cancelled_stale_protected_and_held_imports_are_atomic_and_retired() {
    for reason in 0..5 {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        let (request, ack) = request(&engine, &mut rt, TempoChoice::File(None));
        match reason {
            0 => {
                assert!(ack.cancel());
            }
            1 => {
                rt.tracks[2].clips[7].name = "other edit".into();
            }
            2 => {
                engine
                    .send(Command::ComposeArm { track: 2, scene: 7 })
                    .unwrap();
                rt.process(&mut []);
                engine
                    .send(Command::LiveNoteOn {
                        source: 111,
                        ch: 2,
                        note: 64,
                        vel: 80,
                    })
                    .unwrap();
                rt.process(&mut []);
            }
            3 => {
                engine.send(Command::PerformanceMode(true)).unwrap();
                rt.process(&mut []);
            }
            _ => {
                engine.send(Command::SetBpm(130.0)).unwrap();
                rt.process(&mut []);
            }
        }
        let before = engine.undo.checkpoint();
        let notes = rt.tracks[2].clips[7].notes.clone();
        if reason == 3 {
            assert!(engine.send(Command::MidiImport(request)).is_err());
        } else {
            engine.send(Command::MidiImport(request)).unwrap();
        }
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [])),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].clips[7].notes, notes);
        assert!(rt.tracks[2].clips[6].lanes.is_none() && rt.conductor.is_none());
        assert_eq!(engine.undo.checkpoint(), before);
        if reason == 0 {
            assert_eq!(ack.state(), Outcome::Cancelled);
        } else if reason != 3 {
            assert_eq!(ack.state(), Outcome::Rejected);
        }
    }
}

#[test]
fn file_clock_uses_integer_microseconds_and_reports_meter_without_clamping() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let (request, _) = request(&engine, &mut rt, TempoChoice::File(None));
    engine.send(Command::MidiImport(request)).unwrap();
    rt.process(&mut []);
    let conductor = rt.conductor.as_ref().unwrap().clone();
    assert_eq!(conductor.click_between(31.999, 32.0), None);
    assert_eq!(conductor.click_between(32.0, 32.001), Some(true));
    assert_eq!(conductor.click_between(32.5, 32.501), Some(false));
    assert_eq!(conductor.click_between(35.5, 35.501), Some(true));
    assert_eq!(conductor.click_between(35.75, 35.751), None);
    assert_eq!(conductor.position(32.0).0, 9);
    assert_eq!(conductor.position(35.5).0, 10);
    let (bar, beat, meter) = conductor.position(35.75);
    assert_eq!(
        (bar, beat, meter.numerator, meter.denominator_power),
        (10, 0.5, 7, 3)
    );
    rt.beat = 31.99;
    rt.sync_midi_clock();
    let seconds = conductor.seconds_at(rt.beat);
    rt.playing = true;
    let mut out = vec![0.0; 1001 * 2];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        test_alloc::Counts::default()
    );
    let expected = conductor.beat_at_seconds(seconds + 1001.0 / 48000.0);
    assert!((rt.precise_midi_beat() - expected).abs() < 1e-12);
    assert_eq!(conductor.micros_at(rt.beat), 666667);
    assert!((rt.bpm - (60000000.0 / 666667.0) as f32).abs() < 1e-6);
    engine.send(Command::SetBpm(125.0)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert!(rt.conductor.is_none());
    engine.send(Command::Undo).unwrap();
    rt.process(&mut []);
    assert_eq!(rt.conductor.as_ref(), Some(&conductor));
}

#[test]
fn channel_mapping_merge_conflicts_and_export_precision_choices_are_explicit() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let mut file = fixture();
    assert_eq!(
        sources(&file, true)
            .iter()
            .map(|s| s.channel)
            .collect::<Vec<_>>(),
        vec![None, Some(0), Some(2)]
    );
    let rows = sources(&file, true)
        .into_iter()
        .enumerate()
        .map(|(i, source)| Mapping {
            source,
            destination: Some((4, i as u16)),
        })
        .collect::<Vec<_>>();
    let (request, ack) = Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &rows,
        true,
        false,
        TempoChoice::KeepSession,
        false,
        false,
    )
    .unwrap();
    engine.send(Command::MidiImport(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(rt.tracks[4].clips[1].notes.iter().all(|n| n.channel == 0));
    assert!(rt.tracks[4].clips[2].notes.iter().all(|n| n.channel == 2));
    let state = capture(&engine, &mut rt).state;
    let mut bad = options();
    bad.ppqn = 480;
    assert!(export(&state, &[(4, 1)], bad)
        .unwrap_err()
        .contains("source tick"));
    bad.allow_rounding = true;
    assert!(export(&state, &[(4, 1)], bad).is_ok());
    file.tracks[1].meta.push(smf::Meta {
        tick: 0,
        order: 999,
        value: MetaValue::Tempo(400000),
    });
    assert!(Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &mapping(&file),
        false,
        false,
        TempoChoice::File(None),
        false,
        false
    )
    .is_err());
    assert!(Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &mapping(&file),
        false,
        false,
        TempoChoice::File(Some(0)),
        false,
        false
    )
    .is_ok());
}

#[test]
fn inverse_reservations_over_budget_reject_before_mutation_even_when_payload_fits() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let file = fixture();
    let rows = sources(&file, false)
        .into_iter()
        .map(|source| Mapping {
            source,
            destination: Some((4, 2)),
        })
        .collect::<Vec<_>>();
    let (request, ack) = Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &rows,
        false,
        false,
        TempoChoice::KeepSession,
        false,
        false,
    )
    .unwrap();
    let budget = 2 * project::MAX_NOTES_PER_CLIP * std::mem::size_of::<MidiNote>();
    assert!(request.bytes() < budget);
    let before = engine.undo.checkpoint();
    let notes = rt.tracks[4].clips[2].notes.clone();
    rt.set_undo_budget_for_test(budget);
    engine.send(Command::MidiImport(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(engine.undo.checkpoint(), before);
    assert_eq!(rt.tracks[4].clips[2].notes, notes);
    assert!(rt.tracks[4].clips[2].lanes.is_none());
}

#[test]
fn merge_rescales_existing_ticks_exactly_and_all_source_rows_share_one_cell() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let source = fixture();
    let rows = sources(&source, false)
        .into_iter()
        .map(|source| Mapping {
            source,
            destination: Some((4, 2)),
        })
        .collect::<Vec<_>>();
    let (first, _) = Request::prepare(
        capture(&engine, &mut rt),
        &source,
        &rows,
        false,
        false,
        TempoChoice::KeepSession,
        false,
        false,
    )
    .unwrap();
    engine.send(Command::MidiImport(first)).unwrap();
    rt.process(&mut []);
    let before = rt.tracks[4].clips[2].notes.clone();
    let incoming = smf::File {
        format: smf::Format::Single,
        ppqn: 1920,
        warnings: vec![],
        tracks: vec![smf::Track {
            end_tick: 128 * 1920,
            notes: vec![smf::Note {
                channel: 15,
                pitch: 100,
                velocity: 99,
                release_velocity: 23,
                start_tick: 3,
                duration_ticks: 1919,
                start_order: 1,
                end_order: 2,
            }],
            ..Default::default()
        }],
    };
    let rows = vec![Mapping {
        source: sources(&incoming, false)[0],
        destination: Some((4, 2)),
    }];
    let (merged, ack) = Request::prepare(
        capture(&engine, &mut rt),
        &incoming,
        &rows,
        false,
        true,
        TempoChoice::KeepSession,
        false,
        false,
    )
    .unwrap();
    engine.send(Command::MidiImport(merged)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    let clip = &rt.tracks[4].clips[2];
    assert_eq!(clip.notes.len(), 65);
    assert_eq!(clip.lanes.as_ref().unwrap().end_tick, 128 * 1920);
    assert_eq!(clip.lanes.as_ref().unwrap().messages.len(), 11);
    for original in &before {
        let note = clip
            .notes
            .iter()
            .find(|n| {
                n.channel == original.channel
                    && n.pitch == original.pitch
                    && n.source_start() == original.source_start()
            })
            .unwrap();
        let timing = note.source_timing.unwrap();
        let previous = original.source_timing.unwrap();
        assert_eq!(
            (timing.ppqn, timing.start, timing.duration),
            (1920, previous.start * 2, previous.duration * 2)
        );
    }
    engine.send(Command::Undo).unwrap();
    rt.process(&mut []);
    assert_eq!(rt.tracks[4].clips[2].notes, before);
    engine.send(Command::Redo).unwrap();
    rt.process(&mut []);
    assert_eq!(rt.tracks[4].clips[2].notes.len(), 65);
}

#[test]
fn full_sixteen_bar_fixture_renders_every_exact_gate_across_the_tempo_change() {
    use super::super::midi_schedule::Gate;
    for rate in [48000, 96000] {
        let (engine, mut rt) = Engine::headless_for_test(rate, 256);
        let (request, ack) = request(&engine, &mut rt, TempoChoice::File(None));
        engine.send(Command::MidiImport(request)).unwrap();
        rt.process(&mut []);
        assert_eq!(ack.state(), Outcome::Applied);
        rt.quant = 0.0;
        rt.metronome = true;
        let conductor = rt.conductor.clone().unwrap();
        rt.begin_midi_trace_for_test(2);
        engine
            .send(Command::FireClip {
                track: 2,
                scene: 7,
                looping: false,
            })
            .unwrap();
        rt.process(&mut []);
        let frames = (conductor.seconds_at(65.0) * f64::from(rate)).ceil() as usize;
        let mut output = vec![0.0; 257 * 2];
        assert_eq!(
            test_alloc::measure(|| {
                for _ in 0..frames.div_ceil(257) {
                    rt.process(&mut output);
                }
            }),
            test_alloc::Counts::default()
        );
        let actual = rt.tracks[2]
            .midi_schedule
            .trace
            .as_mut()
            .unwrap()
            .drain(..)
            .map(|(elapsed, gate)| {
                let frame = (conductor.seconds_at(elapsed) * f64::from(rate) - 1.0)
                    .round()
                    .max(0.0) as u64;
                match gate {
                    Gate::On(p, v) => (frame, true, p, v),
                    Gate::Off(p) => (frame, false, p, 0),
                }
            })
            .collect::<Vec<_>>();
        let mut expected = Vec::new();
        for note in &fixture().tracks[1].notes {
            for (tick, on, velocity) in [
                (note.start_tick, true, note.velocity),
                (note.start_tick + note.duration_ticks, false, 0),
            ] {
                // Compute seconds independently from the conductor implementation.
                let before = tick.min(30720) as f64 * 500000.0 / (960.0 * 1000000.0);
                let after = tick.saturating_sub(30720) as f64 * 666667.0 / (960.0 * 1000000.0);
                let frame = ((before + after) * f64::from(rate) + 1e-7).floor() as u64;
                expected.push((frame, on, note.pitch, velocity));
            }
        }
        expected.sort_by_key(|gate| (gate.0, gate.1));
        assert_eq!(actual, expected, "All 128 gates at {rate} Hz");
        assert_eq!(rt.conductor.as_ref().unwrap().position(64.0).0, 18);
    }
}

#[test]
fn large_text_lane_survives_native_codec_above_the_previous_eight_mib_limit() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let mut file = fixture();
    file.tracks[0].meta.push(smf::Meta {
        tick: 2,
        order: 1000,
        value: MetaValue::Text {
            kind: 1,
            bytes: vec![255; 2_200_000],
        },
    });
    let (request, ack) = Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &mapping(&file),
        false,
        false,
        TempoChoice::File(None),
        false,
        false,
    )
    .unwrap();
    engine.send(Command::MidiImport(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    let captured = capture(&engine, &mut rt);
    let json = serde_json::to_vec(&captured.state).unwrap();
    assert!(
        json.len() > 8 * 1024 * 1024 && json.len() < crate::project_file::DEFAULT_METADATA_LIMIT
    );
    let bundle = crate::project_file::Bundle {
        state: captured.state,
        media: captured.media,
    };
    let cancel = AtomicBool::new(false);
    struct NativeFile(std::path::PathBuf);
    impl Drop for NativeFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let path = NativeFile(std::env::temp_dir().join(format!(
        "omat-midi-large-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    )));
    crate::project_file::save(
        &path.0,
        &bundle,
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &cancel,
    )
    .unwrap();
    let decoded: crate::project_file::Bundle<project::State> =
        crate::project_file::load(&path.0, &crate::project_file::Limits::default(), &cancel)
            .unwrap();
    let text = &decoded.state.tracks[2].clips[6]
        .lanes
        .as_ref()
        .unwrap()
        .meta;
    assert!(text.iter().any(|m| matches!(&m.value, MetaValue::Text { kind: 1, bytes } if bytes.len() == 2_200_000 && bytes.iter().all(|b| *b == 255))));
    project::Prepared::from_state(decoded.state, decoded.media, 48000).unwrap();
}

#[test]
fn adapter_cancellation_inside_mapping_and_export_returns_no_partial_request() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let file = fixture();
    let before = capture(&engine, &mut rt);
    let checkpoint = before.checkpoint;
    let mut polls = 0;
    let error = Request::prepare_with_cancel(
        before,
        &file,
        &mapping(&file),
        false,
        false,
        TempoChoice::File(None),
        false,
        false,
        || {
            polls += 1;
            polls == 25
        },
    );
    assert!(error.unwrap_err().contains("cancelled"));
    assert_eq!(polls, 25);
    assert_eq!(engine.undo.checkpoint(), checkpoint);
    let (request, ack) = request(&engine, &mut rt, TempoChoice::File(None));
    engine.send(Command::MidiImport(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    let state = capture(&engine, &mut rt).state;
    polls = 0;
    let error = export_with_cancel(&state, &[(2, 6), (2, 7)], options(), || {
        polls += 1;
        polls == 20
    });
    assert!(error.err().unwrap().contains("cancelled"));
    assert_eq!(polls, 20);
    assert_eq!(
        export(&state, &[(2, 6), (2, 7)], options()).unwrap().tracks[1]
            .notes
            .len(),
        64
    );
}

#[test]
fn all_sixty_four_destinations_plus_conductor_are_one_bounded_inverse() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let source = fixture();
    let one = smf::Track {
        end_tick: 960,
        notes: vec![smf::Note {
            channel: 0,
            pitch: 64,
            velocity: 80,
            release_velocity: 17,
            start_tick: 1,
            duration_ticks: 479,
            start_order: 100,
            end_order: 101,
        }],
        messages: vec![],
        meta: vec![],
    };
    let mut file = smf::File {
        format: smf::Format::Parallel,
        ppqn: 960,
        tracks: vec![one; 64],
        warnings: vec![],
    };
    file.tracks[0].meta = source.tracks[0].meta.clone();
    file.tracks[0].end_tick = source.tracks[0].end_tick;
    let mappings = sources(&file, false)
        .into_iter()
        .enumerate()
        .map(|(i, source)| Mapping {
            source,
            destination: Some(((i / 8) as u8, (i % 8) as u16)),
        })
        .collect::<Vec<_>>();
    let before = capture(&engine, &mut rt);
    let cursor = engine.undo.view().cursor;
    let (request, ack) = Request::prepare(
        capture(&engine, &mut rt),
        &file,
        &mappings,
        false,
        false,
        TempoChoice::File(None),
        false,
        false,
    )
    .unwrap();
    engine.send(Command::MidiImport(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(engine.undo.view().items[cursor].unwrap().patches, 65);
    assert_eq!(
        rt.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.notes.len())
            .sum::<usize>(),
        64
    );
    engine.send(Command::Undo).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    let undone = capture(&engine, &mut rt);
    for t in 0..TRACKS {
        for s in 0..SCENES {
            assert_eq!(
                undone.state.tracks[t].clips[s].notes,
                before.state.tracks[t].clips[s].notes
            );
            assert_eq!(
                undone.state.tracks[t].clips[s].lanes,
                before.state.tracks[t].clips[s].lanes
            );
        }
    }
    assert!(undone.state.conductor.is_none());
}
