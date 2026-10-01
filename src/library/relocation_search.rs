//! Read-only, bounded replacement discovery. Every match is a measured byte
//! equivalence, never a filename/size guess. Publication does not relocate.
use super::*;
use crate::media_location::{Access, Failure, Location, Snapshot};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use walkdir::WalkDir;

pub(crate) const MAX_ROOTS: usize = 64;
#[derive(Clone, Copy)]
struct Limits {
    depth: usize,
    visits: u64,
    files: u64,
    directories: usize,
    matches: usize,
    bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 64,
            visits: 1_000_000,
            files: 100_000,
            directories: 4096,
            matches: 256,
            bytes: 64 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Default)]
pub(crate) struct Progress {
    pub entries: AtomicU64,
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub skipped: AtomicU64,
    pub stage: AtomicU8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Reason {
    Unavailable,
    Unreadable,
    Symlink,
    ForeignMount,
    Invalid,
    Changed,
    Limit,
}
impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable root",
            Self::Unreadable => "unreadable entry",
            Self::Symlink => "symlink not followed",
            Self::ForeignMount => "nested mount not searched",
            Self::Invalid => "invalid media path",
            Self::Changed => "changed during verification",
            Self::Limit => "search limit",
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Skipped {
    pub reason: Reason,
    pub path: String,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub location: Location,
    pub fingerprint: FileFingerprint,
    pub hash: [u8; 32],
    access: Access,
}
impl Candidate {
    /// Recheck an explicitly reviewed choice before the normal relocation
    /// transaction. Copies swapped since review require another search.
    pub fn check(&self, snapshot: &Snapshot) -> Result<(), String> {
        self.access
            .check(snapshot, &self.location.path)
            .map_err(|e| e.to_string())?;
        self.location
            .recheck_with(snapshot)
            .map_err(|e| e.to_string())?;
        let metadata = snapshot
            .inspect(&self.location)
            .map_err(|e| e.to_string())?;
        if !metadata.is_file() || FileFingerprint::from_metadata(&metadata) != self.fingerprint {
            return Err("replacement changed since review; search again".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn verify(
        &self,
        snapshot: &Snapshot,
        mut active: impl FnMut() -> bool,
    ) -> Result<(), String> {
        cancelled(&mut active)?;
        self.check(snapshot)?;
        let measured = content::hash_file(&self.location.path, self.fingerprint, &mut active)?;
        if measured != self.hash {
            return Err("replacement content changed since review".into());
        }
        cancelled(&mut active)?;
        self.check(&Snapshot::discover().map_err(|e| e.to_string())?)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Receipt {
    pub target: Relocate,
    pub original_hash: [u8; 32],
    pub matches: Vec<Candidate>,
    pub complete: bool,
    pub skipped: [u64; 7],
    pub samples: Vec<Skipped>,
    pub entries: u64,
    pub files: u64,
    pub bytes: u64,
}
fn bounded(value: impl ToString, bytes: usize) -> String {
    let mut value = value.to_string();
    if value.len() > bytes {
        let mut end = bytes;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}
fn skip(
    receipt: &mut Receipt,
    progress: &Progress,
    path: &Path,
    reason: Reason,
    detail: impl ToString,
) {
    receipt.complete = false;
    receipt.skipped[reason as usize] += 1;
    progress.skipped.fetch_add(1, Ordering::Relaxed);
    if receipt.samples.len() < 32 {
        receipt.samples.push(Skipped {
            reason,
            path: bounded(path.display(), 1024),
            detail: bounded(detail, 256),
        });
    }
}
fn cancelled(active: &mut impl FnMut() -> bool) -> Result<(), String> {
    if active() {
        Ok(())
    } else {
        Err("replacement search cancelled; no associations changed".into())
    }
}
#[cfg(test)]
pub(crate) fn search(
    catalog: &Catalog,
    target: &Relocate,
    roots: &[PathBuf],
    progress: &Progress,
    active: impl FnMut() -> bool,
) -> Result<Receipt, String> {
    run(
        catalog,
        target,
        roots,
        progress,
        active,
        &mut Snapshot::discover,
        Limits::default(),
    )
}
pub(crate) fn search_with_inventory(
    catalog: &Catalog,
    target: &Relocate,
    roots: &[PathBuf],
    progress: &Progress,
    active: impl FnMut() -> bool,
    inventory: &mut impl FnMut() -> Result<Snapshot, Failure>,
) -> Result<Receipt, String> {
    run(
        catalog,
        target,
        roots,
        progress,
        active,
        inventory,
        Limits::default(),
    )
}
fn run(
    catalog: &Catalog,
    target: &Relocate,
    roots: &[PathBuf],
    progress: &Progress,
    mut active: impl FnMut() -> bool,
    inventory: &mut impl FnMut() -> Result<Snapshot, Failure>,
    limits: Limits,
) -> Result<Receipt, String> {
    cancelled(&mut active)?;
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err("choose 1–64 absolute search folders".into());
    }
    for root in roots {
        validate_source(&LibSource::File(root.clone()))?;
    }
    let track = catalog
        .track(&target.source)
        .ok_or("captured track is no longer in the library")?;
    let version = &track.versions[track.current];
    if track.id != target.id || version.fingerprint != Some(target.fingerprint) {
        return Err("track changed since the search was requested; select it again".into());
    }
    if !matches!(
        target.source,
        LibSource::File(_) | LibSource::Removable { .. }
    ) || target.fingerprint.byte_len() > 8 * 1024 * 1024 * 1024
    {
        return Err(
            "search requires captured local media within the 8 GiB verification limit".into(),
        );
    }
    let snapshot = inventory().map_err(|e| e.to_string())?;
    let original_hash = if let Some(hash) = version.content_hash {
        hash
    } else {
        let original = snapshot
            .resolve(&target.source)
            .map_err(|e| format!("original content was not verified before the move: {e}"))?;
        let access = snapshot.access(&original.path).map_err(|e| e.to_string())?;
        let measured =
            content::hash_file_progress(&original.path, target.fingerprint, &mut active, |n| {
                progress.bytes.fetch_add(n, Ordering::Relaxed);
            })
            .map_err(|e| format!("original content was not verified before the move: {e}"))?;
        let fresh = inventory().map_err(|e| e.to_string())?;
        access
            .check(&fresh, &original.path)
            .map_err(|e| e.to_string())?;
        original.recheck_with(&fresh).map_err(|e| e.to_string())?;
        measured
    };
    let original_path = snapshot
        .resolve(&target.source)
        .ok()
        .map(|location| location.path);
    let mut receipt = Receipt {
        target: target.clone(),
        original_hash,
        matches: Vec::new(),
        complete: true,
        skipped: [0; 7],
        samples: Vec::new(),
        entries: 0,
        files: 0,
        bytes: 0,
    };
    let mut inputs = Vec::new();
    for root in roots {
        cancelled(&mut active)?;
        match snapshot.identify(root).and_then(|location| {
            if !snapshot.inspect(&location)?.is_dir() {
                return Err(Failure::Unsupported);
            }
            let access = snapshot.access(&location.path)?;
            Ok((location, access))
        }) {
            Ok(input) => inputs.push(input),
            Err(error) => skip(&mut receipt, progress, root, Reason::Unavailable, error),
        }
    }
    inputs.sort_by(|a, b| {
        a.0.path
            .components()
            .count()
            .cmp(&b.0.path.components().count())
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    let mut selected: Vec<(Location, Access)> = Vec::new();
    for input in inputs {
        if !selected
            .iter()
            .any(|old| (input.1 == old.1 && input.0.path.starts_with(&old.0.path)) || input.0.source == old.0.source)
        {
            selected.push(input);
        }
    }
    let mut directories = Vec::new();
    progress.stage.store(1, Ordering::Relaxed);
    'roots: for (root, access) in &selected {
        let mut walk = WalkDir::new(&root.path)
            .follow_links(false)
            .max_depth(limits.depth + 1)
            .into_iter();
        while let Some(entry) = walk.next() {
            cancelled(&mut active)?;
            if progress.entries.load(Ordering::Relaxed) >= limits.visits {
                skip(
                    &mut receipt,
                    progress,
                    &root.path,
                    Reason::Limit,
                    "one million visited entries; narrow the search",
                );
                break 'roots;
            }
            progress.entries.fetch_add(1, Ordering::Relaxed);
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    skip(
                        &mut receipt,
                        progress,
                        error.path().unwrap_or(&root.path),
                        Reason::Unreadable,
                        &error,
                    );
                    continue;
                }
            };
            let path = entry.path();
            if entry.file_type().is_symlink() {
                skip(
                    &mut receipt,
                    progress,
                    path,
                    Reason::Symlink,
                    "descendant symlink",
                );
                continue;
            }
            if entry.depth() > limits.depth {
                skip(
                    &mut receipt,
                    progress,
                    path,
                    Reason::Limit,
                    "64 folder levels; narrow the search",
                );
                if entry.file_type().is_dir() {
                    walk.skip_current_dir();
                }
                continue;
            }
            if let Err(error) = access.check(&snapshot, path) {
                skip(&mut receipt, progress, path, Reason::ForeignMount, error);
                if entry.file_type().is_dir() {
                    walk.skip_current_dir();
                }
                continue;
            }
            let location = match root.child(path) {
                Ok(location) => location,
                Err(error) => {
                    skip(&mut receipt, progress, path, Reason::Invalid, error);
                    if entry.file_type().is_dir() {
                        walk.skip_current_dir();
                    }
                    continue;
                }
            };
            if let Err(error) = validate_source(&location.source) {
                // A volume root's empty relative path is allowed for walking.
                if !(entry.depth() == 0 && entry.file_type().is_dir()) {
                    skip(&mut receipt, progress, path, Reason::Invalid, error);
                    if entry.file_type().is_dir() {
                        walk.skip_current_dir();
                    }
                    continue;
                }
            }
            let metadata = match snapshot.inspect(&location) {
                Ok(metadata) => metadata,
                Err(error) => {
                    skip(&mut receipt, progress, path, Reason::Unreadable, error);
                    if entry.file_type().is_dir() {
                        walk.skip_current_dir();
                    }
                    continue;
                }
            };
            if metadata.is_dir() {
                if directories.len() >= limits.directories {
                    skip(
                        &mut receipt,
                        progress,
                        path,
                        Reason::Limit,
                        "4,096 checked directories; narrow the search",
                    );
                    walk.skip_current_dir();
                    continue;
                }
                directories.push((
                    location,
                    access.clone(),
                    FileFingerprint::from_metadata(&metadata),
                ));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if progress.files.load(Ordering::Relaxed) >= limits.files {
                skip(
                    &mut receipt,
                    progress,
                    path,
                    Reason::Limit,
                    "100,000 inspected files; narrow the search",
                );
                break 'roots;
            }
            progress.files.fetch_add(1, Ordering::Relaxed);
            let fingerprint = FileFingerprint::from_metadata(&metadata);
            if fingerprint.byte_len() != target.fingerprint.byte_len()
                || location.source == target.source
                || original_path.as_ref() == Some(&location.path)
            {
                continue;
            }
            if progress
                .bytes
                .load(Ordering::Relaxed)
                .checked_add(fingerprint.byte_len())
                .is_none_or(|n| n > limits.bytes)
            {
                skip(
                    &mut receipt,
                    progress,
                    path,
                    Reason::Limit,
                    "64 GiB hashed bytes; narrow the search",
                );
                break 'roots;
            }
            let hash = match content::hash_file_progress(path, fingerprint, &mut active, |n| {
                progress.bytes.fetch_add(n, Ordering::Relaxed);
            }) {
                Ok(hash) => hash,
                Err(error) => {
                    cancelled(&mut active)?;
                    let reason = if error.contains("changed") || error.contains("grew") {
                        Reason::Changed
                    } else {
                        Reason::Unreadable
                    };
                    skip(&mut receipt, progress, path, reason, error);
                    continue;
                }
            };
            if hash != original_hash || location.source == target.source {
                continue;
            }
            if receipt
                .matches
                .iter()
                .any(|candidate| candidate.location.source == location.source)
            {
                continue;
            }
            if receipt.matches.len() >= limits.matches {
                skip(
                    &mut receipt,
                    progress,
                    path,
                    Reason::Limit,
                    "256 matching paths; choose a narrower root",
                );
                break 'roots;
            }
            receipt.matches.push(Candidate {
                location,
                fingerprint,
                hash,
                access: access.clone(),
            });
        }
    }
    cancelled(&mut active)?;
    progress.stage.store(2, Ordering::Relaxed);
    let fresh = inventory().map_err(|e| e.to_string())?;
    for (location, access, fingerprint) in &directories {
        cancelled(&mut active)?;
        access
            .check(&fresh, &location.path)
            .map_err(|e| format!("search mount changed; retry: {e}"))?;
        location.recheck_with(&fresh).map_err(|e| e.to_string())?;
        if fresh
            .inspect(location)
            .map(|m| FileFingerprint::from_metadata(&m))
            .ok()
            != Some(*fingerprint)
        {
            return Err(
                "search folder changed during traversal; retry before reviewing matches".into(),
            );
        }
    }
    for candidate in &receipt.matches {
        cancelled(&mut active)?;
        candidate
            .access
            .check(&fresh, &candidate.location.path)
            .map_err(|e| e.to_string())?;
        candidate
            .location
            .recheck_with(&fresh)
            .map_err(|e| e.to_string())?;
        if fresh
            .inspect(&candidate.location)
            .map(|m| FileFingerprint::from_metadata(&m))
            .ok()
            != Some(candidate.fingerprint)
        {
            return Err("matching file changed during search; no association was changed".into());
        }
    }
    receipt
        .matches
        .sort_by(|a, b| a.location.path.cmp(&b.location.path));
    receipt.entries = progress.entries.load(Ordering::Relaxed);
    receipt.files = progress.files.load(Ordering::Relaxed);
    receipt.bytes = progress.bytes.load(Ordering::Relaxed);
    progress.stage.store(3, Ordering::Relaxed);
    Ok(receipt)
}

#[cfg(test)]
mod tests;
