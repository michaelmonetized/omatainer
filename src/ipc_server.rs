use crate::engine::{CommandPort, Snapshot};
use crate::ipc_transport::{self, Limits};
use anyhow::Context;
use parking_lot::Mutex;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "ipc_limits_tests.rs"]
mod limits_tests;
#[cfg(test)]
#[path = "ipc_follow_tests.rs"]
mod follow_tests;
#[cfg(test)]
#[path = "ipc_startup_tests.rs"]
mod tests;

type Worker = Box<dyn FnOnce() + Send + 'static>;

struct OwnedEndpoint {
    path: PathBuf,
    device: u64,
    inode: u64,
    owner: u32,
}

impl OwnedEndpoint {
    fn capture(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            return Err(io::Error::other("new IPC endpoint is no longer a socket"));
        }
        if metadata.uid() != crate::instance::effective_uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "new IPC endpoint does not belong to the effective user",
            ));
        }
        Ok(Self {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
        })
    }
}

impl Drop for OwnedEndpoint {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
                && metadata.uid() == self.owner
                && self.owner == crate::instance::effective_uid()
        }) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Default)]
struct ClientStats {
    #[cfg(test)]
    accepted: AtomicUsize,
    #[cfg(test)]
    started: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    rejected: AtomicUsize,
}

struct ClientPermit(Arc<ClientStats>);
impl ClientPermit {
    fn new(stats: Arc<ClientStats>) -> Self {
        let active = stats.active.fetch_add(1, Ordering::AcqRel) + 1;
        stats.peak.fetch_max(active, Ordering::Relaxed);
        Self(stats)
    }
}
impl Drop for ClientPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ClientWorker {
    connection: UnixStream,
    worker: JoinHandle<()>,
}

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _endpoint: OwnedEndpoint,
    #[cfg_attr(not(test), allow(dead_code))]
    clients: Arc<ClientStats>,
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

pub fn has_listener(path: &Path) -> bool {
    matches!(
        crate::instance::probe(path),
        Ok(crate::instance::EndpointState::Connected | crate::instance::EndpointState::Busy)
    )
}

pub fn start(commands: CommandPort, snapshot: Arc<Mutex<Snapshot>>) -> anyhow::Result<IpcServer> {
    let path = crate::theme::socket_path()?;
    start_at(&path, commands, snapshot)
}

pub fn start_at(
    path: &Path,
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
) -> anyhow::Result<IpcServer> {
    start_at_with(
        path,
        commands,
        snapshot,
        |path| UnixListener::bind(path),
        |work| {
            std::thread::Builder::new()
                .name("omatainer-ipc".into())
                .spawn(work)
        },
    )
    .with_context(|| format!("IPC startup at {}", path.display()))
}

fn start_at_with(
    path: &Path,
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
    bind: impl FnOnce(&Path) -> io::Result<UnixListener>,
    spawn: impl FnOnce(Worker) -> io::Result<JoinHandle<()>>,
) -> io::Result<IpcServer> {
    start_at_with_limits(path, commands, snapshot, bind, spawn, Limits::default())
}

fn start_at_with_limits(
    path: &Path,
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
    bind: impl FnOnce(&Path) -> io::Result<UnixListener>,
    spawn: impl FnOnce(Worker) -> io::Result<JoinHandle<()>>,
    limits: Limits,
) -> io::Result<IpcServer> {
    // Binding and spawning are synchronous prerequisites for GUI startup. In
    // particular, a bind conflict never gives us ownership of the old path.
    let listener = bind(path)?;
    let endpoint = OwnedEndpoint::capture(path)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let clients = Arc::new(ClientStats::default());
    let worker_clients = clients.clone();
    let worker = spawn(Box::new(move || {
        let mut handlers: Vec<ClientWorker> = Vec::with_capacity(ipc_transport::CLIENTS);
        let mut last_error = None;
        while !worker_stop.load(Ordering::Acquire) {
            let mut index = 0;
            while index < handlers.len() {
                if handlers[index].worker.is_finished() {
                    let handler = handlers.swap_remove(index);
                    let _ = handler.worker.join();
                } else {
                    index += 1;
                }
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    #[cfg(test)]
                    worker_clients.accepted.fetch_add(1, Ordering::Relaxed);
                    if handlers.len() == ipc_transport::CLIENTS {
                        worker_clients.rejected.fetch_add(1, Ordering::Relaxed);
                        let _ = ipc_transport::reject(
                            &mut stream,
                            serde_json::Value::Null,
                            "server_busy",
                            "IPC client limit reached; retry after a connection closes",
                            limits.write,
                        );
                        continue;
                    }
                    let Ok(connection) = stream.try_clone() else {
                        continue;
                    };
                    let commands = commands.clone();
                    let snapshot = snapshot.clone();
                    let client_stop = worker_stop.clone();
                    let permit = ClientPermit::new(worker_clients.clone());
                    match std::thread::Builder::new()
                        .name("omatainer-ipc-client".into())
                        .spawn(move || {
                            let _permit = permit;
                            let _ = crate::handle_client_with_stop(
                                stream, commands, snapshot, limits, Some(&client_stop),
                            );
                        }) {
                        Ok(worker) => {
                            #[cfg(test)]
                            worker_clients.started.fetch_add(1, Ordering::Relaxed);
                            handlers.push(ClientWorker { connection, worker });
                        }
                        Err(error) => {
                            // A failed spawn drops the captured permit/stream.
                            worker_clients.rejected.fetch_add(1, Ordering::Relaxed);
                            if last_error.is_none_or(|last: Instant| {
                                last.elapsed() >= Duration::from_secs(1)
                            }) {
                                eprintln!("omatainer IPC client startup failed: {error}");
                                last_error = Some(Instant::now());
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::park_timeout(Duration::from_millis(10));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    // Resource pressure is recoverable; do not permanently lose
                    // control service because one accept temporarily fails.
                    if last_error
                        .is_none_or(|last: Instant| last.elapsed() >= Duration::from_secs(1))
                    {
                        eprintln!("omatainer IPC accept failed (retrying): {error}");
                        last_error = Some(Instant::now());
                    }
                    std::thread::park_timeout(Duration::from_millis(10));
                }
            }
        }
        // Interrupt blocked readers/writers before joining. Client threads and
        // their CommandPort/Snapshot references cannot outlive the server guard.
        for handler in &handlers {
            let _ = handler.connection.shutdown(std::net::Shutdown::Both);
        }
        for handler in handlers {
            let _ = handler.worker.join();
        }
    }))?;
    Ok(IpcServer {
        stop,
        worker: Some(worker),
        _endpoint: endpoint,
        clients,
    })
}
