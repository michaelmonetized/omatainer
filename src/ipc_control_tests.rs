use super::*;
use engine::CommandPort;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;

fn request(client: &mut BufReader<UnixStream>, request: &str) -> Value {
    writeln!(client.get_mut(), "{request}").unwrap();
    let mut response = String::new();
    assert!(client.read_line(&mut response).unwrap() > 0);
    serde_json::from_str(&response).unwrap()
}

#[test]
fn ipc_reports_accepted_full_disconnected_and_query_states_truthfully() {
    let (tx, rx) = CommandPort::channel(28);
    let commands = tx.clone();
    let snapshot = Arc::new(Mutex::new(engine::Snapshot::default()));
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let server = std::thread::spawn(move || handle_client(server, commands, snapshot));
    let mut client = BufReader::new(client);

    let response = request(&mut client, r#"{"op":"play"}"#);
    assert_eq!(response["ok"], true);
    assert_eq!(response["accepted"], true);
    assert_eq!(response["command_status"], "accepted");
    assert_eq!(
        response["playing"], false,
        "queue acceptance is not execution"
    );
    assert_eq!(tx.len(), 1);

    tx.send(Command::Master(0.5)).unwrap();
    tx.send(Command::Master(0.6)).unwrap();
    let response = request(&mut client, r#"{"op":"play"}"#);
    assert_eq!(response["ok"], false);
    assert_eq!(response["accepted"], false);
    assert_eq!(response["command_status"], "rejected");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("queue is full"));
    assert_eq!(tx.len(), 3);

    // Ordinary status is read-only; follow streams the same bounded state fields.
    assert!(ipc_command(&serde_json::from_str(STATUS_REQUEST).unwrap())
        .unwrap()
        .is_none());
    let response = request(&mut client, STATUS_REQUEST);
    assert_eq!(response["ok"], true);
    assert_eq!(response["accepted"], Value::Null);
    assert_eq!(response["command_status"], Value::Null);
    assert_eq!(tx.len(), 3);

    assert!(matches!(rx.try_recv(), Ok(Command::Play)));
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    let response = request(&mut client, r#"{"op":"stop"}"#);
    assert_eq!(response["accepted"], true);
    assert!(matches!(
        rx.try_recv(),
        Ok(Command::ReservedStop { lane: 0, .. })
    ));
    let response = request(&mut client, r#"{"op":"stop"}"#);
    assert_eq!(response["accepted"], true);
    assert_eq!(response["command_status"], "coalesced");
    drop(rx);
    let response = request(&mut client, r#"{"op":"play"}"#);
    assert_eq!(response["ok"], false);
    assert_eq!(response["command_status"], "rejected");
    assert!(response["error"].as_str().unwrap().contains("disconnected"));

    for invalid in ["{", "{}", r#"{"op":"not-a-command"}"#] {
        let response = request(&mut client, invalid);
        assert_eq!(response["ok"], false);
        assert_eq!(response["accepted"], false);
    }
    assert_eq!(
        request(&mut client, &json!({"op": "ping"}).to_string())["ok"],
        true
    );
    drop(client);
    server.join().unwrap().unwrap();
}

#[test]
fn performance_ipc_confirms_safety_and_reports_actual_renderer_protection() {
    let (engine, mut rt) = engine::Engine::headless_for_test(48_000, 256);
    let commands = engine.cmd.clone();
    let snapshot = Arc::new(Mutex::new(engine::Snapshot::default()));
    let (client, server) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let server = std::thread::spawn(move || handle_client(server, commands, snapshot));
    let mut client = BufReader::new(client);
    for invalid in [r#"{"op":"emergencySilence"}"#, r#"{"op":"emergencySilence","confirm":false}"#, r#"{"op":"recoverPerformance","inputsReleased":1}"#, r#"{"op":"performanceMode","enabled":"true"}"#] {
        let response = request(&mut client, invalid);
        assert_eq!(response["accepted"], false, "{invalid}");
        assert!(!engine.cmd.performance().status().recovery);
    }
    assert_eq!(request(&mut client, r#"{"op":"performanceMode","enabled":true}"#)["accepted"], true);
    engine.send(Command::DeckPlay { deck: 0 }).unwrap(); rt.process(&mut [0.0; 256]);
    assert!(matches!(engine.send(Command::LoadBuiltin { deck: 0, stem: 1 }), Err(engine::SubmissionError::Performance(_))));
    let ack = request(&mut client, r#"{"op":"emergencySilence","confirm":true}"#);
    assert_eq!(ack["accepted"], true);
    assert_eq!(ack["performance"]["recovery"], true);
    assert_eq!(ack["performance"]["stopped"], false, "accepted is not yet applied");
    rt.process(&mut [0.0; 256]);
    let status = request(&mut client, STATUS_REQUEST);
    assert_eq!(status["performance"]["stopped"], true);
    assert_eq!(status["performance"]["output_muted"], true);
    assert_eq!(request(&mut client, r#"{"op":"play"}"#)["accepted"], false);
    assert_eq!(request(&mut client, r#"{"op":"recoverPerformance","inputsReleased":true}"#)["accepted"], true);
    rt.process(&mut []);
    assert!(!rt.playing && engine.cmd.performance().status().output_muted);
    drop(client); server.join().unwrap().unwrap();
}

#[test]
fn actual_deck_media_socket_requests_obey_explicit_locks_and_cannot_supply_an_override() {
    let (engine, mut rt) = engine::Engine::headless_for_test(48000, 256);
    engine.cmd.performance().set_deck_load_lock(0, true).unwrap();
    rt.apply(engine::Command::DeckPlay { deck: 0 }); rt.process(&mut [0.0; 256]);
    let original = rt.decks[0].audio.clone().unwrap();
    let commands = engine.cmd.clone();
    let snapshot = Arc::new(Mutex::new(engine::Snapshot::default()));
    let (client, server) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let owner = std::thread::spawn(move || handle_client(server, commands, snapshot));
    let mut client = BufReader::new(client);
    for payload in [r#"{"op":"deckLoad","deck":0}"#, r#"{"op":"deckUnload","deck":0}"#,
        r#"{"op":"deckLoad","deck":0,"confirm":true}"#, r#"{"op":"deckUnload","deck":2}"#] {
        let result = request(&mut client, payload);
        assert_eq!(result["accepted"], false, "{payload}: {result}");
    }
    assert_eq!(engine.cmd.ui_request_stats().pending, 0);
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].playing);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    rt.apply(engine::Command::DeckPlay { deck: 0 }); rt.process(&mut [0.0; 256]);
    assert_eq!(request(&mut client, r#"{"op":"deckUnload","deck":0}"#)["accepted"], true);
    assert!(rt.decks[0].audio.is_some());
    rt.process(&mut [0.0; 256]);
    assert!(rt.decks[0].audio.is_none());
    drop(client); owner.join().unwrap().unwrap();
}

#[test]
fn deck_media_cli_payloads_validate_targets_before_connecting() {
    for (op, wire) in [("deck-load", "deckLoad"), ("deck-unload", "deckUnload")] {
        for (target, deck) in [("A",0),("a",0),("B",1),("b",1)] {
            let value: Value = serde_json::from_str(&deck_media_payload(&[op.into(),target.into()]).unwrap()).unwrap();
            assert_eq!(value, json!({"op":wire,"deck":deck}));
        }
        for target in ["", "0", "2", "C", "A B"] { assert!(deck_media_payload(&[op.into(),target.into()]).is_err()); }
        assert!(deck_media_payload(&[op.into()]).is_err());
        assert!(deck_media_payload(&[op.into(),"A".into(),"extra".into()]).is_err());
    }
}

#[test]
fn explicit_file_load_captures_identity_without_changing_the_selected_library_row() {
    let root = std::env::temp_dir().join(format!("omatainer-file-load-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&root).unwrap();
    let media = root.join("stage track.mp3");
    std::fs::write(&media,b"captured original bytes").unwrap();
    let (engine,mut renderer) = engine::Engine::headless_for_test(48000,128);
    let before = renderer.decks[0].pos;
    engine.cmd.send(Command::DeckLoadFile {deck:1,path:media.clone()}).unwrap();
    assert_eq!(engine.cmd.len(),0);
    assert_eq!(renderer.decks[0].pos,before);
    let requests = engine.ui_requests.take_requests();
    let Some(engine::ui_requests::Request::Load(load)) = &requests[0] else { panic!("expected file load"); };
    assert_eq!(load.deck,1);
    assert_eq!(load.selection.source,engine::media_source::LibSource::File(media.clone()));
    assert_eq!(load.selection.fingerprint,engine::media_source::FileFingerprint::read(&media));
    assert_eq!(load.selection.title,"stage track");
    assert!(engine.cmd.send(Command::DeckLoadFile {deck:2,path:media.clone()}).is_err());
    assert!(engine.cmd.send(Command::DeckLoadFile {deck:0,path:"relative.mp3".into()}).is_err());
    assert!(engine.cmd.send(Command::DeckLoadFile {deck:0,path:root.clone()}).is_err());
    renderer.apply(Command::DeckPlay {deck:0});
    renderer.apply(Command::DeckLoadLock {deck:0,enabled:true});
    renderer.process(&mut [0.0;256]);
    assert!(renderer.decks[0].playing);
    assert!(engine.cmd.send(Command::DeckLoadFile {deck:0,path:media}).is_err());
    assert_eq!(engine.cmd.ui_request_stats().pending,0);
    std::fs::remove_dir_all(root).unwrap();
}
