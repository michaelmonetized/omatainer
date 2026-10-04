use super::*;

fn map(binding: Binding) -> MidiMap {
    MidiMap {
        name: "explicit relative fixture".into(),
        matchers: vec![],
        bindings: vec![binding],
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

fn send(map: &MidiMap, message: &[u8], commands: &crate::engine::CommandPort) {
    handle_msg(
        message,
        42,
        map,
        commands,
        &Arc::new(Mutex::new(Vec::new())),
        &Arc::new(Mutex::new([false; 4])),
        "synthetic relative controller",
    );
}

// Independent wire vectors enumerate the complete signed range in wire order.
// This catches the original bug (every reverse value decoded positive), the
// 0x40 boundary, scale applied twice, and one encoding substituted for another.
fn vectors() -> [(RelativeEncoding, Vec<i16>); 2] {
    [
        (RelativeEncoding::OffsetBinary, (-64..=63).collect()),
        (
            RelativeEncoding::TwosComplement,
            (0..=63).chain(-64..=-1).collect(),
        ),
    ]
}

#[test]
fn all_relative_bytes_have_the_declared_sign_neutral_and_scale() {
    for (encoding, expected) in vectors() {
        for scale in [0.125, 0.35, 2.0] {
            let spec = RelativeSpec { encoding, scale };
            let profile = map(rbind(3, 0x21, Action::DeckJog, 1, 0, spec));
            profile.validate().unwrap();
            let (commands, receiver) = crate::engine::CommandPort::channel(32);
            let mut signs = [0; 3];
            for (byte, steps) in expected.iter().copied().enumerate() {
                let expected_delta = steps as f32 * scale;
                assert_eq!(spec.decode(byte as u8), Some(expected_delta));
                send(&profile, &[0xb3, 0x21, byte as u8], &commands);
                let received: Vec<_> = receiver.try_iter().collect();
                if steps == 0 {
                    assert!(received.is_empty());
                    signs[1] += 1;
                } else {
                    assert!(
                        matches!(received.as_slice(), [Command::DeckJog { deck: 1, delta }]
                        if *delta == expected_delta),
                        "{encoding:?} {byte:#04x}: {received:?}"
                    );
                    signs[usize::from(steps > 0) * 2] += 1;
                }
            }
            assert_eq!(signs, [64, 1, 63]);
        }
    }
}

#[test]
fn relative_metadata_is_required_and_invalid_values_never_dispatch() {
    let binding = rbind(0, 1, Action::DeckJog, 0, 0, RelativeSpec::PIONEER_JOG);
    let mut missing = binding;
    missing.relative = None;
    let mut absolute = binding;
    absolute.kind = MsgKind::Cc;
    for invalid in [missing, absolute] {
        assert!(
            map(invalid)
                .validate()
                .unwrap_err()
                .to_string()
                .contains("relative")
        );
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        let invalid = map(rbind(
            0,
            1,
            Action::DeckJog,
            0,
            0,
            RelativeSpec {
                encoding: RelativeEncoding::OffsetBinary,
                scale,
            },
        ));
        assert!(invalid.validate().is_err());
        let (commands, receiver) = crate::engine::CommandPort::channel(32);
        send(&invalid, &[0xb0, 1, 0x41], &commands);
        assert_eq!(receiver.try_iter().count(), 0);
    }
    for (encoding, _) in vectors() {
        let profile = map(rbind(
            0,
            1,
            Action::DeckJog,
            0,
            0,
            RelativeSpec {
                encoding,
                scale: 0.35,
            },
        ));
        let (commands, receiver) = crate::engine::CommandPort::channel(32);
        for byte in 128..=255 {
            assert_eq!(profile.bindings[0].relative.unwrap().decode(byte), None);
            // F8..FF are valid interleaved realtime statuses, not invalid
            // data; issue 41 extracts their own transport/clock actions.
            if byte < 0xf8 {
                send(&profile, &[0xb0, 1, byte], &commands);
            }
        }
        assert_eq!(receiver.try_iter().count(), 0);
    }
}

#[test]
fn every_pioneer_jog_binding_uses_documented_centered_vectors() {
    let profile = pioneer_ddj_fx();
    profile.validate().unwrap();
    let (commands, receiver) = crate::engine::CommandPort::channel(32);
    let relative: Vec<_> = profile
        .bindings
        .iter()
        .filter(|b| b.kind == MsgKind::CcRel)
        .collect();
    assert_eq!(relative.len(), 6);
    for binding in relative {
        assert_eq!(binding.relative, Some(RelativeSpec::PIONEER_JOG));
        assert_eq!(binding.ch, binding.deck);
        assert!([0x21, 0x22, 0x23].contains(&binding.data));
        for (byte, steps) in (0..=127).zip(-64..=63) {
            send(
                &profile,
                &[0xb0 | binding.ch, binding.data, byte],
                &commands,
            );
            let received: Vec<_> = receiver.try_iter().collect();
            if steps == 0 {
                assert!(received.is_empty());
            } else {
                assert!(
                    matches!(received.as_slice(), [Command::DeckJog { deck, delta }]
                    if *deck == binding.deck && *delta == steps as f32 * 0.35)
                );
            }
        }
    }
}

#[test]
fn legacy_ns7_profiles_do_not_guess_a_relative_wheel_protocol() {
    for profile in [numark_ns7(false), numark_ns7(true)] {
        profile.validate().unwrap();
        assert!(!profile.bindings.iter().any(|b| b.action == Action::DeckJog));
        let (commands, receiver) = crate::engine::CommandPort::channel(32);
        for channel in 0..2 {
            for byte in 0..128 {
                send(&profile, &[0xb0 | channel, 0x21, byte], &commands);
                send(&profile, &[0xe0 | channel, byte, byte], &commands);
                assert_eq!(receiver.try_iter().count(), 0);
            }
        }
    }
}

#[test]
fn forward_reverse_and_neutral_reach_the_original_deck_jog_path() {
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48_000, 32);
    for (encoding, forward, reverse, neutral) in [
        (RelativeEncoding::OffsetBinary, 0x41, 0x3f, 0x40),
        (RelativeEncoding::TwosComplement, 0x01, 0x7f, 0x00),
    ] {
        for deck in 0..2 {
            let profile = map(rbind(
                deck,
                0x21,
                Action::DeckJog,
                deck,
                0,
                RelativeSpec {
                    encoding,
                    scale: 0.35,
                },
            ));
            for (playing, touching) in [(false, false), (true, true), (true, false)] {
                for (byte, delta) in [(forward, 0.35f32), (reverse, -0.35)] {
                    for state in &mut rt.decks {
                        state.pos = 20_000.0;
                        state.rate = 1.0;
                        state.target_rate = 1.0;
                        state.scratch = 0.0;
                        state.playing = playing;
                        state.touching = touching;
                    }
                    send(&profile, &[0xb0 | deck, 0x21, byte], &engine.cmd);
                    rt.process(&mut []);
                    let changed = &rt.decks[deck as usize];
                    if !playing || touching {
                        assert_eq!(changed.pos, 20_000.0 + delta as f64 * 400.0);
                        assert_eq!(changed.scratch, delta * 18.0);
                    } else {
                        assert_eq!(changed.pos, 20_000.0);
                        assert_eq!(changed.rate, 1.0 + delta * 0.15);
                    }
                    let state = (changed.pos, changed.rate, changed.scratch);
                    let count = rt.command_stats.received;
                    send(&profile, &[0xb0 | deck, 0x21, neutral], &engine.cmd);
                    rt.process(&mut []);
                    assert_eq!(rt.command_stats.received, count);
                    let changed = &rt.decks[deck as usize];
                    assert_eq!((changed.pos, changed.rate, changed.scratch), state);
                    let untouched = &rt.decks[1 - deck as usize];
                    assert_eq!(
                        (untouched.pos, untouched.rate, untouched.scratch),
                        (20_000.0, 1.0, 0.0)
                    );
                    if playing && touching {
                        // Actual rendering follows the signed scratch rate too.
                        let before = rt.decks[deck as usize].pos;
                        rt.render_deck(deck as usize);
                        let after = rt.decks[deck as usize].pos;
                        assert_eq!(
                            (after - before).is_sign_positive(),
                            delta.is_sign_positive()
                        );
                    }
                }
            }
        }
    }
}
