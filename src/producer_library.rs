//! Read-only producer content discovery and explicit installed Pack resolution.
use crate::{
    dj_library::Candidate,
    engine::media_source::{FileFingerprint, LibSource},
    media_location::{Location, Snapshot},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
mod manifest;
#[cfg(test)]
mod tests;
const MAX_MANIFEST: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pack {
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub sha256: String,
    pub text: String,
    pub fields: BTreeMap<String, String>,
}
impl Pack {
    /// Validate retained installed Pack metadata.
    /// Takes this record; returns its declared identity or rejects malformed state before filesystem access.
    pub fn validate(&self) -> Result<&str, String> {
        crate::media_location::validate_root_source(&self.source).map_err(|e| e.to_string())?;
        if self.text.len() > MAX_MANIFEST
            || self.sha256 != hash(self.text.as_bytes())
            || manifest::parse(&self.text)? != self.fields
        {
            return Err("Retained Pack manifest content or metadata changed".into());
        }
        self.fields
            .get("PackUniqueID")
            .map(String::as_str)
            .ok_or("Installed Pack has no declared identity".into())
    }
    /// Recheck an explicitly reviewed installation and resolve its current content folder.
    /// Takes cancellation; returns the currently mounted installation root or refuses changed, missing or ambiguous content.
    pub fn root(&self, cancel: &AtomicBool) -> Result<PathBuf, String> {
        self.validate()?;
        let location = Location::resolve(&self.source).map_err(|e| e.to_string())?;
        let text = read(&location, self.fingerprint, MAX_MANIFEST, cancel)?;
        if hash(&text) != self.sha256 {
            return Err("Installed Pack changed after review; review its manifest again".into());
        }
        let parent = location
            .path
            .parent()
            .ok_or("Pack manifest has no parent")?;
        if location
            .path
            .file_name()
            .is_none_or(|n| n != "Properties.cfg")
            || parent
                .file_name()
                .is_none_or(|n| n != "Ableton Folder Info")
        {
            return Err(
                "Supported installed Pack metadata must be Ableton Folder Info/Properties.cfg"
                    .into(),
            );
        }
        Ok(parent.parent().ok_or("Pack has no content root")?.into())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(
    location: &Location,
    fingerprint: FileFingerprint,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&location.path)
        .map_err(|e| e.to_string())?;
    location
        .verify_file(&file, fingerprint)
        .map_err(|e| e.to_string())?;
    if fingerprint.byte_len() > limit as u64 {
        return Err("Producer metadata exceeds its source budget".into());
    }
    let mut bytes = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Producer review cancelled".into());
        }
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > limit {
            return Err("Producer metadata grew beyond its budget".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    location
        .verify_file(&file, fingerprint)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
/// Recognize producer sources without decoding audio, executing devices or changing an installation.
/// Takes a regular path and current mount inventory; returns a typed review candidate with stable source identity and explicit format limits.
pub(crate) fn probe(path: &Path, snapshot: &Snapshot) -> Result<Option<Candidate>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let manifest = path.file_name().is_some_and(|n| n == "Properties.cfg")
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "Ableton Folder Info");
    let (format, reviewable) = if manifest {
        ("Installed Pack manifest (Ableton#04I)", true)
    } else {
        match ext.as_str(){
        "als"=>("Live Set (saved format checked during review)",true),
        "adg"|"adv"=>("Ableton user device preset (state retained; engines require resolution)",true),
        "alc"=>("Ableton user clip (saved format checked during review)",true),
        "alp"=>("Pack archive; install with the authorized source application, then review its manifest",false),
        "wav"|"aif"|"aiff"|"flac"|"ogg"|"mp3"|"m4a"=>("Ordinary audio candidate; decode and applicable use checked on import",false),
        "vstpreset"=>("Native VST3 preset; use a compatible installed class",false),
        _=>return Ok(None),
    }
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    if reviewable && fingerprint.byte_len() > crate::ableton::MAX_XML as u64 {
        return Err("Producer source exceeds the 32 MiB review limit".into());
    }
    let location = snapshot.identify(path).map_err(|e| e.to_string())?;
    location
        .verify_file(&file, fingerprint)
        .map_err(|e| e.to_string())?;
    Ok(Some(Candidate {
        path: location.path,
        source: location.source,
        fingerprint,
        format: format.into(),
        reviewable,
    }))
}
fn review(candidate: &Candidate, cancel: &AtomicBool) -> Result<Pack, String> {
    let location = Location::resolve(&candidate.source).map_err(|e| e.to_string())?;
    let bytes = read(&location, candidate.fingerprint, MAX_MANIFEST, cancel)?;
    let text = String::from_utf8(bytes).map_err(|_| "Pack metadata must be UTF-8")?;
    let fields = manifest::parse(&text)?;
    let pack = Pack {
        source: candidate.source.clone(),
        fingerprint: candidate.fingerprint,
        sha256: hash(text.as_bytes()),
        text,
        fields,
    };
    pack.root(cancel)?;
    Ok(pack)
}
/// Review installed Pack metadata in a disposable process.
/// Takes the discovered identity and cancellation; returns bounded retained metadata without reading factory sounds or changing the installation.
pub(crate) fn isolated(candidate: &Candidate, cancel: &AtomicBool) -> Result<Pack, String> {
    #[cfg(test)]
    let executable = std::env::var_os("OMATAINER_TEST_BIN")
        .map(PathBuf::from)
        .ok_or("Producer worker test requires OMATAINER_TEST_BIN")?;
    #[cfg(not(test))]
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(executable);
    command.arg("producer-library-worker");
    let result: Result<Pack, String> = crate::filesystem_worker::invoke(
        command,
        candidate,
        &[],
        &|| !cancel.load(Ordering::Acquire),
        Duration::from_secs(10),
    )?;
    let pack = result?;
    pack.validate()?;
    Ok(pack)
}
/// Serve a single installed-content review before device startup.
/// Takes strict JSON on stdin; writes bounded retained metadata or an explicit source failure.
pub(crate) fn worker() -> Result<(), String> {
    crate::filesystem_worker::serve(|candidate: Candidate| {
        review(&candidate, &AtomicBool::new(false))
    })
}
