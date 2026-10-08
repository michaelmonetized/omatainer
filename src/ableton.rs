//! Reviewed Live Set migration with retained source data and explicit playback differences.
use crate::{
    engine::{
        self,
        dsp::Sample,
        media_source::{FileFingerprint, LibSource},
        project, session,
    },
    interchange_xml::{self, Element},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
mod clips;
mod convert;
mod dependencies;
mod presets;
pub(crate) mod plugins;
pub(crate) mod process;
pub(crate) mod renders;
mod routing;
#[cfg(test)]
pub(crate) mod tests;
mod timing;

pub(crate) const MAX_XML: usize = 32 * 1024 * 1024;
const MAX_RECORDS: usize = 8;
const MAX_ITEMS: usize = 16384;
const MAX_PCM: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Migration {
    pub schema: u32,
    pub sources: Vec<Source>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub path: PathBuf,
    pub file_sha256: String,
    pub xml_sha256: String,
    pub format: String,
    pub creator: String,
    pub xml: String,
    pub remaps: Vec<(String, PathBuf)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<crate::producer_library::Pack>,
    pub tracks: Vec<Track>,
    pub devices: Vec<Device>,
    pub assets: Vec<Asset>,
    pub differences: Vec<Difference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renders: Vec<renders::Record>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Track {
    pub source_id: i64,
    pub native: session::Reference,
    pub role: String,
    pub parent: i64,
    pub color_index: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Device {
    pub track: i64,
    pub path: String,
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub format: Option<String>,
    pub class_id: Option<String>,
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resolution {
    pub native_id: u64,
    pub binary_sha256: String,
    pub installed_version: String,
    pub state_sha256: String,
    pub fidelity: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Availability {
    Embedded,
    Missing,
    Offline,
    Inaccessible,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Asset {
    pub id: String,
    pub original: String,
    pub relative: String,
    pub pack: String,
    pub source: Option<LibSource>,
    pub availability: Availability,
    pub detail: String,
    pub audio_sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Difference {
    pub item: String,
    pub feature: String,
    pub detail: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    pub remaps: Vec<(String, PathBuf)>,
    #[serde(default)]
    pub libraries: Vec<crate::producer_library::Pack>,
}
#[derive(Clone)]
pub(crate) struct Imported {
    pub state: project::State,
    pub media: Vec<Arc<Sample>>,
    pub reviewed_native: Option<NativeProof>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeProof {
    source: LibSource,
    fingerprint: FileFingerprint,
    sha256: String,
}

fn native_hash(file: &mut std::fs::File, cancel: &AtomicBool) -> Result<String, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let maximum = MAX_PCM
        + crate::project_file::DEFAULT_METADATA_LIMIT as u64
        + crate::project_file::CONTAINER_OVERHEAD;
    if length > maximum {
        return Err("Native migration exceeds its 512 MiB audio / 64 MiB metadata budget".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 16384];
    let mut total = 0;
    loop {
        active(cancel)?;
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > maximum {
            return Err("Native migration grew beyond its file budget".into());
        }
        hash.update(&bytes[..count]);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hash.finalize()))
}

/// Review retained migration data without requiring its original Live installation.
/// Takes a saved native migration and cancellation; returns its verified source archive and editable state after descriptor-bound container/digest checks.
pub(crate) fn load_native(path: &Path, cancel: &AtomicBool) -> Result<Imported, String> {
    active(cancel)?;
    let location = crate::media_location::Snapshot::discover()
        .map_err(|e| e.to_string())?
        .identify(path)
        .map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&location.path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Native migration must be a regular file".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    let sha256 = native_hash(&mut file, cancel)?;
    let mut check = file.try_clone().map_err(|e| e.to_string())?;
    let limits = crate::project_file::Limits {
        max_pcm_bytes: MAX_PCM,
        ..Default::default()
    };
    let bundle =
        crate::project_file::load_from_file::<crate::ui::project::Document>(file, &limits, cancel)
            .map_err(|e| e.to_string())?;
    location
        .verify_file(&check, fingerprint)
        .map_err(|e| e.to_string())?;
    if native_hash(&mut check, cancel)? != sha256 {
        return Err("Saved migration changed during review".into());
    }
    location
        .verify_file(&check, fingerprint)
        .map_err(|e| e.to_string())?;
    bundle.state.validate()?;
    bundle.state.engine.validate(&bundle.media)?;
    if bundle.state.engine.migration.is_none() {
        return Err("This native project contains no retained Ableton migration archive".into());
    }
    Ok(Imported {
        state: bundle.state.engine,
        media: bundle.media,
        reviewed_native: Some(NativeProof {
            source: location.source,
            fingerprint,
            sha256,
        }),
    })
}

/// Recheck the draft's actual reviewed source.
/// Takes a draft and cancellation; checks either its saved native container or original Live Set, keeping archived XML usable when the original Live files are offline.
pub(crate) fn verify_draft(draft: &Imported, cancel: &AtomicBool) -> Result<(), String> {
    if let Some(proof) = &draft.reviewed_native {
        draft
            .state
            .migration
            .as_ref()
            .ok_or("Retained migration archive is absent")?
            .validate(draft.media.len())?;
        let location =
            crate::media_location::Location::resolve(&proof.source).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&location.path)
            .map_err(|e| e.to_string())?;
        location
            .verify_file(&file, proof.fingerprint)
            .map_err(|e| e.to_string())?;
        if native_hash(&mut file, cancel)? != proof.sha256 {
            return Err(
                "Saved migration changed after review; review it again before publication".into(),
            );
        }
        location
            .verify_file(&file, proof.fingerprint)
            .map_err(|e| e.to_string())?;
        Ok(())
    } else {
        verify_sources(&draft.state, cancel)
    }
}

fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Ableton migration cancelled".into())
    } else {
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn text_valid(text: &str) -> bool {
    text.len() <= 4096 && !text.chars().any(char::is_control)
}
impl Migration {
    /// Count retained source metadata for Undo admission.
    /// Takes these immutable records; returns conservative encoded bytes or refuses encoding failure.
    pub(crate) fn bytes(&self) -> Result<usize, String> {
        serde_json::to_vec(self)
            .map(|b| b.len() * 2)
            .map_err(|e| e.to_string())
    }
    /// Validate retained migration records before native publication.
    /// Takes the archived source and native media count; returns an error for corrupt identity, excess storage or invalid references.
    pub(crate) fn validate(&self, _media: usize) -> Result<(), String> {
        if !matches!(self.schema, 1 | 2)
            || self.sources.is_empty()
            || self.sources.len() > MAX_RECORDS
            || self.sources.iter().map(|s| s.xml.len()).sum::<usize>() > MAX_XML
        {
            return Err(
                "Ableton migration source storage exceeds its schema or 32 MiB limit".into(),
            );
        }
        for source in &self.sources {
            if source.libraries.len()>32 || self.schema<2 && !source.libraries.is_empty() {return Err("Pack dependency metadata requires migration schema 2 and at most 32 libraries".into());}
            let mut pack_ids=BTreeSet::new();
            for pack in &source.libraries {if !pack_ids.insert(pack.validate()?) {return Err("Duplicate reviewed Pack identity".into());}}
            if source.xml_sha256 != digest(source.xml.as_bytes())
                || source.file_sha256.len() != 64
                || !source.file_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || !text_valid(&source.format)
                || !text_valid(&source.creator)
                || source
                    .path
                    .to_str()
                    .is_none_or(|s| !text_valid(s) || !source.path.is_absolute())
                || source.remaps.len() > 32
                || source.remaps.iter().any(|(from, to)| {
                    from.is_empty()
                        || !text_valid(from)
                        || !to.is_absolute()
                        || to.as_os_str().len() > 4096
                })
                || source.devices.len()
                    + source.assets.len()
                    + source.differences.len()
                    + source.renders.len()
                    > MAX_ITEMS
                || source.tracks.len() > session::MAX_TRACKS
                || source.renders.len() > 32
            {
                return Err("Invalid retained Ableton source identity or item budget".into());
            }
            let mut tracks = BTreeSet::new();
            for render in &source.renders {
                render.validate()?;
            }
            for track in &source.tracks {
                if !tracks.insert(track.source_id)
                    || track.native.namespace == [0, 0]
                    || track.native.id.0 == 0
                    || !["AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack"]
                        .contains(&track.role.as_str())
                {
                    return Err("Invalid retained Ableton track identity".into());
                }
            }
            let mut assets = BTreeSet::new();
            for asset in &source.assets {
                if !assets.insert(&asset.id)
                    || [
                        &asset.id,
                        &asset.original,
                        &asset.relative,
                        &asset.pack,
                        &asset.detail,
                    ]
                    .into_iter()
                    .any(|s| !text_valid(s))
                    || asset
                        .audio_sha256
                        .as_ref()
                        .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
                    || matches!(asset.availability, Availability::Embedded)
                        != asset.audio_sha256.is_some()
                {
                    return Err("Invalid retained Ableton asset".into());
                }
                if let Some(reference) = &asset.source {
                    crate::media_location::validate_root_source(reference)
                        .map_err(|e| e.to_string())?;
                }
            }
            let mut devices = BTreeSet::new();
            for device in &source.devices {
                if !devices.insert((device.track, &device.path))
                    || [&device.path, &device.kind, &device.name]
                        .into_iter()
                        .any(|s| !text_valid(s))
                    || device
                        .class_id
                        .as_ref()
                        .is_some_and(|s| s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
                    || device.version.as_ref().is_some_and(|s| !text_valid(s))
                    || device.resolution.as_ref().is_some_and(|r| {
                        r.native_id == 0
                            || r.binary_sha256.len() != 64
                            || r.state_sha256.len() != 64
                            || !r
                                .binary_sha256
                                .bytes()
                                .chain(r.state_sha256.bytes())
                                .all(|b| b.is_ascii_hexdigit())
                            || !text_valid(&r.installed_version)
                            || !text_valid(&r.fidelity)
                    })
                {
                    return Err("Invalid retained Ableton device".into());
                }
            }
            if source.differences.iter().any(|d| {
                [&d.item, &d.feature, &d.detail]
                    .into_iter()
                    .any(|s| !text_valid(s))
            }) {
                return Err("Invalid Ableton migration review".into());
            }
        }
        Ok(())
    }
}
impl Source {
    fn difference(
        &mut self,
        item: impl Into<String>,
        feature: &str,
        detail: impl Into<String>,
    ) -> Result<(), String> {
        if self.devices.len() + self.assets.len() + self.differences.len() >= MAX_ITEMS {
            return Err("Ableton migration exceeds 16384 retained items".into());
        }
        self.differences.push(Difference {
            item: item.into(),
            feature: feature.into(),
            detail: detail.into(),
        });
        Ok(())
    }
}
/// Read an owned Live 10 or 11 Set without changing its files.
/// Takes an absolute source path, explicit cross-machine path maps and cancellation; returns a validated native draft and its full retained review.
pub(crate) fn load(
    path: &Path,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    active(cancel)?;
    if !path.is_absolute()
        || options.libraries.len() > 32
        || options.remaps.len() > 32
        || options.remaps.iter().any(|(from, to)| {
            from.is_empty() || !text_valid(from) || !to.is_absolute() || to.as_os_str().len() > 4096
        })
    {
        return Err("Choose an absolute Set path and at most 32 absolute path remaps".into());
    }
    for pack in &options.libraries {pack.root(cancel)?;}
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| format!("Cannot read Live Set: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_XML as u64 {
        return Err("Live Set must be a regular file of at most 32 MiB".into());
    }
    let before = FileFingerprint::from_metadata(&meta);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        active(cancel)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_XML {
            return Err("Live Set exceeds 32 MiB".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let xml = expand(&bytes, cancel)?;
    let root = interchange_xml::parse_text(&xml, &|| !cancel.load(Ordering::Acquire))?;
    if root.name != "Ableton" || root.attr("MajorVersion") != "5" {
        return Err("Unsupported Ableton Set root or saved format".into());
    }
    let version = root.attr("MinorVersion");
    let major = version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    if !matches!(major, Some(10 | 11)) {
        return Err(format!(
            "Saved ALS format {version} is outside the Live 10/11 migration matrix"
        ));
    }
    let source = Source {
        path: path.into(),
        file_sha256: digest(&bytes),
        xml_sha256: digest(&xml),
        format: version.into(),
        creator: root.attr("Creator").into(),
        xml: String::from_utf8(xml).map_err(|_| "Live Set XML must be UTF-8")?,
        remaps: options.remaps.clone(),
        libraries: options.libraries.clone(),
        tracks: vec![],
        devices: vec![],
        assets: vec![],
        differences: vec![],
        renders: vec![],
    };
    let preset = !root.children.iter().any(|n|n.name=="LiveSet");
    if preset && path.extension().is_none_or(|e|e!="adg" && e!="adv") {return Err("Non-Set content must be an explicitly selected .adg or .adv user device preset".into());}
    let root=presets::normalize(root)?;
    let mut source=source;
    if preset {source.difference("preset","Source device preset","Original device/rack state is retained intact in a fresh track; compatible native relink or explicit replacement is required")?;}
    let imported = convert::convert(root.one("LiveSet")?, source, options, cancel)?;
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != before
        || FileFingerprint::read(path) != Some(before)
    {
        return Err("Live Set changed during migration; current project was preserved".into());
    }
    active(cancel)?;
    Ok(imported)
}
fn expand(bytes: &[u8], cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut decoder = flate2::bufread::GzDecoder::new(bytes);
    let mut xml = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        active(cancel)?;
        let n = decoder
            .read(&mut chunk)
            .map_err(|e| format!("Invalid compressed Live Set: {e}"))?;
        if n == 0 {
            break;
        }
        if xml.len() + n > MAX_XML {
            return Err("Expanded Live Set exceeds 32 MiB".into());
        }
        xml.extend_from_slice(&chunk[..n]);
    }
    if !decoder.into_inner().is_empty() {
        return Err("Live Set has trailing data or multiple gzip members".into());
    }
    Ok(xml)
}
fn child<'a>(node: &'a Element, name: &str) -> Result<Option<&'a Element>, String> {
    let mut children = node.children.iter().filter(|c| c.name == name);
    let first = children.next();
    if children.next().is_some() {
        return Err(format!("{} repeats {name}", node.name));
    }
    Ok(first)
}
fn at<'a>(mut node: &'a Element, path: &[&str]) -> Result<Option<&'a Element>, String> {
    for name in path {
        let Some(next) = child(node, name)? else {
            return Ok(None);
        };
        node = next;
    }
    Ok(Some(node))
}
fn value<'a>(node: &'a Element, path: &[&str]) -> Result<&'a str, String> {
    Ok(at(node, path)?.map_or("", |n| n.attr("Value")))
}
fn number(node: &Element, path: &[&str], fallback: f64) -> Result<f64, String> {
    let raw = value(node, path)?;
    let n = if raw.is_empty() {
        fallback
    } else {
        raw.parse()
            .map_err(|_| format!("Invalid {} numeric value", path.join("/")))?
    };
    if !n.is_finite() {
        return Err("Live Set contains nonfinite musical values".into());
    }
    Ok(n)
}
fn boolean(node: &Element, path: &[&str], fallback: bool) -> Result<bool, String> {
    match value(node, path)? {
        "" => Ok(fallback),
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("Invalid {} boolean value", path.join("/"))),
    }
}
fn integer(raw: &str) -> Result<i64, String> {
    raw.parse()
        .map_err(|_| "Live Set has an invalid source identity".into())
}
fn walk<'a>(node: &'a Element, name: &str, out: &mut Vec<&'a Element>) {
    if node.name == name {
        out.push(node);
    }
    for c in &node.children {
        walk(c, name, out);
    }
}

/// Verify the source files again before publishing a reviewed draft.
/// Takes saved migration identities and cancellation; refuses changed, absent or oversized sources without altering the destination.
pub(crate) fn verify_sources(state: &project::State, cancel: &AtomicBool) -> Result<(), String> {
    let migration = state
        .migration
        .as_ref()
        .ok_or("Native draft has no Ableton provenance")?;
    for source in &migration.sources {
        active(cancel)?;
        for pack in &source.libraries {pack.root(cancel)?;}
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&source.path)
            .map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_XML as u64 {
            return Err("Reviewed Live Set is no longer a bounded regular file".into());
        }
        let mut hash = Sha256::new();
        let mut bytes = 0;
        let mut chunk = [0u8; 16384];
        loop {
            active(cancel)?;
            let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            bytes += n;
            if bytes > MAX_XML {
                return Err("Reviewed Live Set changed beyond its limit".into());
            }
            hash.update(&chunk[..n]);
        }
        if format!("{:x}", hash.finalize()) != source.file_sha256 {
            return Err("Live Set changed after review; review it again before publishing".into());
        }
    }
    Ok(())
}
