use crate::{
    engine::media_source::FileFingerprint,
    library::{
        crates::{CrateId, Edit},
        Catalog, Metadata, TrackId,
    },
    media_location::{Access, Location, Snapshot},
    ui::bpm::Bpm,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
#[cfg(test)]
mod dj_tests;
mod djxml;
mod m3u;
mod plist;
mod serato;
#[cfg(test)]
mod tests;

const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_REFERENCES: usize = 4096;
const MAX_PLAYLISTS: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct Input {
    pub path: PathBuf,
    pub mapping: Option<Mapping>,
}
#[derive(Clone, Debug)]
pub(crate) struct Mapping {
    pub from: String,
    pub to: PathBuf,
}
impl Input {
    /// Validate an import request. Takes the playlist path and optional explicit prefix replacement; returns an error without accessing files.
    pub fn validate(&self) -> Result<(), String> {
        let path = |p: &Path| {
            p.is_absolute()
                && p.to_str()
                    .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
        };
        if !path(&self.path) {
            return Err("Choose an absolute UTF-8 playlist path within 4096 bytes".into());
        }
        if self.mapping.as_ref().is_some_and(|m| {
            m.from.is_empty()
                || m.from.len() > 4096
                || m.from.trim() != m.from
                || m.from.chars().any(char::is_control)
                || !path(&m.to)
        }) {
            return Err(
                "Path mapping needs a nonempty source prefix and an absolute local directory"
                    .into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct RawEntry {
    reference: String,
    title: String,
    artist: String,
    blocked: Option<&'static str>,
    details: Arc<crate::library::imports::Details>,
}
#[derive(Clone, Debug)]
struct RawPlaylist {
    name: String,
    entries: Vec<RawEntry>,
    note: String,
    folders: Vec<String>,
    key: String,
    parent: Option<String>,
    folder: bool,
}
#[derive(Clone, Debug)]
struct Media {
    location: Location,
    fingerprint: FileFingerprint,
    metadata: Metadata,
    existing: Option<TrackId>,
    tagged_title: bool,
    tagged_artist: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub title: String,
    pub reference: String,
    pub status: String,
    media: Option<Arc<Media>>,
}
impl Row {
    pub fn ready(&self) -> bool {
        self.media.is_some()
    }
    pub fn resolved(&self) -> Option<&Path> {
        self.media.as_ref().map(|m| m.location.path.as_path())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Playlist {
    pub name: String,
    pub destination: String,
    pub rows: Vec<usize>,
    pub note: String,
    pub folders: Vec<String>,
    key: String,
    parent: Option<String>,
    pub folder: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Review {
    path: PathBuf,
    fingerprint: FileFingerprint,
    location: Location,
    access: Access,
    digest: [u8; 32],
    format: String,
    folder_source: crate::engine::media_source::LibSource,
    pub playlists: Vec<Playlist>,
    pub rows: Vec<Row>,
    details: Vec<Arc<crate::library::imports::Details>>,
}
impl Review {
    /// Read retained conversion warnings. Takes a reviewed row index; returns its source messages without copying shared metadata.
    pub fn warnings(&self, index: usize) -> &[String] {
        &self.details[index].warnings
    }
}

fn active(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Ok(())
    } else {
        Err("Playlist operation cancelled before publication".into())
    }
}
fn name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err("Playlist names must be nonempty, control-free and at most 256 UTF-8 bytes".into())
    } else {
        Ok(value.into())
    }
}
fn label(value: &str) -> Result<String, String> {
    if crate::media_tags::text_valid(value) {
        Ok(value.into())
    } else {
        Err("Playlist labels exceed 4096 bytes or contain control characters".into())
    }
}
fn local_path(
    reference: &str,
    parent: &Path,
    mapping: Option<&Mapping>,
) -> Result<PathBuf, String> {
    if reference.is_empty() || reference.len() > 4096 || reference.chars().any(char::is_control) {
        return Err("Invalid path reference".into());
    }
    let file_uri = reference
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"));
    let mut text = if file_uri {
        let uri = url::Url::parse(reference).map_err(|_| "Invalid file URI")?;
        if uri.query().is_some() || uri.fragment().is_some() {
            return Err("File URI has a query or fragment".into());
        }
        uri.to_file_path()
            .map_err(|_| "Unmapped file URI authority")?
            .to_str()
            .ok_or("File URI path is not UTF-8")?
            .to_owned()
    } else {
        reference.replace('\\', "/")
    };
    if text.as_bytes().get(0) == Some(&b'/') && text.as_bytes().get(2) == Some(&b':') {
        text.remove(0);
    }
    let foreign = text.as_bytes().get(1) == Some(&b':') || text.starts_with("//");
    if !foreign && !file_uri && url::Url::parse(reference).is_ok() {
        return Err("Provider-only or remote URI; local audio is required".into());
    }
    if let Some(mapping) = mapping {
        let prefix = mapping
            .from
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_owned();
        if text == prefix
            || text
                .strip_prefix(&prefix)
                .is_some_and(|tail| tail.starts_with('/'))
        {
            let tail = text.strip_prefix(&prefix).unwrap().trim_start_matches('/');
            if Path::new(tail)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("Mapped path escapes the reviewed prefix".into());
            }
            return Ok(mapping.to.join(tail));
        }
    }
    if foreign {
        return Err(
            "Unmapped Windows drive or network path; add an explicit prefix mapping".into(),
        );
    }
    let path = PathBuf::from(text);
    Ok(if path.is_absolute() {
        path
    } else {
        parent.join(path)
    })
}
struct Cancel<'a>(&'a dyn Fn() -> bool);
impl crate::media_tags::Cancellation for Cancel<'_> {
    fn cancelled(&self) -> bool {
        !(self.0)()
    }
}

/// Review a local playlist on the catalog worker. Takes input, current catalog and cancellation check; returns bounded ordered references with explicit per-entry failures and no catalog changes.
pub(crate) fn review(
    input: &Input,
    catalog: &Catalog,
    check: impl Fn() -> bool,
) -> Result<Review, String> {
    input.validate()?;
    active(&check)?;
    if input.mapping.as_ref().is_some_and(|m| !m.to.is_dir()) {
        return Err("Path mapping destination must be an existing local directory".into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&input.path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("Playlist must be a regular file within 32 MiB".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&meta);
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    let mut chunk = [0u8; 65536];
    loop {
        active(&check)?;
        let count = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_BYTES as usize {
            return Err("Playlist exceeds 32 MiB".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != fingerprint
        || FileFingerprint::read(&input.path) != Some(fingerprint)
    {
        return Err("Playlist changed during review".into());
    }
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let location = snapshot.identify(&input.path).map_err(|e| e.to_string())?;
    location
        .verify_file(&file, fingerprint)
        .map_err(|e| e.to_string())?;
    let access = snapshot.access(&location.path).map_err(|e| e.to_string())?;
    let extension = input
        .path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (format, raw) = if bytes.starts_with(b"vrsn") {
        (
            "Serato legacy crate 1.0".to_owned(),
            serato::parse(&bytes, &input.path, &check)?,
        )
    } else if matches!(extension.as_str(), "m3u" | "m3u8")
        || std::str::from_utf8(&bytes)
            .is_ok_and(|s| s.trim_start_matches('\u{feff}').starts_with("#EXTM3U"))
    {
        (
            "M3U UTF-8".to_owned(),
            m3u::parse(
                &bytes,
                input
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported playlist"),
                &check,
            )?,
        )
    } else {
        let decoded = plist::utf8(&bytes)?;
        let mut reader = quick_xml::Reader::from_str(&decoded);
        let root = loop {
            active(&check)?;
            match reader.read_event().map_err(|e|e.to_string())? {quick_xml::events::Event::Start(s)|quick_xml::events::Event::Empty(s)=>break s.name().as_ref().to_vec(),quick_xml::events::Event::Eof=>return Err("Unrecognized DJ database; export M3U, Apple XML, rekordbox XML or Traktor NML 19".into()),_=>{}}
        };
        if root == b"plist" {
            (
                "Apple XML plist 1.0".to_owned(),
                plist::parse(&bytes, &check)?,
            )
        } else {
            djxml::parse(
                &crate::interchange_xml::parse(decoded.as_bytes(), &check)?,
                &check,
            )?
        }
    };
    if raw.is_empty()
        || raw.len() > MAX_PLAYLISTS
        || raw.iter().map(|p| p.entries.len()).sum::<usize>() > MAX_REFERENCES
    {
        return Err("Review accepts 1–128 playlists and at most 4096 total references; export a smaller selection".into());
    }
    let serato = format.starts_with("Serato");
    let parent = if serato {
        snapshot
            .volume_root(&location)
            .map_err(|e| format!("Serato paths need a visible volume root: {e}"))?
    } else {
        input.path.parent().unwrap().to_path_buf()
    };
    let folder_source = if serato {
        snapshot
            .identify(input.path.parent().unwrap())
            .map_err(|e| e.to_string())?
            .source
    } else {
        location.source.clone()
    };
    let mut cache = HashMap::<String, Result<Arc<Media>, String>>::new();
    let mut rows = Vec::new();
    let mut playlists = Vec::new();
    let mut details = Vec::new();
    for playlist in raw {
        active(&check)?;
        let mut indices = Vec::new();
        let mut seen = HashSet::new();
        for mut entry in playlist.entries {
            active(&check)?;
            if entry.details.title != entry.title || entry.details.artist != entry.artist {
                let mut details = (*entry.details).clone();
                details.title = entry.title.clone();
                details.artist = entry.artist.clone();
                entry.details = Arc::new(details);
            }
            entry.details.validate()?;
            let mapped = input.mapping.as_ref().is_some_and(|mapping| {
                let prefix = mapping.from.replace('\\', "/");
                entry.reference == prefix
                    || entry
                        .reference
                        .strip_prefix(prefix.trim_end_matches('/'))
                        .is_some_and(|s| s.starts_with('/'))
            });
            let blocked = entry.blocked.filter(|reason| {
                !(*reason == "Traktor volume name needs an explicit path prefix mapping" && mapped)
            });
            let result = if let Some(reason) = blocked {
                Err(reason.into())
            } else if let Some(result) = cache.get(&entry.reference) {
                result.clone()
            } else {
                let result = (|| {
                    let path = local_path(&entry.reference, &parent, input.mapping.as_ref())?;
                    let extension = path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if extension == "m4p" {
                        return Err("Protected media; obtain an unprotected local file".into());
                    }
                    if !matches!(
                        extension.as_str(),
                        "wav" | "mp3" | "flac" | "ogg" | "aiff" | "aif" | "m4a" | "aac"
                    ) {
                        return Err("Unsupported local audio format".into());
                    }
                    let location = snapshot
                        .identify(&path)
                        .map_err(|e| format!("Missing/unmapped local file: {e}"))?;
                    if location.path.to_str().is_none() || location.path.as_os_str().len() > 4096 {
                        return Err("Local path exceeds the UTF-8 catalog limit".into());
                    }
                    let meta = snapshot.inspect(&location).map_err(|e| e.to_string())?;
                    if !meta.is_file() {
                        return Err("Reference is not a regular local audio file".into());
                    }
                    let fingerprint = FileFingerprint::from_metadata(&meta);
                    let existing = catalog
                        .track_for_version(&location.source, Some(fingerprint))
                        .or_else(|| catalog.track(&location.source));
                    if existing.is_some_and(|track| {
                        track.versions[track.current].fingerprint != Some(fingerprint)
                    }) {
                        return Err("Catalog version differs; resolve changed media before importing this reference".into());
                    }
                    let observed = crate::media_tags::inspect_cancellable(
                        &location,
                        fingerprint,
                        &Cancel(&check),
                    )
                    .map_err(|e| format!("Audio container could not be inspected: {e}"))?;
                    let filename = location
                        .path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Track");
                    let fields = observed.fields;
                    let tagged_title = fields.title.is_some();
                    let tagged_artist = fields.artist.is_some();
                    let metadata = Metadata {
                        title: label(
                            fields
                                .title
                                .as_ref()
                                .map(|f| f.value.as_str())
                                .unwrap_or(filename),
                        )?,
                        artist: label(
                            fields
                                .artist
                                .as_ref()
                                .map(|f| f.value.as_str())
                                .unwrap_or(""),
                        )?,
                        bpm: fields
                            .bpm
                            .as_ref()
                            .and_then(|f| f.value.parse::<f32>().ok())
                            .map_or(Bpm::UNKNOWN, |n| {
                                Bpm::new(n, crate::ui::bpm::Origin::EmbeddedTag)
                            }),
                        key: fields.key.map(|f| f.value).unwrap_or_default(),
                        duration: None,
                        last_play: None,
                    };
                    Ok(Arc::new(Media {
                        location,
                        fingerprint,
                        metadata,
                        existing: existing.map(|t| t.id.clone()),
                        tagged_title,
                        tagged_artist,
                    }))
                })();
                cache.insert(entry.reference.clone(), result.clone());
                result
            };
            let (media, mut status) = match result {
                Ok(media) => {
                    let duplicate = !seen.insert(media.location.source.clone());
                    let status = if duplicate {
                        "Duplicate reference; first position retained; original order archived"
                    } else if media.existing.is_some() {
                        "Ready: existing catalog track; native metadata/preparation retained"
                    } else {
                        "Ready: new local track"
                    };
                    let mut media = (*media).clone();
                    if media.existing.is_none() {
                        if !media.tagged_title && !entry.title.is_empty() {
                            media.metadata.title = entry.title.clone();
                        }
                        if !media.tagged_artist {
                            media.metadata.artist = entry.artist.clone();
                        }
                        if let Some(bpm) = entry.details.bpm {
                            media.metadata.bpm = Bpm::new(bpm, crate::ui::bpm::Origin::Imported);
                        }
                        if !entry.details.key.is_empty() {
                            media.metadata.key = entry.details.key.clone();
                        }
                    }
                    (Some(Arc::new(media)), status.into())
                }
                Err(error) => (None, error),
            };
            if !entry.details.warnings.is_empty() {
                status.push_str(&format!(
                    " · {} source metadata warnings (details on hover)",
                    entry.details.warnings.len()
                ));
            }
            let title = media
                .as_ref()
                .map(|m| m.metadata.title.clone())
                .unwrap_or(entry.title);
            indices.push(rows.len());
            rows.push(Row {
                title,
                reference: entry.reference,
                status,
                media,
            });
            details.push(entry.details);
        }
        playlists.push(Playlist {
            name: name(&playlist.name)?,
            destination: String::new(),
            rows: indices,
            note: playlist.note,
            folders: playlist.folders,
            key: playlist.key,
            parent: playlist.parent,
            folder: playlist.folder,
        });
    }
    let digest = Sha256::digest(&bytes).into();
    destination_names(
        &mut playlists,
        catalog,
        &location.source,
        &folder_source,
        digest,
        serato,
    )?;
    active(&check)?;
    Ok(Review {
        path: input.path.clone(),
        fingerprint,
        location,
        access,
        digest,
        format,
        folder_source,
        playlists,
        rows,
        details,
    })
}

fn destination_names(
    playlists: &mut [Playlist],
    catalog: &Catalog,
    source: &crate::engine::media_source::LibSource,
    folder_source: &crate::engine::media_source::LibSource,
    digest: [u8; 32],
    serato: bool,
) -> Result<(), String> {
    let mut existing = HashMap::new();
    for playlist in playlists.iter() {
        let source = if playlist.folder {
            folder_source
        } else {
            source
        };
        let digest = if playlist.folder && serato {
            [0; 32]
        } else {
            digest
        };
        if let Some(record) = catalog.imports.iter().find(|r| {
            &r.source == source
                && r.digest == digest
                && r.key == playlist.key
                && catalog.crates.node(&r.crate_id).is_some()
        }) {
            existing.insert(playlist.key.clone(), record.crate_id.clone());
        }
    }
    let mut names: HashMap<Option<String>, HashSet<String>> = HashMap::new();
    for playlist in playlists {
        if let Some(id) = existing.get(&playlist.key) {
            playlist.destination = catalog.crates.node(id).unwrap().name.clone();
            continue;
        }
        let used = names.entry(playlist.parent.clone()).or_insert_with(|| {
            let ids = playlist
                .parent
                .as_ref()
                .and_then(|key| existing.get(key))
                .and_then(|id| catalog.crates.node(id))
                .map(|n| n.children.as_slice())
                .unwrap_or_else(|| {
                    if playlist.parent.is_none() {
                        catalog.crates.roots()
                    } else {
                        &[]
                    }
                });
            ids.iter()
                .filter_map(|id| catalog.crates.node(id))
                .map(|n| n.name.clone())
                .collect()
        });
        let mut destination = playlist.name.clone();
        let mut suffix = 2;
        while used.contains(&destination.clone()) {
            let tag = format!(" (import {suffix})");
            let mut end = (256 - tag.len()).min(playlist.name.len());
            while !playlist.name.is_char_boundary(end) {
                end -= 1;
            }
            destination = format!("{}{}", &playlist.name[..end], tag);
            suffix += 1;
            if suffix > 4098 {
                return Err("Import name collision limit reached".into());
            }
        }
        used.insert(destination.clone());
        playlist.destination = destination;
    }
    Ok(())
}

/// Apply an exact reviewed import to a private catalog candidate.
/// Takes selected playlist indices and cancellation predicate; returns retained/fresh crate IDs, refusing changed-source merges unless explicitly reviewed as a new snapshot.
#[cfg(test)]
pub(crate) fn apply(
    review: &Review,
    selected: &[usize],
    catalog: &mut Catalog,
    check: impl Fn() -> bool,
) -> Result<Vec<CrateId>, String> {
    apply_snapshot(review, selected, catalog, false, check)
}

/// Import reviewed folders, membership and source provenance.
/// Takes the exact review, selections, private catalog and explicit snapshot choice; preserves existing preparation and publishes no filesystem changes itself.
pub(crate) fn apply_snapshot(
    review: &Review,
    selected: &[usize],
    catalog: &mut Catalog,
    new_snapshot: bool,
    check: impl Fn() -> bool,
) -> Result<Vec<CrateId>, String> {
    use crate::library::imports::{Collection, Reference};
    active(&check)?;
    if selected.is_empty()
        || selected.len() > MAX_PLAYLISTS
        || selected.iter().any(|&i| i >= review.playlists.len())
        || selected.iter().copied().collect::<HashSet<_>>().len() != selected.len()
    {
        return Err("Select distinct reviewed playlists".into());
    }
    if FileFingerprint::read(&review.path) != Some(review.fingerprint) {
        return Err("Playlist changed after review; inspect it again".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&review.location.path)
        .map_err(|e| e.to_string())?;
    review
        .location
        .verify_file(&file, review.fingerprint)
        .map_err(|e| e.to_string())?;
    review
        .access
        .check(
            &Snapshot::discover().map_err(|e| e.to_string())?,
            &review.location.path,
        )
        .map_err(|e| e.to_string())?;
    if crate::library::hash_project_source(&review.location.path, review.fingerprint, || check())?
        != review.digest
    {
        return Err("Playlist bytes changed after review".into());
    }
    let mut indices: std::collections::BTreeSet<usize> = selected.iter().copied().collect();
    let keys: HashMap<_, _> = review
        .playlists
        .iter()
        .enumerate()
        .map(|(i, p)| (&p.key, i))
        .collect();
    if keys.len() != review.playlists.len() {
        return Err("Playlist source repeats an identity".into());
    }
    for &i in selected {
        let mut current = &review.playlists[i];
        let mut depth = 0;
        while let Some(parent) = &current.parent {
            let index = *keys
                .get(parent)
                .ok_or("Playlist source is missing a parent folder")?;
            if !review.playlists[index].folder || depth >= 32 {
                return Err("Invalid playlist folder ancestry".into());
            }
            indices.insert(index);
            current = &review.playlists[index];
            depth += 1;
        }
    }
    for &i in &indices {
        let playlist = &review.playlists[i];
        if !playlist.folder
            && !new_snapshot
            && catalog.imports.iter().any(|record| {
                record.source == review.location.source
                    && record.key == playlist.key
                    && record.digest != review.digest
                    && catalog.crates.node(&record.crate_id).is_some()
            })
        {
            return Err("This source changed since its last import. Review it as a new snapshot to retain both versions".into());
        }
    }
    let wanted: HashSet<_> = indices
        .iter()
        .flat_map(|&i| review.playlists[i].rows.iter())
        .filter_map(|&i| review.rows[i].media.as_ref())
        .map(|m| m.location.source.clone())
        .collect();
    if wanted.is_empty() && !indices.iter().any(|&i| review.playlists[i].rows.is_empty()) {
        return Err("Selected playlists contain no resolved local audio".into());
    }
    let mut tracks = HashMap::new();
    for (index, media) in indices
        .iter()
        .flat_map(|&i| review.playlists[i].rows.iter())
        .filter_map(|&i| review.rows[i].media.as_ref().map(|m| (i, m)))
    {
        active(&check)?;
        if !wanted.contains(&media.location.source) || tracks.contains_key(&media.location.source) {
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&media.location.path)
            .map_err(|e| e.to_string())?;
        media
            .location
            .verify_file(&file, media.fingerprint)
            .map_err(|e| format!("Reviewed media changed: {e}"))?;
        let known = catalog
            .track_for_version(&media.location.source, Some(media.fingerprint))
            .or_else(|| catalog.track(&media.location.source));
        match (media.existing.as_ref(), known) {
            (Some(expected), Some(track))
                if &track.id == expected
                    && track.versions[track.current].fingerprint == Some(media.fingerprint) => {}
            (None, None) => {
                let version = catalog.upsert(
                    media.location.source.clone(),
                    Some(media.fingerprint),
                    media.metadata.clone(),
                )?;
                if let Some(preparation) = &review.details[index].preparation {
                    version.preparation = **preparation;
                }
                if let Some(annotations) = &review.details[index].annotations {
                    catalog
                        .tracks
                        .iter_mut()
                        .find(|t| t.source == media.location.source)
                        .unwrap()
                        .annotations = annotations.clone();
                }
            }
            _ => {
                return Err(
                    "Catalog source/version changed after review; inspect the playlist again"
                        .into(),
                )
            }
        }
        let track = catalog
            .track_for_version(&media.location.source, Some(media.fingerprint))
            .unwrap();
        tracks.insert(media.location.source.clone(), track.id.clone());
    }
    let mut ids = HashMap::new();
    let mut imported = Vec::new();
    for i in indices {
        active(&check)?;
        let playlist = &review.playlists[i];
        let source = if playlist.folder {
            &review.folder_source
        } else {
            &review.location.source
        };
        let digest = if playlist.folder && review.format.starts_with("Serato") {
            [0; 32]
        } else {
            review.digest
        };
        if let Some(existing) = catalog.imports.iter().find(|record| {
            &record.source == source
                && record.key == playlist.key
                && record.digest == digest
                && catalog.crates.node(&record.crate_id).is_some()
        }) {
            ids.insert(playlist.key.clone(), existing.crate_id.clone());
            if selected.contains(&i) {
                imported.push(existing.crate_id.clone());
            }
            continue;
        }
        let parent = playlist
            .parent
            .as_ref()
            .map(|key| {
                ids.get(key)
                    .cloned()
                    .ok_or("Playlist parents must precede their children")
            })
            .transpose()?;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| e.to_string())?;
        let id = CrateId(bytes.iter().map(|b| format!("{b:02x}")).collect());
        catalog.edit_crates(
            catalog.crates.revision(),
            &Edit::Create {
                id: id.clone(),
                name: playlist.destination.clone(),
                parent,
                before: None,
            },
        )?;
        let mut seen = HashSet::new();
        let members: Vec<_> = playlist
            .rows
            .iter()
            .filter_map(|&r| review.rows[r].media.as_ref())
            .filter_map(|m| tracks.get(&m.location.source))
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect();
        if !members.is_empty() {
            catalog.edit_crates(
                catalog.crates.revision(),
                &Edit::AddMembers {
                    id: id.clone(),
                    members,
                    before: None,
                },
            )?;
        }
        let references = playlist
            .rows
            .iter()
            .map(|&index| Reference {
                reference: review.rows[index].reference.clone(),
                track: review.rows[index]
                    .media
                    .as_ref()
                    .and_then(|m| tracks.get(&m.location.source))
                    .cloned(),
                details: review.details[index].clone(),
            })
            .collect();
        catalog.imports.retain(|record| {
            !(&record.source == source && record.digest == digest && record.key == playlist.key)
        });
        catalog.imports.push(Collection {
            source: source.clone(),
            digest,
            format: review.format.clone(),
            key: playlist.key.clone(),
            crate_id: id.clone(),
            references,
        });
        ids.insert(playlist.key.clone(), id.clone());
        if selected.contains(&i) {
            imported.push(id);
        }
    }
    crate::library::imports::validate(&catalog.imports, &catalog.tracks)?;
    active(&check)?;
    Ok(imported)
}
