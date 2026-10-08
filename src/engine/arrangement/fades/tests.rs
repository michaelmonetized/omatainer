use super::*;
use crate::engine::{
    audio_clip::Fades,
    midi_edit::Outcome,
    project::{Captured, Prepared},
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::atomic::AtomicBool;

fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let worker = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !worker.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    worker.join().unwrap()
}
fn model(captured: &mut Captured) -> Model {
    let audio = Arc::new(Sample {
        name: "correlated multitrack".into(),
        sr: 48000,
        ch: 2,
        data: (0..192000)
            .flat_map(|i| {
                let a = (i as f32 * 0.041 + 0.7).sin() * 0.25;
                [a, -a * 0.4]
            })
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    });
    let index = captured.media.len();
    captured.media.push(audio.clone());
    let mut clip = captured.state.tracks[0].clips[7].clone();
    clip.kind = ClipKind::Audio;
    clip.name = audio.name.clone();
    clip.notes.clear();
    clip.lanes = None;
    clip.region = None;
    clip.audio = Some(index);
    clip.gain = 1.0;
    let region = audio_clip::Region::full(&audio, 120.0).unwrap();
    clip.audio_region = Some(region);
    clip.bars = (region.prepare(&audio).unwrap().duration_beats / 4.0) as f32;
    let layout = captured.state.session.as_ref().unwrap();
    let mut instances = Vec::new();
    for track in 0..2 {
        for leg in 0..2 {
            instances.push(Instance {
                id: 2 + (track * 2 + leg) as u64,
                source: 1,
                track: layout.reference(session::Axis::Track, track).unwrap(),
                start: 1.0 + leg as f64 * 2.0,
                offset: 1.0 + leg as f64 * 2.0,
                duration: 2.0,
                repeating: false,
                gain: 1.0,
                fades: Some(Fades {
                    automatic: true,
                    ..Default::default()
                }),
                fade_link: 0,
                crossfade: None,
            });
        }
    }
    Model {
        enabled: true,
        next_id: 6,
        sources: vec![Source {
            id: 1,
            clip,
            audio_clock: None,
        }],
        instances,
    }
}
fn json(model: &Model) -> String {
    serde_json::to_string(model).unwrap()
}

#[test]
fn adjacent_overlap_crossfades_keep_correlated_pcm_and_linked_tracks_in_phase_through_edits_and_reorder(
) {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let mut model = model(&mut captured);
    model.link_fades(2, 4, &captured.media).unwrap();
    model.link_fades(3, 5, &captured.media).unwrap();
    let original_source = captured.media[model.sources[0].clip.audio.unwrap()].clone();
    let original_data = original_source.data.clone();
    for length in [0.25, 0.5, 1.0] {
        for curve in [-1.0, 0.0, 1.0] {
            model
                .crossfade(2, 3, length, curve, &captured.media)
                .unwrap();
            let a = model.placement(2).unwrap();
            let b = model.placement(3).unwrap();
            assert!((a.start + a.duration - b.start - length).abs() < 1e-9);
            assert_eq!(a.start, 1.0);
            assert!((b.start + b.duration - 5.0).abs() < 1e-9);
            assert_eq!(a.fades, model.placement(4).unwrap().fades);
            assert_eq!(b.fades, model.placement(5).unwrap().fades);
            let plan = Plan::prepare(
                Arc::new(model.clone()),
                &captured.media,
                captured.state.session.as_ref().unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
            let mut playback = Playback::new(Some(plan), 0.0);
            for output_rate in [8000, 44100, 48000] {
                for frame in 0..(f64::from(output_rate) * length / 2.0) as usize {
                    let beat = b.start + frame as f64 * 2.0 / f64::from(output_rate);
                    playback.reset(beat);
                    let expected = model.sources[0]
                        .clip
                        .audio_region
                        .unwrap()
                        .prepare(&original_source)
                        .unwrap()
                        .sample(&original_source, beat, false);
                    let actual = playback.sample_with_tempo(0, beat, 2.0).0;
                    let linked = playback.sample_with_tempo(1, beat, 2.0).0;
                    assert_eq!(actual, linked);
                    for channel in 0..2 {
                        assert!(
                            (actual[channel] - expected[channel]).abs() < 2e-7,
                            "{length} {curve} {beat}: {actual:?} vs {expected:?}"
                        );
                    }
                }
            }
        }
    }
    let mut replacement = model.placement(2).unwrap();
    replacement.offset += 0.5;
    model.edit_fades(2, replacement, &captured.media).unwrap();
    assert_eq!(model.placement(4).unwrap().offset, replacement.offset);
    let mut layout = captured.state.session.as_ref().unwrap().clone();
    layout.track_order.swap(0, 1);
    model.validate(&captured.media, &layout).unwrap();
    assert_eq!(
        model.placement(2).unwrap().track,
        layout.reference(session::Axis::Track, 0).unwrap()
    );
    assert_eq!(original_source.data, original_data);
}

#[test]
fn linked_geometry_missing_handles_and_pair_conflicts_refuse_the_complete_edit_and_cleanup_retains_envelopes(
) {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let mut model = model(&mut captured);
    model.link_fades(2, 4, &captured.media).unwrap();
    model.link_fades(3, 5, &captured.media).unwrap();
    model
        .instances
        .iter_mut()
        .find(|i| i.id == 5)
        .unwrap()
        .offset = 0.01;
    let before = json(&model);
    assert!(model
        .crossfade(2, 3, 0.5, 0.0, &captured.media)
        .unwrap_err()
        .contains("handles"));
    assert_eq!(json(&model), before);
    model
        .instances
        .iter_mut()
        .find(|i| i.id == 5)
        .unwrap()
        .offset = 3.0;
    model.crossfade(2, 3, 0.5, -0.5, &captured.media).unwrap();
    let before = json(&model);
    let mut changed = model.placement(2).unwrap();
    changed.duration += 0.25;
    assert!(model.edit_fades(2, changed, &captured.media).is_err());
    assert_eq!(json(&model), before);
    changed = model.placement(2).unwrap();
    changed.offset = 7.0;
    assert!(model.edit_fades(2, changed, &captured.media).is_err());
    assert_eq!(json(&model), before);
    model.unlink_crossfade(2);
    model
        .instances
        .iter_mut()
        .find(|i| i.id == 4)
        .unwrap()
        .offset = 0.6;
    let linked_before = model.placement(4).unwrap().offset;
    let mut movement = model.placement(2).unwrap();
    movement.offset += 0.25;
    model.edit_fades(2, movement, &captured.media).unwrap();
    assert_eq!(model.placement(4).unwrap().offset, linked_before + 0.25);
    let fades = model.placement(4).unwrap().fades;
    model.unlink_crossfade(2);
    assert_eq!(model.placement(2).unwrap().crossfade, None);
    assert_eq!(model.placement(2).unwrap().fades, fades);
    model.remove_instance(2);
    assert_eq!(model.placement(4).unwrap().fade_link, 0);
    model
        .validate(&captured.media, captured.state.session.as_ref().unwrap())
        .unwrap();
    assert!(model.link_fades(3, 4, &captured.media).is_err());
}

#[test]
fn prepared_crossfade_edits_are_one_undo_and_cancel_stale_or_playing_transactions_without_callback_heap(
) {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let model = model(&mut captured);
    let (request, ack) = edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
    rt.apply(Command::ArrangementEdit(request));
    assert_eq!(ack.state(), Outcome::Applied);
    rt.process(&mut []);
    let captured = capture(&engine, &mut rt);
    let original = captured.state.arrangement.as_ref().unwrap().clone();
    let mut edited = (*original).clone();
    edited.link_fades(2, 4, &captured.media).unwrap();
    edited.link_fades(3, 5, &captured.media).unwrap();
    edited.crossfade(2, 3, 0.75, 0.4, &captured.media).unwrap();
    let cursor = engine.undo.view().cursor;
    let (request, ack) =
        edit::Request::prepare(captured, edited.clone(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::ArrangementEdit(request))),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    rt.process(&mut []);
    assert_eq!(engine.undo.view().cursor, cursor + 1);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    assert_eq!(
        json(&rt.arrangement.plan.as_ref().unwrap().model),
        json(&original)
    );
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Redo)),
        test_alloc::Counts::default()
    );
    assert_eq!(
        json(&rt.arrangement.plan.as_ref().unwrap().model),
        json(&edited)
    );
    rt.process(&mut []);
    let captured = capture(&engine, &mut rt);
    let (request, ack) =
        edit::Request::prepare(captured, edited.clone(), &AtomicBool::new(false)).unwrap();
    ack.cancel();
    rt.apply(Command::ArrangementEdit(request));
    assert_eq!(ack.state(), Outcome::Cancelled);
    let captured = capture(&engine, &mut rt);
    let (request, ack) =
        edit::Request::prepare(captured, edited.clone(), &AtomicBool::new(false)).unwrap();
    rt.apply(Command::TogglePlay);
    rt.apply(Command::ArrangementEdit(request));
    assert_eq!(ack.state(), Outcome::Rejected);
    rt.apply(Command::Stop);
    let captured = capture(&engine, &mut rt);
    let (request, ack) = edit::Request::prepare(captured, edited, &AtomicBool::new(false)).unwrap();
    rt.apply(Command::Xfader(0.2));
    rt.apply(Command::ArrangementEdit(request));
    assert_eq!(ack.state(), Outcome::Rejected);
}

#[test]
fn real_native_fade_files_and_clip_templates_reopen_and_old_headers_refuse_even_null_fields() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let mut model = model(&mut captured);
    model.link_fades(2, 4, &captured.media).unwrap();
    model.link_fades(3, 5, &captured.media).unwrap();
    model.crossfade(2, 3, 0.5, 0.5, &captured.media).unwrap();
    model.sources[0].clip.audio_region.as_mut().unwrap().fades = Fades {
        fade_in: 0.25,
        fade_out: 0.5,
        in_curve: -0.5,
        out_curve: 0.5,
        automatic: true,
    };
    captured.state.version = project::STATE_VERSION;
    captured.state.arrangement = Some(Arc::new(model.clone()));
    let root = std::env::temp_dir().join(format!(
        "omat-fades-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let bundle = crate::project_file::Bundle {
        state: captured.state,
        media: captured.media,
    };
    let limits = crate::project_file::Limits::default();
    let cancel = AtomicBool::new(false);
    let path = root.join("fades.omat");
    crate::project_file::save(
        &path,
        &bundle,
        crate::project_file::Overwrite::Never,
        &limits,
        &cancel,
    )
    .unwrap();
    let reopened = crate::project_file::load::<project::State>(&path, &limits, &cancel).unwrap();
    assert_eq!(
        json(reopened.state.arrangement.as_ref().unwrap()),
        json(&model)
    );
    let prepared = Prepared::from_state(reopened.state, reopened.media, 48000).unwrap();
    assert!(!prepared.rt.playing);
    assert_eq!(
        json(&prepared.rt.arrangement.plan.as_ref().unwrap().model),
        json(&model)
    );
    let clip = model.sources[0].clip.clone();
    let preset =
        crate::engine::clip_management::preset::Preset::capture(clip, &bundle.media, &cancel)
            .unwrap();
    let path = root.join("fades.omatclip");
    crate::engine::clip_management::preset::write(&preset, &path, &cancel).unwrap();
    let preset = crate::engine::clip_management::preset::read(&path, &cancel).unwrap();
    assert_eq!(
        preset.state.clip.audio_region.unwrap().fades,
        model.sources[0].clip.audio_region.unwrap().fades
    );
    let mut old = serde_json::to_value(&bundle.state).unwrap();
    old["version"] = serde_json::json!(25);
    assert!(serde_json::from_value::<project::State>(old).is_err());
    let mut old = serde_json::to_value(&preset.state).unwrap();
    old["clip_schema"] = serde_json::json!(25);
    assert!(serde_json::from_value::<crate::engine::clip_management::preset::Preset>(old).is_err());
    let mut legacy = serde_json::to_value(&bundle.state).unwrap();
    legacy["version"] = serde_json::json!(25);
    legacy["arrangement"] = serde_json::Value::Null;
    let clip = &mut legacy["tracks"][0]["clips"][7];
    clip["audio_region"] = serde_json::json!({"fades":null});
    assert!(serde_json::from_value::<project::State>(legacy).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
