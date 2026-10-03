use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::time::Duration;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    name: String,
    notes: Vec<(u8, f64, f64)>,
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-recovery97-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn sample() -> Arc<Sample> {
    Arc::new(Sample {
        name: "stereo recovery".into(),
        path: "/missing/recovery-original.wav".into(),
        sr: 48_000,
        ch: 2,
        bpm: 127.0,
        data: (0..1024)
            .map(|i| {
                if i == 0 {
                    -0.0
                } else {
                    (i as f32 * 0.017).sin() * 0.3
                }
            })
            .collect(),
        peaks: Arc::new(vec![[-0.3, 0.3, 0.1]; 8]),
    })
}
fn bundle(index: u64, sample: &Arc<Sample>) -> Bundle<State> {
    Bundle {
        state: State {
            name: format!("Untitled {index}"),
            notes: vec![(60, index as f64 / 4.0, 0.75), (67, 2.0, 1.5)],
        },
        media: vec![sample.clone()],
    }
}
fn meta(index: u64) -> RecordMeta {
    RecordMeta {
        epoch: 7,
        revision: index,
        view_revision: index + 1,
        saved_path: None,
        captured_unix_ms: 1000 + index,
    }
}
fn no() -> AtomicBool {
    AtomicBool::new(false)
}
fn latest(root: &Path) -> Candidate {
    let mut inventory = discover(root, &no()).unwrap();
    assert_eq!(inventory.candidates.len(), 1, "{:?}", inventory.warnings);
    inventory.candidates.remove(0)
}

#[test]
fn concurrent_recovery_readers_allow_exact_lookup_and_restore_but_exclude_writers_and_deletion() {
    let root=Root::new();let audio=sample();let mut store=Store::open(&root.0).unwrap();
    let commit=store.append(&bundle(1,&audio),meta(1),&Config::default(),&no()).unwrap();
    let session=store.session_id().to_owned();let digest=session_digest(&session);
    assert!(session_read_lock(&root.0,&session).unwrap().is_none());
    assert!(lookup_exact(&root.0,digest,7,commit.sequence,&no()).is_err());
    drop(store);
    let (_,reader)=session_read_lock(&root.0,&session).unwrap().unwrap();
    let second=session_read_lock(&root.0,&session).unwrap().unwrap();
    assert!(session_lock(&root.0,&session).unwrap().is_none());
    let candidate=lookup_exact(&root.0,digest,7,commit.sequence,&no()).unwrap().unwrap();
    let discovered=discover(&root.0,&no()).unwrap();assert_eq!(discovered.candidates.len(),1);
    assert_eq!(discovered.candidates[0].record_digest(),candidate.record_digest());
    let recovered:Recovered<State>=recover(&candidate,&no()).unwrap();assert_eq!(recovered.bundle.state,bundle(1,&audio).state);assert_sample(&audio,&recovered.bundle.media[0]);
    assert!(discard(&candidate,&no()).is_err());
    assert!(lookup_exact(&root.0,digest,7,commit.sequence,&no()).unwrap().is_some());
    drop(second);drop(reader);
    discard(&candidate,&no()).unwrap();assert!(lookup_exact(&root.0,digest,7,commit.sequence,&no()).unwrap().is_none());
}

#[test]
fn retired_recovery_cleanup_waits_for_readers_and_finishes_after_their_release() {
    let root=Root::new();let audio=sample();let mut store=Store::open(&root.0).unwrap();
    let commit=store.append(&bundle(1,&audio),meta(1),&Config::default(),&no()).unwrap();
    let session=store.session_id().to_owned();drop(store);
    let path=root.0.join(&session);retire_marker(&path,7,commit.sequence,&no()).unwrap();
    let (_,reader)=session_read_lock(&root.0,&session).unwrap().unwrap();
    let inventory=discover(&root.0,&no()).unwrap();
    assert!(inventory.candidates.is_empty());assert!(inventory.warnings.iter().any(|warning|warning.contains("being read; cleanup deferred")));assert!(path.exists());
    drop(reader);let inventory=discover(&root.0,&no()).unwrap();assert!(inventory.candidates.is_empty());assert!(!path.exists());
}
fn assert_sample(a: &Sample, b: &Sample) {
    assert_eq!(
        (a.sr, a.ch, a.bpm, &a.name, &a.path),
        (b.sr, b.ch, b.bpm, &b.name, &b.path)
    );
    assert_eq!(a.peaks, b.peaks);
    assert_eq!(
        a.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}
#[test]
fn journal_roundtrip_untitled_media_dedup_identity_and_active_session_exclusion() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    for revision in 1..=4 {
        let commit = store
            .append(
                &bundle(revision, &audio),
                meta(revision),
                &Config::default(),
                &no(),
            )
            .unwrap();
        assert!(commit.durable);
        assert_eq!(commit.sequence, revision);
    }
    assert!(discover(&root.0, &no()).unwrap().candidates.is_empty());
    assert_eq!(
        files::entries(&store.path.join("assets"), 10, &no())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(segments(&store.path, &no()).unwrap().len(), 1);
    assert_eq!(
        journal::read(&store.path.join(store.segment.as_ref().unwrap()), &no())
            .unwrap()
            .records
            .len(),
        4
    );
    assert_eq!(
        fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(store);
    let candidate = latest(&root.0);
    assert_eq!(candidate.sequence, 4);
    let restored: Recovered<State> = recover(&candidate, &no()).unwrap();
    assert_eq!(restored.bundle.state, bundle(4, &audio).state);
    assert_eq!(restored.metadata, meta(4));
    assert_sample(&restored.bundle.media[0], &audio);
    assert!(restored
        .report
        .iter()
        .any(|text| text.contains("Original external media is unavailable")));
}
#[test]
fn checkpoints_rotate_separately_and_bad_latest_generation_retains_previous() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    for revision in 1..=7 {
        store.last_checkpoint = Instant::now() - Duration::from_secs(31);
        store
            .append(&bundle(revision, &audio), meta(revision), &config, &no())
            .unwrap();
    }
    let paths = segments(&store.path, &no()).unwrap();
    assert_eq!(paths.len(), 3);
    let mut bytes = fs::read(paths.last().unwrap()).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(paths.last().unwrap(), bytes).unwrap();
    drop(store);
    let inventory = discover(&root.0, &no()).unwrap();
    assert_eq!(inventory.candidates[0].sequence, 6);
    assert!(inventory.warnings.iter().any(|s| s.contains("SHA256")));
    let restored: Recovered<State> = recover(&inventory.candidates[0], &no()).unwrap();
    assert_eq!(restored.bundle.state, bundle(6, &audio).state);
}
#[test]
fn actual_write_enospc_cancellation_and_failed_checkpoint_preserve_prior_record() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    store
        .append(&bundle(1, &audio), meta(1), &config, &no())
        .unwrap();
    let path = store.path.join(store.segment.as_ref().unwrap());
    let original = fs::read(&path).unwrap();
    {
        let _fault = super::super::testing::enospc(&root.0);
        let error = store
            .append(&bundle(2, &audio), meta(2), &config, &no())
            .unwrap_err();
        assert!(error.to_string().contains("space"));
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    let cancelled = no();
    assert!(matches!(
        store.append_with(&bundle(2, &audio), meta(2), &config, &cancelled, |phase| {
            if phase == Phase::RecordHalfWritten {
                cancelled.store(true, Ordering::Release);
            }
            Ok(())
        }),
        Err(Error::Cancelled)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    store.last_checkpoint = Instant::now() - Duration::from_secs(31);
    assert!(store
        .append_with(&bundle(2, &audio), meta(2), &config, &no(), |phase| {
            if phase == Phase::BeforePublish {
                Err(std::io::Error::from_raw_os_error(libc::ENOSPC))
            } else {
                Ok(())
            }
        })
        .is_err());
    assert_eq!(segments(&store.path, &no()).unwrap().len(), 1);
    assert_eq!(fs::read(&path).unwrap(), original);
    let commit = store
        .append(&bundle(2, &audio), meta(2), &config, &no())
        .unwrap();
    assert_eq!(commit.sequence, 2);
    drop(store);
    assert_eq!(latest(&root.0).sequence, 2);
}
#[test]
fn committed_directory_sync_warning_is_not_precommit_failure_or_retention_permission() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let mut config = Config::default();
    config.retention = 2;
    for revision in 1..=2 {
        store.last_checkpoint = Instant::now() - Duration::from_secs(31);
        store
            .append(&bundle(revision, &audio), meta(revision), &config, &no())
            .unwrap();
    }
    store.last_checkpoint = Instant::now() - Duration::from_secs(31);
    let outcome = store
        .append_with(&bundle(3, &audio), meta(3), &config, &no(), |phase| {
            if phase == Phase::DirectorySync {
                Err(std::io::Error::from_raw_os_error(libc::EIO))
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert!(!outcome.durable);
    assert!(outcome.warning.unwrap().contains("committed"));
    assert_eq!(outcome.sequence, 3);
    assert_eq!(
        segments(&store.path, &no()).unwrap().len(),
        3,
        "unconfirmed checkpoint cannot prune prior generations"
    );
    drop(store);
    assert_eq!(latest(&root.0).sequence, 3);
}
#[test]
fn storage_cap_includes_other_sessions_and_staging_without_deleting_good_history() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let mut config = Config::default();
    config.max_bytes = MIN_STORAGE_BYTES;
    store
        .append(&bundle(1, &audio), meta(1), &config, &no())
        .unwrap();
    let filler = store.path.join("pending-interrupted.tmp");
    let file = files::open(&filler, true, true).unwrap();
    file.set_len(MIN_STORAGE_BYTES).unwrap();
    drop(file);
    assert!(store
        .append(&bundle(2, &audio), meta(2), &config, &no())
        .unwrap_err()
        .to_string()
        .contains("storage cap"));
    drop(store);
    let inventory = discover(&root.0, &no()).unwrap();
    assert!(inventory.usage_bytes >= MIN_STORAGE_BYTES);
    assert_eq!(inventory.candidates[0].sequence, 1);
}
#[test]
fn missing_or_corrupt_recovery_media_falls_back_with_report_and_preview_is_revalidated() {
    for missing in [true, false] {
        let root = Root::new();
        let audio = sample();
        let mut second = (*audio).clone();
        second.data[3] = 0.99;
        let second = Arc::new(second);
        let mut store = Store::open(&root.0).unwrap();
        let config = Config::default();
        store
            .append(&bundle(1, &audio), meta(1), &config, &no())
            .unwrap();
        store
            .append(&bundle(2, &second), meta(2), &config, &no())
            .unwrap();
        let second_id = store.assets[&(Arc::as_ptr(&second) as usize)].id.clone();
        let path = store.path.join("assets").join(format!("{second_id}.omat"));
        drop(store);
        let candidate = latest(&root.0);
        assert_eq!(candidate.sequence, 2);
        if missing {
            fs::remove_file(&path).unwrap();
        } else {
            let mut bytes = fs::read(&path).unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            fs::write(&path, bytes).unwrap();
        }
        assert!(recover::<State>(&candidate, &no()).is_err());
        let inventory = discover(&root.0, &no()).unwrap();
        assert_eq!(inventory.candidates[0].sequence, 1);
        assert!(inventory.warnings.iter().any(|s| s.contains("unavailable")));
        assert_sample(
            &recover::<State>(&inventory.candidates[0], &no())
                .unwrap()
                .bundle
                .media[0],
            &audio,
        );
    }
}
#[test]
fn retired_epoch_is_idempotent_rejects_delayed_append_and_explicit_discard_preserves_native_file() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    store
        .append(&bundle(1, &audio), meta(1), &config, &no())
        .unwrap();
    store.retire_epoch(7, &no()).unwrap();
    store.retire_epoch(7, &no()).unwrap();
    assert!(store.retire_epoch(8, &no()).is_err());
    assert!(store
        .append(&bundle(2, &audio), meta(2), &config, &no())
        .is_err());
    drop(store);
    assert!(discover(&root.0, &no()).unwrap().candidates.is_empty());
    let explicit = root.0.parent().unwrap().join(format!(
        "explicit-{}.omat",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&explicit, b"last explicitly saved version").unwrap();
    let mut store = Store::open(&root.0).unwrap();
    let mut metadata = meta(3);
    metadata.saved_path = Some(explicit.clone());
    store
        .append(&bundle(3, &audio), metadata, &config, &no())
        .unwrap();
    drop(store);
    let candidate = latest(&root.0);
    assert!(matches!(
        discard(&candidate, &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    assert!(discard(&candidate, &no()).unwrap().is_none());
    assert_eq!(
        fs::read(&explicit).unwrap(),
        b"last explicitly saved version"
    );
    assert!(discover(&root.0, &no()).unwrap().candidates.is_empty());
    fs::remove_file(explicit).unwrap();
}
#[test]
fn hostile_links_permissions_unknown_versions_and_reordered_frames_fail_closed() {
    use std::os::unix::fs::symlink;
    let root = Root::new();
    let outside = Root::new();
    let link = root.0.join("link");
    symlink(&outside.0, &link).unwrap();
    assert!(Store::open(&link).is_err());
    fs::remove_file(link).unwrap();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&root.0).is_err());
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    store
        .append(&bundle(1, &audio), meta(1), &config, &no())
        .unwrap();
    let path = store.path.join(store.segment.as_ref().unwrap());
    let first = fs::read(&path).unwrap();
    store
        .append(&bundle(2, &audio), meta(2), &config, &no())
        .unwrap();
    let full = fs::read(&path).unwrap();
    let mut malformed = first.clone();
    malformed.extend_from_slice(&first);
    malformed.extend_from_slice(&full[first.len()..]);
    fs::write(&path, malformed).unwrap();
    drop(store);
    let inventory = discover(&root.0, &no()).unwrap();
    assert_eq!(inventory.candidates[0].sequence, 1);
    assert!(inventory.warnings.iter().any(|s| s.contains("ordering")));
}

#[test]
fn active_journal_mutation_requires_a_fresh_session_and_bad_retained_generation_cannot_prune() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let mut config = Config::default();
    config.retention = 2;
    for revision in 1..=2 {
        store.last_checkpoint = Instant::now() - Duration::from_secs(31);
        store
            .append(&bundle(revision, &audio), meta(revision), &config, &no())
            .unwrap();
    }
    let older = segments(&store.path, &no()).unwrap()[0].clone();
    let changed = store.path.join(store.segment.as_ref().unwrap());
    let mut bytes = fs::read(&changed).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&changed, bytes).unwrap();
    assert!(store
        .append(&bundle(3, &audio), meta(3), &config, &no())
        .unwrap_err()
        .to_string()
        .contains("changed outside"));
    assert!(store.needs_new_session());
    drop(store);
    assert_eq!(latest(&root.0).sequence, 1);
    assert!(older.exists());

    // A previous generation can be damaged while the current one remains sound.
    // Verify every retained generation before deleting even one fallback.
    let root = Root::new();
    let mut store = Store::open(&root.0).unwrap();
    config.retention = 3;
    for revision in 1..=3 {
        store.last_checkpoint = Instant::now() - Duration::from_secs(31);
        store
            .append(&bundle(revision, &audio), meta(revision), &config, &no())
            .unwrap();
    }
    let paths = segments(&store.path, &no()).unwrap();
    let mut bytes = fs::read(&paths[1]).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&paths[1], bytes).unwrap();
    store.last_checkpoint = Instant::now() - Duration::from_secs(31);
    let result = store
        .append(&bundle(4, &audio), meta(4), &config, &no())
        .unwrap();
    assert!(result.durable);
    assert!(result.warning.unwrap().contains("cleanup deferred"));
    assert_eq!(segments(&store.path, &no()).unwrap().len(), 4);
    assert!(paths[0].exists());
    assert!(!store.needs_new_session());
}

#[test]
fn committed_cancellation_asset_failure_and_root_transaction_contention_keep_truthful_state() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    let cancel = no();
    let result = store
        .append_with(&bundle(1, &audio), meta(1), &config, &cancel, |phase| {
            if phase == Phase::AfterPublish {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        })
        .unwrap();
    assert!(result.durable);
    assert_eq!(result.sequence, 1);
    let held = files::root_lock(&root.0).unwrap();
    assert!(Store::open(&root.0).is_err());
    assert!(store
        .append(&bundle(2, &audio), meta(2), &config, &no())
        .unwrap_err()
        .to_string()
        .contains("transaction"));
    drop(held);
    let mut other = (*audio).clone();
    other.data[0] = 0.9;
    assert!(store
        .append_with(
            &bundle(2, &Arc::new(other)),
            meta(2),
            &config,
            &no(),
            |phase| {
                if phase == Phase::AssetWritten {
                    Err(std::io::Error::from_raw_os_error(libc::ENOSPC))
                } else {
                    Ok(())
                }
            }
        )
        .is_err());
    assert_eq!(
        files::entries(&store.path.join("assets"), 10, &no())
            .unwrap()
            .len(),
        1
    );
    assert!(!store.needs_new_session());
    drop(store);
    assert_eq!(latest(&root.0).sequence, 1);
}

#[test]
fn cleanup_restarts_after_retirement_namespace_and_marker_boundaries_but_preserves_unknown_markers()
{
    for leave_marker in [true, false] {
        let root = Root::new();
        let mut store = Store::open(&root.0).unwrap();
        store
            .append(&bundle(1, &sample()), meta(1), &Config::default(), &no())
            .unwrap();
        retire_marker(&store.path, 7, 1, &no()).unwrap();
        let renamed = root.0.join(format!("retired-{}", store.session));
        fs::rename(&store.path, &renamed).unwrap();
        if !leave_marker {
            fs::remove_dir_all(renamed.join("assets")).unwrap();
            for path in segments(&renamed, &no()).unwrap() {
                fs::remove_file(path).unwrap();
            }
            fs::remove_file(renamed.join("retired.json")).unwrap();
        }
        drop(store);
        let found = discover(&root.0, &no()).unwrap();
        assert!(found.candidates.is_empty());
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert!(!renamed.exists());
        assert_eq!(found.usage_bytes, 0);
    }
    let root = Root::new();
    let mut store = Store::open(&root.0).unwrap();
    store
        .append(&bundle(1, &sample()), meta(1), &Config::default(), &no())
        .unwrap();
    let marker = store.path.join("retired.json");
    let mut file = files::open(&marker, true, true).unwrap();
    let bytes = br#"{"schema":99,"epoch":7,"sequence":1}"#;
    files::write(&mut file, bytes, &no()).unwrap();
    drop(file);
    drop(store);
    let found = discover(&root.0, &no()).unwrap();
    assert!(found
        .warnings
        .iter()
        .any(|s| s.contains("unknown retirement")));
    assert_eq!(fs::read(&marker).unwrap(), bytes);
}

#[test]
fn replay_record_byte_and_schema_bounds_reject_bad_tails_and_automatic_rotation_is_finite() {
    let root = Root::new();
    let mut store = Store::open(&root.0).unwrap();
    let config = Config::default();
    let audio = sample();
    for i in 1..=257 {
        store
            .append(&bundle(i, &audio), meta(i), &config, &no())
            .unwrap();
    }
    let paths = segments(&store.path, &no()).unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(journal::read(&paths[0], &no()).unwrap().records.len(), 256);
    assert_eq!(journal::read(&paths[1], &no()).unwrap().records.len(), 1);
    let saved = fs::read(&paths[1]).unwrap();
    let read = journal::read(&paths[1], &no()).unwrap();
    let previous = read.records[0].digest;
    let ids = read.records[0].body.media.clone();
    drop(store);
    for schema in [1, 99] {
        let bad = journal::Body {
            schema,
            checkpoint: false,
            metadata: meta(258),
            media: ids.clone(),
            state: serde_json::to_value(&bundle(258, &audio).state).unwrap(),
        };
        let (mut frame, _) = journal::frame(258, previous, &bad).unwrap();
        if schema == 1 {
            frame[16..24].copy_from_slice(&((RECORD_LIMIT as u64) + 1).to_le_bytes());
        }
        let mut bytes = saved.clone();
        bytes.extend_from_slice(&frame);
        fs::write(&paths[1], bytes).unwrap();
        let found = latest(&root.0);
        assert_eq!(found.sequence, 257);
        assert!(!found.report.is_empty());
    }
    fs::write(&paths[1], &saved).unwrap();
    let oversize = files::open(&paths[1], true, false).unwrap();
    oversize.set_len(SEGMENT_LIMIT + 1).unwrap();
    assert!(journal::read(&paths[1], &no())
        .unwrap_err()
        .to_string()
        .contains("64 MiB"));
    assert_eq!(latest(&root.0).sequence, 256);
    drop(oversize);
    let mut new = Store::open(&root.0).unwrap();
    let invalid = Bundle {
        state: "x".repeat(RECORD_LIMIT + 1),
        media: Vec::new(),
    };
    assert!(new
        .append(&invalid, meta(1), &config, &no())
        .unwrap_err()
        .to_string()
        .contains("8 MiB"));
    assert!(!new.needs_new_session());
    assert!(segments(&new.path, &no()).unwrap().is_empty());
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Keep the fork/exec window open deterministically. The child only performs
// async-signal-safe calls and never runs Rust destructors, allocates, opens a
// device or writes storage. The parent always kills and reaps this private child.
struct InheritedDescriptors(libc::pid_t);
impl InheritedDescriptors {
    fn hold() -> Self {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            loop {
                unsafe { libc::pause() };
            }
        }
        Self(pid)
    }
}
impl Drop for InheritedDescriptors {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0, libc::SIGKILL) };
        while unsafe { libc::waitpid(self.0, std::ptr::null_mut(), 0) } < 0 {
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                break;
            }
        }
    }
}

#[test]
fn inherited_descriptors_cannot_extend_finished_root_transaction() {
    let root = Root::new();
    let held = files::root_lock(&root.0).unwrap();
    let child = InheritedDescriptors::hold();
    assert!(files::root_lock(&root.0).is_err());
    assert!(Store::open(&root.0).is_err());
    drop(held);
    let next = files::root_lock(&root.0)
        .expect("finished root transaction must unlock while the child still holds its descriptor");
    assert!(files::root_lock(&root.0).is_err());
    drop(child);
    // Closing the old inherited description must not unlock the new writer.
    assert!(files::root_lock(&root.0).is_err());
    drop(next);
    assert!(files::root_lock(&root.0).is_ok());
}

#[test]
fn inherited_descriptors_cannot_delay_next_append_or_completed_session_recovery() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let mut child = None;
    let first = store
        .append_with(&bundle(1, &audio), meta(1), &Config::default(), &no(), |phase| {
            if phase == Phase::RecordHalfWritten {
                child = Some(InheritedDescriptors::hold());
            }
            Ok(())
        })
        .unwrap();
    assert!(child.is_some());
    assert!(first.durable);
    let next = store
        .append(&bundle(2, &audio), meta(2), &Config::default(), &no())
        .expect("a completed append cannot leave the root transaction locked in a forked child");
    assert!(next.durable);
    assert_eq!(next.sequence, 2);
    assert!(discover(&root.0, &no()).unwrap().candidates.is_empty());
    drop(store);
    // The child's inherited owner descriptor cannot keep a finished session
    // falsely classified as live after the actual owner has finished.
    let candidate = latest(&root.0);
    assert_eq!(candidate.sequence, 2);
    let recovered: Recovered<State> = recover(&candidate, &no()).unwrap();
    assert_eq!(recovered.bundle.state, bundle(2, &audio).state);
    assert_sample(&recovered.bundle.media[0], &audio);
    drop(child);
}

#[test]
fn killed_writer_retains_last_durable_record_at_append_asset_and_checkpoint_boundaries() {
    for boundary in ["append", "asset", "checkpoint"] {
        let root = Root::new();
        let ready = root.0.join("ready");
        let log = files::open(&root.0.join("child.log"), true, true).unwrap();
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "recovery::storage::tests::crash_writer_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("OMATAINER_RECOVERY97_CHILD", "1")
                .env("OMATAINER_RECOVERY97_ROOT", &root.0)
                .env("OMATAINER_RECOVERY97_BOUNDARY", boundary)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.exists() {
            if let Some(status) = child.0.try_wait().unwrap() {
                panic!(
                    "crash child exited early {status}: {}",
                    fs::read_to_string(root.0.join("child.log")).unwrap()
                );
            }
            assert!(
                Instant::now() < deadline,
                "crash child failed to reach {boundary}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        // The process, not a test scope destructor, releases both actual locks.
        let _root_lock = files::root_lock(&root.0).unwrap();
        drop(_root_lock);
        fs::remove_file(&ready).unwrap();
        fs::remove_file(root.0.join("child.log")).unwrap();
        let candidate = latest(&root.0);
        assert_eq!(candidate.sequence, 1, "{boundary}");
        let restored: Recovered<State> = recover(&candidate, &no()).unwrap();
        assert_eq!(restored.bundle.state, bundle(1, &sample()).state);
        assert_sample(&restored.bundle.media[0], &sample());
    }
}

#[test]
#[ignore = "child entry point; parent supplies a private root and kills at a controlled write boundary"]
fn crash_writer_child() {
    assert_eq!(
        std::env::var("OMATAINER_RECOVERY97_CHILD").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("OMATAINER_RECOVERY97_ROOT").unwrap());
    files::private_dir(&root, false).unwrap();
    let boundary = std::env::var("OMATAINER_RECOVERY97_BOUNDARY").unwrap();
    let audio = sample();
    let mut store = Store::open(&root).unwrap();
    store
        .append(&bundle(1, &audio), meta(1), &Config::default(), &no())
        .unwrap();
    let mut next_audio = audio.clone();
    let target = match boundary.as_str() {
        "append" => Phase::RecordHalfWritten,
        "asset" => {
            let mut changed = (*audio).clone();
            changed.data[0] = 0.9;
            next_audio = Arc::new(changed);
            Phase::AssetWritten
        }
        "checkpoint" => {
            store.last_checkpoint = Instant::now() - Duration::from_secs(31);
            Phase::BeforePublish
        }
        _ => panic!("unknown child boundary"),
    };
    store
        .append_with(
            &bundle(2, &next_audio),
            meta(2),
            &Config::default(),
            &no(),
            |phase| {
                if phase == target {
                    let mut ready = files::open(&root.join("ready"), true, true).unwrap();
                    files::write(&mut ready, b"at boundary", &no()).unwrap();
                    ready.sync_all().unwrap();
                    // The parent must terminate us. This bounded fallback prevents an
                    // accidentally orphaned ignored fixture from running indefinitely.
                    std::thread::sleep(Duration::from_secs(20));
                    panic!("parent did not terminate crash fixture");
                }
                Ok(())
            },
        )
        .unwrap();
    panic!("target crash boundary was not reached");
}

#[test]
fn aggregate_pcm_and_metadata_limits_reject_before_decoding_the_next_asset() {
    let root = Root::new();
    let audio = sample();
    let mut store = Store::open(&root.0).unwrap();
    let mut two = bundle(1, &audio);
    two.media.push(audio.clone());
    store
        .append(&two, meta(1), &Config::default(), &no())
        .unwrap();
    let asset = store.assets.values().next().unwrap();
    let metadata_size = (asset.identity.2
        - project_file::CONTAINER_OVERHEAD
        - audio.data.len() as u64 * 4) as usize;
    let state_size = journal::json(&two.state).unwrap().len();
    drop(store);
    let candidate = latest(&root.0);
    let mut limits = Limits::default();
    limits.max_pcm_bytes = audio.data.len() as u64 * 4 + 1;
    assert!(recover_with_limits::<State>(&candidate, &no(), limits)
        .unwrap_err()
        .to_string()
        .contains("declared metadata or PCM"));
    limits = Limits::default();
    limits.max_metadata_bytes = state_size + metadata_size * 2 - 1;
    assert!(recover_with_limits::<State>(&candidate, &no(), limits)
        .unwrap_err()
        .to_string()
        .contains("declared metadata or PCM"));
    limits.max_metadata_bytes += 1;
    limits.max_pcm_bytes = audio.data.len() as u64 * 8;
    let restored: Recovered<State> = recover_with_limits(&candidate, &no(), limits).unwrap();
    assert_eq!(restored.bundle.media.len(), 2);
    assert_sample(&restored.bundle.media[1], &audio);
}

#[test]
fn changed_owned_sidecar_requests_new_session_while_generic_validation_failure_does_not() {
    for missing in [true, false] {
        let root = Root::new();
        let audio = sample();
        let mut store = Store::open(&root.0).unwrap();
        store
            .append(&bundle(1, &audio), meta(1), &Config::default(), &no())
            .unwrap();
        let asset = store.assets.values().next().unwrap();
        let path = store.path.join("assets").join(format!("{}.omat", asset.id));
        if missing {
            fs::remove_file(&path).unwrap();
        } else {
            let mut bytes = fs::read(&path).unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            fs::write(&path, bytes).unwrap();
        }
        assert!(store
            .append(&bundle(2, &audio), meta(2), &Config::default(), &no())
            .is_err());
        assert!(store.needs_new_session());
        let mut fresh = Store::open(&root.0).unwrap();
        assert!(
            fresh
                .append(&bundle(2, &audio), meta(2), &Config::default(), &no())
                .unwrap()
                .durable
        );
        let invalid = Config {
            retention: 0,
            ..Config::default()
        };
        assert!(fresh
            .append(&bundle(3, &audio), meta(3), &invalid, &no())
            .is_err());
        assert!(!fresh.needs_new_session());
    }
}

#[test]
fn unicode_diagnostics_and_unrecognized_names_stay_bounded_without_truncation_panics() {
    let mut rows = Vec::new();
    for _ in 0..100 {
        report(&mut rows, "🎛".repeat(4096));
    }
    assert_eq!(rows.len(), REPORT_LIMIT);
    assert!(rows
        .iter()
        .all(|s| s.len() <= 2048 && s.chars().all(|c| c == '🎛')));
    let root = Root::new();
    let mut store = Store::open(&root.0).unwrap();
    store
        .append(&bundle(1, &sample()), meta(1), &Config::default(), &no())
        .unwrap();
    // Same byte length as a segment but a multibyte character at the numeric
    // prefix boundary; no string slicing may assume ASCII before validation.
    let unusual = store.path.join(format!("{}é.journal", "0".repeat(18)));
    files::open(&unusual, true, true).unwrap();
    drop(store);
    assert_eq!(latest(&root.0).sequence, 1);
}

#[test]
fn discovery_hash_budget_is_shared_across_sessions_and_bad_assets_are_charged_once_per_attempt() {
    let root = Root::new();
    let audio = sample();
    let mut size = 0;
    for _ in 0..2 {
        let mut store = Store::open(&root.0).unwrap();
        store
            .append(&bundle(1, &audio), meta(1), &Config::default(), &no())
            .unwrap();
        size = store.assets.values().next().unwrap().identity.2;
    }
    let limited = discover_with_budget(&root.0, &no(), size + 1).unwrap();
    assert_eq!(limited.candidates.len(), 1);
    assert!(limited
        .warnings
        .iter()
        .any(|s| s.contains("verification byte budget")));
    let complete = discover_with_budget(&root.0, &no(), 2 * (size + 1)).unwrap();
    assert_eq!(complete.candidates.len(), 2);
    assert!(complete.warnings.is_empty());
    let candidate = &complete.candidates[0];
    let path = root.0.join(&candidate.session);
    let record = exact_record(candidate, &path, &no()).unwrap();
    let asset = path
        .join("assets")
        .join(format!("{}.omat", record.body.media[0]));
    let mut bytes = fs::read(&asset).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&asset, bytes).unwrap();
    let mut budget = size + 1;
    let mut verified = HashSet::new();
    assert!(
        verify_assets(&path, &record.body.media, &no(), &mut verified, &mut budget)
            .unwrap_err()
            .to_string()
            .contains("SHA256")
    );
    assert_eq!(budget, 0);
    assert!(verified.is_empty());
    assert!(
        verify_assets(&path, &record.body.media, &no(), &mut verified, &mut budget)
            .unwrap_err()
            .to_string()
            .contains("verification byte budget")
    );
}
