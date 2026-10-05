use super::*;
use engine::{CommandPort, RtEngine, Snapshot};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;

#[test]
fn deck_protection_cli_validates_target_and_action_before_connecting() {
    let args = |values: &[&str]|values.iter().map(|value|(*value).to_owned()).collect::<Vec<_>>();
    assert_eq!(deck_protection_payload(&args(&["deck-lock", "A", "on"])).unwrap(), json!({"op":"deckLoadLock", "deck":0, "enabled":true}));
    assert_eq!(deck_protection_payload(&args(&["deck-lock", "b", "off"])).unwrap(), json!({"op":"deckLoadLock", "deck":1, "enabled":false}));
    assert_eq!(deck_protection_payload(&args(&["deck-eject", "B"])).unwrap(), json!({"op":"deckEject", "deck":1}));
    for values in [vec![], vec!["deck-lock"], vec!["deck-lock","C","on"], vec!["deck-lock","A","maybe"], vec!["deck-eject","A","on"], vec!["unknown","A"]] {
        assert!(deck_protection_payload(&args(&values)).is_err(), "{values:?}");
    }
}

fn exchange(client: &mut BufReader<UnixStream>, value: &Value) -> Value {
    writeln!(client.get_mut(), "{value}").unwrap();
    let mut line = String::new();
    assert!(client.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

fn renderer_state(rt: &RtEngine) -> Value {
    json!({
        "playing": rt.playing, "recording": rt.recording, "beat": rt.beat,
        "selection": [rt.selected_track, rt.selected_scene],
        "decks": rt.decks.iter().map(|deck| (deck.playing, deck.pos, deck.cue_pos)).collect::<Vec<_>>(),
        "tracks": rt.tracks.iter().map(|track| json!({
            "clips": track.clips,
            "playing": track.playing.map(|clip| (clip.scene, clip.start_beat, clip.last_beat, clip.looping))
        })).collect::<Vec<_>>()
    })
}

#[test]
fn every_invalid_deck_and_scene_field_is_rejected_before_actual_renderer_mutation() {
    let (port, receiver) = CommandPort::channel(256);
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let mut rt = RtEngine::new(48000.0, receiver, snapshot.clone());
    let before = renderer_state(&rt);
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let commands = port.clone();
    let handler = std::thread::spawn(move || {
        handle_client_with_limits(
            server,
            commands,
            snapshot,
            ipc_transport::Limits {
                requests: 256,
                ..Default::default()
            },
        )
    });
    let mut client = BufReader::new(client);
    for (op, field, limit) in [
        ("deckPlay", "deck", engine::DECKS),
        ("deckCue", "deck", engine::DECKS),
        ("scene", "n", engine::session::MAX_SCENES),
    ] {
        let mut invalid = vec![None];
        invalid.extend(
            [
                Value::Null,
                json!(true),
                json!(false),
                json!("0"),
                json!(""),
                json!([]),
                json!({}),
                json!(-1),
                json!(-256),
                json!(0.0),
                json!(0.5),
                json!(1e300),
                json!(limit),
                json!(255),
                json!(256),
                json!(257),
                json!(u64::MAX),
            ]
            .into_iter()
            .map(Some),
        );
        for (case, target) in invalid.into_iter().enumerate() {
            let mut request = json!({"op":op,"id":format!("{op}-{case}")});
            if let Some(target) = target {
                request[field] = target;
            }
            let response = exchange(&mut client, &request);
            assert_eq!(response["id"], request["id"]);
            assert_eq!(response["ok"], false, "{request}: {response}");
            assert_eq!(response["accepted"], false);
            assert_eq!(response["command_status"], "rejected");
            assert!(
                response["error"].as_str().unwrap().contains(field),
                "{response}"
            );
            assert_eq!(port.len(), 0, "{request}");
            rt.process(&mut []);
            assert_eq!(renderer_state(&rt), before, "{request}");
        }
    }
    drop(client);
    handler.join().unwrap().unwrap();
}

#[test]
fn typed_opcode_schemas_reject_extra_fields_and_dispatch_exact_valid_boundaries() {
    for op in [
        "ping",
        "status",
        "follow",
        "reload-theme",
        "play",
        "stop",
        "togglePlay",
        "record",
        "tap",
    ] {
        assert!(
            ipc_schema::Operation::parse(&json!({"op":op,"id":1})).is_ok(),
            "{op}"
        );
        assert!(
            ipc_schema::Operation::parse(&json!({"op":op,"deck":0})).is_err(),
            "{op} extra field"
        );
    }
    for request in [
        Value::Null,
        json!("status"),
        json!([]),
        json!({}),
        json!({"op":3}),
        json!({"op":"unknown"}),
        json!({"op":"deckPlay","deck":0,"n":1}),
        json!({"op":"scene","n":0,"deck":1}),
    ] {
        assert!(ipc_schema::Operation::parse(&request).is_err(), "{request}");
    }
    let (port, receiver) = CommandPort::channel(256);
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let mut rt = RtEngine::new(48000.0, receiver, snapshot.clone());
    rt.quant = 0.0;
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            clip.kind = engine::ClipKind::Midi;
        }
    }
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let handler = std::thread::spawn(move || handle_client(server, port, snapshot));
    let mut client = BufReader::new(client);
    for deck in 0..engine::DECKS {
        let previous = [rt.decks[0].playing, rt.decks[1].playing];
        let response = exchange(&mut client, &json!({"op":"deckPlay","deck":deck}));
        assert_eq!(response["accepted"], true);
        rt.process(&mut []);
        assert_eq!(rt.decks[deck].playing, !previous[deck]);
        assert_eq!(rt.decks[1 - deck].playing, previous[1 - deck]);
        rt.decks[deck].pos = 432.0;
        let response = exchange(&mut client, &json!({"op":"deckCue","deck":deck}));
        assert_eq!(response["accepted"], true);
        rt.process(&mut []);
        assert_eq!(rt.decks[deck].pos, rt.decks[deck].cue_pos);
    }
    for n in 0..engine::SCENES {
        let response = exchange(&mut client, &json!({"op":"scene","n":n}));
        assert_eq!(response["accepted"], true);
        rt.process(&mut []);
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().scene as usize == n));
    }
    drop(client);
    handler.join().unwrap().unwrap();
}
