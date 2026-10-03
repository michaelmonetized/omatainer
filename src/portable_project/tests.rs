use super::*;
use crate::engine::{decode, media_source::LibSource};
use std::sync::Arc;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omat-portable-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn fixture(&self, name: &str) -> (PathBuf, Bundle<serde_json::Value>, Manifest) {
        let path = self.0.join(name);
        fs::write(
            &path,
            include_bytes!("../../tests/fixtures/audio/tone-tags.wav"),
        )
        .unwrap();
        let sample = Arc::new(
            decode::decode_sampler_file(&path, File::open(&path).unwrap(), 1024 * 1024, || false)
                .unwrap()
                .sample,
        );
        let key = Key {
            audio_hash: crate::project_dependencies::audio_hash(&sample, &AtomicBool::new(false))
                .unwrap(),
            original_path: sample.path.clone(),
        };
        let media = Media {
            key,
            name: sample.name.clone(),
            frames: sample.frames(),
            sample_rate: sample.sr,
            channels: sample.ch,
            source: Some(path.display().to_string()),
            collected: None,
            rights: "Unverified user audio".into(),
        };
        (
            path,
            Bundle {
                state: serde_json::json!({"notes": [1,2,3]}),
                media: vec![sample],
            },
            Manifest {
                schema: 1,
                application: "Omatainer test".into(),
                license_manifest: crate::licenses::MANIFEST.into(),
                license_notices: crate::licenses::NOTICES.into(),
                media: vec![media],
                devices: Vec::new(),
                unresolved: Vec::new(),
                entries: Vec::new(),
            },
        )
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn rewrite_manifest(path: &Path, change: impl FnOnce(&mut Manifest)) {
    let old = fs::read(path).unwrap();
    let length = u64::from_le_bytes(old[12..20].try_into().unwrap()) as usize;
    let mut manifest: Manifest = serde_json::from_slice(&old[20..20 + length]).unwrap();
    change(&mut manifest);
    let metadata = serde_json::to_vec(&manifest).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(metadata.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(&old[20 + length..old.len() - 32]);
    let hash = Sha256::digest(&bytes);
    bytes.extend_from_slice(&hash);
    fs::write(path, bytes).unwrap();
}

#[test]
fn collect_deduplicates_encoded_sources_and_import_survives_removed_originals() {
    let files = Files::new();
    let (original, bundle, mut manifest) = files.fixture("original.wav");
    let original_bytes = fs::read(&original).unwrap();
    let metadata = fs::metadata(&original).unwrap();
    let cancel = AtomicBool::new(false);
    let stage = Stage::new(&files.0).unwrap();
    let source = LibSource::File(original.clone());
    let key = &manifest.media[0].key;
    let (first, bytes) = collect(
        &stage,
        &source,
        key,
        bundle.media[0].data.len() as u64 * 4,
        MAX_SOURCE_BYTES,
        &cancel,
    )
    .unwrap();
    let (second, _) = collect(
        &stage,
        &source,
        key,
        bundle.media[0].data.len() as u64 * 4,
        MAX_SOURCE_BYTES - bytes,
        &cancel,
    )
    .unwrap();
    assert_eq!(first, second);
    manifest.media[0].collected = Some(first.clone());
    let archive = files.0.join("project.ompack");
    export(&archive, &stage, &bundle, manifest, &cancel).unwrap();
    assert_eq!(fs::read(&original).unwrap(), original_bytes);
    assert_eq!(
        crate::engine::media_source::FileFingerprint::from_metadata(&metadata),
        crate::engine::media_source::FileFingerprint::read(&original).unwrap()
    );
    fs::remove_file(&original).unwrap();
    drop(stage);
    let preview = preview(&archive, &cancel).unwrap();
    assert_eq!(preview.entries.len(), 2);
    assert_eq!(preview.license_notices, crate::licenses::NOTICES);
    assert!(preview
        .catalog()
        .unwrap()
        .manifest
        .entries
        .iter()
        .any(|entry| entry.category == "rust-component"));
    let (imported, manifest, restored) =
        extract::<serde_json::Value>(&archive, &files.0, &cancel).unwrap();
    assert_eq!(restored.media[0].data, bundle.media[0].data);
    assert_eq!(restored.state, bundle.state);
    assert_eq!(
        fs::read(
            imported
                .path
                .join(manifest.media[0].collected.as_ref().unwrap())
        )
        .unwrap(),
        original_bytes
    );
    let destination = files.0.join("imported");
    publish(imported, &destination, &cancel).unwrap();
    assert!(destination.join("session.omat").is_file());
    assert!(destination.join(first).is_file());
}

#[test]
fn corrupt_unsafe_oversized_and_incomplete_archives_never_publish() {
    let files = Files::new();
    let (_, bundle, manifest) = files.fixture("original.wav");
    let cancel = AtomicBool::new(false);
    let stage = Stage::new(&files.0).unwrap();
    let good = files.0.join("good.ompack");
    export(&good, &stage, &bundle, manifest, &cancel).unwrap();
    for (name, change) in [
        ("traversal", 0),
        ("absolute", 1),
        ("duplicate", 2),
        ("future", 3),
        ("missing", 4),
        ("shape", 5),
        ("oversized", 6),
        ("missing-notices", 7),
        ("oversized-notice-label", 8),
    ] {
        let path = files.0.join(name);
        fs::copy(&good, &path).unwrap();
        rewrite_manifest(&path, |manifest| match change {
            0 => manifest.entries[0].path = "../escaped".into(),
            1 => manifest.entries[0].path = "/tmp/escaped".into(),
            2 => manifest.entries.push(manifest.entries[0].clone()),
            3 => manifest.schema = 2,
            4 => manifest.media.clear(),
            5 => manifest.media[0].frames += 1,
            6 => manifest.entries[0].bytes = MAX_ARCHIVE_BYTES + 1,
            7 => manifest.license_notices = "{}".into(),
            _ => {
                let mut record: serde_json::Value =
                    serde_json::from_str(&manifest.license_manifest).unwrap();
                record["entries"][0]["name"] = serde_json::Value::String("x".repeat(4097));
                manifest.license_manifest = record.to_string();
            }
        });
        assert!(
            extract::<serde_json::Value>(&path, &files.0, &cancel).is_err(),
            "{name}"
        );
    }
    let damaged = files.0.join("damaged");
    let mut raw = fs::read(&good).unwrap();
    let last = raw.len() - 33;
    raw[last] ^= 1;
    fs::write(&damaged, &raw).unwrap();
    assert!(preview(&damaged, &cancel).is_ok());
    assert!(extract::<serde_json::Value>(&damaged, &files.0, &cancel).is_err());
    raw.truncate(raw.len() - 1);
    fs::write(&damaged, raw).unwrap();
    assert!(preview(&damaged, &cancel).is_err());
    assert!(!files.0.parent().unwrap().join("escaped").exists());
    assert_eq!(
        fs::read_dir(&files.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".omatainer-portable-"))
            .count(),
        1
    );
}

#[test]
fn cancellation_conflicts_and_symlinks_preserve_existing_data_and_clean_staging() {
    let files = Files::new();
    let (original, bundle, mut manifest) = files.fixture("original.wav");
    let cancel = AtomicBool::new(false);
    let stage = Stage::new(&files.0).unwrap();
    let staged = stage.path.clone();
    let destination = files.0.join("keep.ompack");
    fs::write(&destination, b"keep me").unwrap();
    assert!(export(&destination, &stage, &bundle, manifest.clone(), &cancel).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"keep me");
    let linked = files.0.join("linked");
    std::os::unix::fs::symlink(&original, &linked).unwrap();
    assert!(collect(
        &stage,
        &LibSource::File(linked.clone()),
        &manifest.media[0].key,
        1024 * 1024,
        MAX_SOURCE_BYTES,
        &cancel
    )
    .is_err());
    assert!(preview(&linked, &cancel).is_err());
    cancel.store(true, Ordering::Release);
    assert!(publish(stage, &files.0.join("new"), &cancel).is_err());
    assert!(!staged.exists());
    let stage = Stage::new(&files.0).unwrap();
    let staged = stage.path.clone();
    cancel.store(false, Ordering::Release);
    fs::create_dir(files.0.join("existing")).unwrap();
    fs::write(files.0.join("existing/keep"), b"keep").unwrap();
    assert!(publish(stage, &files.0.join("existing"), &cancel).is_err());
    assert!(!staged.exists());
    assert_eq!(fs::read(files.0.join("existing/keep")).unwrap(), b"keep");
    let stage = Stage::new(&files.0).unwrap();
    manifest.media[0].collected = Some(format!("{}.source", "é".repeat(32)));
    assert!(export(&files.0.join("unicode"), &stage, &bundle, manifest, &cancel).is_err());
}

#[test]
fn original_byte_limit_and_wrong_audio_are_rejected_without_touching_original() {
    let files = Files::new();
    let (original, bundle, mut manifest) = files.fixture("original.wav");
    let bytes = fs::read(&original).unwrap();
    let cancel = AtomicBool::new(false);
    let stage = Stage::new(&files.0).unwrap();
    assert!(collect(
        &stage,
        &LibSource::File(original.clone()),
        &manifest.media[0].key,
        1024 * 1024,
        bytes.len() as u64 - 1,
        &cancel
    )
    .is_err());
    manifest.media[0].key.audio_hash[0] ^= 1;
    assert!(collect(
        &stage,
        &LibSource::File(original.clone()),
        &manifest.media[0].key,
        bundle.media[0].data.len() as u64 * 4,
        MAX_SOURCE_BYTES,
        &cancel
    )
    .is_err());
    assert_eq!(fs::read(original).unwrap(), bytes);
}

#[test]
fn empty_directory_publication_refuses_removal_and_restores_an_inode_swapped_after_review() {
    for removed in [true,false] {
        let files = Files::new();
        let root = files.0.join("empty");fs::create_dir(&root).unwrap();
        let metadata=fs::metadata(&root).unwrap();let identity=(metadata.dev(),metadata.ino());
        let stage=Stage::new(&files.0).unwrap();fs::write(stage.path.join("staged.txt"),b"staged store").unwrap();
        let retained=files.0.join("reviewed-empty");
        let result=publish_directory_with(stage,&root,&AtomicBool::new(false),Some(identity),|| {
            fs::rename(&root,&retained).unwrap();
            if !removed {fs::create_dir(&root).unwrap();fs::write(root.join("foreign.txt"),b"foreign contents").unwrap();}
        });
        assert!(result.is_err());
        if removed {assert!(!root.exists());} else {assert_eq!(fs::read(root.join("foreign.txt")).unwrap(),b"foreign contents");assert!(!root.join("staged.txt").exists());}
        assert!(retained.is_dir());assert_eq!(fs::read_dir(&retained).unwrap().count(),0);
    }
}
