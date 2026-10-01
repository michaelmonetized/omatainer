use super::*;
use std::os::unix::{ffi::OsStrExt, fs::symlink};

fn snapshot(point: &str, id: u64) -> Snapshot {
    Snapshot {
        namespace: (1, 2),
        mounts: vec![Mount {
            id,
            device: Device(8, 17),
            access: Device(8, 17),
            btrfs: false,
            qualified: true,
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
fn descendants_keep_volume_identity_and_refuse_escape_or_foreign_mount() {
    let mut s = snapshot("/mnt/a", 1);
    let root = s.resolve(&source("music")).unwrap();
    let child = root.child(Path::new("/mnt/a/music/deep/song.wav")).unwrap();
    assert_eq!(child.source, source("music/deep/song.wav"));
    assert_eq!(child.stamp, root.stamp);
    assert_eq!(
        root.child(Path::new("/mnt/a/music/../outside.wav"))
            .unwrap_err(),
        Failure::Invalid
    );
    assert_eq!(
        root.child(Path::new("/different/song.wav")).unwrap_err(),
        Failure::Invalid
    );
    let mut foreign = s.mounts[0].clone();
    foreign.id = 2;
    foreign.point = "/mnt/a/music/deep".into();
    foreign.block = Some(Device(8, 33));
    s.mounts.push(foreign);
    assert_eq!(s.inspect(&child).unwrap_err(), Failure::Changed);
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
        s.identify_canonical("/mnt/set".into()).unwrap().source,
        source("music")
    );
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
    s.mounts[0].access = s.mounts[0].device;
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

#[test]
fn known_volume_identity_does_not_depend_on_later_removable_classification() {
    let root = std::env::temp_dir().join(format!(
        "omat-volume-classification-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("a.wav");
    std::fs::write(&path, b"fixture").unwrap();
    let mut snapshot = Snapshot::fixture_volume(&root, "TEST-A", 1);
    let location = snapshot.identify(&path).unwrap();
    snapshot.blocks[0].removable = false;
    assert!(matches!(
        snapshot.identify(&path).unwrap().source,
        LibSource::File(_)
    ));
    assert!(location.recheck_with(&snapshot).is_ok());
    let forged = Location {
        source: LibSource::Removable {
            volume_id: "TEST-A".into(),
            relative_path: "wrong.wav".into(),
        },
        ..location
    };
    assert_eq!(
        forged.recheck_with(&snapshot).unwrap_err(),
        Failure::Changed
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn btrfs_read_only_uapi_layout_and_foreign_device_refusal_are_exact() {
    assert_eq!(std::mem::size_of::<linux::BtrfsInfo>(), 1024);
    assert_eq!(std::mem::align_of::<linux::BtrfsInfo>(), 8);
    let root = std::env::temp_dir().join(format!(
        "omat-volume-device-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("a.wav");
    std::fs::write(&path, b"fixture").unwrap();
    let mut snapshot = Snapshot::fixture_volume(&root, "TEST-A", 1);
    let location = snapshot.identify(&path).unwrap();
    snapshot.mounts[0].access = Device(99, 99);
    assert_eq!(snapshot.inspect(&location).unwrap_err(), Failure::Changed);
    snapshot.mounts[0].btrfs = true;
    let location = snapshot.resolve(&location.source).unwrap();
    assert_eq!(
        snapshot.inspect(&location).unwrap_err(),
        Failure::Changed,
        "an unrelated filesystem cannot manufacture Btrfs UUID proof"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_nested_mount_root_is_searched_even_when_parent_cannot_cross_it() {
    use crate::engine::media_source::FileFingerprint;
    use crate::library::{Catalog, Metadata, Relocate, relocation_search};
    let dir = Temp::new();
    let original = dir.0.join("original.wav");
    let nested = dir.0.join("other-mount");
    std::fs::create_dir(&nested).unwrap();
    let copy = nested.join("renamed.wav");
    std::fs::write(&original, b"same verified content").unwrap();
    std::fs::copy(&original, &copy).unwrap();
    let mut snapshot = Snapshot::discover().unwrap();
    let mut foreign = snapshot.mount_at(&dir.0).unwrap().clone();
    foreign.id = snapshot.mounts.iter().map(|m|m.id).max().unwrap() + 1;
    foreign.point = nested.clone(); foreign.root = "/".into(); foreign.block = None;
    snapshot.mounts.push(foreign);
    let source = LibSource::File(original.clone());
    let fingerprint = FileFingerprint::read(&original).unwrap();
    let mut catalog = Catalog::default();
    catalog.upsert(source.clone(), Some(fingerprint), Metadata {
        title: "Original".into(), artist: "".into(), bpm: crate::ui::bpm::Bpm::UNKNOWN,
        key: "".into(), duration: None, last_play: None,
    }).unwrap();
    let target = Relocate { id:catalog.track(&source).unwrap().id.clone(), source, fingerprint, destination:PathBuf::new() };
    let parent_only = relocation_search::search_with_inventory(&catalog, &target, &[dir.0.clone()], &relocation_search::Progress::default(), ||true, &mut ||Ok(snapshot.clone())).unwrap();
    assert!(parent_only.matches.is_empty()); assert!(!parent_only.complete);
    let explicit = relocation_search::search_with_inventory(&catalog, &target, &[dir.0.clone(), nested], &relocation_search::Progress::default(), ||true, &mut ||Ok(snapshot.clone())).unwrap();
    assert_eq!(explicit.matches.len(), 1);
    assert_eq!(explicit.matches[0].location.path, copy);
    assert_eq!(explicit.skipped[relocation_search::Reason::ForeignMount as usize], 1);
}
