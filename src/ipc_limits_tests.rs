use super::*;
use crate::ipc_transport::{CLIENTS, REQUEST_BYTES, RESPONSE_BYTES};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-limits-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("control.sock")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn limits() -> Limits {
    Limits {
        idle: Duration::from_millis(80),
        read: Duration::from_millis(240),
        write: Duration::from_millis(60),
        snapshot: Duration::from_millis(20),
        requests: 32,
    }
}
fn snapshot() -> Arc<Mutex<Snapshot>> {
    Arc::new(Mutex::new(Snapshot::default()))
}
fn server(directory: &Directory, limits: Limits) -> (IpcServer, CommandPort, impl Send) {
    let (commands, rx) = CommandPort::channel(256);
    let server = start_at_with_limits(
        &directory.socket(),
        commands.clone(),
        snapshot(),
        |path| UnixListener::bind(path),
        |work| std::thread::Builder::new().spawn(work),
        limits,
    )
    .unwrap();
    (server, commands, rx)
}
fn connect(directory: &Directory) -> BufReader<UnixStream> {
    let stream = UnixStream::connect(directory.socket()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    BufReader::new(stream)
}
fn response(client: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    assert!(client.read_line(&mut line).unwrap() > 0);
    assert!(line.len() <= RESPONSE_BYTES + 1);
    serde_json::from_str(&line).unwrap()
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "IPC worker cleanup exceeded deadline"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn valid_after(directory: &Directory) {
    let result = crate::exchange_request(
        UnixStream::connect(directory.socket()).unwrap(),
        crate::STATUS_REQUEST,
    )
    .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&result).unwrap()["ok"], true);
}

#[test]
fn request_limit_accepts_exact_boundary_and_rejects_oversize_without_mutation() {
    let directory = Directory::new();
    let (server, commands, _rx) = server(&directory, limits());
    let mut client = connect(&directory);
    let mut payload = crate::STATUS_REQUEST.as_bytes().to_vec();
    payload.resize(REQUEST_BYTES, b' ');
    payload.push(b'\n');
    client.get_mut().write_all(&payload).unwrap();
    assert_eq!(response(&mut client)["ok"], true);
    payload.insert(REQUEST_BYTES, b' ');
    client.get_mut().write_all(&payload).unwrap();
    let rejected = response(&mut client);
    assert_eq!(rejected["error_code"], "request_too_large");
    assert!(rejected.to_string().len() < 512);
    assert_eq!(commands.len(), 0);
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
    valid_after(&directory);
}

#[test]
fn idle_and_dripping_clients_have_distinct_finite_read_deadlines() {
    let directory = Directory::new();
    let read_limits = Limits {
        idle: Duration::from_millis(200),
        read: Duration::from_millis(600),
        ..limits()
    };
    let (server, _, _rx) = server(&directory, read_limits);
    let start = Instant::now();
    let mut idle = connect(&directory);
    assert_eq!(response(&mut idle)["error_code"], "idle_timeout");
    assert!(start.elapsed() < Duration::from_secs(2));
    let mut slow = connect(&directory);
    let mut sender = slow.get_ref().try_clone().unwrap();
    let drip = std::thread::spawn(move || {
        for _ in 0..100 {
            if sender.write_all(b" ").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let start = Instant::now();
    assert_eq!(response(&mut slow)["error_code"], "read_deadline");
    assert!(start.elapsed() < Duration::from_secs(2));
    drip.join().unwrap();
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
    valid_after(&directory);
}

#[test]
fn concurrent_client_count_is_capped_and_valid_service_recovers() {
    let directory = Directory::new();
    let (server, commands, _rx) = server(
        &directory,
        Limits {
            idle: Duration::from_secs(10),
            read: Duration::from_secs(20),
            ..limits()
        },
    );
    let held: Vec<_> = (0..CLIENTS).map(|_| connect(&directory)).collect();
    wait_for(|| server.clients.active.load(Ordering::Acquire) == CLIENTS);
    let attacks: Vec<_> = (0..40)
        .map(|_| {
            let path = directory.socket();
            std::thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                // Busy peers are rejected before their commands are parsed.
                let _ = stream.write_all(b"{\"op\":\"play\"}\n");
                assert_eq!(
                    response(&mut BufReader::new(stream))["error_code"],
                    "server_busy"
                );
            })
        })
        .collect();
    for attack in attacks {
        attack.join().unwrap();
    }
    assert_eq!(server.clients.peak.load(Ordering::Acquire), CLIENTS);
    assert_eq!(server.clients.rejected.load(Ordering::Acquire), 40);
    assert_eq!(commands.len(), 0);
    drop(held);
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
    valid_after(&directory);
}

#[test]
fn server_drop_interrupts_and_joins_all_slow_clients_before_endpoint_cleanup() {
    let directory = Directory::new();
    let (server, _, _rx) = server(
        &directory,
        Limits {
            idle: Duration::from_secs(30),
            read: Duration::from_secs(60),
            ..limits()
        },
    );
    let mut held: Vec<_> = (0..CLIENTS).map(|_| connect(&directory)).collect();
    for client in &mut held {
        client.get_mut().write_all(b"{").unwrap();
    }
    wait_for(|| server.clients.active.load(Ordering::Acquire) == CLIENTS);
    let clients = server.clients.clone();
    let start = Instant::now();
    drop(server);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(clients.active.load(Ordering::Acquire), 0);
    assert!(!directory.socket().exists());
    for mut client in held {
        assert_eq!(client.read(&mut [0; 1]).unwrap(), 0);
    }
}

#[test]
fn bounded_connection_request_count_releases_slot_and_allows_fresh_connection() {
    let directory = Directory::new();
    let (server, _, _rx) = server(&directory, limits());
    let mut client = connect(&directory);
    for id in 0..limits().requests {
        writeln!(client.get_mut(), "{}", json!({"op":"status", "id":id})).unwrap();
        let reply = response(&mut client);
        assert_eq!(reply["id"], id);
        assert_eq!(reply["ok"], true);
    }
    assert_eq!(client.read(&mut [0; 1]).unwrap(), 0);
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
    valid_after(&directory);
}

#[test]
fn nonreading_peer_hits_total_write_deadline() {
    let (mut client, mut peer) = UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let padding = [b'x'; 8192];
    loop {
        match peer.write(&padding) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            result => panic!("failed to fill private socket: {result:?}"),
        }
    }
    peer.set_nonblocking(false).unwrap();
    let (commands, _rx) = CommandPort::channel(256);
    let (done, finished) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let start = Instant::now();
        let result = crate::handle_client_with_limits(peer, commands, snapshot(), limits());
        done.send((result, start.elapsed())).unwrap();
    });
    client.write_all(b"{\"op\":\"status\"}\n").unwrap();
    let (result, elapsed) = finished.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(result.is_err());
    assert!(elapsed < Duration::from_secs(2));
    worker.join().unwrap();
}

#[test]
fn oversized_prefix_memory_is_constant_in_supplied_payload_size() {
    for bytes in [REQUEST_BYTES + 1, 1024 * 1024] {
        let (client, peer) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut sender = client.try_clone().unwrap();
        let payload = vec![b'x'; bytes];
        let send = std::thread::spawn(move || {
            let _ = sender.write_all(&payload);
        });
        let (commands, _rx) = CommandPort::channel(256);
        let snap = snapshot();
        let worker = std::thread::spawn(move || {
            let mut result = Ok(());
            let counts = crate::engine::test_alloc::measure(|| {
                result = crate::handle_client_with_limits(peer, commands, snap, limits());
            });
            result.unwrap();
            counts
        });
        assert_eq!(
            response(&mut BufReader::new(client))["error_code"],
            "request_too_large"
        );
        send.join().unwrap();
        let counts = worker.join().unwrap();
        eprintln!("IPC prefix {bytes} bytes: {counts:?}");
        assert!(
            counts.bytes < 32 * 1024,
            "unbounded prefix allocation: {counts:?}"
        );
    }
}

#[test]
fn snapshot_contention_preserves_accepted_receipts_and_recovers() {
    let (client, peer) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (commands, rx) = CommandPort::channel(256);
    let snap = snapshot();
    let server_snapshot = snap.clone();
    let locked = snap.lock();
    let worker = std::thread::spawn(move || {
        crate::handle_client_with_limits(peer, commands, server_snapshot, limits())
    });
    let mut client = BufReader::new(client);
    writeln!(
        client.get_mut(),
        "{}",
        json!({"op":"play", "id":"accepted"})
    )
    .unwrap();
    let reply = response(&mut client);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["accepted"], true);
    assert_eq!(reply["state_available"], false);
    assert!(matches!(rx.try_recv(), Ok(crate::engine::Command::Play)));
    writeln!(client.get_mut(), "{}", json!({"op":"status", "id":"query"})).unwrap();
    let reply = response(&mut client);
    assert_eq!(reply["id"], "query");
    assert_eq!(reply["error_code"], "snapshot_unavailable");
    drop(locked);
    writeln!(
        client.get_mut(),
        "{}",
        json!({"op":"status", "id":"recovered"})
    )
    .unwrap();
    assert_eq!(response(&mut client)["state_available"], true);
    drop(client);
    worker.join().unwrap().unwrap();
}

#[test]
fn large_snapshot_metadata_has_bounded_response_and_avoids_full_snapshot_clone() {
    let snap = snapshot();
    {
        let mut snapshot = snap.lock();
        snapshot.midi = vec!["\u{1}".repeat(8192); 64];
        snapshot.commands.received = u64::MAX;
        snapshot.commands.applied = u64::MAX;
        snapshot.commands.coalesced = u64::MAX;
        snapshot.commands.budget_exhaustions = u64::MAX;
        snapshot.commands.received_last_block = usize::MAX;
        snapshot.commands.applied_last_block = usize::MAX;
        snapshot.commands.backlog = usize::MAX;
        snapshot.commands.high_water = usize::MAX;
        snapshot.decks = vec![
            crate::engine::DeckSnap {
                title: "\u{1}".repeat(8192),
                ..Default::default()
            };
            2
        ];
    }
    let (mut client, peer) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (commands, _rx) = CommandPort::channel(256);
    let worker = std::thread::spawn(move || {
        let mut result = Ok(());
        let counts = crate::engine::test_alloc::measure(|| {
            result = crate::handle_client_with_limits(peer, commands, snap, limits());
        });
        result.unwrap();
        counts
    });
    writeln!(
        client,
        "{}",
        json!({"op":"status", "id":"\u{1}".repeat(128)})
    )
    .unwrap();
    let mut client = BufReader::new(client);
    let reply = response(&mut client);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["state_truncated"], true);
    assert_eq!(reply["midi"].as_array().unwrap().len(), 8);
    drop(client);
    let counts = worker.join().unwrap();
    eprintln!("IPC bounded metadata response: {counts:?}");
    assert!(
        counts.bytes < 64 * 1024,
        "whole snapshot cloned: {counts:?}"
    );
}

#[test]
fn cli_rejects_oversized_unterminated_peer_response() {
    let (client, peer) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        let mut peer = BufReader::new(peer);
        let mut request = String::new();
        peer.read_line(&mut request).unwrap();
        let _ = peer.get_mut().write_all(&vec![b'x'; RESPONSE_BYTES + 1]);
    });
    let error = crate::exchange_request(client, crate::STATUS_REQUEST).unwrap_err();
    assert!(error.to_string().contains("byte limit"));
    server.join().unwrap();
}
