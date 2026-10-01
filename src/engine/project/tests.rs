use super::*;
use super::model::*;

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
            return Captured { state: frame.state, media: frame.media, revision: frame.revision, playback_receipts: frame.playback_receipts };
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
        rt.tracks[t].fx.slots = fx::FxId::all().iter().enumerate().map(|(i,id)| {
            let mut slot = fx::FxSlot::new(*id, rt.sr); slot.mix = 0.25;
            slot.on = i % 2 == 0; slot.p = [0.2, 0.3, 0.4, 0.5]; slot
        }).collect();
        for s in 0..SCENES {
            rt.tracks[t].clips[s] = Clip { kind: if s == 7 { ClipKind::Audio } else { ClipKind::Midi },
                name: format!("clip {t}:{s}"), bars: 2.0, gain: 0.7,
                notes: vec![MidiNote { pitch: (30+t+s) as u8, start: 1.125, len: 0.75, vel: 93 }],
                audio: (s == 7).then(|| rt.builtin[0].as_ref().unwrap().clone()) };
        }
        rt.scene_fx[t].slots.push(fx::FxSlot::new(fx::FxId::Delay, rt.sr));
    }
    rt.decks[0].hotcues[3] = HotCue { set: true, pos: 12345.125 };
    rt.decks[0].pos = 1987.5; rt.decks[0].cue_pos = 123.75;
    rt.decks[1].loop_on = true; rt.decks[1].loop_start = 321.125; rt.decks[1].loop_len = 4321.5;
    rt.decks[1].eq_cut[1] = true; rt.decks[1].eq_store[1] = 1.75;
    rt.sampler_bank = 2; rt.sampler_inst = SamplerInstrument::Synth(SynthInstrument::Pad); rt.sampler_oct = 5;
    rt.sampler_poly.select_instrument(SynthInstrument::Pad);
    rt.fx_wet = [0.1, 0.2, 0.3]; rt.selected_track = 7; rt.selected_scene = 6; rt.selected_deck = 1;
    rt.view = View::Compose; rt.fx_view = 103;
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
    assert_eq!(again.media.iter().map(|m| m.data.len()).sum::<usize>(), bytes);
    assert!(!restored.rt.playing && restored.rt.decks.iter().all(|d| !d.playing && !d.touching));
    assert!(again.playback_receipts.iter().all(|r| r.as_ref().unwrap().state() == load_receipt::State::Current));
}

#[test]
fn capture_and_install_boundary_allocate_and_free_nothing() {
    let mut live = populated();
    let mut frame = capture::Frame::new();
    let needed = super::super::test_alloc::measure(|| frame.capture(&live));
    assert_eq!(needed, super::super::test_alloc::Counts::default());
    assert!(!frame.complete); frame.prepare();
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
    let mut state = saved.state.clone(); state.version += 1;
    assert!(state.validate(&saved.media).unwrap_err().contains("version"));
    let mut state = saved.state.clone(); state.bpm = f32::NAN;
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone(); state.tracks[0].drums[0] = saved.media.len();
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone(); state.decks[0].hotcues[0] = Some(f64::INFINITY);
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone(); state.tracks[0].clips[0].notes[0].pitch = 255;
    assert!(state.validate(&saved.media).is_err());
    let mut state = saved.state.clone(); state.tracks[0].name = "x".repeat(MAX_TEXT_BYTES+1);
    assert!(state.validate(&saved.media).is_err());
    let mut json = serde_json::to_value(&saved.state).unwrap(); json["future_automation"] = serde_json::json!([1,2]);
    assert!(serde_json::from_value::<State>(json).is_err());
}
