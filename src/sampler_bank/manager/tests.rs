use super::*;
use crate::sampler_bank::{assets, resident};
use std::{os::unix::fs::PermissionsExt, time::{Duration, Instant}};
struct Files(PathBuf);
impl Files {
    fn new() -> Self { let path = std::env::temp_dir().join(format!("omat-bank-manager-{}", BankId::new().unwrap())); std::fs::create_dir(&path).unwrap(); std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap(); Self(path) }
    fn path(&self) -> PathBuf { self.0.join("banks.json") }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn wait(manager: &mut Manager) { let end = Instant::now() + Duration::from_secs(3); while manager.busy { manager.poll(); assert!(Instant::now() < end); std::thread::sleep(Duration::from_millis(1)); } }
fn bank() -> (assets::Owner, sampler::Bank) {
    let owner = assets::Owner::isolated_for_test(assets::Budget::limits());
    let data = resident::Data::prepare(Arc::new(resident::Settings::empty("Reusable empty".into()).unwrap()), std::array::from_fn(|_| None), std::array::from_fn(|_| None)).unwrap();
    let bank = sampler::Bank::imported(owner.pin(data).unwrap()).unwrap(); (owner, bank)
}
#[test]
fn actual_worker_restarts_duplicate_names_and_explicit_id_updates_without_overwriting_by_name() {
    let files = Files::new(); let (_owner, bank) = bank();
    let mut manager = Manager::start(files.path(), performance::Handle::default()).unwrap(); wait(&mut manager);
    assert!(manager.error.is_none()); assert_eq!(manager.collection.as_ref().unwrap().banks.len(), 0);
    manager.save(bank.clone(), "same".into(), None).unwrap(); wait(&mut manager);
    let first = manager.saved.as_ref().unwrap().definition; assert!(manager.saved.as_ref().unwrap().commit.durable);
    manager.save(bank.clone(), "same".into(), None).unwrap(); wait(&mut manager);
    let second = manager.saved.as_ref().unwrap().definition; assert_ne!(first, second);
    manager.save(bank.clone(), "renamed".into(), Some(first)).unwrap(); wait(&mut manager);
    let expected = manager.collection.clone().unwrap(); assert_eq!(expected.banks.len(), 2);
    assert_eq!(expected.banks[1].name, "same");
    drop(manager);
    // The detached worker drops its private Store after reply-channel teardown.
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match Store::open(files.path()) {
            Ok(store) => { assert_eq!(store.collection, *expected); break; }
            Err(error) if error.contains("already open") && Instant::now() < end => std::thread::sleep(Duration::from_millis(1)),
            Err(error) => panic!("{error}"),
        }
    }
    let mut manager = Manager::start(files.path(), performance::Handle::default()).unwrap(); wait(&mut manager);
    assert_eq!(manager.collection.as_ref().unwrap().as_ref(), expected.as_ref());
    manager.save(bank, "wrong identity".into(), Some(BankId::new().unwrap())).unwrap(); wait(&mut manager);
    assert!(manager.error.is_some()); assert_eq!(manager.collection.as_ref().unwrap().as_ref(), expected.as_ref());
}
#[test]
fn cancelled_and_protected_pending_saves_preserve_last_good_definitions_and_malformed_store() {
    for protect in [false, true] {
        let files = Files::new(); let (_owner, bank) = bank(); let policy = performance::Handle::default();
        let (entered, seen) = mpsc::sync_channel(1); let (release, proceed) = mpsc::sync_channel(1);
        let mut manager = Manager::with_hook(files.path(), policy.clone(), move |save| { if save { entered.send(()).unwrap(); proceed.recv().unwrap(); } }).unwrap();
        wait(&mut manager); manager.save(bank, "cancelled".into(), None).unwrap();
        seen.recv_timeout(Duration::from_secs(3)).unwrap();
        if protect { policy.set_enabled(true).unwrap(); policy.set_enabled(false).unwrap(); } else { manager.cancel(); }
        release.send(()).unwrap(); wait(&mut manager);
        assert!(manager.error.is_some()); assert!(manager.collection.as_ref().unwrap().banks.is_empty()); assert!(!files.path().exists());
    }
    let files = Files::new(); let bytes = br#"{"schema":999,"future":true}"#; std::fs::write(files.path(), bytes).unwrap();
    let mut manager = Manager::start(files.path(), performance::Handle::default()).unwrap(); wait(&mut manager);
    assert!(manager.error.as_ref().unwrap().contains("preserved")); assert!(manager.collection.is_none());
    manager.retry().unwrap(); wait(&mut manager); assert!(manager.error.is_some()); assert_eq!(std::fs::read(files.path()).unwrap(), bytes);
}

#[test]
fn cancel_after_store_rename_reports_the_committed_definition_instead_of_losing_it() {
    let files = Files::new(); let (_owner, bank) = bank();
    let mut manager = Manager::start(files.path(), performance::Handle::default()).unwrap(); wait(&mut manager);
    manager.save(bank, "Committed before Cancel".into(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !files.path().exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
    manager.cancel();
    wait(&mut manager);
    let saved = manager.saved.as_ref().expect("late cancellation cannot erase a committed save");
    assert_eq!(saved.commit.revision, 1);
    assert!(manager.error.is_none());
    assert_eq!(manager.collection.as_ref().unwrap().banks[0].name, "Committed before Cancel");
}

#[test]
fn finished_optional_store_reads_honor_user_cancel_and_complete_protection_cycles() {
    for protect in [false, true] {
        let files = Files::new(); let policy = performance::Handle::default();
        let mut manager = Manager::start(files.path(), policy.clone()).unwrap();
        let cancel = manager.cancel.clone();
        let deadline = Instant::now() + Duration::from_secs(3);
        while manager.busy {
            manager.poll_before_publish(|| {
                if protect { policy.set_enabled(true).unwrap(); policy.set_enabled(false).unwrap(); }
                else { cancel.store(true, Ordering::Release); }
            });
            assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1));
        }
        assert!(manager.collection.is_none());
        assert!(manager.error.as_ref().unwrap().contains("not published"));
        manager.retry().unwrap(); wait(&mut manager);
        assert!(manager.collection.is_some()); assert!(manager.error.is_none());
    }
}
