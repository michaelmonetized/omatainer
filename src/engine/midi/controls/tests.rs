use super::*;
use crate::engine::midi::{MidiMap, RelativeEncoding, RelativeSpec, UnmappedNotes};
use crate::engine::{test_alloc, Engine};
use parking_lot::Mutex;
use std::sync::Arc;

fn binding(kind: MsgKind, action: Action) -> Binding {
    Binding {
        kind,
        ch: 3,
        data: 7,
        action,
        deck: 0,
        extra: 2,
        relative: (kind == MsgKind::CcRel).then_some(RelativeSpec {
            encoding: RelativeEncoding::SignedBit,
            scale: 0.01,
        }),
        controls: None,
        pair_order: None,
    }
}
fn map(binding: Binding) -> MidiMap {
    MidiMap {
        name: "Encoder fixture".into(),
        matchers: vec![],
        bindings: vec![binding],
        unmapped_notes: UnmappedNotes::Ignore,
    }
}
#[test]
fn complete_cc_pairs_preserve_every_wire_value_in_both_orders_and_separate_channels() {
    let mut pairs = Pairs::default();
    let now = Instant::now();
    for value in 0..=16383u16 {
        let msb = [0xb3, 7, (value >> 7) as u8];
        let lsb = [0xb3, 39, (value & 127) as u8];
        for (order, messages) in [
            (PairOrder::MsbFirst, [msb, lsb]),
            (PairOrder::LsbFirst, [lsb, msb]),
        ] {
            pairs.clear();
            let first = pairs.input(&messages[0], now);
            assert!(!first.complete(Some(order)));
            assert_eq!(
                first.value(Some(order)),
                (order == PairOrder::MsbFirst).then_some(value & !127)
            );
            pairs.input(&[0xb4, 7, 117], now);
            pairs.input(&[0xb3, 8, 19], now);
            let complete = pairs.input(&messages[1], now);
            assert!(complete.complete(Some(order)));
            assert_eq!(complete.value(Some(order)), Some(value));
        }
    }
    pairs.clear();
    assert_eq!(pairs.input(&[0xb3, 7, 127], now).value(None), Some(16256));
    let late = pairs.input(&[0xb3, 39, 5], now + Duration::from_millis(1001));
    assert_eq!(late.value(None), Some(16261));
    assert!(!late.complete(None));
    assert_eq!(
        pairs
            .input(&[0xb3, 7, 1], now + Duration::from_millis(2002))
            .value(Some(PairOrder::LsbFirst)),
        None
    );
    pairs.clear();
    assert_eq!(pairs.input(&[0xb3, 39, 5], now).value(None), None);
    assert_eq!(
        pairs
            .input(&[0xb3, 7, 2], now)
            .value(Some(PairOrder::LsbFirst)),
        Some(261)
    );
    for msg in [[0xb3, 64, 99], [0x93, 7, 99], [0xb3, 7, 128]] {
        assert_eq!(pairs.input(&msg, now).value(None), None);
    }
    pairs.clear();
    pairs.input(&[0xb3, 7, 2], now);
    pairs.input(&[0xb3, 39, 5], now);
    assert_eq!(
        pairs
            .input(&[0xb3, 39, 6], now + Duration::from_secs(30))
            .value(None),
        Some(262)
    );
    assert_eq!(
        pairs
            .input(&[0xb3, 7, 3], now + Duration::from_secs(31))
            .value(None),
        Some(384)
    );
    pairs.input(&[0xb3, 121, 0], now + Duration::from_secs(32));
    assert_eq!(
        pairs
            .input(&[0xb3, 39, 7], now + Duration::from_secs(33))
            .value(None),
        None
    );
}
#[test]
fn full_cc_and_pitch_ranges_are_monotonic_centered_invertible_and_limited() {
    let (cmd, rx) = crate::engine::CommandPort::channel(32);
    let shift = Arc::new(Mutex::new([false; 4]));
    for kind in [MsgKind::Cc14, MsgKind::Pitch] {
        let mut b = binding(kind, Action::DeckPitch);
        let mut previous = -1.0;
        for value in 0..=16383u16 {
            let msg = if kind == MsgKind::Pitch {
                [0xe3, (value & 127) as u8, (value >> 7) as u8]
            } else {
                [0xb3, 39, (value & 127) as u8]
            };
            super::super::dispatch_value(
                &b,
                11,
                msg[0] & 0xf0,
                msg[2],
                &msg,
                &cmd,
                &shift,
                (kind == MsgKind::Cc14).then_some(value),
            )
            .unwrap();
            let Command::MidiPitch(crate::engine::pitch_pickup::Input {
                binding: Binding {deck:0,..},
                value: actual,..
            }) = rx.try_iter().next().unwrap()
            else {
                panic!("wrong parameter")
            };
            assert!(actual > previous && (0.0..=1.0).contains(&actual));
            if value == 0 {
                assert_eq!(actual, 0.0);
            }
            if value == 16383 {
                assert_eq!(actual, 1.0);
            }
            if kind == MsgKind::Pitch && value == 8192 {
                assert_eq!(actual, 0.5);
            }
            previous = actual;
        }
        b.controls = Some(Spec {
            invert: true,
            min: 0.2,
            max: 0.8,
        });
        for (wire, expected) in [(0, 0.8), (16383, 0.2)] {
            let msg = if kind == MsgKind::Pitch {
                [0xe3, (wire & 127) as u8, (wire >> 7) as u8]
            } else {
                [0xb3, 39, (wire & 127) as u8]
            };
            super::super::dispatch_value(
                &b,
                11,
                msg[0] & 0xf0,
                msg[2],
                &msg,
                &cmd,
                &shift,
                Some(wire),
            )
            .unwrap();
            assert!(
                matches!(rx.try_iter().next().unwrap(), Command::MidiPitch(crate::engine::pitch_pickup::Input { value, .. }) if (value - expected).abs() < 1e-6)
            );
        }
    }
}
#[test]
fn relative_parameter_changes_follow_current_renderer_value_and_preserve_undo_without_heap() {
    let (engine, mut rt) = Engine::headless_for_test(8000, 256);
    let mut b = binding(MsgKind::CcRel, Action::Master);
    b.controls = Some(Spec {
        invert: false,
        min: 0.2,
        max: 0.8,
    });
    let mut input = engine.midi.open_for_test(
        &engine.cmd,
        501,
        map(b),
        "Fixture encoder",
        "fixture:encoder",
    );
    rt.apply(Command::Master(0.4));
    input.push(&[0xb3, 7, 1]);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!((rt.master - 0.41).abs() < 1e-6);
    rt.apply(Command::Master(0.7));
    input.push(&[0xb3, 7, 65]);
    rt.process(&mut []);
    assert!((rt.master - 0.69).abs() < 1e-6);
    rt.apply(Command::Undo);
    assert_eq!(rt.master, 0.7);
    rt.apply(Command::Redo);
    assert!((rt.master - 0.69).abs() < 1e-6);
    for (byte, expected) in [(63, 0.8), (127, 0.2)] {
        input.push(&[0xb3, 7, byte]);
        rt.process(&mut []);
        assert!((rt.master - expected).abs() < 1e-6);
    }
    for neutral in [0, 64] {
        input.push(&[0xb3, 7, neutral]);
        rt.process(&mut []);
        assert_eq!(rt.master, 0.2);
    }
}
#[test]
fn paired_addresses_conflict_with_either_raw_half_and_invalid_controls_refuse() {
    let paired = binding(MsgKind::Cc14, Action::Master);
    for data in [7, 39] {
        let mut raw = binding(MsgKind::Cc, Action::DeckFilter);
        raw.data = data;
        for bindings in [vec![paired, raw], vec![raw, paired]] {
            let mut m = map(paired);
            m.bindings = bindings;
            assert!(m.validate().is_err());
        }
        raw.ch = 4;
        let mut m = map(paired);
        m.bindings.push(raw);
        m.validate().unwrap();
    }
    for spec in [
        Spec {
            invert: false,
            min: f32::NAN,
            max: 1.0,
        },
        Spec {
            invert: false,
            min: 0.8,
            max: 0.2,
        },
        Spec {
            invert: false,
            min: -0.1,
            max: 1.0,
        },
        Spec {
            invert: false,
            min: 0.0,
            max: 1.6,
        },
    ] {
        let mut b = paired;
        b.controls = Some(spec);
        assert!(map(b).validate().is_err());
    }
    for data in [32, 63, 127] {
        let mut b = paired;
        b.data = data;
        assert!(map(b).validate().is_err());
    }
}
#[test]
fn production_workers_never_pair_ports_or_cross_a_mapping_reset() {
    let (engine, mut rt) = Engine::headless_for_test(8000, 256);
    let b = binding(MsgKind::Cc14, Action::Master);
    let mut first =
        engine
            .midi
            .open_for_test(&engine.cmd, 502, map(b), "Fixture encoder A", "fixture:a");
    let mut second =
        engine
            .midi
            .open_for_test(&engine.cmd, 503, map(b), "Fixture encoder B", "fixture:b");
    rt.apply(Command::Master(0.25));
    first.push(&[0xb3, 7, 100]);
    second.push(&[0xb3, 39, 3]);
    rt.process(&mut []);
    assert_eq!(rt.master, 12800.0 / 16383.0);
    second.push(&[0xb3, 7, 12]);
    rt.process(&mut []);
    assert_eq!(rt.master, 1536.0 / 16383.0);
    second.push(&[0xb3, 39, 3]);
    rt.process(&mut []);
    assert_eq!(rt.master, 1539.0 / 16383.0);
    first.push(&[0xb3, 39, 64]);
    rt.process(&mut []);
    assert_eq!(rt.master, 12864.0 / 16383.0);
    first.push(&[0xb3, 7, 90]);
    rt.process(&mut []);
    let before = rt.master;
    let revision = engine.cmd.midi_learn().view().revision;
    engine
        .cmd
        .midi_learn()
        .configure(super::super::learn::Config {
            mappings: vec![super::super::learn::Mapping {
                endpoint: super::super::learn::Endpoint {
                    name: "Fixture encoder A".into(),
                    id: "fixture:a".into(),
                },
                binding: binding(MsgKind::Note, Action::DeckPlay),
            }],
        })
        .unwrap();
    assert!(engine.cmd.midi_learn().view().revision > revision);
    first.push(&[0xb3, 39, 127]);
    rt.process(&mut []);
    assert_eq!(rt.master, before);
    first.push(&[0xb3, 7, 1]);
    rt.process(&mut []);
    assert_eq!(rt.master, 128.0 / 16383.0);
    first.push(&[0xb3, 39, 127]);
    rt.process(&mut []);
    assert_eq!(rt.master, 255.0 / 16383.0);
}
#[test]
fn signed_bit_vectors_keep_both_neutral_bytes_and_per_binding_inversion() {
    let (cmd, rx) = crate::engine::CommandPort::channel(32);
    let shift = Arc::new(Mutex::new([false; 4]));
    let vectors: Vec<i16> = (0..=63).chain((0..=63).map(|step| -step)).collect();
    for invert in [false, true] {
        let mut b = binding(MsgKind::CcRel, Action::DeckJog);
        b.controls = Some(Spec {
            invert,
            min: 0.0,
            max: 1.0,
        });
        for (byte, step) in vectors.iter().enumerate() {
            let msg = [0xb3, 7, byte as u8];
            super::super::dispatch(&b, 11, 0xb0, byte as u8, &msg, &cmd, &shift).unwrap();
            if *step == 0 {
                assert!(rx.try_iter().next().is_none());
            } else {
                let expected = *step as f32 * 0.01 * if invert { -1.0 } else { 1.0 };
                assert!(
                    matches!(rx.try_iter().next().unwrap(), Command::DeckJog { delta, .. } if delta == expected)
                );
            }
        }
    }
}
#[test]
fn encoder_settings_and_legacy_defaults_roundtrip_and_old_versions_refuse_injected_fields() {
    use crate::engine::midi::{
        learn::{Config, Endpoint, Mapping},
        presets::{Layer, Preset},
    };
    let endpoint = Endpoint {
        name: "Fixture encoder".into(),
        id: "fixture:persist".into(),
    };
    let legacy = Preset {
        version: 1,
        name: "Legacy".into(),
        device_hint: endpoint.name.clone(),
        revision_hint: String::new(),
        layer: Layer::FactoryOverlay,
        bindings: vec![binding(MsgKind::Cc, Action::Master)],
    };
    legacy.validate().unwrap();
    assert_eq!(
        Preset::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap(),
        legacy
    );
    let mut preferences =
        crate::preferences::Preferences::defaults(std::path::Path::new("/home/fixture"));
    preferences.version = 18;
    preferences.profiles.get_mut("Studio").unwrap().midi_learn = Config {
        mappings: vec![Mapping {
            endpoint: endpoint.clone(),
            binding: legacy.bindings[0],
        }],
    };
    let (migrated, changed) =
        crate::preferences::storage::decode(&serde_json::to_vec(&preferences).unwrap()).unwrap();
    assert!(changed);
    preferences.version = crate::preferences::VERSION;
    assert_eq!(migrated, preferences);
    for kind in [MsgKind::Cc, MsgKind::Cc14, MsgKind::Pitch, MsgKind::CcRel] {
        let mut b = binding(kind, Action::Master);
        b.controls = Some(Spec {
            invert: true,
            min: 0.2,
            max: 0.8,
        });
        let config = Config {
            mappings: vec![Mapping {
                endpoint: endpoint.clone(),
                binding: b,
            }],
        };
        let current = Preset::capture("Encoder".into(), "".into(), &endpoint, &config).unwrap();
        assert_eq!(
            Preset::decode(&serde_json::to_vec(&current).unwrap()).unwrap(),
            current
        );
        assert_eq!(
            current.target(&endpoint, &Config::default()).unwrap(),
            config
        );
        let profile = preferences.profiles.get_mut("Studio").unwrap();
        profile.midi_learn = config;
        profile.midi_presets = vec![current.clone()];
        assert_eq!(
            crate::preferences::storage::decode(&serde_json::to_vec(&preferences).unwrap())
                .unwrap(),
            (preferences.clone(), false)
        );
        let mut old = serde_json::to_value(&current).unwrap();
        old["version"] = 1.into();
        assert!(Preset::decode(&serde_json::to_vec(&old).unwrap()).is_err());
        let mut old = serde_json::to_value(&preferences).unwrap();
        old["version"] = 18.into();
        assert!(crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    }
    for field in [
        serde_json::Value::Null,
        serde_json::to_value(Spec::default()).unwrap(),
    ] {
        let mut old = serde_json::to_value(&legacy).unwrap();
        old["bindings"][0]["controls"] = field.clone();
        assert!(Preset::decode(&serde_json::to_vec(&old).unwrap()).is_err());
        let mut old = serde_json::to_value(&migrated).unwrap();
        old["version"] = 18.into();
        old["profiles"]["Studio"]["midi_learn"]["mappings"][0]["binding"]["controls"] = field;
        assert!(crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    }
}
