use super::*;
use crate::engine::midi::{Action, MsgKind, RelativeEncoding, RelativeSpec};

fn port(id: &str) -> Endpoint {
    Endpoint {
        name: "Same visible controller".into(),
        id: id.into(),
    }
}
fn binding(action: Action, data: u8) -> Binding {
    let kind = super::super::learn::kind(action);
    Binding {
        action,
        kind,
        ch: 0,
        data,
        deck: 0,
        extra: 0,
        relative: (kind == MsgKind::CcRel).then_some(RelativeSpec {
            encoding: RelativeEncoding::OffsetBinary,
            scale: 1.0,
        }),
    }
}
fn fixture() -> Preset {
    let first = port("machine-a:7");
    Preset::capture(
        "Stage".into(),
        "Original".into(),
        &first,
        &Config {
            mappings: vec![Mapping {
                endpoint: first.clone(),
                binding: binding(Action::DeckCueHold, 60),
            }],
        },
    )
    .unwrap()
}

#[test]
fn portable_all_actions_roundtrip_to_another_exact_port_without_machine_ids() {
    let first = port("machine-a:7");
    let second = port("machine-b:12");
    let other = port("independent:3");
    let config = Config {
        mappings: super::super::learn::actions()
            .iter()
            .enumerate()
            .map(|(i, &action)| Mapping {
                endpoint: first.clone(),
                binding: binding(action, i as u8),
            })
            .chain([Mapping {
                endpoint: other.clone(),
                binding: binding(Action::Master, 100),
            }])
            .collect(),
    };
    let preset =
        Preset::capture("Stage".into(), "16 year controller".into(), &first, &config).unwrap();
    let bytes = serde_json::to_vec_pretty(&preset).unwrap();
    assert!(!String::from_utf8(bytes.clone())
        .unwrap()
        .contains(&first.id));
    let imported = Preset::decode(&bytes).unwrap();
    assert_eq!(imported, preset);
    let target = imported
        .target(
            &second,
            &Config {
                mappings: vec![config.mappings.last().unwrap().clone()],
            },
        )
        .unwrap();
    assert_eq!(target.mappings[0], *config.mappings.last().unwrap());
    assert!(target.mappings[1..]
        .iter()
        .all(|row| row.endpoint == second));
    assert_eq!(
        target.mappings[1..]
            .iter()
            .map(|row| row.binding)
            .collect::<Vec<_>>(),
        preset.bindings
    );
    assert_eq!(
        defaults(&second, &target).mappings,
        vec![config.mappings.last().unwrap().clone()]
    );
}

#[test]
fn unknown_fields_actions_ambiguous_addresses_and_limits_refuse_complete_definitions() {
    let preset = fixture();
    let base = serde_json::to_value(&preset).unwrap();
    for value in ["MadeUpAction", "DeckCueHold "] {
        let mut bad = base.clone();
        bad["bindings"][0]["action"] = value.into();
        assert!(Preset::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let mut bad = base.clone();
    bad["endpoint"] = "machine-a:7".into();
    assert!(Preset::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    for version in [0, 2, u32::MAX] {
        let mut bad = preset.clone();
        bad.version = version;
        assert!(bad.validate().is_err());
    }
    for name in ["", " padded", "line\nbreak"] {
        let mut bad = preset.clone();
        bad.name = name.into();
        assert!(bad.validate().is_err());
    }
    let mut bad = preset.clone();
    bad.bindings.push(bad.bindings[0]);
    assert!(bad.validate().is_err());
    let mut bad = preset.clone();
    bad.bindings[0].ch = 0xff;
    assert!(bad.validate().is_err());
    let mut bad = preset.clone();
    bad.bindings[0].kind = MsgKind::Cc;
    assert!(bad.validate().is_err());
    let mut bad = preset.clone();
    bad.bindings[0].deck = 2;
    assert!(bad.validate().is_err());
    assert!(Preset::decode(&vec![b' '; 65_537]).is_err());
    assert!(validate_bank(&vec![preset.clone(); 33]).is_err());
    assert!(validate_bank(&[preset.clone(), preset.clone()]).is_err());
    let full = Config {
        mappings: (0..256)
            .map(|i| Mapping {
                endpoint: port(&format!("different:{i}")),
                binding: binding(Action::DeckPlay, 0),
            })
            .collect(),
    };
    assert!(preset.target(&port("new:1"), &full).is_err());
    assert_eq!(full.mappings.len(), 256);
    let mut bad = preset.clone();
    bad.bindings = vec![binding(Action::DeckJog, 60), binding(Action::Master, 60)];
    assert!(bad.validate().is_err());
}

#[test]
fn reviewed_config_is_atomic_against_mapping_changes_reconnects_and_duplicate_ports() {
    let handle = super::super::learn::Shared::default();
    let endpoint = port("destination:1");
    handle.connected(101, &endpoint.name, &endpoint.id);
    let view = handle.view();
    let preset = fixture();
    let next = preset.target(&endpoint, &view.config).unwrap();
    handle
        .configure_reviewed(view.revision, 101, &endpoint, next.clone())
        .unwrap();
    assert_eq!(handle.view().config, next);
    assert!(handle
        .configure_reviewed(view.revision, 101, &endpoint, Config::default())
        .is_err());
    assert_eq!(handle.view().config, next);
    let view = handle.view();
    handle.disconnected(101);
    handle.connected(102, &endpoint.name, &endpoint.id);
    assert!(handle
        .configure_reviewed(view.revision, 101, &endpoint, Config::default())
        .is_err());
    assert_eq!(handle.view().config, next);
    handle.connected(103, &endpoint.name, &endpoint.id);
    assert!(handle
        .configure_reviewed(view.revision, 102, &endpoint, Config::default())
        .is_err());
    handle.disconnected(103);
    handle
        .configure_reviewed(view.revision, 102, &endpoint, Config::default())
        .unwrap();
    assert_eq!(handle.view().config, Config::default());
}

#[test]
fn preferences_seventeen_migrate_exactly_and_preset_banks_require_eighteen() {
    let mut old = crate::preferences::Preferences::defaults(std::path::Path::new("/home/fixture"));
    old.version = 17;
    old.profiles.get_mut("Studio").unwrap().shortcuts.insert(
        "beat_jump_back".into(),
        Some(crate::preferences::Shortcut {
            key: "J".into(),
            ctrl: false,
            shift: false,
            alt: false,
        }),
    );
    let (mut migrated, changed) =
        crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(changed);
    old.version = 18;
    assert_eq!(migrated, old);
    migrated
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_presets
        .push(fixture());
    let bytes = serde_json::to_vec(&migrated).unwrap();
    assert_eq!(
        crate::preferences::storage::decode(&bytes).unwrap(),
        (migrated.clone(), false)
    );
    migrated.version = 17;
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&migrated).unwrap()).is_err());
}
