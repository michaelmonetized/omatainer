//! Validated per-user paths for IPC and its single-instance lock. Never use a
//! shared socket pathname, repair an untrusted directory, or remove occupants.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

pub fn socket_path() -> io::Result<PathBuf> {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    // Effective identity governs filesystem access. Do not trust $UID/$USER.
    let uid = crate::instance::effective_uid();
    let path = socket_path_for(xdg.as_deref(), &std::env::temp_dir(), uid)?;
    if xdg.is_none() {
        static WARNING: std::sync::Once = std::sync::Once::new();
        WARNING.call_once(|| {
            eprintln!(
                "omatainer: XDG_RUNTIME_DIR is unset; using private runtime directory {}",
                path.parent().unwrap().display()
            )
        });
    }
    Ok(path)
}

fn rejected(path: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("untrusted runtime path {}: {reason}", path.display()),
    )
}

fn validate_tree(path: &Path, uid: u32, private_leaf: bool) -> io::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "runtime directory must be an absolute path without '..': {}",
                path.display()
            ),
        ));
    }
    // Normalize redundant separators and '.' before checking each component;
    // otherwise `symlink/.` can hide the symlink from the ancestor walk.
    let normalized: PathBuf = path.components().collect();
    let path = normalized.as_path();
    // Checking every ancestor also rejects symlink aliases and private leaves
    // inside directories another UID can rename. Root and this EUID are the
    // only trusted owners; sticky shared temp directories protect owned leaves.
    for current in path.ancestors() {
        let metadata = fs::symlink_metadata(current).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("runtime directory {}: {error}", current.display()),
            )
        })?;
        if !metadata.file_type().is_dir() {
            return Err(rejected(
                current,
                "must be a directory, not a symlink or other file",
            ));
        }
        let mode = metadata.mode() & 0o7777;
        if current == path && private_leaf {
            if metadata.uid() != uid {
                return Err(rejected(
                    current,
                    &format!("must belong to effective UID {uid}"),
                ));
            }
            if mode != 0o700 {
                return Err(rejected(
                    current,
                    "must have permissions 0700; existing permissions were not changed",
                ));
            }
        } else {
            if metadata.uid() != 0 && metadata.uid() != uid {
                return Err(rejected(
                    current,
                    "ancestor must belong to root or the effective user",
                ));
            }
            if mode & 0o022 != 0 && mode & 0o1000 == 0 {
                return Err(rejected(
                    current,
                    "shared writable ancestor must have the sticky bit",
                ));
            }
        }
    }
    Ok(())
}

fn validate_endpoint(path: &Path, uid: u32) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(metadata) if !metadata.file_type().is_socket() => Err(rejected(
            path,
            "existing IPC endpoint must be a socket, not a symlink or other file",
        )),
        Ok(metadata) if metadata.uid() != uid => Err(rejected(
            path,
            &format!("existing socket must belong to effective UID {uid}"),
        )),
        Ok(_) => Ok(()),
    }
}

fn socket_path_for(xdg: Option<&Path>, temporary: &Path, uid: u32) -> io::Result<PathBuf> {
    let directory = match xdg {
        Some(directory) => {
            // Explicit but invalid XDG configuration fails closed. Silently
            // switching endpoints would separate the GUI and its ctl clients.
            validate_tree(directory, uid, true)?;
            directory.to_path_buf()
        }
        None => {
            validate_tree(temporary, uid, false)?;
            let directory = temporary.join(format!("omatainer-{uid}"));
            match DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "create private runtime directory {}: {error}",
                            directory.display()
                        ),
                    ))
                }
            }
            // No chmod/chown/unlink of pre-existing paths: collisions are
            // rejected, even if permissions alone would allow replacing them.
            validate_tree(&directory, uid, true)?;
            directory
        }
    };
    let socket = directory.join("omatainer.sock");
    validate_endpoint(&socket, uid)?;
    Ok(socket)
}

#[cfg(test)]
mod tests;
