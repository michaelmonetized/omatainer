use super::*;
use crate::engine::{
    midi_edit::{Document, Outcome, Request},
    test_alloc, Command, Engine, RtEngine, SamplerInstrument, SynthInstrument,
};
use std::sync::{atomic::AtomicBool, Arc};

fn captured(engine: &Engine, rt: &mut RtEngine) -> crate::engine::project::Captured {
    let handle = engine.project.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        sender
            .send(handle.capture(&AtomicBool::new(false)))
            .unwrap()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        rt.process(&mut []);
        match receiver.try_recv() {
            Ok(result) => return result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(error) => panic!("Actual project capture disconnected: {error}"),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Actual project capture did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
fn tick(rt: &mut RtEngine) {
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
}

#[test]
fn song_key_and_instrument_participation_undo_without_rewriting_notes_or_held_pitches() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
    let original_notes: Vec<Vec<Vec<_>>> = rt.tracks.iter().map(|track| track.clips.iter().map(|clip| clip.notes.clone()).collect()).collect();
    let song = Context {
        tonic: 0,
        scale: Scale::Dorian,
    };
    let other = Context {
        tonic: 2,
        scale: Scale::Phrygian,
    };
    engine.send(Command::SongContext(Some(song))).unwrap();
    tick(&mut rt);
    engine.send(Command::SamplerScale(true)).unwrap();
    tick(&mut rt);
    engine
        .send(Command::SamplerInst(SamplerInstrument::Synth(
            SynthInstrument::Keys,
        )))
        .unwrap();
    tick(&mut rt);
    engine
        .send(Command::SamplerPad { pad: 1, on: true })
        .unwrap();
    tick(&mut rt);
    assert_eq!(
        rt.pad_targets[1].unwrap().pitch,
        pad_pitch(song, rt.sampler_oct, 1).unwrap()
    );
    let held = rt.pad_targets[1].unwrap().pitch;
    engine.send(Command::SongContext(Some(other))).unwrap();
    tick(&mut rt);
    assert_eq!(rt.pad_targets[1].unwrap().pitch, held);
    engine
        .send(Command::SamplerPad { pad: 2, on: true })
        .unwrap();
    tick(&mut rt);
    assert_eq!(
        rt.pad_targets[2].unwrap().pitch,
        pad_pitch(other, rt.sampler_oct, 2).unwrap()
    );
    engine.send(Command::Undo).unwrap();
    tick(&mut rt);
    assert_eq!(rt.musical_context, Some(song));
    assert_eq!(rt.pad_targets[1].unwrap().pitch, held);
    engine.send(Command::Redo).unwrap();
    tick(&mut rt);
    assert_eq!(rt.musical_context, Some(other));
    engine
        .send(Command::SamplerPad { pad: 1, on: false })
        .unwrap();
    tick(&mut rt);
    assert!(rt.pad_targets[1].is_none());
    for scale in Scale::ALL {
        for tonic in 0..12 {
            let context = Context { tonic, scale };
            for pad in 0..16 {
                if let Some(pitch) = pad_pitch(context, 3, pad) {
                    assert!(context.contains(pitch));
                }
            }
        }
    }
    let song_notes: Vec<Vec<Vec<_>>> = captured(&engine, &mut rt).state.tracks.iter().map(|track| track.clips.iter().map(|clip| clip.notes.clone()).collect()).collect();
    assert_eq!(song_notes, original_notes);
    engine
        .send(Command::SongContext(Some(Context {
            tonic: 12,
            scale: Scale::Major,
        })))
        .unwrap();
    tick(&mut rt);
    assert_eq!(rt.musical_context, Some(other));
    println!("MIDI_SCALE_INSTRUMENT {{\"actual_synth_gates\":true,\"held_pitch_retained\":true,\"callback_allocations\":0,\"callback_frees\":0,\"physical_devices_opened\":false}}");
}

#[test]
fn captured_clip_and_song_keys_guard_atomic_apply_cancel_and_one_allocation_free_undo() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
    let dorian = Context {
        tonic: 0,
        scale: Scale::Dorian,
    };
    let phrygian = Context {
        tonic: 2,
        scale: Scale::Phrygian,
    };
    let base = Document::capture(captured(&engine, &mut rt), 2, 7).unwrap();
    let (request, ack, next) = Request::with_context(
        base.clone(),
        "Scale clip".into(),
        base.playback_region(),
        vec![],
        None,
        Some(dorian),
    )
    .unwrap();
    engine.send(Command::MidiEdit(request)).unwrap();
    tick(&mut rt);
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.tracks[2].clips[7].properties.context, Some(dorian));
    assert_eq!(next.context, Some(dorian));
    engine.send(Command::Undo).unwrap();
    tick(&mut rt);
    assert_eq!(rt.tracks[2].clips[7].properties.context, None);
    engine.send(Command::Redo).unwrap();
    tick(&mut rt);
    assert_eq!(rt.tracks[2].clips[7].properties.context, Some(dorian));
    for stale_song in [true, false] {
        let base = Document::capture(captured(&engine, &mut rt), 2, 7).unwrap();
        let (request, ack, _) = Request::with_context(
            base.clone(),
            base.name.clone(),
            base.playback_region(),
            base.notes.clone(),
            base.lanes.clone(),
            Some(phrygian),
        )
        .unwrap();
        if stale_song {
            engine.send(Command::SongContext(Some(dorian))).unwrap();
            tick(&mut rt);
        } else {
            rt.tracks[2].clips[7].properties.context = None;
        }
        engine.send(Command::MidiEdit(request)).unwrap();
        tick(&mut rt);
        assert_eq!(ack.state(), Outcome::Rejected);
        assert_ne!(rt.tracks[2].clips[7].properties.context, Some(phrygian));
    }
    let base = Document::capture(captured(&engine, &mut rt), 2, 7).unwrap();
    let (request, ack, _) = Request::with_context(
        base.clone(),
        base.name.clone(),
        base.playback_region(),
        base.notes.clone(),
        base.lanes.clone(),
        Some(phrygian),
    )
    .unwrap();
    assert!(ack.cancel());
    engine.send(Command::MidiEdit(request)).unwrap();
    tick(&mut rt);
    assert_eq!(ack.state(), Outcome::Cancelled);
    assert_eq!(rt.tracks[2].clips[7].properties.context, None);
}

#[test]
fn native_metadata_retains_clip_song_keys_and_refuses_legacy_future_fields_or_invalid_tonics() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
    rt.musical_context = Some(Context {
        tonic: 1,
        scale: Scale::HarmonicMinor,
    });
    rt.sampler_scale = true;
    rt.tracks[2].clips[7].properties.context = Some(Context {
        tonic: 5,
        scale: Scale::Phrygian,
    });
    let saved = captured(&engine, &mut rt);
    let bytes = serde_json::to_vec(&saved.state).unwrap();
    let decoded: crate::engine::project::State = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.musical_context, saved.state.musical_context);
    assert!(decoded.sampler_scale);
    assert_eq!(
        decoded.tracks[2].clips[7].properties.context,
        rt.tracks[2].clips[7].properties.context
    );
    let prepared =
        crate::engine::project::Prepared::from_state(decoded, saved.media.clone(), 48_000).unwrap();
    assert_eq!(prepared.rt.musical_context, rt.musical_context);
    assert_eq!(
        prepared.rt.tracks[2].clips[7].properties.context,
        rt.tracks[2].clips[7].properties.context
    );
    let mut old: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    old["version"] = 32.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(old.clone()).is_err());
    old.as_object_mut().unwrap().remove("musical_context");
    old.as_object_mut().unwrap().remove("sampler_scale");
    assert!(serde_json::from_value::<crate::engine::project::State>(old.clone()).is_err());
    old["tracks"][2]["clips"][7]["properties"]
        .as_object_mut()
        .unwrap()
        .remove("context");
    let legacy: crate::engine::project::State = serde_json::from_value(old).unwrap();
    legacy.validate(&saved.media).unwrap();
    assert_eq!(legacy.musical_context, None);
    assert!(!legacy.sampler_scale);
    let mut invalid = saved.state;
    invalid.musical_context.as_mut().unwrap().tonic = 12;
    assert!(invalid.validate(&saved.media).is_err());
    println!("MIDI_SCALE_METADATA {{\"native_state_roundtrip\":true,\"prepared_renderer_context\":true,\"legacy_refusal\":true,\"physical_devices_opened\":false}}");
}
