//! Worker-only, journaled tag replacement. The source is never edited in place.
//! A durable intent precedes Linux atomic exchange; the displaced source stays
//! in a private sibling directory until the catalog has durably accepted proof.
use super::*;
use crate::{
    engine::performance::WorkPermit,
    library::tags::{Patch, Review, VerifiedRewrite},
};
use lofty::{
    config::WriteOptions,
    tag::{Tag, TagExt},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs,
    io::Write,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::Arc,
};

// Lofty's pinned writers buffer file bytes. Refuse larger rewrites before that
// allocation; inspection/sidecar edits remain available for larger media.
pub(crate) const MAX_REWRITE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_JOURNALS: usize = 256;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;
const MAX_OUTPUT: u64 = MAX_REWRITE_BYTES + TAG_READ_BYTES;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    device: u64,
    inode: u64,
}
impl Node {
    fn of(m: &fs::Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
        }
    }
    fn matches(&self, m: &fs::Metadata) -> bool {
        self == &Self::of(m)
    }
    /// Only a freshly resolved persistent volume UUID can qualify a changed
    /// kernel device number across remounts. Inodes and measured bytes remain
    /// mandatory; plain File locations retain the complete device guard.
    fn matches_recovered(
        &self,
        m: &fs::Metadata,
        source: &crate::engine::media_source::LibSource,
    ) -> bool {
        self.inode == m.ino()
            && (self.device == m.dev()
                || matches!(
                    source,
                    crate::engine::media_source::LibSource::Removable { .. }
                ))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    schema: u32,
    id: String,
    pub review: Review,
    pub patch: Patch,
    pub old_hash: [u8; 32],
    pub new_hash: [u8; 32],
    pub original_audio: payload::Identity,
    pub staged_audio: payload::Identity,
    root: PathBuf,
    root_node: Node,
    parent_node: Node,
    original_node: Node,
    stage_node: Node,
    stage_dir_node: Node,
    pub notices: Vec<String>,
}
impl Record {
    pub fn journal_path(&self) -> PathBuf {
        self.root.join(format!("{}.json", self.id))
    }
    fn stage_name(&self) -> String {
        format!(".omatainer-tags-{}", self.id)
    }
    fn stage_path(&self, location: &Location) -> Result<PathBuf, String> {
        Ok(location
            .path
            .parent()
            .ok_or("media parent missing")?
            .join(self.stage_name())
            .join("media"))
    }
    fn valid(&self) -> Result<(), String> {
        if self.schema != 1
            || self.id.len() != 32
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !self.root.is_absolute()
            || !self.original_audio.valid()
            || self.original_audio != self.staged_audio
            || self.notices.len() > 16
            || self.notices.iter().any(|s| s.len() > 1024)
        {
            return Err("invalid tag recovery record".into());
        }
        self.patch.validate()?;
        Ok(())
    }
}
#[derive(Debug)]
pub(crate) struct Applied {
    pub record: Record,
    pub observation: Observation,
    pub proof: VerifiedRewrite,
    pub notices: Vec<String>,
}
#[derive(Debug)]
pub(crate) struct Failure {
    pub message: String,
    /// False when recovery inventory was not safely checked. No sidecar
    /// fallback is allowed even if no valid record could be decoded.
    pub fallback_safe: bool,
    /// A durable intent may exist. Do not fall back or repeat this source until
    /// recovery has classified it; the media may already have been installed.
    pub record: Option<Record>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
#[derive(Debug)]
pub(crate) enum Recovery {
    Applied(Applied),
    Unchanged(Record),
    Conflict {
        record: Option<Record>,
        message: String,
    },
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn open(path: &Path, write: bool) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(err)
}
fn directory(path: &Path, private: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(err)?;
    let meta = file.metadata().map_err(err)?;
    if !meta.is_dir()
        || private && (meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0)
    {
        return Err("tag recovery directory must be owned and private".into());
    }
    if path.canonicalize().map_err(err)? != path {
        return Err("tag transaction directories must have canonical paths".into());
    }
    Ok(file)
}
fn root_directory(path: &Path) -> Result<File, String> {
    if !path.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(err)?;
        File::open(path.parent().ok_or("recovery parent missing")?)
            .and_then(|f| f.sync_all())
            .map_err(err)?;
    }
    directory(path, true)
}
fn check_node(path: &Path, node: &Node, directory_expected: bool) -> Result<(), String> {
    let meta = path.symlink_metadata().map_err(err)?;
    if !node.matches(&meta) || meta.file_type().is_symlink() || meta.is_dir() != directory_expected
    {
        return Err("tag transaction path changed; all bytes retained".into());
    }
    Ok(())
}
fn hash(file: &mut File, cancel: &AtomicBool) -> Result<[u8; 32], String> {
    let before = file.metadata().map_err(err)?;
    if !before.is_file() || before.len() > MAX_OUTPUT {
        return Err("tag rewrite file exceeds bounded size".into());
    }
    let fp = FileFingerprint::from_metadata(&before);
    file.rewind().map_err(err)?;
    let mut left = before.len();
    let mut buf = [0u8; 64 * 1024];
    let mut digest = Sha256::new();
    while left > 0 {
        active(cancel)?;
        let want = left.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want]).map_err(err)?;
        if n == 0 {
            return Err("tag source truncated during hash".into());
        }
        digest.update(&buf[..n]);
        left -= n as u64;
    }
    if FileFingerprint::from_metadata(&file.metadata().map_err(err)?) != fp {
        return Err("tag source changed during hash".into());
    }
    Ok(digest.finalize().into())
}
fn copy(source: &mut File, target: &mut File, cancel: &AtomicBool) -> Result<(), String> {
    let before = source.metadata().map_err(err)?;
    let fp = FileFingerprint::from_metadata(&before);
    source.rewind().map_err(err)?;
    let mut left = before.len();
    let mut buf = [0u8; 64 * 1024];
    while left > 0 {
        active(cancel)?;
        let want = left.min(buf.len() as u64) as usize;
        let n = source.read(&mut buf[..want]).map_err(err)?;
        if n == 0 {
            return Err("tag source truncated during copy".into());
        }
        target.write_all(&buf[..n]).map_err(err)?;
        left -= n as u64;
    }
    if FileFingerprint::from_metadata(&source.metadata().map_err(err)?) != fp {
        return Err("tag source changed during copy".into());
    }
    Ok(())
}
/// Limits Lofty's buffered rewrite by file size, growth, total I/O, operations,
/// and cancellation. A failure affects only the private disposable candidate.
struct Bounded<'a> {
    file: &'a mut File,
    cancel: &'a AtomicBool,
    bytes: u64,
    ops: u64,
}
impl Bounded<'_> {
    fn check(&mut self, n: usize) -> io::Result<()> {
        active(self.cancel).map_err(io::Error::other)?;
        self.ops = self
            .ops
            .checked_sub(1)
            .ok_or_else(|| io::Error::other("tag write operation limit"))?;
        self.bytes = self
            .bytes
            .checked_sub(n as u64)
            .ok_or_else(|| io::Error::other("tag write I/O limit"))?;
        Ok(())
    }
}
impl Read for Bounded<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.check(b.len())?;
        self.file.read(b)
    }
}
impl Write for Bounded<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.check(b.len())?;
        if self.file.stream_position()?.saturating_add(b.len() as u64) > MAX_OUTPUT {
            return Err(io::Error::other("tag write output limit"));
        }
        self.file.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check(0)?;
        self.file.flush()
    }
}
impl Seek for Bounded<'_> {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.check(0)?;
        let target = match p {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.file.stream_position()? as i128 + n as i128,
            SeekFrom::End(n) => self.file.metadata()?.len() as i128 + n as i128,
        };
        if target < 0 || target > MAX_OUTPUT as i128 {
            return Err(io::Error::other("tag write seek limit"));
        }
        self.file.seek(SeekFrom::Start(target as u64))
    }
}
impl lofty::io::Truncate for Bounded<'_> {
    type Error = io::Error;
    fn truncate(&mut self, n: u64) -> io::Result<()> {
        self.check(0)?;
        if n > MAX_OUTPUT {
            return Err(io::Error::other("tag write truncate limit"));
        }
        self.file.set_len(n)
    }
}
impl lofty::io::Length for Bounded<'_> {
    type Error = io::Error;
    fn len(&self) -> io::Result<u64> {
        self.file.metadata().map(|m| m.len())
    }
}
fn patch_tag(
    parsed: &lofty::file::TaggedFile,
    patch: &Patch,
) -> Result<(Tag, super::preservation::ChangedFields, Vec<String>), String> {
    let kind = match parsed.file_type() {
        FileType::Mpeg | FileType::Wav | FileType::Aiff => TagType::Id3v2,
        FileType::Flac => TagType::VorbisComments,
        _ => return Err("this format supports sidecar edits only".into()),
    };
    let mut tag = parsed.tag(kind).cloned().unwrap_or_else(|| Tag::new(kind));
    let mut changed = super::preservation::ChangedFields {
        title: patch.title.is_some(),
        artist: patch.artist.is_some(),
        bpm: patch.bpm.is_some(),
        key: patch.key.is_some(),
    };
    let mut notices = Vec::new();
    let bpm=patch.bpm.as_ref().filter(|value|{
        if kind==TagType::Id3v2 && !value.is_empty() && value.parse::<f64>().is_ok_and(|n|n.fract()!=0.0) {changed.bpm=false;notices.push("Precise fractional BPM remains in the library sidecar; ID3 TBPM only supports whole numbers.".into());false}else{true}
    });
    for (key, value) in [
        (ItemKey::TrackTitle, patch.title.as_ref()),
        (ItemKey::TrackArtist, patch.artist.as_ref()),
        (ItemKey::InitialKey, patch.key.as_ref()),
    ] {
        if let Some(value) = value {
            tag.remove_key(key);
            if !value.is_empty() && !tag.insert_text(key, value.clone()) {
                return Err("tag field cannot be represented without conversion".into());
            }
        }
    }
    if let Some(value) = bpm {
        tag.remove_key(ItemKey::Bpm);
        tag.remove_key(ItemKey::IntegerBpm);
        if !value.is_empty() {
            let key = if kind == TagType::Id3v2 {
                ItemKey::IntegerBpm
            } else {
                ItemKey::Bpm
            };
            let value = if kind == TagType::Id3v2 {
                format!("{:.0}", value.parse::<f64>().map_err(err)?)
            } else {
                value.clone()
            };
            if !tag.insert_text(key, value) {
                return Err("BPM cannot be represented by this tag".into());
            }
        }
    }
    Ok((tag, changed, notices))
}
/// Generic Tag conversion discards unknown Vorbis comments in pinned Lofty.
/// Keep the concrete map, vendor, and picture information for FLAC rewrites.
fn write_flac(
    original: &mut File,
    candidate: &mut File,
    patch: &Patch,
    cancel: &AtomicBool,
    options: WriteOptions,
) -> Result<(), String> {
    use lofty::{
        file::AudioFile,
        flac::FlacFile,
        ogg::{OggPictureStorage, VorbisComments},
    };
    original.rewind().map_err(err)?;
    let length = original.metadata().map_err(err)?.len();
    let parsed = FlacFile::read_from(
        &mut Budget {
            file: original,
            cancel,
            length,
            bytes: TAG_READ_BYTES,
            operations: TAG_OPERATIONS,
        },
        parse_options(true),
    )
    .map_err(err)?;
    let previous = parsed.vorbis_comments().cloned().unwrap_or_default();
    if previous.items().len() > 4096 || parsed.pictures().len() > 4096 {
        return Err("FLAC metadata exceeds 4096 preserved fields/pictures".into());
    }
    let mut comments = VorbisComments::new();
    comments.set_vendor(previous.vendor().to_owned());
    for (key, value) in previous.items() {
        let edited = key.eq_ignore_ascii_case("TITLE") && patch.title.is_some()
            || key.eq_ignore_ascii_case("ARTIST") && patch.artist.is_some()
            || key.eq_ignore_ascii_case("BPM") && patch.bpm.is_some()
            || (key.eq_ignore_ascii_case("INITIALKEY") || key.eq_ignore_ascii_case("KEY"))
                && patch.key.is_some();
        if !edited {
            comments.push(key.to_owned(), value.to_owned());
        }
    }
    for (picture, information) in parsed.pictures() {
        comments
            .insert_picture(picture.clone(), Some(*information))
            .map_err(err)?;
    }
    for (key, value) in [
        ("TITLE", &patch.title),
        ("ARTIST", &patch.artist),
        ("BPM", &patch.bpm),
        ("INITIALKEY", &patch.key),
    ] {
        if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
            comments.push(key.into(), value.clone());
        }
    }
    candidate.rewind().map_err(err)?;
    comments
        .save_to(
            &mut Bounded {
                file: candidate,
                cancel,
                bytes: MAX_OUTPUT * 8,
                ops: 4_000_000,
            },
            options,
        )
        .map_err(|e| format!("candidate FLAC tag write refused: {e}"))
}

fn journal_names(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(err)? {
        let entry = entry.map_err(err)?;
        if paths.len() >= MAX_JOURNALS {
            return Err("tag recovery exceeds 256 entries; existing records retained".into());
        }
        let path = entry.path();
        if path.extension().is_none_or(|v| v != "json") {
            return Err("unrecognized file in private tag recovery directory; retained".into());
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}
fn read_record(root: &Path, path: &Path) -> Result<Record, String> {
    let mut f = open(path, false)?;
    let m = f.metadata().map_err(err)?;
    if !m.is_file()
        || m.len() > MAX_JOURNAL_BYTES
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err("tag recovery record has unsafe size or permissions".into());
    }
    let fp = FileFingerprint::from_metadata(&m);
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if FileFingerprint::from_metadata(&f.metadata().map_err(err)?) != fp
        || FileFingerprint::read(path) != Some(fp)
    {
        return Err("tag recovery record changed while reading".into());
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(err)?;
    record.valid()?;
    if record.root != root || record.journal_path() != path {
        return Err("tag recovery record path mismatch".into());
    }
    Ok(record)
}
fn durable_record(record: &Record) -> Result<(), String> {
    record.valid()?;
    let bytes = serde_json::to_vec(record).map_err(err)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("tag recovery record too large".into());
    }
    check_node(&record.root, &record.root_node, true)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(record.journal_path())
        .map_err(err)?;
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(err)?;
    directory(&record.root, true)?.sync_all().map_err(err)
}
fn exchange(parent: &File, leaf: &std::ffi::OsStr, stage: &File) -> Result<(), String> {
    let leaf = CString::new(leaf.as_bytes()).map_err(err)?;
    let media = c"media";
    let result = unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            stage.as_raw_fd(),
            media.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result != 0 {
        return Err(format!(
            "atomic tag exchange unavailable: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// All expensive validation precedes the short commit admission. A failure
/// with a record means recovery must determine actual disk state before retry.
pub(crate) fn apply(
    location: &Location,
    review: &Review,
    patch: &Patch,
    recovery_root: &Path,
    work: &WorkPermit,
) -> Result<Applied, Failure> {
    apply_with(location, review, patch, recovery_root, work, |_| Ok(()))
}
fn apply_with(
    location: &Location,
    review: &Review,
    patch: &Patch,
    recovery_root: &Path,
    work: &WorkPermit,
    mut checkpoint: impl FnMut(u8) -> Result<(), String>,
) -> Result<Applied, Failure> {
    let cancel = work.cancel();
    let mut record = None;
    let mut fallback_safe = false;
    let mut owned_stage: Option<(PathBuf, Node, Option<Node>)> = None;
    let attempt = (|| -> Result<Applied, String> {
        active(&cancel)?;
        patch.validate()?;
        if patch.is_empty() {
            return Err("no tag fields selected".into());
        }
        if location.source != review.source {
            return Err("tag review source differs from resolved media".into());
        }
        let root = root_directory(recovery_root)?;
        for path in journal_names(recovery_root)? {
            let pending = read_record(recovery_root, &path)?;
            if pending.review.source == review.source {
                record = Some(pending);
                return Err(
                    "this source has an unresolved tag transaction; recover it before another edit"
                        .into(),
                );
            }
        }
        fallback_safe = true;
        let snapshot = Snapshot::discover().map_err(err)?;
        let access = snapshot.access(&location.path).map_err(err)?;
        let parent_path = location.path.parent().ok_or("media parent missing")?;
        let parent = directory(parent_path, false)?;
        let mut original = open(&location.path, true)?;
        location
            .verify_file(&original, review.fingerprint)
            .map_err(err)?;
        let before = original.metadata().map_err(err)?;
        if before.len() > MAX_REWRITE_BYTES {
            return Err(
                "embedded tag rewrite exceeds 128 MiB memory bound; use library sidecar values"
                    .into(),
            );
        }
        if before.nlink() != 1 {
            return Err("hard-linked media requires sidecar values; rewriting one link would split its identity".into());
        }
        if before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o222 == 0
            || before.mode() & 0o7000 != 0
        {
            return Err(
                "media ownership or read-only/special permissions require sidecar values".into(),
            );
        }
        // Replacing an inode must not silently remove ACLs, capabilities, or
        // application xattrs. Until these have explicit preservation support,
        // the safe alternative is a sidecar edit.
        let attributes = unsafe { libc::flistxattr(original.as_raw_fd(), std::ptr::null_mut(), 0) };
        if attributes > 0 {
            return Err("media has extended attributes; preserve them with a sidecar edit".into());
        }
        if attributes < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ENOTSUP) {
            return Err(format!(
                "cannot inspect media attributes: {}",
                io::Error::last_os_error()
            ));
        }
        let parsed = read_tagged(&mut original, &cancel, true)?;
        let format = parsed.file_type();
        let (tag, changed, notices) = patch_tag(&parsed, patch)?;
        if !changed.title && !changed.artist && !changed.bpm && !changed.key {
            return Err(notices
                .first()
                .cloned()
                .unwrap_or_else(|| "no embedded fields selected".into()));
        }
        let old_hash = hash(&mut original, &cancel)?;
        let original_audio = payload::measure(original.try_clone().map_err(err)?, cancel.clone())?;
        let id = crate::sampler_bank::BankId::new()?.to_string();
        let stage_path = parent_path.join(format!(".omatainer-tags-{id}"));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&stage_path)
            .map_err(err)?;
        let stage = directory(&stage_path, true)?;
        let stage_dir_node = Node::of(&stage.metadata().map_err(err)?);
        owned_stage = Some((stage_path.clone(), stage_dir_node.clone(), None));
        let media_path = stage_path.join("media");
        let mut candidate = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&media_path)
            .map_err(err)?;
        let stage_node = Node::of(&candidate.metadata().map_err(err)?);
        owned_stage.as_mut().unwrap().2 = Some(stage_node.clone());
        copy(&mut original, &mut candidate, &cancel)?;
        candidate.rewind().map_err(err)?;
        let version = super::preservation::id3_version(&mut original, format, &cancel)?;
        let options = WriteOptions::new()
            .lossy_text_encoding(false)
            .use_id3v23(version == Some(3));
        if format == FileType::Flac {
            write_flac(&mut original, &mut candidate, patch, &cancel, options)?;
        } else {
            tag.save_to(
                &mut Bounded {
                    file: &mut candidate,
                    cancel: &cancel,
                    bytes: MAX_OUTPUT * 8,
                    ops: 4_000_000,
                },
                options,
            )
            .map_err(|e| format!("candidate tag write refused: {e}"))?;
        }
        let staged = read_tagged(&mut candidate, &cancel, true)?;
        let (observed, _) = fields(&staged);
        // Check actual written values. Read-only tag frames or format encoding
        // cannot turn a successful library call into a false write claim.
        for (wanted, actual, name) in [
            (&patch.title, &observed.title, "title"),
            (&patch.artist, &observed.artist, "artist"),
            (&patch.key, &observed.key, "key"),
        ] {
            if let Some(wanted) = wanted {
                if actual.as_ref().map(|v| v.value.as_str()).unwrap_or("") != wanted {
                    return Err(format!(
                        "candidate {name} does not match the reviewed value"
                    ));
                }
            }
        }
        if changed.bpm {
            if let Some(wanted) = &patch.bpm {
                let actual = observed
                    .bpm
                    .as_ref()
                    .map(|v| v.value.as_str())
                    .unwrap_or("");
                let matches = if wanted.is_empty() {
                    actual.is_empty()
                } else {
                    wanted.parse::<f64>().ok() == actual.parse::<f64>().ok()
                };
                if !matches {
                    return Err("candidate BPM does not match the reviewed value".into());
                }
            }
        }
        super::preservation::verify(&mut original, &mut candidate, format, changed, &cancel)?;
        let new_hash = hash(&mut candidate, &cancel)?;
        let staged_audio = payload::measure(candidate.try_clone().map_err(err)?, cancel.clone())?;
        if original_audio != staged_audio {
            return Err(
                "candidate changed audio packets or playback parameters; original preserved".into(),
            );
        }
        // Preserve ownership, group and ordinary mode; the old descriptor is
        // still open, and no source bytes are ever written.
        if unsafe { libc::fchown(candidate.as_raw_fd(), before.uid(), before.gid()) } != 0 {
            return Err(format!(
                "cannot preserve media ownership: {}",
                io::Error::last_os_error()
            ));
        }
        candidate
            .set_permissions(fs::Permissions::from_mode(before.mode() & 0o777))
            .map_err(err)?;
        candidate.sync_all().map_err(err)?;
        original.sync_all().map_err(err)?;
        stage.sync_all().map_err(err)?;
        parent.sync_all().map_err(err)?;
        location
            .verify_file(&original, review.fingerprint)
            .map_err(err)?;
        access
            .check(&Snapshot::discover().map_err(err)?, &location.path)
            .map_err(err)?;
        check_node(
            parent_path,
            &Node::of(&parent.metadata().map_err(err)?),
            true,
        )?;
        let pending = Record {
            schema: 1,
            id,
            review: review.clone(),
            patch: patch.clone(),
            old_hash,
            new_hash,
            original_audio,
            staged_audio,
            root: recovery_root.into(),
            root_node: Node::of(&root.metadata().map_err(err)?),
            parent_node: Node::of(&parent.metadata().map_err(err)?),
            original_node: Node::of(&before),
            stage_node,
            stage_dir_node,
            notices,
        };
        // Set the conservative outcome before attempting journal durability:
        // a failed fsync can leave a valid durable record despite its error.
        record = Some(pending.clone());
        durable_record(&pending)?;
        checkpoint(0)?;
        active(&cancel)?;
        location
            .verify_file(&original, review.fingerprint)
            .map_err(err)?;
        check_node(&media_path, &pending.stage_node, false)?;
        let claim = work.commit().map_err(err)?;
        checkpoint(2)?;
        exchange(
            &parent,
            location.path.file_name().ok_or("media filename missing")?,
            &stage,
        )?;
        // Cancellation after this point cannot turn an installed file into an
        // untouched/sidecar result. The journal is already durable.
        let sync = parent
            .sync_all()
            .and_then(|_| stage.sync_all())
            .map_err(err);
        drop(claim);
        sync?;
        checkpoint(1)?;
        applied(&pending)
    })();
    match attempt {
        Ok(applied) => Ok(applied),
        Err(message) => {
            if record.is_none() {
                if let Some((path, dir_node, file_node)) = owned_stage {
                    if check_node(&path, &dir_node, true).is_ok() {
                        if let Some(node) = file_node {
                            let file = path.join("media");
                            if check_node(&file, &node, false).is_ok() {
                                let _ = fs::remove_file(file);
                            }
                        }
                        let _ = fs::remove_dir(path);
                    }
                }
            }
            Err(Failure {
                message,
                fallback_safe: fallback_safe && record.is_none(),
                record,
            })
        }
    }
}
fn checked_paths(record: &Record) -> Result<(Location, File, File, File), String> {
    record.valid()?;
    let root = directory(&record.root, true)?;
    if !record.root_node.matches(&root.metadata().map_err(err)?) {
        return Err("tag recovery directory identity changed".into());
    }
    let disk = read_record(&record.root, &record.journal_path())?;
    if serde_json::to_vec(&disk).map_err(err)? != serde_json::to_vec(record).map_err(err)? {
        return Err("tag recovery record changed externally".into());
    }
    let location = Location::resolve(&record.review.source).map_err(err)?;
    let parent = directory(location.path.parent().ok_or("media parent missing")?, false)?;
    if !record
        .parent_node
        .matches_recovered(&parent.metadata().map_err(err)?, &location.source)
    {
        return Err("media parent changed; retained tag transaction requires review".into());
    }
    let stage_path = record.stage_path(&location)?;
    let stage = directory(stage_path.parent().unwrap(), true)?;
    if !record
        .stage_dir_node
        .matches_recovered(&stage.metadata().map_err(err)?, &location.source)
    {
        return Err("private tag staging directory changed".into());
    }
    Ok((location, root, parent, stage))
}
fn applied(record: &Record) -> Result<Applied, String> {
    let (location, _root, _parent, _stage) = checked_paths(record)?;
    let snapshot = Snapshot::discover().map_err(err)?;
    let access = snapshot.access(&location.path).map_err(err)?;
    let mut installed = open(&location.path, false)?;
    let mut displaced = open(&record.stage_path(&location)?, false)?;
    let no_cancel = Arc::new(AtomicBool::new(false));
    let installed_metadata = installed.metadata().map_err(err)?;
    let displaced_metadata = displaced.metadata().map_err(err)?;
    if !record
        .stage_node
        .matches_recovered(&installed_metadata, &location.source)
        || !record
            .original_node
            .matches_recovered(&displaced_metadata, &location.source)
        || installed_metadata.nlink() != 1
        || displaced_metadata.nlink() != 1
    {
        return Err(
            "tag exchange paths contain unexpected files; both retained for recovery".into(),
        );
    }
    if hash(&mut installed, &no_cancel)? != record.new_hash
        || hash(&mut displaced, &no_cancel)? != record.old_hash
    {
        return Err("tag exchange content changed; installed and displaced bytes retained".into());
    }
    let original_audio = payload::measure(displaced.try_clone().map_err(err)?, no_cancel.clone())?;
    let staged_audio = payload::measure(installed.try_clone().map_err(err)?, no_cancel.clone())?;
    if original_audio != record.original_audio
        || staged_audio != record.staged_audio
        || original_audio != staged_audio
    {
        return Err("tag recovery audio proof differs; all files retained".into());
    }
    if FileFingerprint::from_metadata(&installed.metadata().map_err(err)?)
        != FileFingerprint::from_metadata(&installed_metadata)
        || FileFingerprint::from_metadata(&displaced.metadata().map_err(err)?)
            != FileFingerprint::from_metadata(&displaced_metadata)
        || FileFingerprint::read(&record.stage_path(&location)?)
            != Some(FileFingerprint::from_metadata(&displaced_metadata))
    {
        return Err("tag recovery files changed during verification; all retained".into());
    }
    let new_fingerprint = FileFingerprint::from_metadata(&installed_metadata);
    location
        .verify_file(&installed, new_fingerprint)
        .map_err(err)?;
    access
        .check(&Snapshot::discover().map_err(err)?, &location.path)
        .map_err(err)?;
    let observation = inspect(&location, new_fingerprint, &no_cancel)?;
    Ok(Applied {
        record: record.clone(),
        observation,
        proof: VerifiedRewrite {
            old_hash: record.old_hash,
            new_hash: record.new_hash,
            new_fingerprint,
            original_audio,
            staged_audio,
        },
        notices: record.notices.clone(),
    })
}
/// Reconstruct intent by measured current media and retained originals. Recovery
/// never rewrites a source path. The caller must hold the library Store lock.
pub(crate) fn recover(root: &Path) -> Vec<Recovery> {
    if !root.exists() {
        return Vec::new();
    }
    let paths = match directory(root, true).and_then(|_| journal_names(root)) {
        Ok(v) => v,
        Err(message) => {
            return vec![Recovery::Conflict {
                record: None,
                message,
            }]
        }
    };
    paths
        .into_iter()
        .map(|path| {
            let record = match read_record(root, &path) {
                Ok(v) => v,
                Err(message) => {
                    return Recovery::Conflict {
                        record: None,
                        message,
                    }
                }
            };
            match classify(&record) {
                Ok(true) => match applied(&record) {
                    Ok(a) => Recovery::Applied(a),
                    Err(message) => Recovery::Conflict {
                        record: Some(record),
                        message,
                    },
                },
                Ok(false) => Recovery::Unchanged(record),
                Err(message) => Recovery::Conflict {
                    record: Some(record),
                    message,
                },
            }
        })
        .collect()
}
fn classify(record: &Record) -> Result<bool, String> {
    let (location, _, _, _) = checked_paths(record)?;
    let mut current = open(&location.path, false)?;
    let mut spare = open(&record.stage_path(&location)?, false)?;
    let cm = current.metadata().map_err(err)?;
    let sm = spare.metadata().map_err(err)?;
    let access = Snapshot::discover()
        .map_err(err)?
        .access(&location.path)
        .map_err(err)?;
    let cancel = AtomicBool::new(false);
    if record
        .original_node
        .matches_recovered(&cm, &location.source)
        && record.stage_node.matches_recovered(&sm, &location.source)
        && hash(&mut current, &cancel)? == record.old_hash
        && hash(&mut spare, &cancel)? == record.new_hash
    {
        location
            .verify_file(&current, FileFingerprint::from_metadata(&cm))
            .map_err(err)?;
        if FileFingerprint::from_metadata(&spare.metadata().map_err(err)?)
            != FileFingerprint::from_metadata(&sm)
            || FileFingerprint::read(&record.stage_path(&location)?)
                != Some(FileFingerprint::from_metadata(&sm))
        {
            return Err("staged tag candidate changed during recovery; retained".into());
        }
        access
            .check(&Snapshot::discover().map_err(err)?, &location.path)
            .map_err(err)?;
        return Ok(false);
    }
    if record.stage_node.matches_recovered(&cm, &location.source)
        && record
            .original_node
            .matches_recovered(&sm, &location.source)
    {
        return Ok(true);
    }
    Err("tag recovery conflicts with external media changes; all paths retained".into())
}
/// Retire an unused candidate after a cancelled/pre-exchange intent. This is
/// only valid for measured Unchanged recovery; it never removes source media.
pub(crate) fn discard_staged(record: &Record) -> Result<(), String> {
    if classify(record)? {
        return Err("media was installed; save its catalog proof before retiring recovery".into());
    }
    retire(record, false)
}
/// Call only after the catalog save is known committed and durably synced.
/// Re-verify the retained original before deleting it. If media changed after
/// commit, preserve the journal/original and report a conflict instead.
pub(crate) fn finalize(record: &Record) -> Result<(), String> {
    applied(record)?;
    retire(record, true)
}
fn retire(record: &Record, installed: bool) -> Result<(), String> {
    let (location, root, parent, stage) = checked_paths(record)?;
    let stage_path = record.stage_path(&location)?;
    let retained = stage_path.symlink_metadata().map_err(err)?;
    let expected = if installed {
        &record.original_node
    } else {
        &record.stage_node
    };
    if !retained.is_file() || !expected.matches_recovered(&retained, &location.source) {
        return Err("retained tag transaction file changed; preserved".into());
    }
    // Catalog acceptance (or proof of no exchange) already makes the recovery
    // intent unnecessary. Remove it durably first: a crash may leave a private
    // orphan backup, but can never turn a partial cleanup into false recovery.
    fs::remove_file(record.journal_path()).map_err(err)?;
    root.sync_all().map_err(err)?;
    fs::remove_file(&stage_path).map_err(|e| {
        format!(
            "journal retired; private backup retained at {}: {e}",
            stage_path.display()
        )
    })?;
    stage.sync_all().map_err(err)?;
    fs::remove_dir(stage_path.parent().unwrap()).map_err(err)?;
    parent.sync_all().map_err(err)
}

#[cfg(test)]
mod tests;
