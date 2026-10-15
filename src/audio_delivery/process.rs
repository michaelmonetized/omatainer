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
/// Bind exact file bytes with bounded cancellable reads.
/// Args: a regular input path and cancellation. Returns SHA256 only after checking cancellation before opening and between every fixed-size read.
pub(crate) fn sha256_cancelled(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut file = regular(path, cancel)?;
    digest(&mut file, cancel)
}
#[cfg(test)]
pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    sha256_cancelled(path, &AtomicBool::new(false))
}
fn regular(path: &Path, cancel: &AtomicBool) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    super::active(cancel)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Delivery input must be a regular file".into());
    }
    super::active(cancel)?;
    Ok(file)
}
fn digest(reader: &mut impl Read, cancel: &AtomicBool) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        super::active(cancel)?;
        let n = reader.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    super::active(cancel)?;
    Ok(format!("{:x}", hash.finalize()))
}
/// Read a bounded regular metadata file without waiting on pipes or devices.
/// Args: path, maximum bytes and cancellation. Returns complete bytes; nonregular descriptors, overflow and cancellation are refused without modifying input.
pub(crate) fn read_regular(
    path: &Path,
    maximum: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut file = regular(path, cancel)?;
    read_bounded(&mut file, maximum, cancel)
}
fn read_bounded(
    reader: &mut impl Read,
    maximum: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut bytes = [0; 8192];
    loop {
        super::active(cancel)?;
        let n = reader.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if result.len().checked_add(n).is_none_or(|n| n > maximum) {
            return Err("Delivery metadata exceeds its byte limit".into());
        }
        result.extend_from_slice(&bytes[..n]);
    }
    super::active(cancel)?;
    Ok(result)
}
#[cfg(test)]
mod tests;
