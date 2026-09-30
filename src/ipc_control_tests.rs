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
    let (tx, rx) = crossbeam_channel::bounded(1);
    let commands = CommandPort::new(tx.clone());
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

    let response = request(&mut client, r#"{"op":"stop"}"#);
    assert_eq!(response["ok"], false);
    assert_eq!(response["accepted"], false);
    assert_eq!(response["command_status"], "rejected");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("queue is full"));
    assert_eq!(tx.len(), 1);

    // ctl follow and ordinary status share this exact wire payload.
    assert!(ipc_command(&serde_json::from_str(STATUS_REQUEST).unwrap())
        .unwrap()
        .is_none());
    let response = request(&mut client, STATUS_REQUEST);
    assert_eq!(response["ok"], true);
    assert_eq!(response["accepted"], Value::Null);
    assert_eq!(response["command_status"], Value::Null);
    assert_eq!(tx.len(), 1);

    assert!(matches!(rx.try_recv(), Ok(Command::Play)));
    let response = request(&mut client, r#"{"op":"stop"}"#);
    assert_eq!(response["accepted"], true);
    assert!(matches!(rx.try_recv(), Ok(Command::Stop)));
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
