use super::*;
use crate::project_versions::tests::{count, fixture, Folder};
use std::os::unix::fs::PermissionsExt;

fn snapshot(store: &mut Store, name: &str, value: f32, cancel: &AtomicBool) -> Entry {
    store
        .snapshot(&fixture(value), name.into(), String::new(), cancel, || {
            Ok(())
        })
        .unwrap()
        .0
}
fn originals(root: &Path) -> BTreeMap<String, Vec<u8>> {
    ["audio", "revisions"]
        .into_iter()
        .flat_map(|folder| {
            fs::read_dir(root.join(folder)).unwrap().map(move |file| {
                let file = file.unwrap();
                (
                    format!("{folder}/{}", file.file_name().to_str().unwrap()),
                    fs::read(file.path()).unwrap(),
                )
            })
        })
        .collect()
}

#[test]
fn shared_dependencies_quarantine_reopen_and_lifo_restore_keep_every_original_byte() {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let a = snapshot(&mut store, "A", 0.25, &cancel);
    let b = snapshot(&mut store, "B", 0.25, &cancel);
    let c = snapshot(&mut store, "C", 0.5, &cancel);
    let before = originals(&root);
    let sentinel = dir.0.join("original-recording.wav");
    fs::write(&sentinel, b"never selected").unwrap();
    let first = store
        .prune(
            store.review_prune(&[a.id.clone()], &cancel).unwrap(),
            &cancel,
            || Ok(()),
        )
        .unwrap();
    assert_eq!(first.reclaimed_audio, 0);
    let first_id = first.recovery_id.unwrap();
    let second = store
        .prune(
            store.review_prune(&[b.id.clone()], &cancel).unwrap(),
            &cancel,
            || Ok(()),
        )
        .unwrap();
    assert_eq!(second.reclaimed_audio, 1);
    assert!(second.warning.is_none());
    let second_id = second.recovery_id.unwrap();
    assert_eq!(count(&root, "audio"), 1);
    assert_eq!(count(&root, "revisions"), 1);
    assert_eq!(
        inspect(&store, &cancel)
            .unwrap()
            .iter()
            .map(|r| r.files)
            .sum::<usize>(),
        3
    );
    assert!(restore(&mut store, &first_id, &cancel, || Ok(())).is_err());
    assert_eq!(store.entries(), [c.clone()]);
    drop(store);
    let mut store = Store::open(&root, false, &cancel).unwrap();
    assert!(matches!(
        restore(&mut store, &second_id, &cancel, || Ok(())).unwrap(),
        SaveOutcome::Durable
    ));
    assert_eq!(store.entries(), [b.clone(), c.clone()]);
    assert!(matches!(
        restore(&mut store, &first_id, &cancel, || Ok(())).unwrap(),
        SaveOutcome::Durable
    ));
    assert_eq!(store.entries(), [a.clone(), b.clone(), c.clone()]);
    assert_eq!(originals(&root), before);
    assert!(inspect(&store, &cancel)
        .unwrap()
        .iter()
        .all(|r| r.restored && r.files == 0 && r.bytes == 0 && !r.pending));
    for (entry, value) in [(&a, 0.25), (&b, 0.25), (&c, 0.5)] {
        assert_eq!(
            store
                .reopen::<serde_json::Value>(entry, &cancel)
                .unwrap()
                .media[0]
                .data,
            fixture(value).media[0].data
        );
    }
    let next = store
        .prune(
            store.review_prune(&[c.id.clone()], &cancel).unwrap(),
            &cancel,
            || Ok(()),
        )
        .unwrap();
    assert!(next.warning.is_none());
    assert_eq!(inspect(&store, &cancel).unwrap().len(), 3);
    assert_eq!(fs::read(sentinel).unwrap(), b"never selected");
    println!(
        "PROJECT_CLEANUP_RECOVERY {}",
        serde_json::json!({"actual_filesystem":true,"all_saved_version_dependencies":true,"shared_pcm_retained":true,"lifo_restore":true,"native_reopen":true,"every_original_byte_restored":true,"new_batch_after_restore":true,"unselected_original_untouched":true,"physical_devices_opened":false})
    );
}

#[test]
fn admission_refusal_cancellation_and_real_failed_index_write_keep_sources_and_recoverable_records()
{
    for mode in 0..3 {
        let dir = Folder::new();
        let root = dir.0.join("versions");
        let cancel = AtomicBool::new(false);
        let mut store = Store::open(&root, true, &cancel).unwrap();
        let a = snapshot(&mut store, "A", 0.25, &cancel);
        let index = fs::read(root.join(INDEX)).unwrap();
        let before = originals(&root);
        let review = store.review_prune(&[a.id.clone()], &cancel).unwrap();
        let result = store.prune(review, &cancel, || match mode {
            0 => Err("admission refused".into()),
            1 => {
                cancel.store(true, Ordering::Release);
                Ok(())
            }
            _ => {
                fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
                Ok(())
            }
        });
        assert!(result.is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        cancel.store(false, Ordering::Release);
        assert_eq!(fs::read(root.join(INDEX)).unwrap(), index);
        assert_eq!(originals(&root), before);
        let saved = inspect(&store, &cancel).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].pending);
        assert_eq!(saved[0].files, 0);
        assert!(store
            .prune(
                store.review_prune(&[a.id.clone()], &cancel).unwrap(),
                &cancel,
                || Ok(())
            )
            .is_err());
        let id = saved[0].id.clone();
        drop(store);
        let mut reopened = Store::open(&root, false, &cancel).unwrap();
        assert!(matches!(
            restore(&mut reopened, &id, &cancel, || Ok(())).unwrap(),
            SaveOutcome::Durable
        ));
        assert_eq!(originals(&root), before);
        assert_eq!(reopened.entries(), [a]);
    }
    println!(
        "PROJECT_CLEANUP_FAILED_SAVE {}",
        serde_json::json!({"admission_refusal":true,"cancellation":true,"actual_permission_failure":true,"index_and_source_bytes_unchanged":true,"private_record_reopened":true,"explicit_restore":true,"physical_devices_opened":false})
    );
}

#[test]
fn a_real_second_file_collision_preserves_partial_quarantine_and_foreign_bytes_for_explicit_recovery(
) {
    let dir = Folder::new();
    let root = dir.0.join("versions");
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(&root, true, &cancel).unwrap();
    let a = snapshot(&mut store, "A", 0.25, &cancel);
    let b = snapshot(&mut store, "B", 0.5, &cancel);
    let before = originals(&root);
    let review = store
        .review_prune(&[a.id.clone(), b.id.clone()], &cancel)
        .unwrap();
    let second = review
        .files
        .keys()
        .filter(|p| p.starts_with("audio/"))
        .nth(1)
        .unwrap()
        .clone();
    let result = store
        .prune(review, &cancel, || {
            let recovery = fs::read_dir(root.join(FOLDER))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            fs::write(recovery.join(&second), b"foreign destination").unwrap();
            Ok(())
        })
        .unwrap();
    assert!(result.warning.is_some());
    assert!(store.entries().is_empty());
    assert_eq!(count(&root, "audio"), 1);
    let id = result.recovery_id.unwrap();
    let foreign = root.join(FOLDER).join(&id).join(&second);
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign destination");
    assert!(restore(&mut store, &id, &cancel, || Ok(())).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign destination");
    let preserved = dir.0.join("foreign-preserved.omat");
    fs::rename(&foreign, &preserved).unwrap();
    drop(store);
    let mut store = Store::open(&root, false, &cancel).unwrap();
    restore(&mut store, &id, &cancel, || Ok(())).unwrap();
    assert_eq!(store.entries(), [a, b]);
    assert_eq!(originals(&root), before);
    assert_eq!(fs::read(preserved).unwrap(), b"foreign destination");
    println!(
        "PROJECT_CLEANUP_PARTIAL {}",
        serde_json::json!({"actual_atomic_second_file_collision":true,"first_file_quarantined":true,"foreign_destination_preserved":true,"partial_work_retained":true,"reopen_restore":true,"every_original_byte_restored":true,"physical_devices_opened":false})
    );
}

#[test]
fn changed_retained_dependencies_and_occupied_restore_paths_refuse_without_moving_the_last_copy() {
    for retained in [false, true] {
        let dir = Folder::new();
        let root = dir.0.join("versions");
        let cancel = AtomicBool::new(false);
        let mut store = Store::open(&root, true, &cancel).unwrap();
        let a = snapshot(&mut store, "A", 0.25, &cancel);
        snapshot(&mut store, "B", 0.5, &cancel);
        let result = store
            .prune(
                store.review_prune(&[a.id], &cancel).unwrap(),
                &cancel,
                || Ok(()),
            )
            .unwrap();
        let id = result.recovery_id.unwrap();
        let (_, folder) = owned(&store, false).unwrap();
        let prepared = read(&folder, &id, &store, &cancel).unwrap();
        let relative = prepared
            .record
            .files
            .keys()
            .find(|p| p.starts_with("audio/"))
            .unwrap();
        let archived = root.join(FOLDER).join(&id).join(relative);
        let bytes = fs::read(&archived).unwrap();
        let changed = if retained {
            root.join(
                prepared
                    .record
                    .retained
                    .keys()
                    .find(|p| p.starts_with("audio/"))
                    .unwrap(),
            )
        } else {
            root.join(relative)
        };
        fs::write(&changed, b"changed by another writer").unwrap();
        let index = fs::read(root.join(INDEX)).unwrap();
        assert!(restore(&mut store, &id, &cancel, || Ok(())).is_err());
        assert_eq!(fs::read(&archived).unwrap(), bytes);
        assert_eq!(fs::read(changed).unwrap(), b"changed by another writer");
        assert_eq!(fs::read(root.join(INDEX)).unwrap(), index);
    }
    println!(
        "PROJECT_CLEANUP_REFUSAL {}",
        serde_json::json!({"changed_retained_dependencies":true,"occupied_original_path":true,"last_copy_retained":true,"foreign_bytes_preserved":true,"index_unchanged":true,"physical_devices_opened":false})
    );
}
