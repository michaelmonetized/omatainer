//! Cooperative process ownership precedes audio startup. Socket probes are
//! nonblocking and never depend on application replies or delete an endpoint.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;

pub enum Acquisition {
    Owner(InstanceGuard),
    Existing,
}

pub struct InstanceGuard {
    // Closing this descriptor releases the kernel lock, including after a
    // crash. Never remove its pathname: a replacement inode would split locks.
    _lock: File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointState {
    Absent,
    Refused,
    Connected,
    // A full listen backlog or an in-progress connection is not stale.
    Busy,
}

pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no pointer arguments or preconditions.
    unsafe { libc::geteuid() }
}

pub fn acquire(socket: &Path) -> io::Result<Acquisition> {
    let lock_path = socket.with_extension("lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&lock_path)?;
    let identity = lock.metadata()?;
    if !identity.is_file() || identity.uid() != effective_uid() || identity.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "instance lock must be a private regular file owned by the effective user",
        ));
    }
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Acquisition::Existing),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    let named = std::fs::symlink_metadata(&lock_path)?;
    if !named.is_file() || named.dev() != identity.dev() || named.ino() != identity.ino() {
        return Err(io::Error::other(
            "instance lock pathname changed during acquisition",
        ));
    }
    let guard = InstanceGuard { _lock: lock };
    let endpoint = match std::fs::symlink_metadata(socket) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(metadata) = &endpoint {
        if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "existing endpoint must be a socket owned by the effective user",
            ));
        }
    }
    match probe(socket)? {
        EndpointState::Connected | EndpointState::Busy => Ok(Acquisition::Existing),
        EndpointState::Absent => Ok(Acquisition::Owner(guard)),
        EndpointState::Refused => {
            let original = endpoint.ok_or_else(|| {
                io::Error::other("IPC endpoint appeared during the ownership probe")
            })?;
            let current = std::fs::symlink_metadata(socket)?;
            if !current.file_type().is_socket()
                || current.uid() != effective_uid()
                || current.dev() != original.dev()
                || current.ino() != original.ino()
            {
                return Err(io::Error::other(
                    "IPC endpoint changed during stale cleanup",
                ));
            }
            // Only the exclusive lock holder may clean a refused, same-user
            // socket. Other probe errors and all responsive/busy peers survive.
            std::fs::remove_file(socket)?;
            Ok(Acquisition::Owner(guard))
        }
    }
}

pub fn probe(path: &Path) -> io::Result<EndpointState> {
    connection_attempt(path).map(|(state, _socket)| state)
}

/// One nonblocking connect attempt; callers choose their bounded retry policy.
pub(crate) fn connect(path: &Path) -> io::Result<std::os::unix::net::UnixStream> {
    let (state, socket) = connection_attempt(path)?;
    if state != EndpointState::Connected {
        let kind = match state {
            EndpointState::Absent => io::ErrorKind::NotFound,
            EndpointState::Refused => io::ErrorKind::ConnectionRefused,
            _ => io::ErrorKind::WouldBlock,
        };
        return Err(io::Error::new(kind, "omatainer control endpoint is unavailable"));
    }
    let stream = std::os::unix::net::UnixStream::from(socket);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

fn connection_attempt(path: &Path) -> io::Result<(EndpointState, OwnedFd)> {
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: all-zero sockaddr_un is valid storage before filling its family
    // and NUL-terminated pathname. The checked slice stays within sun_path.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Unix socket pathname",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    // SOCK_NONBLOCK is set at creation: even connect to a full listen backlog
    // cannot wait. CLOEXEC prevents spawned desktop helpers retaining the fd.
    // SAFETY: socket has no pointer arguments; ownership is checked below.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw is a fresh successful socket descriptor with unique ownership.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    // SAFETY: address points to initialized sockaddr_un storage; length covers
    // only its family and bounded, terminated pathname for this AF_UNIX socket.
    let connected = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    let state = if connected == 0 {
        EndpointState::Connected
    } else {
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ENOENT) => EndpointState::Absent,
            Some(libc::ECONNREFUSED) => EndpointState::Refused,
            Some(libc::EAGAIN | libc::EINPROGRESS | libc::EALREADY | libc::EINTR) => {
                EndpointState::Busy
            }
            Some(libc::EISCONN) => EndpointState::Connected,
            _ => return Err(error),
        }
    };
    Ok((state, socket))
}
