use crate::engine::{CommandPort, Snapshot};
use anyhow::Context;
use parking_lot::Mutex;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(test)]
#[path = "ipc_startup_tests.rs"]
mod tests;

type Worker = Box<dyn FnOnce() + Send + 'static>;

struct OwnedEndpoint {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl OwnedEndpoint {
    fn capture(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            return Err(io::Error::other("new IPC endpoint is no longer a socket"));
        }
        Ok(Self {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

impl Drop for OwnedEndpoint {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        }) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _endpoint: OwnedEndpoint,
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
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

pub fn start(commands: CommandPort, snapshot: Arc<Mutex<Snapshot>>) -> anyhow::Result<IpcServer> {
    let path = crate::theme::socket_path();
    start_at_with(
        &path,
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
    // Binding and spawning are synchronous prerequisites for GUI startup. In
    // particular, a bind conflict never gives us ownership of the old path.
    let listener = bind(path)?;
    let endpoint = OwnedEndpoint::capture(path)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = spawn(Box::new(move || {
        while !worker_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let commands = commands.clone();
                    let snapshot = snapshot.clone();
                    if let Err(error) = std::thread::Builder::new()
                        .name("omatainer-ipc-client".into())
                        .spawn(move || {
                            let _ = crate::handle_client(stream, commands, snapshot);
                        })
                    {
                        eprintln!("omatainer IPC client startup failed: {error}");
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::park_timeout(Duration::from_millis(20));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    eprintln!("omatainer IPC listener failed: {error}");
                    break;
                }
            }
        }
    }))?;
    Ok(IpcServer {
        stop,
        worker: Some(worker),
        _endpoint: endpoint,
    })
}
