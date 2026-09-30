use super::*;
use engine::CommandPort;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;

fn read_response(client: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    assert!(client.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

#[test]
fn malformed_unknown_and_invalid_requests_are_correlated_rejections_without_mutation() {
    let (port, rx) = CommandPort::channel(256);
    let commands = port.clone();
    let snapshot = Arc::new(Mutex::new(engine::Snapshot::default()));
    let before = serde_json::to_value(&*snapshot.lock()).unwrap();
    let server_snapshot = snapshot.clone();
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let handler = std::thread::spawn(move || handle_client(server, commands, server_snapshot));
    let mut client = BufReader::new(client);
    for (input, id) in [
        ("{", Value::Null),
        ("status", Value::Null),
        ("null", Value::Null),
        ("[]", Value::Null),
        ("42", Value::Null),
        (r#"{"id":1}"#, json!(1)),
        (r#"{"id":"bad-op","op":false}"#, json!("bad-op")),
        (r#"{"id":2,"op":"unknown"}"#, json!(2)),
        (r#"{"id":3,"op":"deckPlay","deck":256}"#, json!(3)),
        (r#"{"id":4,"op":"deckCue","deck":-1}"#, json!(4)),
        (r#"{"id":5,"op":"deckPlay"}"#, json!(5)),
        (r#"{"id":{},"op":"play"}"#, Value::Null),
    ] {
        writeln!(client.get_mut(), "{input}").unwrap();
        let response = read_response(&mut client);
        assert_eq!(response["ok"], false, "{input}");
        assert_eq!(response["id"], id);
        assert_eq!(response["accepted"], false);
        assert_eq!(response["command_status"], "rejected");
        assert!(response["error_code"].is_string());
        assert!(response["error"].is_string());
        assert_eq!(port.len(), 0);
        assert_eq!(serde_json::to_value(&*snapshot.lock()).unwrap(), before);
    }
    for id in [json!("valid-status"), json!(u64::MAX), Value::Null] {
        writeln!(client.get_mut(), "{}", json!({"op":"status", "id":id})).unwrap();
        let response = read_response(&mut client);
        assert_eq!(response["ok"], true);
        assert_eq!(response["id"], id);
        assert_eq!(response["accepted"], Value::Null);
    }
    writeln!(
        client.get_mut(),
        "{}",
        json!({"op":"deckPlay", "deck":1, "id":"go"})
    )
    .unwrap();
    let response = read_response(&mut client);
    assert_eq!(response["id"], "go");
    assert_eq!(response["accepted"], true);
    assert!(matches!(rx.try_recv(), Ok(Command::DeckPlay { deck: 1 })));
    drop(client);
    handler.join().unwrap().unwrap();
}

#[test]
fn partial_lines_wait_for_completion_and_truncated_eof_is_rejected() {
    let (port, _rx) = CommandPort::channel(256);
    let commands = port.clone();
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let handler = std::thread::spawn(move || {
        handle_client(
            server,
            commands,
            Arc::new(Mutex::new(engine::Snapshot::default())),
        )
    });
    client.write_all(br#"{"id":"split","op":"sta"#).unwrap();
    let error = client.read(&mut [0_u8; 1]).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    client.write_all(b"tus\"}\n").unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut client = BufReader::new(client);
    let response = read_response(&mut client);
    assert_eq!(response["ok"], true);
    assert_eq!(response["id"], "split");
    client.get_mut().write_all(b"{\"op\":").unwrap();
    client
        .get_mut()
        .shutdown(std::net::Shutdown::Write)
        .unwrap();
    assert_eq!(read_response(&mut client)["error_code"], "invalid_json");
    assert_eq!(port.len(), 0);
    handler.join().unwrap().unwrap();
}

#[test]
fn cli_exchange_rejects_server_protocol_failures_and_mismatched_ids() {
    for case in 0..7 {
        let (client, server) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            let mut peer = BufReader::new(server);
            let mut line = String::new();
            peer.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"], "status");
            assert!(request["id"].is_string());
            let reply = match case {
                0 => "not-json".into(),
                1 => json!({"id":request["id"],"ok":false,"error":"queue full"}).to_string(),
                2 => json!({"id":request["id"]}).to_string(),
                3 => json!({"id":request["id"],"ok":"true"}).to_string(),
                4 => json!({"id":"wrong","ok":true}).to_string(),
                5 => return,
                _ => json!({"id":request["id"],"ok":true,"accepted":null}).to_string(),
            };
            writeln!(peer.get_mut(), "{reply}").unwrap();
        });
        let result = exchange_request(client, STATUS_REQUEST);
        assert_eq!(result.is_ok(), case == 6, "case={case}: {result:?}");
        peer.join().unwrap();
    }
}
