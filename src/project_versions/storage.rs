use super::*;
use crate::ui::project;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
pub(crate) mod tests;

#[derive(Clone, Default)]
pub(crate) struct Roots {
    pub archive: Option<PathBuf>,
    pub recordings: Option<PathBuf>,
    pub renders: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub recovery: Option<PathBuf>,
}
pub(crate) struct Compacted {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub source_fingerprint: FileFingerprint,
    pub bundle: Bundle<project::Document>,
    pub before_media: usize,
    pub after_media: usize,
    pub before_pcm: u64,
    pub after_pcm: u64,
}
/// Count decoded PCM once per native embedded source entry.
/// Takes bounded native media; returns its stored float sample bytes before any content deduplication.
fn bytes(media: &[Arc<Sample>]) -> u64 {
    media.iter().map(|s| s.data.len() as u64 * 4).sum()
}
/// Read one native archive without changing its project or source media.
/// Takes an absolute path and cancellation; returns a fully validated document and embedded audio.
fn archive(path: &Path, cancel: &AtomicBool) -> Result<Bundle<project::Document>, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute native project path".into());
    }
    let bundle = load::<project::Document>(path, cancel)?;
    bundle.state.validate()?;
    bundle.state.engine.validate(&bundle.media)?;
    Ok(bundle)
}
/// Prepare a compacted copy while preserving all source metadata and musical references.
/// Takes an existing native archive, a new destination and cancellation; returns an owned worker draft with exact source identity and before/after PCM totals.
pub(crate) fn compact(
    source: PathBuf,
    destination: PathBuf,
    cancel: &AtomicBool,
) -> Result<Compacted, String> {
    if !destination.is_absolute() || destination == source {
        return Err("Choose a new absolute destination for the compacted copy".into());
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("Compacted copy destination already exists".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let fingerprint = FileFingerprint::read(&source).ok_or("Cannot inspect source archive")?;
    let mut bundle = archive(&source, cancel)?;
    let before_media = bundle.media.len();
    let before_pcm = bytes(&bundle.media);
    let mut known = HashMap::<([u8; 32], [u8; 32]), Arc<Sample>>::new();
    for sample in &mut bundle.media {
        active(cancel)?;
        let audio = crate::project_dependencies::audio_hash(sample, cancel)?;
        let metadata: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&project_file::Media::from_sample(sample))
                .map_err(|e| e.to_string())?,
        )
        .into();
        if let Some(retained) = known.get(&(audio, metadata)) {
            *sample = retained.clone();
        } else {
            known.insert((audio, metadata), sample.clone());
        }
    }
    bundle.state.engine.compact_media(&mut bundle.media)?;
    bundle.state.validate()?;
    bundle.state.engine.validate(&bundle.media)?;
    if FileFingerprint::read(&source) != Some(fingerprint) {
        return Err("Source archive changed while compacting; review again".into());
    }
    active(cancel)?;
    Ok(Compacted {
        source,
        destination,
        source_fingerprint: fingerprint,
        before_media,
        after_media: bundle.media.len(),
        before_pcm,
        after_pcm: bytes(&bundle.media),
        bundle,
    })
}
impl Compacted {
    /// Publish the reviewed compacted copy without replacing another file.
    /// Takes the worker-owned draft, cancellation and commit admission; returns the native save outcome while preserving its original archive.
    pub(crate) fn publish<G>(
        &self,
        cancel: &AtomicBool,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<SaveOutcome, String> {
        active(cancel)?;
        if FileFingerprint::read(&self.source) != Some(self.source_fingerprint) {
            return Err(
                "Reviewed source archive changed; original and destination preserved".into(),
            );
        }
        let _commit = authorize()?;
        active(cancel)?;
        if FileFingerprint::read(&self.source) != Some(self.source_fingerprint) {
            return Err("Reviewed source archive changed at publication; original and destination preserved".into());
        }
        project_file::save(
            &self.destination,
            &self.bundle,
            Overwrite::Never,
            &Limits::default(),
            cancel,
        )
        .map_err(|e| e.to_string())
    }
    /// Summarize a worker draft without sending its audio to the GUI.
    /// Takes the verified draft; returns counts and the two explicit archive paths.
    pub(crate) fn description(&self) -> String {
        format!(
            "{} → {} embedded sources; {} → {} PCM bytes. Original: {}. New copy: {}.",
            self.before_media,
            self.after_media,
            self.before_pcm,
            self.after_pcm,
            self.source.display(),
            self.destination.display()
        )
    }
}
/// Measure a chosen folder with bounded read-only metadata work.
/// Takes an absolute real directory and cancellation; returns sampled file bytes/count or refuses links, special files, excess depth or a changed root.
fn folder(path: &Path, cancel: &AtomicBool) -> Result<(u64, usize), String> {
    if !path.is_absolute() {
        return Err("Storage folders must be absolute".into());
    }
    let before = directory(path)?;
    let mut bytes = 0u64;
    let mut files = 0;
    let mut entries = 0;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .max_depth(17)
    {
        active(cancel)?;
        entries += 1;
        if entries > 65_536 {
            return Err("Storage inventory exceeds 65536 entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.depth() > 16 {
            return Err("Storage inventory exceeds 16 directory levels".into());
        }
        if entry.file_type().is_symlink() {
            return Err("Storage inventory preserves symlinks without following them".into());
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("Storage inventory encountered a special file; preserved".into());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Storage file changed during inspection".into());
        }
        bytes = bytes
            .checked_add(metadata.len())
            .filter(|n| *n <= 64 * 1024 * 1024 * 1024)
            .ok_or("Storage inventory exceeds 64 GiB")?;
        files += 1;
    }
    let after = directory(path)?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err("Storage root changed during inspection".into());
    }
    Ok((bytes, files))
}
/// Explain a project's stored audio, shared slices, chosen output folders and recovery.
/// Takes optional named-version storage, explicit roots and cancellation; returns a read-only bounded report without deleting files or opening audio devices.
pub(crate) fn inspect(
    store: Option<&Store>,
    roots: &Roots,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut lines = vec!["Storage usage is sampled from files. Shared folders can include other projects; category totals can overlap.".into()];
    if let Some(path) = &roots.archive {
        let before = FileFingerprint::read(path).ok_or("Cannot inspect native archive")?;
        let bundle = archive(path, cancel)?;
        lines.push(format!(
            "Native archive: {} bytes; {} embedded sources; {} decoded PCM bytes. {}",
            before.byte_len(),
            bundle.media.len(),
            bytes(&bundle.media),
            path.display()
        ));
        let mut sources = HashSet::new();
        let mut source_bytes = 0u64;
        let mut unavailable = 0;
        for sample in &bundle.media {
            active(cancel)?;
            if !Path::new(&sample.path).is_absolute() || !sources.insert(sample.path.clone()) {
                continue;
            }
            match fs::symlink_metadata(&sample.path) {
                Ok(m) if m.is_file() => {
                    source_bytes = source_bytes
                        .checked_add(m.len())
                        .ok_or("Original source size overflow")?;
                }
                _ => unavailable += 1,
            }
        }
        let state = &bundle.state.engine;
        let clips = state.tracks.iter().flat_map(|t| t.clips.iter()).chain(
            state
                .arrangement
                .iter()
                .flat_map(|a| a.sources.iter().map(|s| &s.clip)),
        );
        let mut slices = 0;
        let mut slice_pcm = 0u64;
        for clip in clips {
            if let (Some(index), Some(region)) = (clip.audio, clip.audio_region) {
                let sample = &bundle.media[index];
                slices += 1;
                slice_pcm = slice_pcm
                    .checked_add((region.end - region.start) * u64::from(sample.ch) * 4)
                    .ok_or("Slice size overflow")?;
            }
        }
        lines.push(format!("Original source files: {source_bytes} bytes available; {unavailable} source paths unavailable. Names alone do not classify a source as a recording or render."));
        lines.push(format!("Referenced slices: {slices}; {slice_pcm} logical PCM bytes. They share embedded source data and add no separate PCM files."));
        if FileFingerprint::read(path) != Some(before) {
            return Err("Native archive changed during storage inspection".into());
        }
    }
    if let Some(store) = store {
        let unused = store.review_prune(&[], cancel)?;
        let (audio, audio_files) = folder(&store.root.join("audio"), cancel)?;
        let (revisions, revision_files) = folder(&store.root.join("revisions"), cancel)?;
        let cleanup = if store.root.join(".cleanup").exists() {
            folder(&store.root.join(".cleanup"), cancel)?.0
        } else {
            0
        };
        lines.push(format!("Named versions: {}; shared source PCM containers: {audio} bytes in {audio_files} files; revision metadata: {revisions} bytes in {revision_files} files. All saved versions were inspected before identifying {} unused audio and {} unused revision files.", store.entries().len(), unused.orphaned_audio.len(), unused.orphaned_revisions.len()));
        lines.push(format!("Cleanup recovery: {cleanup} retained bytes. Quarantine changes references and file locations; it does not reclaim those bytes."));
    }
    for (name, path) in [
        ("Original recording folder", &roots.recordings),
        ("Rendered audio folder", &roots.renders),
        ("Shared library waveform cache", &roots.cache),
        ("Project recovery folder", &roots.recovery),
    ] {
        if let Some(path) = path {
            match folder(path, cancel) {
                Ok((bytes, files)) => lines.push(format!(
                    "{name}: {bytes} bytes in {files} files. {}",
                    path.display()
                )),
                Err(e) => {
                    active(cancel)?;
                    lines.push(format!("{name}: unavailable ({e}). {}", path.display()));
                }
            }
        }
    }
    active(cancel)?;
    Ok(lines.join("\n"))
}
