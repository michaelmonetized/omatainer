//! Worker-only bounded reads and atomic, exclusive MIDI export publication.
use crate::{engine::media_source::FileFingerprint, engine::performance::WorkPermit, midi_file};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("MIDI operation cancelled".into())
    } else {
        Ok(())
    }
}
pub(crate) fn read(
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(midi_file::File, FileFingerprint), String> {
    cancelled(cancel)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Open MIDI file: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > midi_file::MAX_BYTES as u64 {
        return Err("Choose a regular MIDI file of at most 16 MiB".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut chunk = [0u8; 32768];
    loop {
        cancelled(cancel)?;
        let size = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        if bytes.len() + size > midi_file::MAX_BYTES {
            return Err("MIDI file grew beyond 16 MiB".into());
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != fingerprint
        || FileFingerprint::read(path) != Some(fingerprint)
    {
        return Err("MIDI file changed during inspection; inspect it again".into());
    }
    let result = midi_file::decode_with_cancel(&bytes, || cancel.load(Ordering::Acquire))
        .map_err(|e| e.to_string())?;
    Ok((result, fingerprint))
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Existing exports are never changed. The hard-link commit atomically claims
/// an absent destination, including against a competing file creation.
pub(crate) fn write_new(path: &Path, bytes: &[u8], work: &WorkPermit) -> Result<(), String> {
    write_new_at_boundary(path, bytes, work, || {})
}
fn write_new_at_boundary(
    path: &Path,
    bytes: &[u8],
    work: &WorkPermit,
    before_commit: impl FnOnce(),
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    if bytes.len() > midi_file::MAX_BYTES || path.as_os_str().len() > 4096 {
        return Err("MIDI export exceeds supported size/path bounds".into());
    }
    let cancel = work.cancel();
    cancelled(&cancel)?;
    let leaf = path.file_name().ok_or("Choose a MIDI file name")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(parent)
        .map_err(|e| format!("Open MIDI export directory: {e}"))?;
    let stable_parent = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    let destination = stable_parent.join(leaf);
    let temporary = Temporary(stable_parent.join(format!(
        ".omatainer-midi-{}.tmp",
        crate::sampler_bank::BankId::new()?
    )));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary.0)
        .map_err(|e| e.to_string())?;
    for chunk in bytes.chunks(32768) {
        cancelled(&cancel)?;
        file.write_all(chunk)
            .map_err(|e| format!("Write MIDI temporary: {e}"))?;
    }
    file.sync_all()
        .map_err(|e| format!("Sync MIDI export: {e}"))?;
    before_commit();
    cancelled(&cancel)?;
    let current =
        fs::metadata(parent).map_err(|e| format!("MIDI export directory changed: {e}"))?;
    let held = directory.metadata().map_err(|e| e.to_string())?;
    if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
        return Err("MIDI export directory changed during writing; choose the path again".into());
    }
    let _publication = work.commit().map_err(|e| e.to_string())?;
    cancelled(&cancel)?;
    fs::hard_link(&temporary.0, &destination)
        .map_err(|e| format!("Publish MIDI export (choose an unused path): {e}"))?;
    // A post-publication durability error must report that the destination
    // exists. It is never mislabeled cancellation or removed after commit.
    directory
        .sync_all()
        .map_err(|e| format!("MIDI export was published, but directory sync failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "omat-midi-io-{}",
                crate::sampler_bank::BankId::new().unwrap()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn exclusive_publication_rejects_existing_racing_and_symlink_destinations() {
        let root = Directory::new();
        let gate = crate::engine::performance::Handle::default();
        let work = gate.optional_work().unwrap();
        let bytes = include_bytes!("../tests/fixtures/midi/sixteen-bars-ppqn960.mid");
        let output = root.0.join("new.mid");
        write_new(&output, bytes, &work).unwrap();
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert!(write_new(&output, b"replacement", &work)
            .unwrap_err()
            .contains("unused path"));
        assert_eq!(fs::read(&output).unwrap(), bytes);
        let race = root.0.join("race.mid");
        assert!(
            write_new_at_boundary(&race, bytes, &work, || fs::write(&race, b"another writer")
                .unwrap())
            .unwrap_err()
            .contains("unused path")
        );
        assert_eq!(fs::read(&race).unwrap(), b"another writer");
        let link = root.0.join("link.mid");
        std::os::unix::fs::symlink(&output, &link).unwrap();
        assert!(write_new(&link, bytes, &work).is_err());
        assert!(read(&link, &AtomicBool::new(false)).is_err());
        assert_eq!(
            fs::read_dir(&root.0).unwrap().count(),
            3,
            "Private temporary files were removed"
        );
    }
    #[test]
    fn cancellation_and_directory_replacement_leave_no_published_or_temporary_file() {
        let root = Directory::new();
        let gate = crate::engine::performance::Handle::default();
        let work = gate.optional_work().unwrap();
        let cancel = work.cancel();
        let bytes = include_bytes!("../tests/fixtures/midi/sixteen-bars-ppqn960.mid");
        assert!(
            write_new_at_boundary(&root.0.join("cancel.mid"), bytes, &work, || cancel
                .store(true, Ordering::Release))
            .unwrap_err()
            .contains("cancelled")
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
        drop(work);
        drop(cancel);
        let work = gate.optional_work().unwrap();
        let parent = root.0.join("parent");
        let moved = root.0.join("moved");
        fs::create_dir(&parent).unwrap();
        assert!(
            write_new_at_boundary(&parent.join("output.mid"), bytes, &work, || {
                fs::rename(&parent, &moved).unwrap();
                fs::create_dir(&parent).unwrap();
            })
            .unwrap_err()
            .contains("directory changed")
        );
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
        assert!(read(&root.0, &AtomicBool::new(false)).is_err());
        assert!(read(&root.0.join("absent.mid"), &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled"));
    }
}
