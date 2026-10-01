use super::*;
use std::os::unix::{ffi::OsStrExt, fs::symlink};

fn snapshot(point: &str, id: u64) -> Snapshot {
    Snapshot {
        namespace: (1, 2),
        mounts: vec![Mount {
            id,
            device: Device(8, 17),
            root: "/".into(),
            point: point.into(),
            source: "/dev/sdb1".into(),
            block: Some(Device(8, 17)),
        }],
        blocks: vec![Block {
            device: Device(8, 17),
            uuid: "A1B2-C3D4".into(),
            removable: true,
        }],
    }
}
fn source(path: &str) -> LibSource {
    LibSource::Removable {
        volume_id: "A1B2-C3D4".into(),
        relative_path: path.into(),
    }
}

#[test]
fn remount_changes_only_io_location_not_persisted_source() {
    let first = snapshot("/run/media/user/OLD", 20);
    let picked = first
        .identify_canonical("/run/media/user/OLD/sets/deep/track.wav".into())
        .unwrap();
    assert_eq!(picked.source, source("sets/deep/track.wav"));
    let next = snapshot("/mnt/reconnected", 4);
    let resolved = next.resolve(&picked.source).unwrap();
    assert_eq!(
        resolved.path,
        Path::new("/mnt/reconnected/sets/deep/track.wav")
    );
    assert_eq!(resolved.source, picked.source);
    assert_ne!(resolved.stamp, picked.stamp);
    let serialized = serde_json::to_string(&picked.source).unwrap();
    assert!(!serialized.contains("OLD") && !serialized.contains("reconnected"));
}
#[test]
fn unmounted_and_duplicate_uuid_are_not_missing_file_observations() {
    let mut disconnected = snapshot("/mnt/a", 1);
    disconnected.mounts.clear();
    assert_eq!(
        disconnected.resolve(&source("x.wav")).unwrap_err(),
        Failure::Offline
    );
    disconnected.blocks.clear();
    assert_eq!(
        disconnected.resolve(&source("x.wav")).unwrap_err(),
        Failure::Offline
    );
    let mut cloned = snapshot("/mnt/a", 1);
    cloned.blocks.push(Block {
        device: Device(8, 33),
        uuid: "A1B2-C3D4".into(),
        removable: true,
    });
    assert_eq!(
        cloned.resolve(&source("x.wav")).unwrap_err(),
        Failure::Ambiguous
    );
    assert_eq!(
        cloned
            .identify_canonical("/mnt/a/x.wav".into())
            .unwrap_err(),
        Failure::Ambiguous
    );
}
#[test]
fn bind_roots_keep_filesystem_relative_identity_and_foreign_overmount_refuses() {
    let mut s = snapshot("/mnt/full", 1);
    let mut bind = s.mounts[0].clone();
    bind.id = 2;
    bind.root = "/music".into();
    bind.point = "/mnt/set".into();
    s.mounts.push(bind);
    assert_eq!(
        s.identify_canonical("/mnt/set/deep/a.wav".into())
            .unwrap()
            .source,
        source("music/deep/a.wav")
    );
    assert_eq!(
        s.resolve(&source("music/deep/a.wav")).unwrap().path,
        Path::new("/mnt/full/music/deep/a.wav")
    );
    s.mounts.remove(0);
    assert_eq!(
        s.resolve(&source("music/deep/a.wav")).unwrap().path,
        Path::new("/mnt/set/deep/a.wav")
    );
    assert_eq!(
        s.resolve(&source("other/a.wav")).unwrap_err(),
        Failure::NotVisible
    );
    let mut foreign = s.mounts[0].clone();
    foreign.id = 3;
    foreign.point = "/mnt/set/deep".into();
    foreign.block = Some(Device(8, 33));
    s.mounts.push(foreign);
    assert_eq!(
        s.resolve(&source("music/deep/a.wav")).unwrap_err(),
        Failure::NotVisible
    );
}
#[test]
fn malformed_relative_paths_and_unknown_namespaces_refuse_without_io() {
    let s = snapshot("/mnt/a", 1);
    for path in [
        "../x",
        "x/../y",
        "/absolute",
        "x/./y",
        "x//y",
        "x/",
        "nul\0x",
    ] {
        assert_eq!(
            s.resolve(&source(path)).unwrap_err(),
            Failure::Invalid,
            "{path:?}"
        );
    }
    assert_eq!(s.resolve(&source("")).unwrap().path, Path::new("/mnt/a"));
    let mut malformed = source("a");
    if let LibSource::Removable { volume_id, .. } = &mut malformed {
        *volume_id = "../../other".into();
    }
    assert_eq!(s.resolve(&malformed).unwrap_err(), Failure::Invalid);
    assert_eq!(
        s.resolve(&LibSource::File("relative".into())).unwrap_err(),
        Failure::Invalid
    );
    assert_eq!(
        s.resolve(&LibSource::Provider {
            provider: "test".into(),
            media_id: "1".into()
        })
        .unwrap_err(),
        Failure::Unsupported
    );
}
#[test]
fn parser_handles_kernel_escapes_optional_fields_and_non_utf8() {
    let mounts = parse_mounts(b"20 1 8:17 /music\\040set /mnt/a\\040b rw,nosuid shared:3 master:2 - ext4 /dev/sdb1 rw\n21 1 0:4 / /tmp/\xff rw - tmpfs tmpfs rw\n").unwrap();
    assert_eq!(mounts[0].root, Path::new("/music set"));
    assert_eq!(mounts[0].point, Path::new("/mnt/a b"));
    assert_eq!(mounts[0].source, Path::new("/dev/sdb1"));
    assert_eq!(mounts[1].point.as_os_str().as_bytes(), b"/tmp/\xff");
    assert_eq!(
        unescape(b"x\\011y\\012z\\134q")
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"x\ty\nz\\q"
    );
}
#[test]
fn parser_rejects_malformed_records_duplicates_and_capacity() {
    for bytes in [
        b"".as_slice(),
        b"1 0 8:1 / /mnt rw ext4 /dev/a rw\n",
        b"0 1 8:1 / /mnt rw - ext4 /dev/a rw\n",
        b"1 0 8:1 / /mnt\\099 rw - ext4 /dev/a rw\n",
        b"1 0 8:1:2 / /mnt rw - ext4 /dev/a rw\n",
        b"1 0 8:1 / /mnt rw - ext4 /dev/a rw extra\n",
        b"1 0 8:1 / /mnt rw - ext4 /dev/a rw\n1 0 8:1 / /mnt rw - ext4 /dev/a rw\n",
    ] {
        assert!(parse_mounts(bytes).is_err());
    }
    let mut bytes = Vec::new();
    for id in 1..=MAX_MOUNTS + 1 {
        bytes.extend_from_slice(format!("{id} 0 8:1 / /mnt/{id} rw - ext4 /dev/a rw\n").as_bytes());
    }
    assert_eq!(parse_mounts(&bytes).unwrap_err(), Failure::Capacity);
}
#[test]
fn overmount_ids_are_not_assumed_monotonic_and_absent_uuid_is_explicit() {
    let mut s = snapshot("/mnt/a", 10);
    let mut newer = s.mounts[0].clone();
    newer.id = 2;
    s.mounts.push(newer);
    assert_eq!(
        s.identify_canonical("/mnt/a/x".into()).unwrap_err(),
        Failure::Ambiguous
    );
    assert_eq!(s.resolve(&source("x")).unwrap_err(), Failure::Ambiguous);
    s.mounts.pop();
    s.blocks[0].uuid.clear();
    assert_eq!(
        s.identify_canonical("/mnt/a/x".into()).unwrap_err(),
        Failure::Unsupported
    );
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omat-location-{}",
            crate::performance_history::storage::new_id().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn actual_file_missing_symlink_and_namespace_guard_are_distinct() {
    let dir = Temp::new();
    let path = dir.0.join("a.wav");
    std::fs::write(&path, b"fixture").unwrap();
    let meta = path.metadata().unwrap();
    let mut s = snapshot(dir.0.to_str().unwrap(), 1);
    s.mounts[0].device = Device::from_raw(meta.dev());
    let location = s.resolve(&source("a.wav")).unwrap();
    assert_eq!(s.inspect(&location).unwrap().len(), 7);
    let mut reused = s.clone();
    reused.mounts[0].root = "/different".into();
    assert_eq!(reused.inspect(&location).unwrap_err(), Failure::Changed);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(s.inspect(&location).unwrap_err(), Failure::Missing);
    symlink("other.wav", &path).unwrap();
    assert_eq!(s.inspect(&location).unwrap_err(), Failure::Unsupported);
    std::fs::remove_file(&path).unwrap();
    s.namespace.1 += 1;
    assert_eq!(s.inspect(&location).unwrap_err(), Failure::Changed);
}
#[test]
fn real_host_mount_and_udev_inventory_are_read_only_and_bounded() {
    let s = Snapshot::discover().unwrap();
    assert!(!s.mounts.is_empty() && s.mounts.len() <= MAX_MOUNTS && s.blocks.len() <= MAX_BLOCKS);
    let dir = Temp::new();
    let path = dir.0.join("local.wav");
    std::fs::write(&path, b"local").unwrap();
    let location = s.identify(&path).unwrap();
    assert_eq!(location.source, LibSource::File(path.clone()));
    assert_eq!(s.inspect(&location).unwrap().len(), 5);
    location.recheck().unwrap();
}
