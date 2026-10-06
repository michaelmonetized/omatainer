use super::*;

fn binding(action: Action) -> Binding {
    let kind = kind(action);
    Binding {
        kind,
        ch: 0,
        data: 60,
        action,
        deck: 0,
        extra: 0,
        relative: (kind == MsgKind::CcRel).then_some(RelativeSpec {
            encoding: super::super::relative::RelativeEncoding::OffsetBinary,
            scale: 1.0,
        }),
     controls: None,
     pair_order: None,
    }
}
fn map(bindings: Vec<Binding>) -> MidiMap {
    MidiMap {
        name: "Fixture".into(),
        matchers: vec![],
        bindings,
        unmapped_notes: super::super::UnmappedNotes::Live,
    }
}
fn connect(shared: &Shared, source: u64, id: &str) {
    shared.connected(source, "Fixture controller", id);
}

#[test]
fn one_gesture_conflict_review_replacement_and_release_do_not_swallow_other_input() {
    let shared = Shared::default();
    connect(&shared, 7, "port-a");
    let builtin = map(vec![binding(Action::DeckPlay)]);
    shared.begin(binding(Action::DeckCue), None).unwrap();
    assert!(matches!(
        shared.input(7, "Fixture controller", "port-a", &[0xb0, 1, 127], &builtin),
        Dispatch::Normal
    ));
    assert!(shared.view().armed);
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Consume
    ));
    let view = shared.view();
    assert!(!view.armed);
    assert_eq!(view.capture.as_ref().unwrap().conflicts.len(), 1);
    assert!(
        shared
            .assign(view.revision, false)
            .unwrap_err()
            .contains("already assigned")
    );
    shared
        .configure(shared.assign(view.revision, true).unwrap())
        .unwrap();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Binding(Binding {
            action: Action::DeckCue,
            ..
        })
    ));
    assert!(matches!(
        shared.input(7, "Fixture controller", "port-a", &[0x80, 60, 0], &builtin),
        Dispatch::Binding(Binding {
            action: Action::DeckCue,
            ..
        })
    ));
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Binding(Binding {
            action: Action::DeckCue,
            ..
        })
    ));
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-b",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Normal
    ));
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x91, 60, 100],
            &builtin
        ),
        Dispatch::Normal
    ));
}

#[test]
fn cancel_timeout_stale_configuration_and_disconnect_preserve_installed_mappings() {
    let shared = Shared::default();
    connect(&shared, 7, "port-a");
    let builtin = map(vec![]);
    shared.begin(binding(Action::DeckPlay), None).unwrap();
    shared.cancel();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Normal
    ));
    let endpoint = shared.view().devices[0].endpoint.clone();
    shared
        .begin(binding(Action::DeckPlay), Some(endpoint.clone()))
        .unwrap();
    shared.state.lock().armed.as_mut().unwrap().deadline = Instant::now() - Duration::from_secs(1);
    assert!(!shared.view().armed);
    shared.begin(binding(Action::DeckPlay), None).unwrap();
    shared.input(
        7,
        "Fixture controller",
        "port-a",
        &[0x90, 60, 100],
        &builtin,
    );
    let view = shared.view();
    let config = shared.assign(view.revision, false).unwrap();
    shared.configure(config.clone()).unwrap();
    assert!(shared.assign(view.revision, false).is_err());
    shared
        .begin(binding(Action::DeckCue), Some(endpoint))
        .unwrap();
    shared.disconnected(7);
    assert!(!shared.view().armed);
    assert!(shared.view().capture.is_none());
    assert_eq!(shared.view().config, config);
    assert!(
        shared
            .begin(
                binding(Action::DeckCue),
                Some(Endpoint {
                    name: "Fixture controller".into(),
                    id: "port-a".into()
                })
            )
            .is_err()
    );
}

#[test]
fn note_release_zero_velocity_and_wrong_message_types_do_not_complete_capture() {
    let shared = Shared::default();
    connect(&shared, 7, "port-a");
    let builtin = map(vec![]);
    shared.begin(binding(Action::DeckPlay), None).unwrap();
    for bytes in [
        [0x80, 60, 0],
        [0x90, 60, 0],
        [0xb0, 60, 127],
        [0xe0, 60, 127],
    ] {
        assert!(matches!(
            shared.input(7, "Fixture controller", "port-a", &bytes, &builtin),
            Dispatch::Normal
        ));
    }
    assert!(shared.view().armed);
    let mut pitch = binding(Action::DeckPitch);
    pitch.kind = MsgKind::Pitch;
    shared.begin(pitch, None).unwrap();
    shared.input(7, "Fixture controller", "port-a", &[0xe2, 1, 65], &builtin);
    let capture = shared.view().capture.unwrap();
    assert_eq!(capture.mapping.binding.ch, 2);
    assert_eq!(capture.mapping.binding.data, 0);
    shared
        .configure(shared.assign(shared.view().revision, false).unwrap())
        .unwrap();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0xe2, 127, 100],
            &builtin
        ),
        Dispatch::Binding(_)
    ));
}

#[test]
fn bounded_storage_invalid_targets_and_overlapping_addresses_are_refused_atomically() {
    let endpoint = Endpoint {
        name: "Fixture controller".into(),
        id: "port-a".into(),
    };
    let mut config = Config {
        mappings: vec![Mapping {
            endpoint: endpoint.clone(),
            binding: binding(Action::DeckPlay),
        }],
    };
    config.validate().unwrap();
    config.mappings.push(config.mappings[0].clone());
    assert!(config.validate().is_err());
    config.mappings.truncate(1);
    config.mappings[0].binding.ch = 0xff;
    assert!(config.validate().is_err());
    config.mappings[0].binding = binding(Action::DeckPlay);
    config.mappings[0].binding.kind = MsgKind::Cc;
    assert!(config.validate().is_err());
    config.mappings[0].binding = binding(Action::Clip);
    config.mappings[0].binding.deck = 127;
    config.mappings[0].binding.extra = 511;
    config.validate().unwrap();
    config.mappings[0].binding.deck = 128;
    assert!(config.validate().is_err());
    config.mappings[0].binding = binding(Action::DeckPlay);
    config.mappings[0].endpoint.id = "x".repeat(257);
    assert!(config.validate().is_err());
    let shared = Shared::default();
    assert!(shared.configure(config).is_err());
    assert!(shared.view().config.mappings.is_empty());
    let bounded = Config {
        mappings: (0..256)
            .map(|index| Mapping {
                endpoint: endpoint.clone(),
                binding: Binding {
                    ch: (index / 128) as u8,
                    data: (index % 128) as u8,
                    ..binding(Action::DeckPlay)
                },
            })
            .collect(),
    };
    bounded.validate().unwrap();
    let mut large = bounded.clone();
    large.mappings.push(large.mappings[0].clone());
    assert!(large.validate().is_err());
    assert_eq!(
        serde_json::from_slice::<Config>(&serde_json::to_vec(&bounded).unwrap()).unwrap(),
        bounded
    );
}

#[test]
fn ambiguous_exact_ports_cannot_install_unreviewed_actions_and_cancel_resumes_input() {
    let shared = Shared::default();
    connect(&shared, 7, "port-a");
    connect(&shared, 8, "port-a");
    shared.begin(binding(Action::DeckPlay), None).unwrap();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &map(vec![])
        ),
        Dispatch::Normal
    ));
    assert!(shared.view().capture.is_none());
    shared.disconnected(8);
    shared.begin(binding(Action::DeckPlay), None).unwrap();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &map(vec![])
        ),
        Dispatch::Consume
    ));
    shared.cancel();
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &map(vec![])
        ),
        Dispatch::Normal
    ));
}

#[test]
fn saved_overrides_and_captured_assignment_refuse_new_port_ambiguity() {
    let shared = Shared::default();
    connect(&shared, 7, "port-a");
    let builtin = map(vec![]);
    let endpoint = shared.view().devices[0].endpoint.clone();
    shared
        .configure(Config {
            mappings: vec![Mapping {
                endpoint: endpoint.clone(),
                binding: binding(Action::DeckCue),
            }],
        })
        .unwrap();
    connect(&shared, 8, "port-a");
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Consume
    ));
    shared.disconnected(8);
    assert!(matches!(
        shared.input(
            7,
            "Fixture controller",
            "port-a",
            &[0x90, 60, 100],
            &builtin
        ),
        Dispatch::Binding(_)
    ));
    shared
        .begin(binding(Action::DeckPlay), Some(endpoint))
        .unwrap();
    shared.input(
        7,
        "Fixture controller",
        "port-a",
        &[0x90, 61, 100],
        &builtin,
    );
    let view = shared.view();
    connect(&shared, 9, "port-a");
    assert!(
        shared
            .assign(view.revision, false)
            .unwrap_err()
            .contains("ambiguous")
    );
    assert_eq!(shared.view().config.mappings.len(), 1);
}
