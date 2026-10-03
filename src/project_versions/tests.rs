use super::*;
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-versions-{:x?}",
            crate::engine::midi_edit::NoteId::new().words()
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(value: f32) -> Bundle<serde_json::Value> {
    Bundle {
        state: serde_json::json!({"mix": value}),
        media: vec![Arc::new(Sample {
            name: "Recording".into(),
            path: "/missing/recording.wav".into(),
            sr: 48000,
            ch: 2,
            bpm: 120.0,
            peaks: Arc::new(vec![[0.0, 0.5, 0.25]]),
            data: vec![value; 8192],
        })],
    }
}
fn count(root: &Path, folder: &str) -> usize {
    fs::read_dir(root.join(folder)).unwrap().count()
}
#[test]
fn revisions_share_pcm_but_retain_per_version_metadata_and_exact_native_reopen() {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let one = fixture(0.25);
    let a = store
        .snapshot(
            &one,
            "First mix".into(),
            "Original balance".into(),
            &cancel,
            || Ok(()),
        )
        .unwrap()
        .0;
    let mut two = fixture(0.25);
    let sample = Arc::make_mut(&mut two.media[0]);
    sample.name = "Renamed recording".into();
    sample.path = "/new/alias.wav".into();
    sample.bpm = 128.0;
    let b = store
        .snapshot(
            &two,
            "Alt mix".into(),
            "Same recording".into(),
            &cancel,
            || Ok(()),
        )
        .unwrap()
        .0;
    assert_eq!(count(&root, "audio"), 1);
    assert_eq!(count(&root, "revisions"), 2);
    assert!(Store::open(&root, false, &cancel).is_err());
    drop(store);
    let store = Store::open(&root, false, &cancel).unwrap();
    assert_eq!(store.entries(), [a.clone(), b.clone()]);
    for (entry, expected) in [(&a, &one), (&b, &two)] {
        let actual = store.reopen::<serde_json::Value>(entry, &cancel).unwrap();
        assert_eq!(actual.state, expected.state);
        assert_eq!(actual.media[0].data, expected.media[0].data);
        assert_eq!(actual.media[0].name, expected.media[0].name);
        assert_eq!(actual.media[0].path, expected.media[0].path);
        assert_eq!(actual.media[0].bpm, expected.media[0].bpm);
    }
}
#[test]
fn pruning_previews_shared_dependencies_and_reclaims_only_reviewed_unused_assets() {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let a = store
        .snapshot(
            &fixture(0.25),
            "A".into(),
            String::new(),
            &cancel,
            || Ok(()),
        )
        .unwrap()
        .0;
    let b = store
        .snapshot(
            &fixture(0.25),
            "B".into(),
            String::new(),
            &cancel,
            || Ok(()),
        )
        .unwrap()
        .0;
    let c = store
        .snapshot(&fixture(0.5), "C".into(), String::new(), &cancel, || Ok(()))
        .unwrap()
        .0;
    let preview = store.review_prune(&[a.id.clone()], &cancel).unwrap();
    assert!(preview.orphaned_audio.is_empty());
    assert_eq!(preview.orphaned_revisions.len(), 1);
    let pruned = store.prune(preview, &cancel, || Ok(())).unwrap();
    assert_eq!(pruned.removed, 1);
    assert_eq!(pruned.reclaimed_audio, 0);
    assert_eq!(count(&root, "audio"), 2);
    let preview = store.review_prune(&[b.id.clone()], &cancel).unwrap();
    assert_eq!(preview.orphaned_audio.len(), 1);
    store.prune(preview, &cancel, || Ok(())).unwrap();
    assert_eq!(count(&root, "audio"), 1);
    assert_eq!(count(&root, "revisions"), 1);
    assert_eq!(
        store
            .reopen::<serde_json::Value>(&c, &cancel)
            .unwrap()
            .media[0]
            .data,
        fixture(0.5).media[0].data
    );
}
#[test]
fn refused_publication_preserves_index_and_leftovers_are_previewed_before_cleanup() {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let a = store
        .snapshot(
            &fixture(0.25),
            "A".into(),
            String::new(),
            &cancel,
            || Ok(()),
        )
        .unwrap()
        .0;
    let before = fs::read(root.join(INDEX)).unwrap();
    assert!(store
        .snapshot(
            &fixture(0.5),
            "Unpublished".into(),
            String::new(),
            &cancel,
            || Err::<(), _>("Protected".into())
        )
        .is_err());
    assert_eq!(fs::read(root.join(INDEX)).unwrap(), before);
    assert_eq!(store.entries(), [a.clone()]);
    let preview = store.review_prune(&[], &cancel).unwrap();
    assert!(preview.entries.is_empty());
    assert_eq!(preview.orphaned_audio.len(), 1);
    assert_eq!(preview.orphaned_revisions.len(), 1);
    let cancelled = AtomicBool::new(true);
    assert!(store.prune(preview, &cancelled, || Ok(())).is_err());
    assert_eq!(count(&root, "audio"), 2);
    let preview = store.review_prune(&[], &cancel).unwrap();
    store.prune(preview, &cancel, || Ok(())).unwrap();
    assert_eq!(count(&root, "audio"), 1);
    assert_eq!(count(&root, "revisions"), 1);
    assert!(store
        .snapshot(
            &fixture(0.25),
            " ".into(),
            String::new(),
            &cancel,
            || Ok(())
        )
        .is_err());
    let preview = store.review_prune(&[a.id.clone()], &cancel).unwrap();
    let asset = fs::read_dir(root.join("audio"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&asset, b"damaged").unwrap();
    assert!(store.prune(preview, &cancel, || Ok(())).is_err());
    assert_eq!(fs::read(root.join(INDEX)).unwrap(), before);
    assert!(store.reopen::<serde_json::Value>(&a, &cancel).is_err());
}
#[test]
fn foreign_symlink_and_changed_index_refuse_storage_mutation() {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    store
        .snapshot(
            &fixture(0.25),
            "A".into(),
            String::new(),
            &cancel,
            || Ok(()),
        )
        .unwrap();
    let outside = dir.0.join("original.omat");
    fs::write(&outside, b"preserve this").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        root.join("audio").join(format!("{}.omat", "0".repeat(64))),
    )
    .unwrap();
    assert!(store.review_prune(&[], &cancel).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"preserve this");
    fs::write(root.join(INDEX), b"damaged index").unwrap();
    assert!(store
        .snapshot(&fixture(0.5), "B".into(), String::new(), &cancel, || Ok(()))
        .is_err());
    drop(store);
    assert!(Store::open(&root, false, &cancel).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"preserve this");
}

#[test]
fn first_snapshot_initializes_an_existing_empty_folder_and_refuses_foreign_contents() {
    let dir = Folder::new();
    let root = dir.0.join("already-created");
    fs::create_dir(&root).unwrap();
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let entry = store.snapshot(&fixture(0.25), "Existing folder".into(), String::new(), &cancel, || Ok(())).unwrap().0;
    assert_eq!(store.reopen::<serde_json::Value>(&entry, &cancel).unwrap().media[0].data, fixture(0.25).media[0].data);
    drop(store);
    assert_eq!(Store::open(&root, true, &cancel).unwrap().entries(), [entry]);
    let foreign = dir.0.join("not-empty");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep.txt"), b"foreign contents").unwrap();
    assert!(Store::open(&foreign, true, &cancel).is_err());
    assert_eq!(fs::read(foreign.join("keep.txt")).unwrap(), b"foreign contents");
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);
    let link = dir.0.join("link");
    std::os::unix::fs::symlink(&foreign, &link).unwrap();
    assert!(Store::open(&link, true, &cancel).is_err());
    cancel.store(true, Ordering::Release);
    let cancelled = dir.0.join("cancelled-empty");
    fs::create_dir(&cancelled).unwrap();
    assert!(Store::open(&cancelled, true, &cancel).is_err());
    assert_eq!(fs::read_dir(&cancelled).unwrap().count(), 0);
}
