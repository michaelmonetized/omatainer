use super::*;
use crate::engine::Engine;
use std::time::Instant;

fn request(token: &str, payload: &Value) -> Vec<u8> {
    let payload = serde_json::to_vec(payload).unwrap();
    let mut packet = Vec::new();
    push_string(&mut packet, "/omatainer/v1");
    push_string(&mut packet, ",sb");
    push_string(&mut packet, token);
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(&payload);
    while packet.len() % 4 != 0 {
        packet.push(0);
    }
    packet
}
fn response(packet: &[u8]) -> Value {
    let mut cursor = 0;
    assert_eq!(string(packet, &mut cursor).unwrap(), "/omatainer/v1/reply");
    assert_eq!(string(packet, &mut cursor).unwrap(), ",b");
    let size = u32::from_be_bytes(packet[cursor..cursor + 4].try_into().unwrap()) as usize;
    cursor += 4;
    serde_json::from_slice(&packet[cursor..cursor + size]).unwrap()
}
fn wait(manager: &Manager, config: Config) -> Status {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = manager.status();
        if status.requested == config && !status.pending {
            return status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn query(socket: &UdpSocket, token: &str, payload: Value) -> Value {
    socket.send(&request(token, &payload)).unwrap();
    let mut packet = [0u8; 9000];
    let size = socket.recv(&mut packet).unwrap();
    response(&packet[..size])
}

#[test]
fn real_loopback_adapter_requires_the_current_token_and_uses_the_typed_api() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    let mut manager = Manager::new(engine.cmd.clone(), engine.snap.clone());
    let config = Config {
        enabled: true,
        port: 0,
    };
    manager.configure(config).unwrap();
    let status = wait(&manager, config);
    assert!(status.error.is_none());
    let port = status.port.unwrap();
    let token = status.token.unwrap();
    assert_eq!(token.len(), 32);
    let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket
        .connect((std::net::Ipv4Addr::LOCALHOST, port))
        .unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let discover = json!({"op":"api","version":1,"request":{"op":"discover"}});
    assert_eq!(
        query(&socket, &"0".repeat(32), discover.clone())["error_code"],
        "permission_denied"
    );
    let valid = query(&socket, &token, discover);
    assert_eq!(valid["result"]["version"], 1);
    assert_eq!(
        query(
            &socket,
            &token,
            json!({"op":"api","version":1,"request":{"op":"subscribe"}})
        )["error_code"],
        "unsupported_transport"
    );
    assert_eq!(
        query(&socket, &token, json!({"op":"play"}))["error_code"],
        "invalid_operation"
    );
    drop(manager);
    assert!(UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
}

#[test]
fn bind_failure_keeps_the_previous_listener_and_disable_releases_it() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let mut manager = Manager::new(engine.cmd.clone(), engine.snap.clone());
    let config = Config {
        enabled: true,
        port: 0,
    };
    manager.configure(config).unwrap();
    let old = wait(&manager, config);
    let old_port = old.port.unwrap();
    let occupied = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let blocked = Config {
        enabled: true,
        port: occupied.local_addr().unwrap().port(),
    };
    manager.configure(blocked).unwrap();
    let failed = wait(&manager, blocked);
    assert!(failed.error.is_some());
    assert_eq!(failed.port, old.port);
    assert_eq!(failed.token, old.token);
    assert_eq!(failed.applied, Some(config));
    let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket
        .connect((std::net::Ipv4Addr::LOCALHOST, old_port))
        .unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(
        query(
            &socket,
            old.token.as_ref().unwrap(),
            json!({"op":"api","version":1,"request":{"op":"discover"}})
        )["ok"],
        true
    );
    manager.configure(config).unwrap();
    let restored = wait(&manager, config);
    assert!(restored.error.is_none());
    assert_eq!(restored.token, old.token);
    let disabled = Config {
        enabled: false,
        port: 0,
    };
    manager.configure(disabled).unwrap();
    let stopped = wait(&manager, disabled);
    assert!(stopped.port.is_none() && stopped.token.is_none());
    assert!(UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, old_port)).is_ok());
    manager.configure(config).unwrap();
    let restarted = wait(&manager, config);
    assert_ne!(restarted.token, old.token);
}

#[test]
fn osc_codec_checks_alignment_padding_tags_lengths_and_keeps_unicode_in_the_blob() {
    let payload = json!({"op":"api","version":1,"request":{"op":"edit","name":"日本語"}});
    let packet = request(&"a".repeat(32), &payload);
    let (access, json) = decode(&packet).unwrap();
    assert_eq!(access, "a".repeat(32));
    assert_eq!(serde_json::from_slice::<Value>(json).unwrap(), payload);
    for size in 0..packet.len() {
        assert!(decode(&packet[..size]).is_err());
    }
    let mut extra = packet.clone();
    extra.extend_from_slice(&[0; 4]);
    assert!(decode(&extra).is_err());
    let mut padding = packet.clone();
    padding[13] = 1;
    assert!(decode(&padding).is_err());
    let mut tags = packet.clone();
    tags[18] = b's';
    assert!(decode(&tags).is_err());
    assert!(decode(b"#bundle\0\0\0\0\0\0\0\0\x01").is_err());
    assert!(Config {
        enabled: true,
        port: 80
    }
    .validate()
    .is_err());
}
