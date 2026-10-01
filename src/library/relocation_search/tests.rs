use super::*;
use crate::engine::preparation::Preparation;
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-relocation-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn target(tree: &Tree, verified: bool) -> (Catalog, Relocate) {
    let path = tree.file(
        "original/set/track.wav",
        b"actual verified relocation bytes",
    );
    let source = LibSource::File(path.clone());
    let fingerprint = FileFingerprint::read(&path).unwrap();
    let mut catalog = Catalog::default();
    let metadata = Metadata {
        title: "Prepared performance track".into(),
        artist: "Composer".into(),
        bpm: Bpm::new(125.0, Origin::User),
        key: "Am".into(),
        duration: Some(30.0),
        last_play: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123)),
    };
    let version = catalog
        .upsert(source.clone(), Some(fingerprint), metadata)
        .unwrap();
    version.preparation = Preparation::default();
    version.preparation.hotcues[0] = Some(3.25);
    let id = catalog.track(&source).unwrap().id.clone();
    if verified {
        let digest = content::hash_file(&path, fingerprint, || true).unwrap();
        catalog
            .qualify_verified_content(&id, &source, fingerprint, digest)
            .unwrap();
    }
    let crate_id = crates::CrateId("1".repeat(32));
    catalog
        .edit_crates(
            0,
            &crates::Edit::Create {
                id: crate_id.clone(),
                name: "Set".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    catalog
        .edit_crates(
            1,
            &crates::Edit::AddMembers {
                id: crate_id,
                members: vec![id.clone()],
                before: None,
            },
        )
        .unwrap();
    (
        catalog,
        Relocate {
            id,
            source,
            fingerprint,
            destination: PathBuf::new(),
        },
    )
}
#[test]
fn moved_renamed_tree_matches_bytes_and_reviews_every_identical_copy_without_mutation() {
    let tree = Tree::new();
    let (mut catalog, mut request) = target(&tree, true);
    let original = tree.0.join("original/set/track.wav");
    let good = tree.0.join("replacement/renamed/performance.data");
    fs::create_dir_all(good.parent().unwrap()).unwrap();
    fs::rename(&original, &good).unwrap();
    let duplicate = tree.file("replacement/another/track.wav", &fs::read(&good).unwrap());
    tree.file(
        "replacement/decoy/track.wav",
        b"actual unrelated relocation bytes",
    );
    let before = serde_json::to_vec(&catalog).unwrap();
    let prep = catalog.track(&request.source).unwrap().versions[0].preparation;
    let forest = catalog.crates.clone();
    let progress = Progress::default();
    let receipt = search(
        &catalog,
        &request,
        &[
            tree.0.join("replacement"),
            tree.0.join("replacement/renamed"),
        ],
        &progress,
        || true,
    )
    .unwrap();
    assert!(receipt.complete);
    assert_eq!(receipt.matches.len(), 2);
    assert_eq!(receipt.files, 3);
    assert_eq!(
        serde_json::to_vec(&catalog).unwrap(),
        before,
        "search/Cancel may not write associations"
    );
    assert!(receipt.matches.iter().any(|c| c.location.path == good));
    assert!(receipt.matches.iter().any(|c| c.location.path == duplicate));
    let candidate = receipt
        .matches
        .iter()
        .find(|c| c.location.path == good)
        .unwrap();
    candidate
        .verify(&Snapshot::discover().unwrap(), || true)
        .unwrap();
    request.destination = candidate.location.path.clone();
    catalog
        .relocate_reviewed(
            &request,
            candidate,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    let relocated = catalog.track(&LibSource::File(good.clone())).unwrap();
    assert_eq!(relocated.id, request.id);
    assert_eq!(relocated.versions[relocated.current].preparation, prep);
    assert_eq!(
        relocated.versions[relocated.current].metadata.last_play,
        Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123))
    );
    assert_eq!(catalog.crates, forest);
    assert_eq!(fs::read(&good).unwrap(), fs::read(duplicate).unwrap());
    assert!(catalog
        .version(&request.source, Some(request.fingerprint))
        .is_some());
}
#[test]
fn unverified_missing_original_and_stale_capture_refuse_search_and_leave_catalog_unchanged() {
    let tree = Tree::new();
    let (mut catalog, request) = target(&tree, false);
    tree.file("replacement/track.wav", b"actual verified relocation bytes");
    fs::remove_file(tree.0.join("original/set/track.wav")).unwrap();
    let before = serde_json::to_vec(&catalog).unwrap();
    assert!(search(
        &catalog,
        &request,
        &[tree.0.join("replacement")],
        &Progress::default(),
        || true
    )
    .unwrap_err()
    .contains("not verified"));
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
    let mut stale = request.clone();
    stale.id = TrackId("2".repeat(32));
    assert!(search(
        &catalog,
        &stale,
        &[tree.0.join("replacement")],
        &Progress::default(),
        || true
    )
    .unwrap_err()
    .contains("changed"));
    catalog
        .qualify_verified_content(&request.id, &request.source, request.fingerprint, [1; 32])
        .unwrap();
    assert!(search(
        &catalog,
        &request,
        &[tree.0.join("replacement")],
        &Progress::default(),
        || false
    )
    .unwrap_err()
    .contains("cancelled"));
}
#[test]
fn measured_original_digest_and_progress_are_real_without_implicit_persistence() {
    let tree = Tree::new();
    let (catalog, request) = target(&tree, false);
    let copy = tree.file(
        "replacement/renamed.mp3",
        b"actual verified relocation bytes",
    );
    let before = serde_json::to_vec(&catalog).unwrap();
    let progress = Progress::default();
    let receipt = search(
        &catalog,
        &request,
        &[tree.0.join("replacement")],
        &progress,
        || true,
    )
    .unwrap();
    assert_eq!(receipt.matches.len(), 1);
    assert_eq!(receipt.matches[0].location.path, copy);
    assert_eq!(receipt.bytes, 2 * request.fingerprint.byte_len());
    assert_eq!(
        receipt.original_hash,
        content::hash_file(&copy, FileFingerprint::read(&copy).unwrap(), || true).unwrap()
    );
    assert_eq!(progress.stage.load(Ordering::Relaxed), 3);
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
}
#[test]
fn incomplete_roots_symlinks_and_actual_bounds_cannot_claim_a_unique_complete_search() {
    let tree = Tree::new();
    let (catalog, request) = target(&tree, true);
    let good = tree.file(
        "replacement/a/renamed.wav",
        b"actual verified relocation bytes",
    );
    tree.file(
        "replacement/b/another.wav",
        b"actual verified relocation bytes",
    );
    std::os::unix::fs::symlink(&good, tree.0.join("replacement/shortcut.wav")).unwrap();
    let receipt = search(
        &catalog,
        &request,
        &[tree.0.join("replacement"), tree.0.join("unavailable")],
        &Progress::default(),
        || true,
    )
    .unwrap();
    assert!(!receipt.complete);
    assert_eq!(receipt.matches.len(), 2);
    assert!(
        receipt.skipped[Reason::Symlink as usize] > 0
            && receipt.skipped[Reason::Unavailable as usize] > 0
    );
    let before = serde_json::to_vec(&catalog).unwrap();
    for limits in [
        Limits {
            matches: 1,
            ..Limits::default()
        },
        Limits {
            bytes: 1,
            ..Limits::default()
        },
        Limits {
            visits: 1,
            ..Limits::default()
        },
        Limits {
            depth: 0,
            ..Limits::default()
        },
        Limits {
            directories: 1,
            ..Limits::default()
        },
    ] {
        let bounded = run(
            &catalog,
            &request,
            &[tree.0.join("replacement")],
            &Progress::default(),
            || true,
            &mut Snapshot::discover,
            limits,
        )
        .unwrap();
        assert!(!bounded.complete);
        assert!(bounded.skipped[Reason::Limit as usize] > 0);
        assert!(bounded.samples.len() <= 32);
        assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
    }
}
#[test]
fn changed_mount_and_reviewed_file_swap_refuse_before_association() {
    let tree = Tree::new();
    let (catalog, request) = target(&tree, true);
    let copy = tree.file("replacement/track.wav", b"actual verified relocation bytes");
    let first = Snapshot::fixture_volume(&tree.0, "search-108", 40);
    let second = Snapshot::fixture_volume(&tree.0, "search-108", 41);
    let mut calls = 0;
    let mut inventory = || {
        calls += 1;
        Ok(if calls == 1 {
            first.clone()
        } else {
            second.clone()
        })
    };
    assert!(run(
        &catalog,
        &request,
        &[tree.0.join("replacement")],
        &Progress::default(),
        || true,
        &mut inventory,
        Limits::default()
    )
    .unwrap_err()
    .contains("changed"));
    let receipt = search(
        &catalog,
        &request,
        &[tree.0.join("replacement")],
        &Progress::default(),
        || true,
    )
    .unwrap();
    fs::remove_file(&copy).unwrap();
    fs::write(&copy, b"wrong replacement bytes entirely").unwrap();
    assert!(receipt.matches[0]
        .verify(&Snapshot::discover().unwrap(), || true)
        .unwrap_err()
        .contains("changed"));
    let before = serde_json::to_vec(&catalog).unwrap();
    assert!(receipt.matches[0]
        .verify(&Snapshot::discover().unwrap(), || false)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
}
