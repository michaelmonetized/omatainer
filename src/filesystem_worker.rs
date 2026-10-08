//! Disposable filesystem work with bounded pipes and nonblocking retirement.
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::{
        io::{AsRawFd, FromRawFd},
        process::CommandExt,
    },
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const WIRE_LIMIT: usize = 2 * 1024 * 1024;
static OWNERS: AtomicUsize = AtomicUsize::new(0);
static RETIRING: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
fn retiring() -> &'static Mutex<Vec<Child>> {
    RETIRING.get_or_init(|| Mutex::new(Vec::new()))
}
fn reap() {
    retiring()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain_mut(|child| {
            if matches!(child.try_wait(), Ok(Some(_))) {
                OWNERS.fetch_sub(1, Ordering::AcqRel);
                false
            } else {
                true
            }
        });
}
struct Process(Option<Child>);
impl Drop for Process {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(Some(_))) {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            if !matches!(child.try_wait(), Ok(Some(_))) {
                retiring()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(child);
                return;
            }
        }
        OWNERS.fetch_sub(1, Ordering::AcqRel);
    }
}
fn nonblocking(fd: i32) -> Result<(), String> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

/// Exchange one bounded filesystem job with a disposable process.
/// Takes the app command, request, private descriptors, cancellation predicate and deadline; returns strict JSON, stopping failed/stalled jobs without waiting on kernel I/O.
pub(crate) fn invoke<Q: Serialize, R: DeserializeOwned>(
    mut command: Command,
    request: &Q,
    files: &[&File],
    active: &impl Fn() -> bool,
    timeout: Duration,
) -> Result<R, String> {
    if !active() {
        return Err("Filesystem job cancelled before startup".into());
    }
    if files.len() > 8 || timeout.is_zero() || timeout > Duration::from_secs(120) {
        return Err("Filesystem job exceeds descriptor/deadline bounds".into());
    }
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.len() > WIRE_LIMIT {
        return Err("Filesystem request exceeds 2 MiB".into());
    }
    let mut duplicates = Vec::new();
    for file in files {
        let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 16) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        duplicates.push(unsafe { File::from_raw_fd(fd) });
    }
    let descriptors = duplicates
        .iter()
        .map(AsRawFd::as_raw_fd)
        .collect::<Vec<_>>();
    let parent = std::process::id();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) < 0
                || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0
                || libc::getppid() as u32 != parent
            {
                return Err(std::io::Error::other(
                    "Filesystem owner exited before startup",
                ));
            }
            for (index, fd) in descriptors.iter().enumerate() {
                let target = 3 + index as i32;
                if libc::dup2(*fd, target) < 0 || libc::fcntl(target, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            for (kind, value) in [
                (libc::RLIMIT_CORE, 0),
                (libc::RLIMIT_NOFILE, 128),
                (libc::RLIMIT_AS, 3 * 1024 * 1024 * 1024),
                (libc::RLIMIT_CPU, 120),
                (libc::RLIMIT_FSIZE, 576 * 1024 * 1024 + 4096),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(kind, &limit) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            libc::nice(10);
            Ok(())
        });
    }
    reap();
    OWNERS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<4).then_some(n+1)).map_err(|_|"Four filesystem workers are active or awaiting kernel I/O retirement; narrow the roots and retry".to_string())?;
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            OWNERS.fetch_sub(1, Ordering::AcqRel);
            return Err(e.to_string());
        }
    };
    drop(duplicates);
    let mut process = Process(Some(child));
    let child = process.0.as_mut().unwrap();
    let mut input = Some(child.stdin.take().ok_or("Filesystem worker has no input")?);
    let mut output = child
        .stdout
        .take()
        .ok_or("Filesystem worker has no output")?;
    nonblocking(input.as_ref().unwrap().as_raw_fd())?;
    nonblocking(output.as_raw_fd())?;
    let started = Instant::now();
    let mut written = 0;
    let mut result = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        if !active() {
            return Err("Filesystem job cancelled; its worker was stopped".into());
        }
        if started.elapsed() > timeout {
            return Err(
                "Filesystem worker timed out on a stalled source; previous session preserved"
                    .into(),
            );
        }
        if let Some(input) = input.as_mut() {
            match input.write(&bytes[written..]) {
                Ok(n) => written += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if written == bytes.len() {
            input = None;
        }
        let exited = process
            .0
            .as_mut()
            .unwrap()
            .try_wait()
            .map_err(|e| e.to_string())?;
        loop {
            match output.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if result.len() + n > WIRE_LIMIT {
                        return Err("Filesystem worker output exceeds 2 MiB".into());
                    }
                    result.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if let Some(status) = exited {
            process.0 = None;
            OWNERS.fetch_sub(1, Ordering::AcqRel);
            if !status.success() {
                return Err("Filesystem worker crashed or exceeded its resource limits; previous session preserved".into());
            }
            if !active() {
                return Err("Filesystem job cancelled before publication".into());
            }
            return serde_json::from_slice(&result)
                .map_err(|e| format!("Invalid filesystem worker response: {e}"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Serve one strict bounded filesystem request before device startup.
/// Takes an operation over deserialized input; emits a bounded success/failure document without opening audio or MIDI devices.
pub(crate) fn serve<Q: DeserializeOwned, R: Serialize>(
    operation: impl FnOnce(Q) -> Result<R, String>,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((WIRE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > WIRE_LIMIT {
        return Err("Filesystem request exceeds 2 MiB".into());
    }
    let result = serde_json::from_slice(&bytes)
        .map_err(|e| e.to_string())
        .and_then(operation);
    let response = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    if response.len() > WIRE_LIMIT {
        return Err("Filesystem response exceeds 2 MiB".into());
    }
    std::io::stdout()
        .write_all(&response)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(text: &str) -> Command {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", text]);
        c
    }
    #[test]
    fn stalled_crashed_corrupt_and_oversized_workers_retire_without_blocking() {
        let started = Instant::now();
        assert!(invoke::<_, u32>(
            shell("sleep 20"),
            &0,
            &[],
            &|| true,
            Duration::from_millis(100)
        )
        .unwrap_err()
        .contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            invoke::<_, u32>(shell("exit 7"), &0, &[], &|| true, Duration::from_secs(1))
                .unwrap_err()
                .contains("crashed")
        );
        assert!(invoke::<_, u32>(
            shell("printf 'not-json'"),
            &0,
            &[],
            &|| true,
            Duration::from_secs(1)
        )
        .unwrap_err()
        .contains("Invalid"));
        assert!(invoke::<_, u32>(
            shell("head -c 2097153 /dev/zero"),
            &0,
            &[],
            &|| true,
            Duration::from_secs(1)
        )
        .unwrap_err()
        .contains("exceeds"));
        assert_eq!(
            invoke::<_, u32>(
                shell("cat >/dev/null; printf '42'"),
                &0,
                &[],
                &|| true,
                Duration::from_secs(1)
            )
            .unwrap(),
            42
        );
        let started = Instant::now();
        assert!(invoke::<_, u32>(
            shell("sleep 20"),
            &0,
            &[],
            &|| started.elapsed() < Duration::from_millis(50),
            Duration::from_secs(1)
        )
        .unwrap_err()
        .contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(invoke::<_, u32>(
            shell("sleep 20"),
            &0,
            &[],
            &|| false,
            Duration::from_secs(1)
        )
        .unwrap_err()
        .contains("cancelled"));
    }
}
