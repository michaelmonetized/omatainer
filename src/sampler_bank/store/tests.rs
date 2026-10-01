use super::*;
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "omatainer-sampler-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> PathBuf {
        self.0.join("banks.json")
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn changed(store: &Store, name: &str) -> Collection {
    let mut value = store.collection.clone();
    if let Some(bank) = value.banks.first_mut() {
        bank.name = name.into();
    } else {
        value.banks.push(Definition::empty(name.into()).unwrap());
    }
    value.revision += 1;
    value
}

#[test]
fn private_store_roundtrips_without_name_aliases_or_live_writer_collision() {
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(files.path()).unwrap();
    assert!(Store::open(files.path()).is_err());
    let next = changed(&store, "User kit");
    assert_eq!(
        store.save(next.clone(), &cancel).unwrap(),
        Commit {
            durable: true,
            warning: None,
            revision: 1
        }
    );
    assert_eq!(fs::metadata(files.path()).unwrap().mode() & 0o777, 0o600);
    drop(store);
    let reopened = Store::open(files.path()).unwrap();
    assert_eq!(reopened.collection, next);
    assert!(reopened.collection.banks[0]
        .slots
        .iter()
        .all(|slot| slot.source.is_none()));
}

#[test]
fn each_interrupted_stage_preserves_prior_data_and_postcommit_is_not_rejection() {
    for stage in 0..=4 {
        let files = Files::new();
        let cancel = AtomicBool::new(false);
        let mut store = Store::open(files.path()).unwrap();
        store.save(changed(&store, "before"), &cancel).unwrap();
        let result = store.save_with(changed(&store, "after"), &cancel, |at| {
            if at == stage {
                Err("injected ENOSPC".into())
            } else {
                Ok(())
            }
        });
        if stage != 3 {
            assert!(result.is_err());
            assert_eq!(read(&files.path()).unwrap().0.banks[0].name, "before");
            assert_eq!(store.collection.banks[0].name, "before");
        } else {
            let result = result.unwrap();
            assert!(!result.durable);
            assert!(result.warning.unwrap().contains("committed"));
            assert_eq!(read(&files.path()).unwrap().0.banks[0].name, "after");
            assert_eq!(store.collection.banks[0].name, "after");
            assert!(
                store
                    .save(store.collection.clone(), &cancel)
                    .unwrap()
                    .durable
            );
        }
        // Same-inode backup rename after a precommit failure must leave no
        // backup-PID link that would permanently break a third save.
        store.save(changed(&store, "retry"), &cancel).unwrap();
        store.save(changed(&store, "third"), &cancel).unwrap();
        assert_eq!(read(&files.path()).unwrap().0.banks[0].name, "third");
        assert!(!files
            .path()
            .with_extension(format!("backup-{}", std::process::id()))
            .exists());
        assert!(!files
            .path()
            .with_extension(format!("tmp-{}", std::process::id()))
            .exists());
    }
}

#[test]
fn cancellation_before_rename_preserves_previous_and_after_rename_reports_committed() {
    for stage in [0, 1, 2, 3] {
        let files = Files::new();
        let cancel = AtomicBool::new(false);
        let mut store = Store::open(files.path()).unwrap();
        store.save(changed(&store, "before"), &cancel).unwrap();
        let result = store.save_with(changed(&store, "after"), &cancel, |at| {
            if at == stage {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        });
        if stage == 3 {
            assert!(result.unwrap().durable);
        } else {
            assert!(result.unwrap_err().contains("cancelled"));
        }
        assert_eq!(
            read(&files.path()).unwrap().0.banks[0].name,
            if stage == 3 { "after" } else { "before" }
        );
    }
}

#[test]
fn malformed_future_oversized_symlink_and_external_replacement_are_preserved() {
    for bytes in [
        b"broken json".to_vec(),
        b"{\"schema\":2,\"revision\":0,\"banks\":[]}".to_vec(),
        vec![b' '; MAX_BYTES as usize + 1],
    ] {
        let files = Files::new();
        fs::write(files.path(), &bytes).unwrap();
        assert!(Store::open(files.path()).is_err());
        assert_eq!(fs::read(files.path()).unwrap(), bytes);
    }
    let files = Files::new();
    let destination = files.0.join("original");
    fs::write(&destination, b"preserve").unwrap();
    std::os::unix::fs::symlink(&destination, files.path()).unwrap();
    assert!(Store::open(files.path()).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"preserve");
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    let mut store = Store::open(files.path()).unwrap();
    store.save(changed(&store, "before"), &cancel).unwrap();
    fs::write(files.path(), b"external bytes").unwrap();
    assert!(store.save(changed(&store, "after"), &cancel).is_err());
    assert_eq!(fs::read(files.path()).unwrap(), b"external bytes");
    drop(store);
    fs::remove_file(files.path()).unwrap();
    fs::write(
        files.path().with_extension("backup.json"),
        b"retained backup",
    )
    .unwrap();
    assert!(Store::open(files.path()).err().unwrap().contains("backup"));
}

#[test]
fn primary_replacement_between_backup_link_and_refresh_is_never_adopted() {
    for inplace in [false, true] {
        let files = Files::new();
        let cancel = AtomicBool::new(false);
        let mut store = Store::open(files.path()).unwrap();
        store.save(changed(&store, "before"), &cancel).unwrap();
        let original = store.collection.clone();
        let result = store.save_with(changed(&store, "must not publish"), &cancel, |stage| {
            if stage == 4 {
                if inplace {
                    fs::write(files.path(), b"external in-place update").unwrap();
                } else {
                    let replacement = files.0.join("external.json");
                    fs::write(&replacement, b"external replacement bytes").unwrap();
                    fs::rename(replacement, files.path()).unwrap();
                }
            }
            Ok(())
        });
        assert!(result.unwrap_err().contains("changed during backup"));
        assert_eq!(store.collection, original);
        assert_eq!(
            fs::read(files.path()).unwrap(),
            if inplace {
                b"external in-place update".as_slice()
            } else {
                b"external replacement bytes".as_slice()
            }
        );
        assert!(store
            .save(changed(&store, "retry must still refuse"), &cancel)
            .is_err());
        assert!(!files
            .path()
            .with_extension(format!("backup-{}", std::process::id()))
            .exists());
    }
}

#[test]
fn closing_writer_unlocks_even_while_a_duplicate_description_survives() {
    let files = Files::new();
    let store = Store::open(files.path()).unwrap();
    let inherited = store._lock.try_clone().unwrap();
    assert!(Store::open(files.path()).is_err());
    drop(store);
    let next = Store::open(files.path()).unwrap();
    assert!(Store::open(files.path()).is_err(), "a new active writer still excludes competitors");
    drop(inherited);
    assert!(Store::open(files.path()).is_err(), "dropping an old description cannot unlock the new writer");
    drop(next);
    assert!(Store::open(files.path()).is_ok());
}
