use super::*;
use crate::ipc_follow::Cache;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::process::{Child, Command, Stdio};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-follow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("omatainer.sock")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn start(
    directory: &Directory,
    snapshot: Arc<Mutex<Snapshot>>,
) -> (IpcServer, CommandPort, impl Send) {
    let (commands, receiver) = CommandPort::channel(256);
    let server = start_at(&directory.socket(), commands.clone(), snapshot).unwrap();
    (server, commands, receiver)
}
fn connect(directory: &Directory) -> BufReader<UnixStream> {
    let mut stream = UnixStream::connect(directory.socket()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"{\"op\":\"follow\",\"id\":\"fixture\"}\n")
        .unwrap();
    BufReader::new(stream)
}
fn response(client: &mut impl BufRead) -> Value {
    let mut line = String::new();
    assert!(client.read_line(&mut line).unwrap() > 0);
    assert!(line.len() <= ipc_transport::RESPONSE_BYTES + 1);
    serde_json::from_str(&line).unwrap()
}
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn subscription_outlives_request_cap_without_accepting_commands_and_releases_its_worker() {
    let directory = Directory::new();
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let (server, commands, _receiver) = start(&directory, snapshot.clone());
    let mut client = connect(&directory);
    // Requests after subscription are never processed as control commands.
    client.get_mut().write_all(b"{\"op\":\"play\"}\n").unwrap();
    for frame in 0..36 {
        if frame == 10 {
            snapshot.lock().playing = true;
        }
        let status = response(&mut client);
        assert_eq!(status["id"], "fixture");
        assert_eq!(status["ok"], true);
        assert_eq!(status["follow"], true);
        assert_eq!(status["accepted"], Value::Null);
        assert_eq!(status["command_status"], Value::Null);
        if frame > 11 {
            assert_eq!(status["playing"], true);
        }
    }
    assert_eq!(server.clients.accepted.load(Ordering::Relaxed), 1);
    assert_eq!(server.clients.started.load(Ordering::Relaxed), 1);
    assert_eq!(commands.len(), 0);
    drop(client);
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
}

#[test]
fn cached_projection_is_exact_status_and_ignores_large_unrelated_state_without_allocation() {
    let (commands, _receiver) = CommandPort::channel(256);
    let mut snapshot = Snapshot::default();
    snapshot.midi = vec!["\u{1}".repeat(8192); 64];
    snapshot.decks = vec![
        crate::engine::DeckSnap {
            title: "\u{1}".repeat(8192),
            ..Default::default()
        };
        2
    ];
    let id = json!("\u{1}".repeat(128));
    let mut cache = Cache::default();
    let first: Value =
        serde_json::from_str(cache.update(&snapshot, &commands, &id).unwrap()).unwrap();
    assert_eq!(first["state_truncated"], true);
    let shared = Arc::new(Mutex::new(snapshot.clone()));
    let (client, peer) = UnixStream::pair().unwrap();
    let port = commands.clone();
    let worker = std::thread::spawn(move || crate::handle_client(peer, port, shared));
    let ordinary = crate::exchange_request(client, crate::STATUS_REQUEST).unwrap();
    worker.join().unwrap().unwrap();
    let mut ordinary: Value = serde_json::from_str(&ordinary).unwrap();
    ordinary["id"] = id.clone();
    ordinary["follow"] = json!(true);
    assert_eq!(first, ordinary);
    snapshot.tracks = vec![
        crate::engine::TrackSnap {
            name: "x".repeat(1_000_000),
            ..Default::default()
        };
        8
    ];
    snapshot.decks[0].peaks = Arc::new(vec![[1.0; 3]; 1_000_000]);
    let counts = crate::engine::test_alloc::measure(|| {
        for _ in 0..1000 {
            assert!(cache.update(&snapshot, &commands, &id).is_ok());
        }
    });
    assert_eq!(counts.allocations, 0, "{counts:?}");
    assert_eq!(counts.bytes, 0, "{counts:?}");
    assert_eq!(cache.serializations, 1);
    snapshot.beat_in_bar = 2.5;
    let changed: Value =
        serde_json::from_str(cache.update(&snapshot, &commands, &id).unwrap()).unwrap();
    assert_eq!(changed["beat"], 2.5);
    assert_eq!(cache.serializations, 2);
    snapshot.midi = vec!["controller connected".into()];
    snapshot.decks[0].title = "new deck".into();
    let changed: Value =
        serde_json::from_str(cache.update(&snapshot, &commands, &id).unwrap()).unwrap();
    assert_eq!(changed["midi"], json!(["controller connected"]));
    assert_eq!(changed["deckA"], "new deck");
    snapshot.fx_kind[2] = crate::engine::FxKind::Echo;
    snapshot.fx_wet[2] = 0.75;
    let changed: Value =
        serde_json::from_str(cache.update(&snapshot, &commands, &id).unwrap()).unwrap();
    assert_eq!(changed["master_fx"]["types"][2], "echo");
    assert_eq!(changed["master_fx"]["wet"][2], 0.75);
    assert_eq!(cache.serializations, 4);
}

#[test]
fn follow_slots_snapshot_contention_shutdown_and_slow_writes_remain_bounded() {
    let directory = Directory::new();
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let (server, commands, _receiver) = start(&directory, snapshot.clone());
    let mut clients: Vec<_> = (0..ipc_transport::CLIENTS)
        .map(|_| connect(&directory))
        .collect();
    for client in &mut clients {
        assert_eq!(response(client)["ok"], true);
    }
    let mut excess = connect(&directory);
    assert_eq!(response(&mut excess)["error_code"], "server_busy");
    assert_eq!(
        server.clients.peak.load(Ordering::Relaxed),
        ipc_transport::CLIENTS
    );
    let lock = snapshot.lock();
    let mut unavailable = false;
    for _ in 0..8 {
        if response(&mut clients[0])["error_code"] == "snapshot_unavailable" {
            unavailable = true;
            break;
        }
    }
    assert!(unavailable);
    drop(lock);
    assert_eq!(response(&mut clients[0])["ok"], true);
    let stats = server.clients.clone();
    let before = Instant::now();
    drop(server);
    assert!(before.elapsed() < Duration::from_secs(1));
    assert_eq!(stats.active.load(Ordering::Acquire), 0);
    assert!(!directory.socket().exists());
    assert_eq!(commands.len(), 0);

    let (mut peer, mut client) = UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    while peer.write(&[b'x'; 8192]).is_ok() {}
    peer.set_nonblocking(false).unwrap();
    let (commands, _receiver) = CommandPort::channel(256);
    let (sent, received) = std::sync::mpsc::channel();
    let handler = std::thread::spawn(move || {
        let result = crate::handle_client_with_limits(peer, commands, snapshot, Limits::default());
        sent.send(result).unwrap();
    });
    client.write_all(b"{\"op\":\"follow\"}\n").unwrap();
    assert!(received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_err());
    handler.join().unwrap();
}

struct Follower(Child);
impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Explicitly opt in with the freshly built production CLI. The server uses the
// same native start_at/handler code, on a fresh private runtime path only.
#[test]
#[ignore = "130-second native CLI soak; requires OMATAINER_TEST_BINARY"]
fn native_cli_multiminute_connection_counts_restart_backoff_and_clean_exit() {
    let binary = std::env::var_os("OMATAINER_TEST_BINARY").expect("build production CLI first");
    let directory = Directory::new();
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let (server, _, _receiver) = start(&directory, snapshot.clone());
    let mut follower = Follower(
        Command::new(binary)
            .args(["ctl", "follow"])
            .env("XDG_RUNTIME_DIR", &directory.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = follower.0.stdout.take().unwrap();
    let (sent, received) = std::sync::mpsc::sync_channel(16);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sent.send((Instant::now(), line.unwrap())).is_err() {
                break;
            }
        }
    });
    let began = Instant::now();
    let mut updates = 0;
    while began.elapsed() < Duration::from_secs(65) {
        let (_, line) = received.recv_timeout(Duration::from_secs(3)).unwrap();
        let status: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(status["ok"], true);
        updates += 1;
    }
    assert!(updates >= 240);
    assert_eq!(server.clients.accepted.load(Ordering::Relaxed), 1);
    assert_eq!(server.clients.started.load(Ordering::Relaxed), 1);
    assert_eq!(server.clients.peak.load(Ordering::Relaxed), 1);
    let first = server.clients.clone();
    drop(server);
    assert_eq!(first.active.load(Ordering::Acquire), 0);
    let mut errors = Vec::new();
    let outage = Instant::now();
    while outage.elapsed() < Duration::from_millis(2600) {
        let (when, line) = received.recv_timeout(Duration::from_secs(3)).unwrap();
        let status: Value = serde_json::from_str(&line).unwrap();
        if status["ok"] == false {
            errors.push(when);
        }
    }
    assert!(
        errors.len() >= 4 && errors.len() <= 5,
        "{} attempts",
        errors.len()
    );
    for (index, pair) in errors.windows(2).enumerate() {
        assert!(pair[1].duration_since(pair[0]) >= Duration::from_millis(220 * (1 << index)));
    }
    snapshot.lock().bpm = 147.0;
    let (server, _, _receiver2) = start(&directory, snapshot);
    let restored = Instant::now();
    loop {
        let (_, line) = received.recv_timeout(Duration::from_secs(5)).unwrap();
        let status: Value = serde_json::from_str(&line).unwrap();
        if status["ok"] == true {
            assert_eq!(status["bpm"], 147.0);
            break;
        }
        assert!(restored.elapsed() < Duration::from_secs(6));
    }
    while began.elapsed() < Duration::from_secs(130) {
        let (_, line) = received.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        updates += 1;
    }
    assert_eq!(server.clients.accepted.load(Ordering::Relaxed), 1);
    assert_eq!(server.clients.started.load(Ordering::Relaxed), 1);
    assert_eq!(server.clients.peak.load(Ordering::Relaxed), 1);
    // Closing the output consumer causes BrokenPipe, successful CLI exit, and
    // prompt socket/worker release without needing to kill the child.
    drop(received);
    reader.join().unwrap();
    wait_for(|| follower.0.try_wait().unwrap().is_some());
    assert!(follower.0.wait().unwrap().success());
    wait_for(|| server.clients.active.load(Ordering::Acquire) == 0);
    eprintln!("native ctl follow: elapsed={:?}, updates={updates}, accepted=2, handlers=2, peak=1, outage_attempts={}, clean_exit=true", began.elapsed(), errors.len());
}
