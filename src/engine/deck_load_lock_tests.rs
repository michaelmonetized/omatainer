use super::*;
use load_receipt::{Media, Receipt, State};
fn tick(rt: &mut RtEngine) {
    rt.process(&mut [0.0; 256]);
}
#[test]
fn reviewed_eject_never_wraps_an_invalid_raw_deck_index() {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    let old = rt.decks[0].audio.clone().unwrap();
    let key = rt.decks[0].history_key;
    rt.apply(Command::DeckEjectConfirmed { deck: DECKS as u8, expected: key });
    assert!(Arc::ptr_eq(&old, rt.decks[0].audio.as_ref().unwrap()));
}
#[test]
fn load_lock_rechecks_queued_replacements_at_renderer_without_callback_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let old = rt.decks[0].audio.clone().unwrap();
    engine
        .send(Command::DeckLoadLock {
            deck: 0,
            enabled: true,
        })
        .unwrap();
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    let receipt = Receipt::new();
    engine
        .send(Command::DeckLoadRequested {
            deck: 0,
            media: Media::Builtin(1),
            receipt: receipt.clone(),
        })
        .unwrap();
    tick(&mut rt);
    assert!(Arc::ptr_eq(&old, rt.decks[0].audio.as_ref().unwrap()));
    assert_eq!(receipt.state(), State::Protected);
    assert!(engine.cmd.performance().deck_load_locked(0));
    assert_eq!(
        test_alloc::measure(|| tick(&mut rt)),
        test_alloc::Counts::default()
    );
    for command in [
        Command::DeckLoadSelected { deck: 0 },
        Command::DeckUnload { deck: 0 },
        Command::LoadBuiltin { deck: 0, stem: 1 },
        Command::DeckAudio {
            deck: 0,
            audio: old.clone(),
        },
    ] {
        assert!(matches!(
            engine.send(command),
            Err(SubmissionError::Performance(
                performance::Error::PlayingDeck
            ))
        ));
    }
    rt.apply(Command::DeckPlay { deck: 0 });
    assert!(engine.send(Command::DeckUnload { deck: 0 }).is_ok());
    tick(&mut rt);
    assert!(rt.decks[0].audio.is_none());
}
#[test]
fn reviewed_override_is_single_use_and_cannot_replace_another_current_media_identity() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::DeckLoadLock {
        deck: 0,
        enabled: true,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    let key = rt.decks[0].history_key;
    let first = Receipt::with_override(None, Some(key));
    let stale = Receipt::with_override(None, Some(key));
    for receipt in [&first, &stale] {
        engine
            .send(Command::DeckLoadRequested {
                deck: 0,
                media: Media::Builtin(1),
                receipt: receipt.clone(),
            })
            .unwrap();
    }
    tick(&mut rt);
    assert_eq!(first.state(), State::Current);
    assert_eq!(stale.state(), State::Protected);
    assert!(rt.decks[0].load_locked);
    assert!(!rt.decks[0].playing);
    let previous = rt.decks[0].audio.clone().unwrap();
    rt.apply(Command::DeckEjectConfirmed {
        deck: 0,
        expected: key,
    });
    assert!(Arc::ptr_eq(&previous, rt.decks[0].audio.as_ref().unwrap()));
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.apply(Command::DeckEjectConfirmed {
        deck: 0,
        expected: rt.decks[0].history_key,
    });
    assert!(rt.decks[0].audio.is_none());
}
#[test]
fn midi_lock_binding_and_typed_ipc_share_renderer_state_and_target_validation() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let operation = crate::ipc_schema::Operation::parse(
        &serde_json::json!({"op":"deckLoadLock","deck":0,"enabled":true}),
    )
    .unwrap();
    engine.send(operation.command().unwrap()).unwrap();
    tick(&mut rt);
    assert!(engine.cmd.performance().deck_load_locked(0));
    assert!(crate::ipc_schema::Operation::parse(
        &serde_json::json!({"op":"deckLoadLock","deck":2,"enabled":true})
    )
    .is_err());
    assert!(crate::ipc_schema::Operation::parse(
        &serde_json::json!({"op":"deckLoadLock","deck":0,"enabled":1})
    )
    .is_err());
    assert_eq!(
        engine.send(Command::DeckLoadLock {
            deck: 255,
            enabled: true
        }),
        Err(SubmissionError::InvalidTarget)
    );
    let map = midi::MidiMap {
        name: "Load lock fixture".into(),
        matchers: vec![],
        unmapped_notes: midi::UnmappedNotes::Ignore,
        bindings: vec![midi::Binding {
            ch: 0,
            kind: midi::MsgKind::Note,
            data: 20,
            action: midi::Action::DeckLoadLock,
            deck: 0,
            extra: 0,
            relative: None,
        }],
    };
    engine
        .midi
        .receive_map_for_test(&engine.cmd, 9, map, "fixture", &[0x90, 20, 127]);
    tick(&mut rt);
    assert!(!engine.cmd.performance().deck_load_locked(0));
}
#[test]
fn undo_cannot_replace_media_in_a_locked_playing_deck() {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    rt.apply(Command::DeckLoadLock {
        deck: 0,
        enabled: true,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    let original = rt.decks[0].audio.clone().unwrap();
    rt.apply(Command::Undo);
    assert!(Arc::ptr_eq(&original, rt.decks[0].audio.as_ref().unwrap()));
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.apply(Command::Undo);
    assert!(!Arc::ptr_eq(&original, rt.decks[0].audio.as_ref().unwrap()));
}
