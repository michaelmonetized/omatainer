use super::*;
use crate::engine::performance;
use std::sync::atomic::{AtomicU64, Ordering};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!("omatainer-history-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap(); Self(root)
    }
    fn store(&self) -> PathBuf { self.0.join("history") }
}
impl Drop for Files { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn session() -> Session { Session::new("a".repeat(32), 1, 1000, 0).unwrap() }

#[test]
fn actual_restart_retains_manual_marks_and_recovers_active_session_as_unclean() {
    let files = Files::new(); let mut store = Store::open(files.store()).unwrap();
    assert!(Store::open(files.store()).is_err(), "single writer lock");
    let mut set = session();
    let entry = set.external(0, "Guest vinyl".into(), "Guest".into()).unwrap();
    set.mark(1, entry, Some(false)).unwrap();
    assert_eq!(store.save(&set).unwrap(), Commit::Durable);
    let path = files.store().join(format!("{}.json", set.id));
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(files.store()).unwrap().permissions().mode() & 0o777, 0o700);
    drop(store);
    let reopened = Store::open(files.store()).unwrap();
    let restored = &reopened.sessions[&set.id];
    assert_eq!(restored.state, super::super::State::Unclean);
    assert_eq!(restored.ended_ns, Some(1000));
    assert!(!restored.entries[0].played()); assert!(restored.incomplete);
    assert_eq!(restored.entries[0].source.title(), "Guest vinyl");
}

#[test]
fn precommit_failure_preserves_old_bytes_postcommit_failure_reports_the_actual_new_file() {
    let files = Files::new(); let mut store = Store::open(files.store()).unwrap(); let mut set = session();
    store.save(&set).unwrap();
    let path = files.store().join(format!("{}.json", set.id)); let old = fs::read(&path).unwrap();
    set.external(0, "New external track".into(), String::new()).unwrap();
    assert!(store.save_checked(&set, None, |stage| if stage == 0 { Err("simulated precommit failure".into()) } else { Ok(()) }).is_err());
    assert_eq!(fs::read(&path).unwrap(), old); assert!(store.sessions[&set.id].entries.is_empty());
    assert!(matches!(store.save_checked(&set, None, |stage| if stage == 1 { Err("simulated directory sync failure".into()) } else { Ok(()) }).unwrap(), Commit::CommittedUnconfirmed(_)));
    assert_eq!(serde_json::from_slice::<Session>(&fs::read(&path).unwrap()).unwrap(), set);
    assert_eq!(store.sessions[&set.id], set);
    assert_eq!(store.save(&set).unwrap(), Commit::Durable, "retry confirms the same committed candidate without repeating the edit");
}

#[test]
fn changed_invalid_and_symlinked_history_are_preserved_instead_of_reset() {
    let files = Files::new(); let mut store = Store::open(files.store()).unwrap(); let mut set = session();
    store.save(&set).unwrap();
    let path = files.store().join(format!("{}.json", set.id));
    fs::write(&path, b"{invalid but must survive}").unwrap();
    set.external(0, "Must not replace".into(), String::new()).unwrap();
    assert!(store.save(&set).is_err());
    drop(store); assert!(Store::open(files.store()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{invalid but must survive}");
    fs::remove_file(&path).unwrap();
    let victim = files.0.join("victim.json"); fs::write(&victim, b"untouched").unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    assert!(Store::open(files.store()).is_err()); assert_eq!(fs::read(&victim).unwrap(), b"untouched");
}

#[test]
fn export_is_private_allowlisted_non_overwriting_and_protection_qualified() {
    let files = Files::new(); let store = Store::open(files.store()).unwrap(); let mut set = session();
    set.external(0, "Turntable".into(), "Guest".into()).unwrap();
    let show = performance::Handle::default();
    let permit = show.optional_work().unwrap();
    let destination = files.0.join("set.json");
    assert_eq!(store.export(&set, &destination, &permit).unwrap(), Commit::Durable);
    let bytes = fs::read(&destination).unwrap();
    assert_eq!(fs::metadata(&destination).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(store.export(&set, &destination, &permit).is_err()); assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert!(store.export(&set, &files.store().join("export.json"), &permit).is_err());
    let symlink = files.0.join("existing-link.json"); std::os::unix::fs::symlink(&destination, &symlink).unwrap();
    assert!(store.export(&set, &symlink, &permit).is_err()); assert_eq!(fs::read(&destination).unwrap(), bytes);
    let cancelled = show.optional_work().unwrap(); show.set_enabled(true).unwrap();
    let blocked = files.0.join("blocked.json");
    assert!(store.export(&set, &blocked, &cancelled).is_err()); assert!(!blocked.exists());
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["entries"][0]["source"]["kind"], "external");
    assert!(value["entries"][0].get("load_key").is_none());
}

#[test]
fn optional_edits_cancel_before_publication_but_essential_saves_continue() {
    let files = Files::new(); let mut store = Store::open(files.store()).unwrap(); let mut set = session();
    store.save(&set).unwrap();
    let show = performance::Handle::default(); let optional = show.optional_work().unwrap();
    set.external(0, "Pending edit".into(), String::new()).unwrap();
    let result = store.save_checked(&set, Some(&optional), |stage| {
        if stage == 0 { show.set_enabled(true).unwrap(); } Ok(())
    });
    assert!(result.is_err()); assert!(store.sessions[&set.id].entries.is_empty());
    assert_eq!(store.save(&set).unwrap(), Commit::Durable);
}
