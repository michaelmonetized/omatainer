use std::{
    io::Read,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Resolve a concrete installed tool.
/// Args: executable name. Returns a canonical executable path, never a shell command.
pub(super) fn executable(name: &str) -> Result<PathBuf, String> {
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let p = directory.join(name);
        if p.is_file() {
            return std::fs::canonicalize(p).map_err(|e| e.to_string());
        }
    }
    Err(format!("Install {name} for audio delivery"))
}
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// Stream bounded tool output with cancellation.
/// Args: argv command, cancellation, stderr policy and stdout consumer. Returns bounded stderr after successful process exit; every error kills and reaps the child.
pub(super) fn stream(
    command: &mut Command,
    cancel: &AtomicBool,
    logs: bool,
    mut consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    super::active(cancel)?;
    let mut child = Reap(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let mut stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
            return Err("Cannot configure cancellable audio tool pipes".into());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(12 * 3600);
    let mut errors = Vec::new();
    let mut bytes = [0; 16384];
    let mut eof = false;
    loop {
        super::active(cancel)?;
        if Instant::now() > deadline {
            return Err("Audio delivery tool deadline exceeded".into());
        }
        loop {
            match stderr.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    if errors.len() + n > 65536 {
                        return Err("Audio tool diagnostics exceed64KiB".into());
                    }
                    errors.extend_from_slice(&bytes[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        match stdout.read(&mut bytes) {
            Ok(0) => eof = true,
            Ok(n) => {
                consume(&bytes[..n])?;
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
        if eof {
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                let mut tail = Vec::new();
                stderr.read_to_end(&mut tail).map_err(|e| e.to_string())?;
                if errors.len() + tail.len() > 65536 {
                    return Err("Audio tool diagnostics exceed64KiB".into());
                }
                errors.extend_from_slice(&tail);
                if !status.success() || !logs && !errors.is_empty() {
                    return Err(format!(
                        "Audio tool failed ({status}): {}",
                        String::from_utf8_lossy(&errors)
                    ));
                }
                return Ok(errors);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
/// Collect small tool metadata through the same cancellable process boundary.
/// Args: command and cancellation. Returns up to64KiB stdout, refusing oversized metadata.
pub(super) fn capture(command: &mut Command, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    stream(command, cancel, false, |chunk| {
        if bytes.len() + chunk.len() > 65536 {
            return Err("Audio metadata exceeds64KiB".into());
        }
        bytes.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(bytes)
}
/// Bind exact local tool or input bytes.
/// Args: regular file path. Returns SHA256 using a fixed read buffer.
pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
