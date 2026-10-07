use super::*;
use std::{
    io::{Read, Write},
    os::unix::{io::AsRawFd, net::UnixStream, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct Process {
    child: Option<Child>,
    wire: UnixStream,
}
impl Process {
    /// Start a private plugin host process. Takes the current executable; returns a connected owner without opening audio or MIDI devices.
    pub fn start(executable: &Path) -> Result<Self, String> {
        let (wire, child_wire) = UnixStream::pair().map_err(|e| e.to_string())?;
        wire.set_nonblocking(true).map_err(|e| e.to_string())?;
        let fd = child_wire.as_raw_fd();
        let mut command = Command::new(executable);
        command
            .arg("vst3-worker")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let parent = std::process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent {
                    return Err(std::io::Error::other("Plugin owner exited before startup"));
                }
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                libc::setrlimit(libc::RLIMIT_CORE, &limit);
                Ok(())
            });
        }
        reserve()?;
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
                return Err(e.to_string());
            }
        };
        drop(child_wire);
        Ok(Self {
            child: Some(child),
            wire,
        })
    }
    /// Exchange one bounded request off the audio/UI threads.
    /// Takes a request, cancellation flag and deadline; returns a validated message or kills and reaps the failed worker.
    pub fn exchange(
        &mut self,
        request: &Request,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Response, String> {
        let data = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if data.len() > MAX_MESSAGE {
            return Err("Plugin request exceeds 32 MiB".into());
        }
        let mut message = Vec::with_capacity(data.len() + 4);
        message.extend_from_slice(&(data.len() as u32).to_le_bytes());
        message.extend_from_slice(&data);
        let started = Instant::now();
        let result = (|| {
            self.transfer(&mut message, false, cancel, started, timeout)?;
            let mut size = [0u8; 4];
            self.transfer(&mut size, true, cancel, started, timeout)?;
            let len = u32::from_le_bytes(size) as usize;
            if len > MAX_MESSAGE {
                return Err("Plugin worker exceeded its response bound".into());
            }
            let mut output = vec![0; len];
            self.transfer(&mut output, true, cancel, started, timeout)?;
            let response: Response = serde_json::from_slice(&output)
                .map_err(|e| format!("Invalid plugin worker response: {e}"))?;
            if let Response::Error { message } = &response {
                return Err(message.clone());
            }
            Ok(response)
        })();
        if result.is_err() {
            self.stop();
        }
        result
    }
    fn transfer(
        &mut self,
        bytes: &mut [u8],
        read: bool,
        cancel: &AtomicBool,
        started: Instant,
        timeout: Duration,
    ) -> Result<(), String> {
        let mut offset = 0;
        while offset < bytes.len() {
            if cancel.load(Ordering::Acquire) {
                return Err("Plugin work cancelled".into());
            }
            if started.elapsed() >= timeout {
                return Err(
                    "Plugin worker timed out; binary quarantined until an explicit retry".into(),
                );
            }
            match if read {
                self.wire.read(&mut bytes[offset..])
            } else {
                self.wire.write(&bytes[offset..])
            } {
                Ok(0) => return Err("Plugin worker disconnected or crashed".into()),
                Ok(n) => offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("Plugin worker failed: {e}")),
            }
        }
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            match child.try_wait() {
                Ok(Some(_)) => {
                    ACTIVE.fetch_sub(1, Ordering::AcqRel);
                }
                _ => retiring()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(child),
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static RETIRING: std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<Vec<Child>>>> =
    std::sync::OnceLock::new();
fn retiring() -> &'static std::sync::Arc<std::sync::Mutex<Vec<Child>>> {
    RETIRING.get_or_init(|| {
        let children = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Child>::with_capacity(96)));
        let owner = children.clone();
        let _ = std::thread::Builder::new()
            .name("plugin-reaper".into())
            .spawn(move || loop {
                {
                    let mut children = owner.lock().unwrap_or_else(|p| p.into_inner());
                    let mut i = 0;
                    while i < children.len() {
                        if matches!(children[i].try_wait(), Ok(Some(_))) {
                            children.swap_remove(i);
                            ACTIVE.fetch_sub(1, Ordering::AcqRel);
                        } else {
                            i += 1;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            });
        children
    })
}
fn reserve() -> Result<(), String> {
    retiring();
    ACTIVE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 96).then_some(n + 1)
        })
        .map(|_| ())
        .map_err(|_| {
            "Plugin worker limit reached; wait for retiring processes or narrow the graph".into()
        })
}
