//! Fontconfig is Omarchy's canonical monospace selection source. Resolve it
//! afresh on the theme worker, including all fontconfig includes and caches.
use super::{FontSource, Resolver};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LIMIT: usize = 4096;
const DEADLINE: Duration = Duration::from_millis(500);
pub(super) struct Fontconfig {
    program: PathBuf,
    #[cfg(test)]
    pub(super) environment: Vec<(String, String)>,
}
impl Default for Fontconfig {
    fn default() -> Self {
        Self {
            program: "fc-match".into(),
            #[cfg(test)]
            environment: Vec::new(),
        }
    }
}
impl Resolver for Fontconfig {
    fn resolve(&mut self) -> Result<FontSource, String> {
        let mut command = Command::new(&self.program);
        command
            .args(["monospace", "-f", "%{family}\n%{file}\n%{index}\n"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(test)]
        command.envs(self.environment.iter().cloned());
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let result = (|| {
            nonblocking(stdout.as_raw_fd())?;
            nonblocking(stderr.as_raw_fd())?;
            let end = Instant::now() + DEADLINE;
            let mut out = Vec::new();
            let mut err = Vec::new();
            loop {
                drain(&mut stdout, &mut out)?;
                drain(&mut stderr, &mut err)?;
                if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                    drain(&mut stdout, &mut out)?;
                    drain(&mut stderr, &mut err)?;
                    if !status.success() || !err.is_empty() {
                        return Err(
                            "fontconfig reported an error; retaining the previous selection".into(),
                        );
                    }
                    let text = std::str::from_utf8(&out)
                        .map_err(|_| "fontconfig returned invalid UTF-8")?;
                    let mut fields = text.lines();
                    let family = fields
                        .next()
                        .and_then(|line| line.split(',').next())
                        .unwrap_or("")
                        .trim();
                    let path = fields.next().unwrap_or("");
                    let index = fields
                        .next()
                        .and_then(|value| value.parse().ok())
                        .ok_or("fontconfig returned an invalid face index")?;
                    if family.is_empty() || path.is_empty() || fields.next().is_some() {
                        return Err("fontconfig returned an incomplete selection".into());
                    }
                    return Ok(FontSource {
                        family: family.into(),
                        path: path.into(),
                        index,
                    });
                }
                if Instant::now() >= end {
                    return Err("fontconfig resolution exceeded 500 ms".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }
}
fn nonblocking(fd: RawFd) -> Result<(), String> {
    // These are newly owned child pipes; no other thread changes their flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        Err(io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
fn drain(input: &mut impl Read, output: &mut Vec<u8>) -> Result<(), String> {
    let mut bytes = [0; 1024];
    for _ in 0..8 {
        match input.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.len() + count > LIMIT {
                    return Err("fontconfig output exceeded 4096 bytes".into());
                }
                output.extend_from_slice(&bytes[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::reload::test_support::Fixture;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_fontconfig_process_has_finite_time_and_output_budgets() {
        let fixture = Fixture::new();
        let program = fixture.root.join("private-fontconfig");
        for (body, expected) in [
            ("exec sleep 2", "500 ms"),
            ("exec head -c 5000 /dev/zero", "4096 bytes"),
            ("printf 'bad\\nselection\\n'", "face index"),
            ("printf 'diagnostic\\n' >&2; exit 0", "reported an error"),
        ] {
            fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
            let mut resolver = Fontconfig {
                program: program.clone(),
                environment: Vec::new(),
            };
            let started = Instant::now();
            let error = resolver.resolve().unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }
}
