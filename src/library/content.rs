//! Bounded, worker-only content qualification. A path, inode, or filename is
//! insufficient proof that saved cues belong to a relocated track.
use super::*;
use sha2::{Digest, Sha256};

const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub(super) fn hash_file(
    path: &Path,
    expected: FileFingerprint,
    mut keep_going: impl FnMut() -> bool,
) -> Result<[u8; 32], String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("cannot verify track content: {e}"))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file()
        || before.len() > MAX_SOURCE_BYTES
        || FileFingerprint::from_metadata(&before) != expected
        || FileFingerprint::read(path) != Some(expected)
    {
        return Err(
            "track changed or is not a regular file within the 8 GiB verification limit".into(),
        );
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if !keep_going() {
            return Err("track verification cancelled".into());
        }
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or("track length overflow")?;
        if total > before.len() {
            return Err("track grew during verification".into());
        }
        hash.update(&buffer[..n]);
    }
    if total != before.len()
        || FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != expected
        || FileFingerprint::read(path) != Some(expected)
    {
        return Err("track changed during verification; no association was changed".into());
    }
    Ok(hash.finalize().into())
}

#[derive(Clone, Debug)]
pub(crate) struct Relocate {
    pub id: TrackId,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub destination: PathBuf,
}

impl Catalog {
    /// Retain a digest already measured by the sampler's same-descriptor
    /// hash/decode operation. Callers must not pass hashes merely read from a
    /// project or reusable definition. This method does no filesystem work and
    /// cannot change the current version, visible metadata or preparation.
    pub(crate) fn qualify_verified_content(
        &mut self,
        id: &TrackId,
        source: &LibSource,
        fingerprint: FileFingerprint,
        hash: [u8; 32],
    ) -> Result<(), String> {
        if !matches!(source, LibSource::File(_) | LibSource::Removable {..}) {
            return Err("sampler content proof must refer to a local file".into());
        }
        let index = self.version_track(source, Some(fingerprint))
            .ok_or("sampler content proof no longer matches a catalog version")?;
        let track = &mut self.tracks[index];
        if &track.id != id {
            return Err("sampler content proof belongs to a different track identity".into());
        }
        let equivalent=track.versions.iter().rev().find(|v|v.content_hash==Some(hash)).cloned();
        let version = track.versions.iter_mut()
            .find(|version| version.fingerprint == Some(fingerprint))
            .ok_or("sampler content proof version is missing")?;
        if version.content_hash.is_some_and(|old| old != hash) {
            return Err("sampler content proof conflicts with the saved digest".into());
        }
        if version.content_hash.is_none() {
            if let Some(old)=equivalent {
                if version.preparation==Preparation::default() {version.preparation=old.preparation;}
                if version.metadata.bpm.origin!=Origin::User {version.metadata.bpm=old.metadata.bpm.reconcile(version.metadata.bpm);}
                version.metadata.duration=version.metadata.duration.or(old.metadata.duration);
                version.metadata.last_play=version.metadata.last_play.max(old.metadata.last_play);
                if version.analysis.is_none() {version.analysis=old.analysis;}
            }
        }
        version.content_hash = Some(hash);
        Ok(())
    }

    /// Hash only a captured track selected by the metadata worker, never all
    /// imported rows. Missing/unavailable originals keep cues usable and remain
    /// explicitly unverified for relocation.
    pub fn qualify_cues_cancellable(
        &mut self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
        cancel: &std::sync::atomic::AtomicBool,
    ) {
        let Some(index) = self.version_track(source, fingerprint) else {
            return;
        };
        let Some(version) = self.tracks[index]
            .versions
            .iter_mut()
            .find(|v| v.fingerprint == fingerprint)
        else {
            return;
        };
        if version.content_hash.is_some() || (version.preparation.grid.is_none() && version.preparation.hotcues.iter().all(Option::is_none))
        {
            return;
        }
        let Some(expected)=fingerprint else {return;};
        let Ok(location)=crate::media_location::Location::resolve(source) else {return;};
        if let Ok(hash) = hash_file(&location.path, expected, || {
            !cancel.load(std::sync::atomic::Ordering::Acquire)
        }) {
            if location.recheck().is_ok() && !cancel.load(std::sync::atomic::Ordering::Acquire) {version.content_hash=Some(hash);}
        }
    }
    #[cfg(test)]
    pub fn qualify_cues(&mut self, source: &LibSource, fingerprint: Option<FileFingerprint>) {
        self.qualify_cues_cancellable(
            source,
            fingerprint,
            &std::sync::atomic::AtomicBool::new(false),
        );
    }

    pub fn update_preparation(
        &mut self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
        preparation: Option<Preparation>,
        played: Option<SystemTime>,
    ) {
        let Some(index) = self.version_track(source, fingerprint) else {
            return;
        };
        let track = &mut self.tracks[index];
        let Some(version) = track.versions.iter().find(|v| v.fingerprint == fingerprint) else {
            return;
        };
        let hash = version.content_hash;
        // Loaded/history-retained receipts at an old location still refer to
        // this stable track. Exact verified copies share their preparation.
        for version in &mut track.versions {
            if version.fingerprint == fingerprint || hash.is_some() && version.content_hash == hash
            {
                if let Some(preparation) = preparation.filter(|p| p.valid()) {
                    version.preparation = preparation;
                }
                version.metadata.last_play = version.metadata.last_play.max(played);
            }
        }
    }

    #[cfg(test)]
    pub fn relocate(&mut self, request: &Relocate) -> Result<(), String> {
        self.relocate_cancellable(request, &std::sync::atomic::AtomicBool::new(false))
    }
    pub fn relocate_cancellable(
        &mut self,
        request: &Relocate,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), String> {
        let active = || !cancel.load(std::sync::atomic::Ordering::Acquire);
        if !active() {
            return Err("Performance protection cancelled track verification".into());
        }
        let i = self
            .index
            .get(&request.source)
            .copied()
            .ok_or("original track is no longer in the library")?;
        let original = &self.tracks[i];
        if original.id != request.id
            || original.versions[original.current].fingerprint != Some(request.fingerprint)
        {
            return Err("track changed since relocation was requested; select it again".into());
        }
        if !matches!(original.source,LibSource::File(_) | LibSource::Removable {..}) {return Err("only local files or removable tracks can be relocated".into());}
        let source = LibSource::File(request.destination.clone());
        validate_source(&source)?;
        if self.index.contains_key(&source) {
            return Err("destination already belongs to a library track".into());
        }
        if original.previous_locations.len() >= 64 {
            return Err("track relocation history is full (64 locations)".into());
        }
        let fingerprint = FileFingerprint::read(&request.destination)
            .ok_or("destination is not an available regular file")?;
        let expected = original.versions[original.current]
            .content_hash
            .map(Ok::<_,String>)
            .unwrap_or_else(|| {
                let location=crate::media_location::Location::resolve(&original.source).map_err(|e|e.to_string())?;
                let hash=hash_file(&location.path,request.fingerprint,active)?;
                location.recheck().map_err(|e|e.to_string())?;Ok(hash)
            })
            .map_err(|e| format!("original content was not verified before the move: {e}"))?;
        let actual = hash_file(&request.destination, fingerprint, active)?;
        if expected != actual {
            return Err(
                "destination contains different bytes; saved cues were not reassigned".into(),
            );
        }
        let mut candidate = self.clone();
        let track = &mut candidate.tracks[i];
        track.versions[track.current].content_hash = Some(expected);
        let mut version = track.versions[track.current].clone();
        version.fingerprint = Some(fingerprint);
        let previous = PreviousLocation {
            source: track.source.clone(),
            fingerprint: request.fingerprint,
        };
        if !track.previous_locations.contains(&previous) {
            track.previous_locations.push(previous);
        }
        if let Some(index) = track
            .versions
            .iter()
            .position(|v| v.fingerprint == Some(fingerprint))
        {
            track.versions[index] = version;
            track.current = index;
        } else {
            track.versions.push(version);
            track.current = track.versions.len() - 1;
        }
        track.source = source;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
