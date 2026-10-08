use super::*;

fn placeholder(identifier: &str, state: Option<fx::DeviceState>) -> Effect {
    Effect {
        id: fx::FxId::Unavailable,
        on: true,
        mix: 0.75,
        p: [0.25, 0.5, 0.0, 0.0],
        offline: Some(Arc::new(fx::OfflineDevice::new(identifier.into(), state).unwrap())),
    }
}

#[test]
fn unavailable_effect_state_survives_native_file_and_snapshot_without_callback_heap() {
    let mut saved = captured(&rt());
    let name = "org.omatainer.missing-device/".to_owned() + &"x".repeat(512);
    let effect = placeholder(&name, Some(fx::DeviceState { schema: 7, data: vec![0, 255, 1, 128, 42] }));
    saved.state.tracks[2].fx.push(effect);
    let expected = serde_json::to_value(&saved.state).unwrap();
    let notes = saved.state.tracks[2].clips[0].notes.clone();
    let directory = std::env::temp_dir().join(format!("omatainer-offline-{}", crate::sampler_bank::BankId::new().unwrap()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("offline.omat");
    crate::project_file::save(&path, &crate::project_file::Bundle { state: saved.state, media: saved.media },
        crate::project_file::Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
    let reopened: crate::project_file::Bundle<State> = crate::project_file::load(&path, &Default::default(), &AtomicBool::new(false)).unwrap();
    assert_eq!(serde_json::to_value(&reopened.state).unwrap(), expected);
    let mut prepared = Prepared::from_state(reopened.state, reopened.media, 48000).unwrap();
    let slot = &mut prepared.rt.tracks[2].fx.slots[0];
    assert_eq!(slot.id(), fx::FxId::Unavailable);
    assert_eq!(slot.offline.as_ref().unwrap().identifier, name);
    assert_eq!(slot.offline.as_ref().unwrap().state.as_ref().unwrap().data, vec![0, 255, 1, 128, 42]);
    assert_eq!(test_alloc::measure(|| {
        for _ in 0..1024 { assert_eq!(slot.tick_stereo([0.2, -0.3], 48000.0), [0.2, -0.3]); }
    }), test_alloc::Counts::default());
    assert_eq!(prepared.rt.tracks[2].clips[0].notes, notes);
    prepared.rt.fx_view = 2;
    prepared.rt.publish_for_test();
    assert_eq!(prepared.rt.snap.lock().fx_slots[0].0, format!("Unavailable · {name}"));
    assert_eq!(test_alloc::measure(|| prepared.rt.publish()), test_alloc::Counts::default());
    let recaptured = captured(&prepared.rt);
    assert_eq!(serde_json::to_value(&recaptured.state).unwrap()["tracks"][2]["fx"], expected["tracks"][2]["fx"]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn returning_compatible_device_restores_real_processing_and_unsupported_state_stays_offline() {
    let wire = serde_json::to_value(placeholder("delay", None)).unwrap();
    let restored: Effect = serde_json::from_value(wire).unwrap();
    assert_eq!(restored.id, fx::FxId::Delay);
    assert!(restored.offline.is_none());
    let mut saved = captured(&rt());
    saved.state.tracks[2].fx.push(restored);
    let mut prepared = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    let actual = &mut prepared.rt.tracks[2].fx.slots[0];
    let mut expected = fx::FxSlot::new(fx::FxId::Delay, 48000.0);
    expected.on = true; expected.mix = 0.75; expected.p = [0.25, 0.5, 0.0, 0.0];
    let mut tail = false;
    for i in 0..13000 {
        let input = if i == 0 { [0.8, -0.4] } else { [0.0; 2] };
        let got = actual.tick_stereo(input, 48000.0);
        assert_eq!(got, expected.tick_stereo(input, 48000.0));
        tail |= i > 100 && got != [0.0; 2];
    }
    assert!(tail, "restored device must execute its real delay processor");
    let unsupported = placeholder("delay", Some(fx::DeviceState { schema: 999, data: vec![1, 2, 3] }));
    let wire = serde_json::to_value(unsupported).unwrap();
    let retained: Effect = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(retained.id, fx::FxId::Unavailable);
    assert_eq!(serde_json::to_value(retained).unwrap(), wire);
}

#[test]
fn legacy_unknown_devices_and_oversized_serialized_state_refuse_before_installation() {
    let saved = captured(&rt());
    for version in [7, 8] {
        for effect in [
            serde_json::json!({"id":"missing", "on":true, "mix":0.5, "p":[0.0,0.0,0.0,0.0]}),
            serde_json::json!({"id":"delay", "on":true, "mix":0.5, "p":[0.0,0.0,0.0,0.0], "state":null}),
        ] {
            let mut raw = serde_json::to_value(&saved.state).unwrap();
            raw["version"] = version.into(); raw["tracks"][2]["fx"] = serde_json::json!([effect]);
            assert!(serde_json::from_value::<State>(raw).is_err());
        }
    }
    for version in [7, 8] {
        let mut raw = serde_json::to_value(&saved.state).unwrap(); raw["version"] = version.into();
        raw["sampler_synth"]["kind"] = "unknown-instrument".into();
        assert!(serde_json::from_value::<State>(raw).is_err());
        let mut raw = serde_json::to_value(&saved.state).unwrap(); raw["version"] = version.into();
        raw["tracks"][2]["synth"]["state"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<State>(raw).is_err());
    }
    assert!(fx::OfflineDevice::new("missing".into(), Some(fx::DeviceState {schema:1, data:vec![0;1024*1024+1]})).is_err());
    assert!(fx::OfflineDevice::new("bad\nidentifier".into(), None).is_err());
    let mut invalid = saved.state;
    invalid.version = 8; invalid.tracks[2].fx.push(placeholder("missing", None));
    assert!(invalid.validate(&saved.media).is_err());
}

#[test]
fn unavailable_instruments_preserve_state_use_rendered_fallback_and_restore_compatible_synths() {
    let mut original = rt();
    let device = Arc::new(fx::OfflineDevice::new("org.example.instrument".into(), Some(fx::DeviceState {schema: 2, data: vec![255, 0, 42]})).unwrap());
    original.tracks[2].poly.offline = Some(device.clone());
    let fallback = Arc::new(Sample { spectrum: None, name: "Rendered instrument".into(), sr: 48000, ch: 2,
        data: (0..1024).flat_map(|i| [i as f32 / 2048.0, -(i as f32) / 4096.0]).collect(),
        peaks: Arc::new(Vec::new()), bpm: 120.0, path: "rendered".into() });
    original.tracks[2].clips[0].audio = Some(fallback.clone());
    original.tracks[2].clips[0].lanes = Some(midi_data::Lanes::new(960, 15360,
        vec![crate::midi_file::Message { tick: 960, order: 0, bytes: [0xb0, 74, 91], length: 3 }], vec![]).unwrap());
    let saved = captured(&original); let expected = serde_json::to_value(&saved.state).unwrap();
    let decoded: State = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    let report = crate::project_dependencies::inspect(&decoded, &saved.media, &[], &AtomicBool::new(false)).unwrap();
    assert!(report.devices.iter().any(|entry| entry.identifier == device.identifier && entry.rendered_fallbacks == Some(1)));
    let mut prepared = Prepared::from_state(decoded, saved.media, 48000).unwrap();
    let live = &mut prepared.rt; let notes = live.tracks[2].clips[0].notes.clone();
    assert_eq!(serde_json::to_value(&live.tracks[2].clips[0].lanes).unwrap(), expected["tracks"][2]["clips"][0]["lanes"]);
    live.tracks[2].poly.note_on(60, 1.0);
    assert_eq!(test_alloc::measure(|| { for _ in 0..1024 { assert_eq!(live.tracks[2].poly.tick(48000.0), 0.0); } }), test_alloc::Counts::default());
    live.tracks[2].playing = Some(PlayingClip { scene: 0, start_beat: 0.0, midi_start_beat: 0.0, looping: true, last_beat: 0.0 });
    live.beat = 1.0; live.legacy_gain_math = true;
    let expected_pos = (1.0 - midi_schedule::BEAT_EPSILON) / (f64::from(live.tracks[2].clips[0].bars) * 4.0) * fallback.frames() as f64;
    let (l, r) = fallback.at(expected_pos);
    let [gain_l, gain_r] = mixer_gain::pan_gains(live.tracks[2].gain, live.tracks[2].pan);
    let mut left = live.tracks[2].eq; let mut right = live.tracks[2].eq_right;
    let expected = (left.tick(l * clip_gain(live.tracks[2].clips[0].gain)) * gain_l, right.tick(r * clip_gain(live.tracks[2].clips[0].gain)) * gain_r);
    let mut got = (0.0, 0.0, false);
    assert_eq!(test_alloc::measure(|| got = live.render_track(2, false)), test_alloc::Counts::default());
    assert!((got.0 - expected.0).abs() < 1e-6 && (got.1 - expected.1).abs() < 1e-6, "{got:?} {expected:?}");
    assert_ne!((got.0, got.1), (0.0, 0.0));
    assert_eq!(live.tracks[2].clips[0].notes, notes);
    live.tracks[2].mute = true; assert_eq!(live.render_track(2, false), (0.0, 0.0, false));
    live.tracks[2].mute = false;
    live.conductor = Some(midi_data::Conductor::native(960, vec![midi_data::Tempo::new(0, 120.0, false).unwrap()], vec![midi_data::Meter { tick: 0, numerator: 4, denominator_power: 2, clocks: 24, thirty_seconds: 8 }], midi_data::TimingSettings { count_in: 1, ..Default::default() }).unwrap());
    live.start_count_in(); assert!(live.count_in.is_some());
    assert_eq!(live.render_track(2, false), (0.0, 0.0, false));
    let mut wire = serde_json::to_value(&captured(&rt()).state.tracks[2].synth).unwrap();
    wire["kind"] = "keys".into();
    let restored: Synth = serde_json::from_value(wire.clone()).unwrap(); assert!(restored.offline.is_none());
    let mut saved = captured(&rt()); saved.state.tracks[2].synth = restored;
    let mut real = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    let value = &mut real.rt.tracks[2].poly;
    let mut reference = Poly::new(48000.0, SynthInstrument::Keys, 8); reference.cutoff = value.cutoff;
    value.note_on(60, 0.75); reference.note_on(60, 0.75);
    let mut audible = false;
    for _ in 0..1024 { let output = value.tick(48000.0); assert_eq!(output, reference.tick(48000.0)); audible |= output != 0.0; }
    assert!(audible);
    wire["state"] = serde_json::json!({"schema": 999, "data": [1, 2, 3]});
    let unsupported: Synth = serde_json::from_value(wire.clone()).unwrap(); assert!(unsupported.offline.is_some());
    assert_eq!(serde_json::to_value(unsupported).unwrap(), wire);
}
