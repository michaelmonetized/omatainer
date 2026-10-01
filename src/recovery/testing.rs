//! Scoped private filesystem fault injection for actual recovery-worker/UI tests.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
fn roots() -> &'static Mutex<HashSet<PathBuf>> {
    static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    ROOTS.get_or_init(Default::default)
}
pub struct Guard(PathBuf);
pub fn enospc(root: &Path) -> Guard {
    assert!(roots().lock().unwrap().insert(root.into()));
    Guard(root.into())
}
impl Drop for Guard {
    fn drop(&mut self) {
        roots().lock().unwrap().remove(&self.0);
    }
}
pub(super) fn write_check(root: &Path) -> std::io::Result<()> {
    if roots().lock().unwrap().contains(root) {
        Err(std::io::Error::from_raw_os_error(libc::ENOSPC))
    } else {
        Ok(())
    }
}
