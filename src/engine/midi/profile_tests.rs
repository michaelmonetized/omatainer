use super::*;

fn observe(map: &MidiMap, message: &[u8]) -> Vec<Command> {
    let (commands, receiver) = crate::engine::CommandPort::channel(16);
    handle_msg(
        message,
        1,
        map,
        &commands,
        &Arc::new(Mutex::new(Vec::new())),
        &Arc::new(Mutex::new(None)),
        &Arc::new(Mutex::new([false; 4])),
        "synthetic controller",
    );
    receiver.try_iter().collect()
}

fn with_bindings(bindings: Vec<Binding>) -> MidiMap {
    MidiMap {
        name: "validation fixture".into(),
        matchers: vec!["fixture".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

#[test]
fn every_factory_profile_has_unambiguous_wire_addresses() {
    let maps = builtin_maps().unwrap();
    assert_eq!(maps.len(), 8);
    for map in maps {
        map.validate().unwrap();
    }
}

#[test]
fn validation_rejects_duplicates_conflicts_wildcards_and_decoder_aliases() {
    let first = cbind(0, 7, Action::TrackFader, 0, 0);
    for conflict in [
        first,
        cbind(0, 7, Action::TrackFader, 0, 1),
        cbind(0, 7, Action::Master, 0, 0),
        cbind(0xff, 7, Action::TrackFader, 0, 0),
        rbind(0, 7, Action::DeckJog, 0, 0, RelativeSpec::PIONEER_JOG),
    ] {
        for bindings in [vec![first, conflict], vec![conflict, first]] {
            let error = with_bindings(bindings).validate().unwrap_err().to_string();
            assert!(error.contains("validation fixture"));
            assert!(error.contains("bindings 0 and 1 overlap"));
        }
    }
    let pitch = Binding {
        kind: MsgKind::Pitch,
        ch: 1,
        data: 0,
        action: Action::DeckJog,
        deck: 1,
        extra: 0,
        relative: None,
    };
    let mut second = pitch;
    second.data = 127; // Dispatch ignores data1 for pitch bend.
    assert!(with_bindings(vec![pitch, second]).validate().is_err());
    let note = nbind(0xff, 60, Action::Clip, 0, 0);
    assert!(
        with_bindings(vec![note, nbind(3, 60, Action::Scene, 0, 0)])
            .validate()
            .is_err()
    );
}

#[test]
fn validation_accepts_distinct_addresses_and_rejects_invalid_midi_bytes() {
    with_bindings(vec![
        cbind(0, 7, Action::Master, 0, 0),
        cbind(1, 7, Action::TrackFader, 0, 1),
        cbind(0, 8, Action::TrackFader, 0, 2),
        nbind(0xff, 7, Action::Scene, 0, 0),
        nbind(0, 8, Action::Scene, 0, 1),
    ])
    .validate()
    .unwrap();
    for binding in [
        cbind(16, 7, Action::Master, 0, 0),
        cbind(0, 128, Action::Master, 0, 0),
    ] {
        assert!(
            with_bindings(vec![binding])
                .validate()
                .unwrap_err()
                .to_string()
                .contains("invalid")
        );
    }
}

#[test]
fn apc40_faders_exhaust_all_channels_controllers_and_values_without_fanout() {
    for map in [akai_apc40(), akai_apc40_mk2()] {
        map.validate().unwrap();
        for channel in 0..16u8 {
            // Exhaust the entire value range at the actual fader CC.
            for value in 0..128u8 {
                let commands = observe(&map, &[0xb0 | channel, 7, value]);
                if channel < 8 {
                    assert!(
                        matches!(commands.as_slice(), [Command::TrackGain {track, value: gain}]
                        if *track == channel && *gain == value as f32 / 127.0),
                        "{} ch{channel}: {commands:?}",
                        map.name
                    );
                } else {
                    assert!(
                        commands.is_empty(),
                        "unused fader channel {channel}: {commands:?}"
                    );
                }
            }
            // Wrong CCs must not hit the faders. Master is the only other
            // absolute controller intentionally mapped in these profiles.
            for controller in 0..128u8 {
                if controller == 7 {
                    continue;
                }
                for value in [0, 64, 127] {
                    let commands = observe(&map, &[0xb0 | channel, controller, value]);
                    if channel == 0 && controller == 14 {
                        assert!(
                            matches!(commands.as_slice(), [Command::Master(gain)] if *gain == value as f32 / 127.0)
                        );
                    } else {
                        assert!(
                            commands.is_empty(),
                            "{} ch{channel} cc{controller}: {commands:?}",
                            map.name
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn apc40_fader_dispatch_changes_only_its_own_engine_track() {
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48_000, 16);
    let log = Arc::new(Mutex::new(Vec::new()));
    let learn = Arc::new(Mutex::new(None));
    let shift = Arc::new(Mutex::new([false; 4]));
    for map in [akai_apc40(), akai_apc40_mk2()] {
        for track in 0..8u8 {
            for value in [0, 64, 127] {
                for state in &mut rt.tracks {
                    state.gain = 0.25;
                }
                let master = rt.master;
                let before = rt.command_stats.received;
                handle_msg(
                    &[0xb0 | track, 7, value],
                    1,
                    &map,
                    &engine.cmd,
                    &log,
                    &learn,
                    &shift,
                    "synthetic APC40",
                );
                rt.process(&mut []);
                assert_eq!(rt.command_stats.received - before, 1);
                for (index, state) in rt.tracks.iter().enumerate() {
                    assert_eq!(
                        state.gain,
                        if index == track as usize {
                            value as f32 / 127.0
                        } else {
                            0.25
                        }
                    );
                }
                assert_eq!(rt.master, master);
            }
        }
    }
}

#[test]
fn apc40_generations_use_separate_clip_grids_and_ignore_unmapped_buttons() {
    for (map, mk2) in [(akai_apc40(), false), (akai_apc40_mk2(), true)] {
        for track in 0..8u8 {
            for scene in 0..5u8 {
                let address = if mk2 {
                    [0x90, scene * 8 + track, 127]
                } else {
                    [0x90 | track, 0x35 + scene, 127]
                };
                assert!(
                    matches!(observe(&map, &address).as_slice(), [Command::LaunchClip {track: t, scene: s}] if *t == track && *s == scene)
                );
                if mk2 {
                    // MkII grid notes identify their own track; the MIDI
                    // channel does not select a different track strip.
                    for channel in 0..16 {
                        assert!(
                            matches!(observe(&map, &[0x90 | channel, address[1], 127]).as_slice(),
                            [Command::LaunchClip {track: t, scene: s}] if *t == track && *s == scene)
                        );
                    }
                }
                for release in [
                    [address[0] - 0x10, address[1], 127],
                    [address[0], address[1], 0],
                ] {
                    assert!(observe(&map, &release).is_empty());
                }
                let other = if mk2 {
                    [0x90 | track, 0x35 + scene, 127]
                } else {
                    [0x90, scene * 8 + track, 127]
                };
                assert!(
                    observe(&map, &other).is_empty(),
                    "{} accepted other generation's grid",
                    map.name
                );
            }
        }
        for scene in 0..5u8 {
            for channel in 0..16 {
                assert!(
                    matches!(observe(&map, &[0x90 | channel, 0x52 + scene, 127]).as_slice(),
                    [Command::LaunchScene {scene:s}] if *s == scene)
                );
            }
        }
        for channel in 0..16u8 {
            for note in [0x30, 0x31, 0x32, 0x33, 0x34, 0x50, 0x5b, 0x5d] {
                for status in [0x90, 0x80] {
                    assert!(
                        observe(&map, &[status | channel, note, 127]).is_empty(),
                        "unmapped surface control must not play notes or operate another control"
                    );
                }
            }
        }
    }
}

#[test]
fn profile_name_selection_distinguishes_original_and_mkii_without_claiming_hardware_qa() {
    let maps = builtin_maps().unwrap();
    for name in ["APC40", "Akai APC-40", "APC 40 MIDI 1", "APC40 mk1"] {
        assert_eq!(pick_map(&maps, name).name, "Akai APC40 (original)");
    }
    for name in [
        "Akai APC40 mkII",
        "APC40 mk2",
        "APC40MKII MIDI 1",
        "APC-40 mk II",
        "APC 40 mk 2",
    ] {
        assert_eq!(pick_map(&maps, name).name, "Akai APC40 mkII");
    }
}

#[test]
fn mpk_cc1_has_one_destination_and_generic_keyboards_keep_live_notes() {
    let map = akai_mpk();
    assert!(
        matches!(observe(&map, &[0xb0, 1, 64]).as_slice(), [Command::DeckFilter {deck:0, value}] if *value == 64.0 / 127.0)
    );
    for map in [map, class_compliant()] {
        assert!(matches!(
            observe(&map, &[0x92, 60, 100]).as_slice(),
            [Command::LiveNoteOn {
                source: 1,
                ch: 2,
                note: 60,
                vel: 100
            }]
        ));
    }
}
