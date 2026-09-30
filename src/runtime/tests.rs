use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-rt-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn private(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        path
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn uid() -> u32 {
    crate::instance::effective_uid()
}

#[test]
fn trusted_xdg_directory_and_owned_socket_are_reused_without_changes() {
    let root = Directory::new();
    let runtime = root.private("runtime");
    let path = socket_path_for(Some(&runtime), &root.0.join("unused"), uid()).unwrap();
    assert_eq!(path, runtime.join("omatainer.sock"));
    let listener = UnixListener::bind(&path).unwrap();
    let identity = fs::symlink_metadata(&path).unwrap().ino();
    assert_eq!(
        socket_path_for(Some(&runtime), &root.0, uid()).unwrap(),
        path
    );
    assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), identity);
    assert_eq!(fs::metadata(runtime).unwrap().mode() & 0o7777, 0o700);
    drop(listener);
}

#[test]
fn missing_xdg_creates_a_private_uid_scoped_fallback_and_preserves_shared_occupant() {
    let root = Directory::new();
    let shared = root.0.join("omatainer.sock");
    fs::write(&shared, b"unrelated old shared endpoint").unwrap();
    let expected = root
        .0
        .join(format!("omatainer-{}", uid()))
        .join("omatainer.sock");
    assert_eq!(socket_path_for(None, &root.0, uid()).unwrap(), expected);
    assert_eq!(socket_path_for(None, &root.0, uid()).unwrap(), expected);
    let directory = fs::metadata(expected.parent().unwrap()).unwrap();
    assert_eq!(directory.uid(), uid());
    assert_eq!(directory.mode() & 0o7777, 0o700);
    assert_eq!(fs::read(shared).unwrap(), b"unrelated old shared endpoint");
    assert_ne!(
        expected.parent().unwrap(),
        root.0.join(format!("omatainer-{}", uid() + 1))
    );
}

#[test]
fn invalid_xdg_modes_fail_closed_without_repair_or_fallback() {
    let root = Directory::new();
    let runtime = root.private("runtime");
    for mode in [0o755, 0o770, 0o000, 0o1700] {
        fs::set_permissions(&runtime, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            socket_path_for(Some(&runtime), &root.0, uid())
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(fs::metadata(&runtime).unwrap().mode() & 0o7777, mode);
        assert!(!root.0.join(format!("omatainer-{}", uid())).exists());
    }
    fs::set_permissions(runtime, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn runtime_symlinks_including_dot_aliases_and_unsafe_ancestors_are_rejected() {
    let root = Directory::new();
    let actual = root.private("actual");
    let leaf = actual.join("leaf");
    DirBuilder::new().mode(0o700).create(&leaf).unwrap();
    let alias = root.0.join("alias");
    symlink(&actual, &alias).unwrap();
    for path in [&alias, &alias.join("."), &alias.join("leaf")] {
        assert!(
            socket_path_for(Some(path), &root.0, uid()).is_err(),
            "accepted {}",
            path.display()
        );
    }
    assert!(fs::symlink_metadata(alias)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::set_permissions(&actual, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(socket_path_for(Some(&leaf), &root.0, uid()).is_err());
    fs::set_permissions(&actual, fs::Permissions::from_mode(0o1777)).unwrap();
    assert_eq!(
        socket_path_for(Some(&leaf), &root.0, uid()).unwrap(),
        leaf.join("omatainer.sock")
    );
}

#[test]
fn fallback_collisions_and_bad_permissions_are_preserved() {
    let root = Directory::new();
    let fallback = root.0.join(format!("omatainer-{}", uid()));
    fs::write(&fallback, b"existing file").unwrap();
    assert!(socket_path_for(None, &root.0, uid()).is_err());
    assert_eq!(fs::read(&fallback).unwrap(), b"existing file");
    fs::remove_file(&fallback).unwrap();
    let actual = root.private("actual");
    symlink(&actual, &fallback).unwrap();
    assert!(socket_path_for(None, &root.0, uid()).is_err());
    assert!(fs::symlink_metadata(&fallback)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::remove_file(&fallback).unwrap();
    DirBuilder::new().mode(0o700).create(&fallback).unwrap();
    fs::set_permissions(&fallback, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(socket_path_for(None, &root.0, uid()).is_err());
    assert_eq!(fs::metadata(&fallback).unwrap().mode() & 0o7777, 0o755);
}

#[test]
fn wrong_owner_socket_and_directory_checks_do_not_modify_endpoints() {
    let root = Directory::new();
    let path = root.0.join("omatainer.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let inode = fs::symlink_metadata(&path).unwrap().ino();
    // Vary the expected effective UID instead of requiring privileged chown or
    // changing this test process's credentials. Exercise the real metadata test.
    assert!(validate_endpoint(&path, uid() + 1).is_err());
    assert!(validate_tree(&root.0, uid() + 1, true).is_err());
    assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), inode);
    assert_eq!(fs::metadata(&root.0).unwrap().uid(), uid());
    drop(listener);
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"regular collision").unwrap();
    assert!(socket_path_for(Some(&root.0), &root.0, uid()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"regular collision");
    fs::remove_file(&path).unwrap();
    let other = root.0.join("other");
    fs::write(&other, b"unrelated target").unwrap();
    symlink(&other, &path).unwrap();
    assert!(socket_path_for(Some(&root.0), &root.0, uid()).is_err());
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(other).unwrap(), b"unrelated target");
}

#[test]
fn concurrent_fallback_creation_converges_on_one_private_directory() {
    let root = Directory::new();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let path = root.0.clone();
            std::thread::spawn(move || socket_path_for(None, &path, uid()).unwrap())
        })
        .collect();
    let paths: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(paths.iter().all(|path| *path == paths[0]));
    assert_eq!(
        fs::metadata(paths[0].parent().unwrap()).unwrap().mode() & 0o7777,
        0o700
    );
}

#[test]
fn relative_parent_and_invalid_temporary_paths_cannot_create_runtime_state() {
    let root = Directory::new();
    for path in [
        Path::new(""),
        Path::new("relative"),
        &root.0.join("../escape"),
    ] {
        assert_eq!(
            socket_path_for(Some(path), &root.0, uid())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let missing = root.0.join("missing");
    assert!(socket_path_for(Some(&missing), &root.0, uid()).is_err());
    assert!(!missing.exists());
    let file = root.0.join("file");
    fs::write(&file, b"untouched").unwrap();
    assert!(socket_path_for(None, &file, uid()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"untouched");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}
