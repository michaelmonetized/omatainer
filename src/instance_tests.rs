use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("omatainer-instance-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
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

fn owner(path: &Path) -> InstanceGuard {
    match acquire(path).unwrap() {
        Acquisition::Owner(guard) => guard,
        Acquisition::Existing => panic!("expected ownership"),
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "private fixture timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn lock_serializes_startup_before_binding_and_keeps_one_persistent_inode() {
    let directory = Directory::new();
    let path = directory.socket();
    let first = owner(&path);
    assert!(!path.exists(), "ownership must precede audio/IPC startup");
    let lock_path = path.with_extension("lock");
    let identity = std::fs::metadata(&lock_path).unwrap();
    assert_eq!(identity.mode() & 0o777, 0o600);
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    drop(first);
    let second = owner(&path);
    assert_eq!(std::fs::metadata(&lock_path).unwrap().ino(), identity.ino());
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    drop(second);
    assert!(
        lock_path.exists(),
        "removing the lock path would permit split ownership"
    );
}

#[test]
fn delayed_and_malformed_peer_does_not_gate_ownership_probe_or_lose_its_socket() {
    let directory = Directory::new();
    let path = directory.socket();
    let listener = UnixListener::bind(&path).unwrap();
    let identity = std::fs::metadata(&path).unwrap().ino();
    let worker = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(350));
        let _ = first.write_all(b"malformed response, deliberately late\n");
        let (mut second, _) = listener.accept().unwrap();
        second.write_all(b"still reachable\n").unwrap();
    });
    let started = Instant::now();
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    assert!(started.elapsed() < Duration::from_millis(200));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    let mut stream = UnixStream::connect(&path).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert_eq!(reply, "still reachable\n");
    worker.join().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
}

#[test]
fn full_listener_backlog_is_busy_and_probe_returns_without_waiting() {
    let directory = Directory::new();
    let path = directory.socket();
    let listener = UnixListener::bind(&path).unwrap();
    // SAFETY: this is a live fixture listener descriptor. Lower its listen
    // backlog so the test can fill it without relying on system defaults.
    assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 1) }, 0);
    let identity = std::fs::metadata(&path).unwrap().ino();
    let mut connections = Vec::new();
    let mut busy = false;
    for _ in 0..16 {
        let (state, connection) = connection_attempt(&path).unwrap();
        if state == EndpointState::Busy {
            busy = true;
            break;
        }
        assert_eq!(state, EndpointState::Connected);
        connections.push(connection);
    }
    assert!(busy, "fixture did not fill the listen backlog");
    let (tx, rx) = mpsc::channel();
    let probe_path = path.clone();
    let worker = std::thread::spawn(move || {
        let state = probe(&probe_path).unwrap();
        let existing = matches!(acquire(&probe_path).unwrap(), Acquisition::Existing);
        tx.send((state, existing)).unwrap();
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(500)).unwrap(),
        (EndpointState::Busy, true)
    );
    worker.join().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    drop(connections);
    drop(listener);
}

#[test]
fn only_exclusive_owner_removes_a_refused_stale_socket() {
    let directory = Directory::new();
    let path = directory.socket();
    let guard = owner(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let identity = std::fs::metadata(&path).unwrap().ino();
    drop(listener);
    assert_eq!(probe(&path).unwrap(), EndpointState::Refused);
    assert!(path.exists(), "a probe must not remove any endpoint");
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    drop(guard);
    let next = owner(&path);
    assert!(
        !path.exists(),
        "new lock owner should clear the refused stale socket"
    );
    let listener = UnixListener::bind(&path).unwrap();
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    assert!(UnixStream::connect(&path).is_ok());
    drop(listener);
    drop(next);
}

#[test]
fn regular_files_symlinks_and_invalid_paths_are_preserved() {
    let directory = Directory::new();
    let path = directory.socket();
    std::fs::write(&path, b"unrelated user file").unwrap();
    assert!(acquire(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated user file");
    std::fs::remove_file(&path).unwrap();
    let target = directory.0.join("real.sock");
    let listener = UnixListener::bind(&target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(acquire(&path).is_err());
    assert!(path.is_symlink());
    assert!(UnixStream::connect(&target).is_ok());
    std::fs::remove_file(&path).unwrap();
    let lock = path.with_extension("lock");
    std::fs::remove_file(&lock).unwrap();
    let user_file = directory.0.join("user-file");
    std::fs::write(&user_file, b"preserve me").unwrap();
    std::os::unix::fs::symlink(&user_file, &lock).unwrap();
    assert!(acquire(&path).is_err());
    assert_eq!(std::fs::read(&user_file).unwrap(), b"preserve me");
    assert!(lock.is_symlink());
    assert_eq!(
        probe(&directory.0.join("s".repeat(120)))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    drop(listener);
}

struct Children(Vec<Child>);
impl Children {
    fn spawn(&mut self, directory: &Directory) {
        self.0.push(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "instance::tests::process_contender",
                    "--nocapture",
                ])
                .env("OMATAINER_TEST_INSTANCE_DIRECTORY", &directory.0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
}
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn simultaneous_process_launches_have_one_owner_and_preserve_winning_endpoint() {
    let directory = Directory::new();
    let mut children = Children(Vec::new());
    for _ in 0..8 {
        children.spawn(&directory);
    }
    wait_for(|| {
        std::fs::read_dir(&directory.0)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-"))
            .count()
            == 8
    });
    std::fs::write(directory.0.join("start"), b"go").unwrap();
    wait_for(|| {
        std::fs::read_dir(&directory.0)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("result-"))
            .count()
            == 8
    });
    let results: Vec<_> = std::fs::read_dir(&directory.0)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("result-"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|value| value.as_str() == "owner")
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|value| value.as_str() == "existing")
            .count(),
        7
    );
    let path = directory.socket();
    let identity = std::fs::metadata(&path).unwrap().ino();
    assert_eq!(probe(&path).unwrap(), EndpointState::Connected);
    assert!(matches!(acquire(&path).unwrap(), Acquisition::Existing));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), identity);
    std::fs::write(directory.0.join("release"), b"done").unwrap();
    for child in &mut children.0 {
        assert!(child.wait().unwrap().success());
    }
    let _next = owner(&path);
}

#[test]
fn terminated_owner_releases_kernel_lock_and_next_process_can_recover_stale_socket() {
    let directory = Directory::new();
    std::fs::write(directory.0.join("start"), b"go").unwrap();
    let mut children = Children(Vec::new());
    children.spawn(&directory);
    wait_for(|| {
        std::fs::read_dir(&directory.0)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("result-"))
    });
    assert!(directory.socket().exists());
    let lock_path = directory.socket().with_extension("lock");
    let lock_inode = std::fs::metadata(&lock_path).unwrap().ino();
    children.0[0].kill().unwrap();
    children.0[0].wait().unwrap();
    assert_eq!(probe(&directory.socket()).unwrap(), EndpointState::Refused);
    let _guard = owner(&directory.socket());
    assert_eq!(std::fs::metadata(lock_path).unwrap().ino(), lock_inode);
    assert!(!directory.socket().exists());
    assert!(UnixListener::bind(directory.socket()).is_ok());
}

#[test]
fn process_contender() {
    let Some(directory) = std::env::var_os("OMATAINER_TEST_INSTANCE_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let pid = std::process::id();
    std::fs::write(directory.join(format!("ready-{pid}")), b"ready").unwrap();
    wait_for(|| directory.join("start").exists());
    let socket = directory.join("control.sock");
    let report = |result: &[u8]| {
        let temporary = directory.join(format!(".result-{pid}"));
        std::fs::write(&temporary, result).unwrap();
        std::fs::rename(temporary, directory.join(format!("result-{pid}"))).unwrap();
    };
    match acquire(&socket).unwrap() {
        Acquisition::Existing => report(b"existing"),
        Acquisition::Owner(_guard) => {
            let listener = UnixListener::bind(&socket).unwrap();
            report(b"owner");
            wait_for(|| directory.join("release").exists());
            drop(listener);
            std::fs::remove_file(socket).unwrap();
        }
    }
}
