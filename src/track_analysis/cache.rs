//! Immutable, digest-addressed waveform blobs. Only the metadata owner does I/O;
//! missing/corrupt blobs are unavailable, never silently substituted or trusted.
use super::{WaveformRef, ALGORITHM, MAX_PCM_BYTES, MAX_WAVEFORM_BINS, MAX_WAVEFORM_BYTES};
use crate::engine::media_source::FileFingerprint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 32768;
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Waveform {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Full-source mean magnitudes: low, mid and high bands, in source order.
    pub bands: Vec<[f32; 3]>,
}
impl Waveform {
    pub fn validate(&self) -> Result<(), String> {
        if self.frames == 0
            || !(1..=crate::project_file::MAX_SAMPLE_RATE).contains(&self.sample_rate)
            || !(1..=crate::project_file::MAX_CHANNELS).contains(&self.channels)
            || !(1..=MAX_WAVEFORM_BINS).contains(&self.bands.len())
            || self
                .frames
                .checked_mul(u64::from(self.channels))
                .and_then(|v| v.checked_mul(4))
                .is_none_or(|bytes| bytes > MAX_PCM_BYTES)
            || self
                .bands
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("invalid or oversized analysis waveform".into());
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    schema: u32,
    algorithm: u32,
    // A fresh reanalysis can repair a corrupt canonical cache entry without
    // overwriting it. The nonce is part of the hashed immutable blob itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repair_nonce: Option<[u8; 16]>,
    waveform: Waveform,
}

pub(crate) struct Cache {
    root: PathBuf,
    directory: File,
    lock: File,
}
impl Drop for Cache {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}
fn private_regular(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0
}
fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("waveform cache operation cancelled".into())
    } else {
        Ok(())
    }
}
fn basename(hash: &[u8; 32]) -> String {
    let hex: String = hash.iter().map(|v| format!("{v:02x}")).collect();
    format!("{hex}.wave.json")
}
fn encoded(
    waveform: &Waveform,
    repair_nonce: Option<[u8; 16]>,
) -> Result<(Vec<u8>, WaveformRef), String> {
    let bytes = serde_json::to_vec(&Blob {
        schema: 1,
        algorithm: ALGORITHM,
        repair_nonce,
        waveform: waveform.clone(),
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_WAVEFORM_BYTES as usize {
        return Err("serialized waveform exceeds 256 KiB".into());
    }
    let reference = WaveformRef {
        sha256: Sha256::digest(&bytes).into(),
        bytes: bytes.len() as u32,
        frames: waveform.frames,
        sample_rate: waveform.sample_rate,
        channels: waveform.channels,
        bins: waveform.bands.len() as u16,
    };
    Ok((bytes, reference))
}
impl Cache {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("analysis cache location must be absolute".into());
        }
        if !root.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)
                .map_err(|e| e.to_string())?;
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)
            .map_err(|e| e.to_string())?;
        let meta = directory.metadata().map_err(|e| e.to_string())?;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err("analysis cache must be a private directory owned by this user".into());
        }
        let lock_path = root.join("writer.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|e| e.to_string())?;
        let meta = lock.metadata().map_err(|e| e.to_string())?;
        if !private_regular(&meta) || meta.nlink() != 1 {
            return Err("analysis cache lock is not a private regular file".into());
        }
        lock.try_lock()
            .map_err(|_| "analysis cache already has an active owner")?;
        let cache = Self {
            root,
            directory,
            lock,
        };
        cache.check_directory()?;
        cache.check_lock()?;
        Ok(cache)
    }
    fn check_directory(&self) -> Result<(), String> {
        let before = self.directory.metadata().map_err(|e| e.to_string())?;
        let path = fs::symlink_metadata(&self.root).map_err(|e| e.to_string())?;
        if !path.is_dir()
            || before.dev() != path.dev()
            || before.ino() != path.ino()
            || path.uid() != unsafe { libc::geteuid() }
            || path.mode() & 0o077 != 0
        {
            return Err("analysis cache directory changed; path preserved".into());
        }
        Ok(())
    }
    fn check_lock(&self) -> Result<(), String> {
        let expected = self.lock.metadata().map_err(|e| e.to_string())?;
        let path =
            fs::symlink_metadata(self.root.join("writer.lock")).map_err(|e| e.to_string())?;
        if !private_regular(&path)
            || expected.dev() != path.dev()
            || expected.ino() != path.ino()
            || path.nlink() != 1
        {
            return Err("analysis cache lock changed; work was refused".into());
        }
        Ok(())
    }
    pub fn read(
        &self,
        reference: &WaveformRef,
        cancel: impl Fn() -> bool,
    ) -> Result<Waveform, String> {
        cancelled(&cancel)?;
        if !reference.valid() {
            return Err("invalid waveform cache reference".into());
        }
        self.check_directory()?;
        self.check_lock()?;
        let path = self.root.join(basename(&reference.sha256));
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|e| format!("cached waveform unavailable: {e}"))?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !private_regular(&before) || before.len() != u64::from(reference.bytes) {
            return Err("cached waveform identity or size changed".into());
        }
        let mut bytes = Vec::with_capacity(reference.bytes as usize);
        Read::by_ref(&mut file)
            .take(u64::from(MAX_WAVEFORM_BYTES) + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        cancelled(&cancel)?;
        if bytes.len() != reference.bytes as usize
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != reference.sha256
            || FileFingerprint::from_metadata(&before)
                != FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?)
            || fs::symlink_metadata(&path)
                .ok()
                .as_ref()
                .map(FileFingerprint::from_metadata)
                != Some(FileFingerprint::from_metadata(&before))
        {
            return Err("cached waveform changed or failed its digest check".into());
        }
        let blob: Blob =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid cached waveform: {e}"))?;
        blob.waveform.validate()?;
        if blob.schema != 1
            || blob.algorithm != ALGORITHM
            || blob.waveform.frames != reference.frames
            || blob.waveform.sample_rate != reference.sample_rate
            || blob.waveform.channels != reference.channels
            || blob.waveform.bands.len() != usize::from(reference.bins)
        {
            return Err(
                "cached waveform geometry or algorithm does not match its reference".into(),
            );
        }
        self.check_directory()?;
        self.check_lock()?;
        Ok(blob.waveform)
    }
    fn capacity(&self, additional: u64, cancel: &impl Fn() -> bool) -> Result<(), String> {
        let mut files = 0usize;
        let mut bytes = 0u64;
        for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            cancelled(cancel)?;
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == "writer.lock" {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if !private_regular(&meta) {
                return Err("unexpected entry in analysis cache; preserved".into());
            }
            files += 1;
            bytes = bytes
                .checked_add(meta.len())
                .ok_or("analysis cache size overflow")?;
            if files >= MAX_FILES || bytes > MAX_BYTES.saturating_sub(additional) {
                return Err("analysis waveform cache is full (1 GiB / 32768 files); existing results retained".into());
            }
        }
        Ok(())
    }
    pub fn write(
        &mut self,
        waveform: Waveform,
        cancel: impl Fn() -> bool,
    ) -> Result<WaveformRef, String> {
        self.write_with(waveform, cancel, |_, _| {})
    }
    fn write_with(
        &mut self,
        waveform: Waveform,
        cancel: impl Fn() -> bool,
        mut checkpoint: impl FnMut(u8, &Path),
    ) -> Result<WaveformRef, String> {
        waveform.validate()?;
        cancelled(&cancel)?;
        self.check_directory()?;
        self.check_lock()?;
        let (mut bytes, mut reference) = encoded(&waveform, None)?;
        let mut target = self.root.join(basename(&reference.sha256));
        match fs::symlink_metadata(&target) {
            Ok(meta) => {
                if !private_regular(&meta) {
                    return Err(
                        "waveform cache entry is redirected or not private; preserved".into(),
                    );
                }
                if self.read(&reference, &cancel).is_ok() {
                    return Ok(reference);
                }
                cancelled(&cancel)?;
                self.check_directory()?;
                self.check_lock()?;
                let mut nonce = [0u8; 16];
                File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut nonce))
                    .map_err(|e| e.to_string())?;
                (bytes, reference) = encoded(&waveform, Some(nonce))?;
                target = self.root.join(basename(&reference.sha256));
                // One attempt only. No corruption repair can chase or replace
                // an attacker-controlled path, and every blob counts in quota.
                match fs::symlink_metadata(&target) {
                    Ok(_) => {
                        return Err(
                            "waveform repair identity already exists; paths preserved".into()
                        )
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        self.capacity(bytes.len() as u64, &cancel)?;
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let suffix: String = random.iter().map(|v| format!("{v:02x}")).collect();
        let temporary = self.root.join(format!(".pending-{suffix}"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let original = file.metadata().map_err(|e| e.to_string())?;
        let result = (|| {
            checkpoint(0, &temporary);
            for part in bytes.chunks(64 * 1024) {
                cancelled(&cancel)?;
                file.write_all(part).map_err(|e| e.to_string())?;
            }
            file.sync_all().map_err(|e| e.to_string())?;
            checkpoint(1, &temporary);
            cancelled(&cancel)?;
            self.check_directory()?;
            self.check_lock()?;
            let written = file.metadata().map_err(|e| e.to_string())?;
            if fs::symlink_metadata(&temporary)
                .ok()
                .as_ref()
                .map(FileFingerprint::from_metadata)
                != Some(FileFingerprint::from_metadata(&written))
            {
                return Err("temporary waveform path changed; preserved".into());
            }
            // Create a complete immutable blob without replacing any existing
            // path. A late cache cancellation can leave only an unused blob;
            // publishing its catalog reference remains separately guarded.
            match fs::hard_link(&temporary, &target) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.read(&reference, || false)?;
                }
                Err(e) => return Err(e.to_string()),
            }
            checkpoint(2, &temporary);
            self.directory
                .sync_all()
                .map_err(|e| format!("waveform cached but durability unconfirmed: {e}"))?;
            self.read(&reference, || false)?;
            Ok(reference)
        })();
        match fs::symlink_metadata(&temporary) {
            Ok(meta) if meta.dev() == original.dev() && meta.ino() == original.ino() => {
                fs::remove_file(&temporary)
                    .map_err(|e| format!("waveform temporary cleanup failed: {e}"))?;
            }
            Ok(_) => return Err("temporary waveform path changed; external entry preserved".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        result
    }
}

#[cfg(test)]
mod tests;
