use super::*;
use std::os::unix::net::UnixStream;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("omatainer-startup-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
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

fn snapshot() -> Arc<Mutex<Snapshot>> {
    Arc::new(Mutex::new(Snapshot::default()))
}
fn spawn(work: Worker) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().spawn(work)
}

#[test]
fn bind_conflicts_preserve_live_socket_and_regular_file() {
    let directory = Directory::new();
    let path = directory.socket();
    let listener = UnixListener::bind(&path).unwrap();
    let identity = std::fs::metadata(&path).unwrap().ino();
    let (commands, _rx) = CommandPort::channel(256);
    let result = start_at_with(
        &path,
        commands,
        snapshot(),
        |p| UnixListener::bind(p),
        spawn,
    );
    assert!(matches!(result, Err(ref error) if error.kind() == io::ErrorKind::AddrInUse));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    assert!(UnixStream::connect(&path).is_ok());
    drop(listener);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"unrelated endpoint occupant").unwrap();
    let (commands, _rx) = CommandPort::channel(256);
    assert!(start_at_with(
        &path,
        commands,
        snapshot(),
        |p| UnixListener::bind(p),
        spawn
    )
    .is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"unrelated endpoint occupant"
    );
}

#[test]
fn bind_permission_and_worker_start_failures_return_before_startup_continues() {
    let directory = Directory::new();
    let path = directory.socket();
    let (commands, _rx) = CommandPort::channel(256);
    let result = start_at_with(
        &path,
        commands,
        snapshot(),
        |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected bind denial",
            ))
        },
        |_| panic!("worker started after failed bind"),
    );
    assert!(matches!(result, Err(ref error) if error.kind() == io::ErrorKind::PermissionDenied));
    assert!(!path.exists());
    let (commands, _rx) = CommandPort::channel(256);
    let result = start_at_with(
        &path,
        commands,
        snapshot(),
        |p| UnixListener::bind(p),
        |_| {
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "injected thread-start failure",
            ))
        },
    );
    assert!(matches!(result, Err(ref error) if error.to_string().contains("thread-start")));
    assert!(
        !path.exists(),
        "failed startup left its own stale socket behind"
    );
    assert!(UnixListener::bind(&path).is_ok());
}

#[test]
fn successful_startup_serves_requests_and_drop_removes_only_owned_endpoint() {
    let directory = Directory::new();
    let path = directory.socket();
    let (commands, _rx) = CommandPort::channel(256);
    let server = start_at_with(
        &path,
        commands,
        snapshot(),
        |p| UnixListener::bind(p),
        spawn,
    )
    .unwrap();
    let reply = crate::exchange_request(UnixStream::connect(&path).unwrap(), crate::STATUS_REQUEST)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply).unwrap()["ok"],
        true
    );
    drop(server);
    assert!(!path.exists());

    let (commands, _rx) = CommandPort::channel(256);
    let server = start_at_with(
        &path,
        commands,
        snapshot(),
        |p| UnixListener::bind(p),
        spawn,
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    let replacement = UnixListener::bind(&path).unwrap();
    let identity = std::fs::metadata(&path).unwrap().ino();
    drop(server);
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    assert!(UnixStream::connect(&path).is_ok());
    drop(replacement);
}

#[test]
fn connected_listener_is_preserved_without_requiring_a_prompt_reply() {
    let directory = Directory::new();
    let path = directory.socket();
    assert!(!has_listener(&path));
    let listener = UnixListener::bind(&path).unwrap();
    let identity = std::fs::metadata(&path).unwrap().ino();
    // The peer has not accepted or written any reply, but remains live.
    assert!(has_listener(&path));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    drop(listener);
    assert!(!has_listener(&path));
    assert!(
        path.exists(),
        "connection refusal must not authorize unlinking"
    );
}

#[test]
fn endpoint_cleanup_requires_matching_effective_owner_as_well_as_inode() {
    let directory = Directory::new();
    let path = directory.socket();
    let listener = UnixListener::bind(&path).unwrap();
    let inode = std::fs::symlink_metadata(&path).unwrap().ino();
    let mut endpoint = OwnedEndpoint::capture(&path).unwrap();
    // Inject an ownership mismatch without privileged chown or changing this
    // process's credentials. The same path/inode must still be preserved.
    endpoint.owner = crate::instance::effective_uid() + 1;
    drop(endpoint);
    assert_eq!(std::fs::symlink_metadata(&path).unwrap().ino(), inode);
    assert!(UnixStream::connect(&path).is_ok());
    drop(listener);
}
