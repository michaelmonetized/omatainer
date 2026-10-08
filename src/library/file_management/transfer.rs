use super::*;
use sha2::{Digest, Sha256};
use std::{
    ffi::{CString, OsStr},
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
};

struct Folder {
    path: PathBuf,
    file: File,
    device: u64,
    inode: u64,
}
impl Folder {
    fn open(path: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Choose an absolute existing destination folder".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Copy destination is not a regular folder".into());
        }
        Ok(Self {
            path,
            file,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    fn check(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err("Destination folder changed; reviewed files were not reassigned".into());
        }
        Ok(())
    }
    fn anchored(&self, name: &OsStr) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd())).join(name)
    }
    fn rename_new(&self, old: &OsStr, new: &OsStr) -> Result<(), String> {
        let old = CString::new(old.as_bytes()).map_err(|_| "Invalid temporary filename")?;
        let new = CString::new(new.as_bytes()).map_err(|_| "Invalid destination filename")?;
        let result = unsafe {
            libc::renameat2(
                self.file.as_raw_fd(),
                old.as_ptr(),
                self.file.as_raw_fd(),
                new.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "Destination was not overwritten: {}",
                std::io::Error::last_os_error()
            ))
        }
    }
}

struct Output {
    target: Target,
    source: crate::media_location::Location,
    source_fingerprint: FileFingerprint,
    name: std::ffi::OsString,
    current: std::ffi::OsString,
    fingerprint: FileFingerprint,
    hash: [u8; 32],
    installed: bool,
}

pub(crate) struct Staged {
    folder: Folder,
    outputs: Vec<Output>,
    retain: bool,
}
impl Drop for Staged {
    fn drop(&mut self) {
        if self.retain {
            return;
        }
        for output in &self.outputs {
            let path = self.folder.anchored(&output.current);
            if FileFingerprint::read(&path) == Some(output.fingerprint) {
                let _ = fs::remove_file(path);
            }
        }
        let _ = self.folder.file.sync_all();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Installed {
    pub target: Target,
    pub destination: PathBuf,
    pub fingerprint: FileFingerprint,
    pub hash: [u8; 32],
}
impl Staged {
    #[cfg(test)]
    pub(crate) fn preserve_for_crash_test(&mut self) { self.retain=true; }
    /// Roll back only this transaction's unchanged output files.
    /// Takes the copy owner; returns after syncing the folder, or preserves any changed path for review.
    pub(crate) fn rollback(&mut self) -> Result<(), String> {
        self.folder.check()?;
        for output in &self.outputs {
            let path = self.folder.anchored(&output.current);
            match fs::symlink_metadata(&path) {
                Ok(_) if FileFingerprint::read(&path) == Some(output.fingerprint) => fs::remove_file(path).map_err(|e|e.to_string())?,
                Ok(_) => return Err("A transaction output changed; preserved for file recovery".into()),
                Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(e.to_string()),
            }
        }
        self.folder.file.sync_all().map_err(|e|e.to_string())?;
        self.retain=true;
        Ok(())
    }
    /// Atomically install each verified copy without replacing an existing path.
    /// Takes the staged owner and cancellation; returns new file proofs or retains rollback ownership until publication is explicitly confirmed.
    pub(crate) fn install(&mut self, cancel: &AtomicBool) -> Result<Vec<Installed>, String> {
        self.install_with(cancel, |_| Ok(()))
    }
    #[cfg(test)]
    pub(crate) fn install_for_test(
        &mut self,
        cancel: &AtomicBool,
        checkpoint: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<Vec<Installed>, String> {
        self.install_with(cancel, checkpoint)
    }
    fn install_with(
        &mut self,
        cancel: &AtomicBool,
        mut checkpoint: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<Vec<Installed>, String> {
        self.folder.check()?;
        for output in &self.outputs {
            output.source.recheck().map_err(|e| e.to_string())?;
            if FileFingerprint::read(&output.source.path) != Some(output.source_fingerprint)
                || FileFingerprint::read(&self.folder.anchored(&output.current))
                    != Some(output.fingerprint)
            {
                return Err("Source or staged bytes changed before copy installation".into());
            }
            if fs::symlink_metadata(self.folder.anchored(&output.name)).is_ok() {
                return Err("A destination now exists; no existing file was overwritten".into());
            }
        }
        for (index, output) in self.outputs.iter_mut().enumerate() {
            checkpoint(index)?;
            if cancel.load(Ordering::Acquire) {
                return Err(
                    "Copy installation cancelled; original files and references are unchanged"
                        .into(),
                );
            }
            self.folder.rename_new(&output.current, &output.name)?;
            output.current = output.name.clone();
            output.fingerprint = FileFingerprint::read(&self.folder.anchored(&output.current))
                .ok_or("Installed file could not be inspected; keep the operation review")?;
            output.installed = true;
        }
        self.folder.file.sync_all().map_err(|e| e.to_string())?;
        self.folder.check()?;
        for output in &self.outputs {
            output.source.recheck().map_err(|e| e.to_string())?;
            if FileFingerprint::read(&output.source.path) != Some(output.source_fingerprint) {
                return Err(
                    "Source changed during installation; no references were reassigned".into(),
                );
            }
            if content::hash_file(
                &self.folder.anchored(&output.current),
                output.fingerprint,
                || !cancel.load(Ordering::Acquire),
            )? != output.hash
            {
                return Err("Installed copy changed during verification".into());
            }
        }
        Ok(self
            .outputs
            .iter()
            .map(|output| Installed {
                target: output.target.clone(),
                destination: self.folder.path.join(&output.name),
                fingerprint: output.fingerprint,
                hash: output.hash,
            })
            .collect())
    }

    /// Preserve installed copies once the catalog owns their locations.
    /// Takes this transaction owner; disables rollback after a confirmed or possibly committed catalog publication.
    pub(crate) fn retain_installed(&mut self) -> Result<(), String> {
        if self.outputs.iter().any(|output| !output.installed) {
            return Err("An incomplete copy cannot become the saved library location".into());
        }
        self.retain = true;
        Ok(())
    }
}

pub(crate) mod journal;

/// Stage exact byte copies on the destination filesystem outside the audio renderer.
/// Takes reviewed rows, one existing destination folder and cancellation; returns an owner of verified private temporary files after all name conflicts are refused.
pub(crate) fn stage(
    catalog: &Catalog,
    targets: &[Target],
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<Staged, String> {
    checked_indices(catalog, targets)?;
    let folder = Folder::open(destination)?;
    let mut names = HashSet::new();
    let mut sources = Vec::new();
    let mut total = 0u64;
    for target in targets {
        if cancel.load(Ordering::Acquire) {
            return Err("Copy cancelled before creating files".into());
        }
        let version = &target.track.versions[target.track.current];
        let fingerprint = version
            .fingerprint
            .ok_or("Selected track has no current file proof")?;
        total = total
            .checked_add(fingerprint.byte_len())
            .ok_or("Copy selection length overflow")?;
        if total > 64 * 1024 * 1024 * 1024 {
            return Err("One reviewed copy accepts at most 64 GiB; select a smaller batch".into());
        }
        let location = crate::media_location::Location::resolve(&target.track.source)
            .map_err(|e| e.to_string())?;
        location.recheck().map_err(|e| e.to_string())?;
        let name = location
            .path
            .file_name()
            .ok_or("Selected source has no filename")?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err("Selected files share a filename. Choose separate destination folders; no files were overwritten".into());
        }
        if fs::symlink_metadata(folder.anchored(&name)).is_ok() {
            return Err(
                "Destination contains a selected filename; existing files were preserved".into(),
            );
        }
        sources.push((target.clone(), location, fingerprint, name));
    }
    let mut staged = Staged {
        folder,
        outputs: Vec::new(),
        retain: false,
    };
    for (target, source, source_fingerprint, name) in sources {
        if cancel.load(Ordering::Acquire) {
            return Err("Copy cancelled; original files and references are unchanged".into());
        }
        staged.folder.check()?;
        let hash = content::hash_file(&source.path, source_fingerprint, || {
            !cancel.load(Ordering::Acquire)
        })?;
        let version = &target.track.versions[target.track.current];
        if version.content_hash.is_some_and(|saved| saved != hash) {
            return Err(
                "Selected bytes disagree with saved preparation identity; rescan and review again"
                    .into(),
            );
        }
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&source.path)
            .map_err(|e| e.to_string())?;
        if FileFingerprint::from_metadata(&input.metadata().map_err(|e| e.to_string())?)
            != source_fingerprint
        {
            return Err("Selected source changed before copying".into());
        }
        let temporary: std::ffi::OsString =
            format!(".omatainer-copy-{}", crate::sampler_bank::BankId::new()?).into();
        let path = staged.folder.anchored(&temporary);
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let attempt = (|| {
            let mut buffer = [0u8; 64 * 1024];
            let mut copied = Sha256::new();
            let mut bytes = 0u64;
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("Copy cancelled before publication".to_string());
                }
                let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                bytes = bytes
                    .checked_add(read as u64)
                    .ok_or("Copy length overflow")?;
                if bytes > source_fingerprint.byte_len() {
                    return Err("Selected source grew during copying".into());
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|e| e.to_string())?;
                copied.update(&buffer[..read]);
            }
            if bytes != source_fingerprint.byte_len()
                || <[u8; 32]>::from(copied.finalize()) != hash
                || FileFingerprint::from_metadata(&input.metadata().map_err(|e| e.to_string())?)
                    != source_fingerprint
                || FileFingerprint::read(&source.path) != Some(source_fingerprint)
            {
                return Err(
                    "Selected source changed during copying; preparation was not reassigned".into(),
                );
            }
            output.sync_all().map_err(|e| e.to_string())?;
            Ok(())
        })();
        let fingerprint =
            FileFingerprint::from_metadata(&output.metadata().map_err(|e| e.to_string())?);
        staged.outputs.push(Output {
            target,
            source,
            source_fingerprint,
            name,
            current: temporary,
            fingerprint,
            hash,
            installed: false,
        });
        attempt?;
        if content::hash_file(&path, fingerprint, || !cancel.load(Ordering::Acquire))? != hash {
            return Err("Written copy failed independent byte verification".into());
        }
    }
    staged.folder.file.sync_all().map_err(|e| e.to_string())?;
    staged.folder.check()?;
    Ok(staged)
}
