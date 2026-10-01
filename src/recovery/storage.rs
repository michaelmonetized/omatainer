use super::*;
use crate::{
    engine::dsp::Sample,
    project_file::{self, Bundle, Limits, Overwrite},
};
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Weak,
};
use std::time::Instant;

static NEXT: AtomicU64 = AtomicU64::new(0);
const ASSET_LIMIT: u64 = project_file::DEFAULT_PCM_LIMIT + RECORD_LIMIT as u64 + 64;
type FileIdentity = (u64, u64, u64, i64, i64, i64, i64);
struct Asset {
    sample: Weak<Sample>,
    id: String,
    identity: FileIdentity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    AssetWritten,
    RecordHalfWritten,
    BeforeSync,
    BeforePublish,
    AfterPublish,
    DirectorySync,
    BeforePrune,
}

pub struct Store {
    root: PathBuf,
    path: PathBuf,
    session: String,
    _lock: files::Lock,
    epoch: Option<u64>,
    retired: bool,
    sequence: u64,
    previous: [u8; 32],
    segment: Option<String>,
    segment_identity: Option<FileIdentity>,
    segment_bytes: u64,
    segment_records: u64,
    last_checkpoint: Instant,
    assets: HashMap<usize, Asset>,
    poisoned: bool,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, Error> {
        files::private_dir(root, true)?;
        let _root_lock = files::root_lock(root)?;
        if files::entries(root, SESSIONS_LIMIT + 1, &AtomicBool::new(false))?
            .iter()
            .filter(|path| path.file_name().is_none_or(|name| name != "storage.lock"))
            .count()
            >= SESSIONS_LIMIT
        {
            return Err(Error::invalid("128 recovery sessions already exist; review and discard old sessions before starting another"));
        }
        let session = format!(
            "session-{:016x}-{:08x}-{:016x}",
            now(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(&session);
        files::private_dir(&path, true)?;
        files::private_dir(&path.join("assets"), true)?;
        let lock = files::lock(&path.join("owner.lock"), true)?
            .ok_or_else(|| Error::invalid("new recovery session is unexpectedly locked"))?;
        files::sync(&path)?;
        files::sync(root)?;
        Ok(Self {
            root: root.into(),
            path,
            session,
            _lock: lock,
            epoch: None,
            retired: false,
            sequence: 0,
            previous: [0; 32],
            segment: None,
            segment_identity: None,
            segment_bytes: 0,
            segment_records: 0,
            last_checkpoint: Instant::now(),
            assets: HashMap::new(),
            poisoned: false,
        })
    }
    /// Opaque application-owned session identity; never a project/media path.
    pub fn session_id(&self) -> &str { &self.session }
    /// Only irreversible lifecycle or an uncertain tail requires a new session.
    /// Invalid documents/configuration remain errors the caller must correct.
    pub fn needs_new_session(&self) -> bool {
        self.poisoned || self.retired
    }
    pub fn append<T: Serialize>(
        &mut self,
        bundle: &Bundle<T>,
        metadata: RecordMeta,
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<Commit, Error> {
        #[cfg(test)]
        let root = self.root.clone();
        self.append_with(bundle, metadata, config, cancel, |phase| {
            #[cfg(test)]
            if phase == Phase::RecordHalfWritten {
                return super::testing::write_check(&root);
            }
            let _ = phase;
            Ok(())
        })
    }
    fn append_with<T: Serialize>(
        &mut self,
        bundle: &Bundle<T>,
        metadata: RecordMeta,
        config: &Config,
        cancel: &AtomicBool,
        mut hook: impl FnMut(Phase) -> std::io::Result<()>,
    ) -> Result<Commit, Error> {
        check(cancel)?;
        let _root_lock = files::root_lock(&self.root)?;
        config.validate().map_err(Error::invalid)?;
        metadata.validate()?;
        if self.poisoned || self.retired || self.epoch.is_some_and(|epoch| epoch != metadata.epoch)
        {
            return Err(Error::invalid("session is retired, belongs to another epoch, or has an uncertain journal tail; open a new session"));
        }
        if bundle.media.len() > project_file::DEFAULT_MEDIA_LIMIT {
            return Err(Error::invalid("too many recovery media assets"));
        }
        let total_pcm = bundle
            .media
            .iter()
            .try_fold(0u64, |total, sample| {
                total.checked_add(sample.data.len() as u64 * 4)
            })
            .filter(|bytes| *bytes <= project_file::DEFAULT_PCM_LIMIT)
            .ok_or_else(|| Error::invalid("recovery media exceeds native project PCM limit"))?;
        let _ = total_pcm;
        let state_bytes = journal::json(&bundle.state)?;
        let state =
            serde_json::from_slice(&state_bytes).map_err(|e| Error::invalid(e.to_string()))?;
        let reserve = (state_bytes.len() + bundle.media.len() * 68 + 4096 * 6 + 1024) as u64;
        self.reserve(reserve, config, cancel)?;
        let mut ids = Vec::with_capacity(bundle.media.len());
        self.assets
            .retain(|_, asset| asset.sample.strong_count() != 0);
        let mut metadata_bytes = state_bytes.len() as u64;
        for sample in &bundle.media {
            ids.push(self.asset(sample, reserve, config, cancel, &mut hook)?);
            let asset = &self.assets[&(Arc::as_ptr(sample) as usize)];
            let encoded_metadata = asset
                .identity
                .2
                .checked_sub(sample.data.len() as u64 * 4 + project_file::CONTAINER_OVERHEAD)
                .ok_or_else(|| Error::invalid("invalid recovery sidecar size"))?;
            metadata_bytes = metadata_bytes.checked_add(encoded_metadata)
                .filter(|bytes| *bytes <= project_file::DEFAULT_METADATA_LIMIT as u64)
                .ok_or_else(|| Error::invalid("combined recovery metadata exceeds native project limit; prior checkpoint preserved"))?;
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| Error::invalid("journal sequence exhausted"))?;
        let mut checkpoint = self.segment.is_none()
            || self.last_checkpoint.elapsed().as_secs() >= config.checkpoint_seconds as u64
            || self.segment_records >= RECORDS_LIMIT;
        let mut body = journal::Body {
            schema: 1,
            checkpoint,
            metadata,
            media: ids,
            state,
        };
        let (mut bytes, mut digest) = journal::frame(sequence, self.previous, &body)?;
        if self.segment_bytes + bytes.len() as u64 > SEGMENT_LIMIT {
            checkpoint = true;
            body.checkpoint = true;
            (bytes, digest) = journal::frame(sequence, self.previous, &body)?;
        }
        self.reserve(bytes.len() as u64, config, cancel)?;
        let before_usage = files::usage(&self.root, cancel)?;
        let name = if checkpoint {
            format!("{sequence:020}.journal")
        } else {
            self.segment.clone().unwrap()
        };
        let final_path = self.path.join(&name);
        let temporary = checkpoint
            .then(|| files::Temporary(self.path.join(format!("pending-{sequence:020}.journal"))));
        let mut file = files::open(
            temporary.as_ref().map_or(&final_path, |t| &t.0),
            true,
            checkpoint,
        )?;
        let original = if checkpoint {
            0
        } else {
            let identity = files::identity(
                &file
                    .metadata()
                    .map_err(|e| Error::io("inspect journal", e))?,
            );
            if Some(identity) != self.segment_identity {
                self.poisoned = true;
                return Err(Error::invalid(
                    "active journal changed outside its owner; open a new session",
                ));
            }
            file.seek(SeekFrom::End(0))
                .map_err(|e| Error::io("seek journal", e))?
        };
        let result = (|| {
            let half = bytes.len() / 2;
            files::write(&mut file, &bytes[..half], cancel)?;
            hook(Phase::RecordHalfWritten).map_err(|e| Error::io("write journal", e))?;
            files::write(&mut file, &bytes[half..], cancel)?;
            hook(Phase::BeforeSync).map_err(|e| Error::io("sync journal", e))?;
            file.sync_all().map_err(|e| Error::io("sync journal", e))?;
            check(cancel)?;
            hook(Phase::BeforePublish).map_err(|e| Error::io("prepare journal commit", e))?;
            check(cancel)?;
            if let Some(temporary) = &temporary {
                fs::hard_link(&temporary.0, &final_path)
                    .map_err(|e| Error::io("publish checkpoint", e))?;
            }
            Ok::<_, Error>(())
        })();
        if let Err(error) = result {
            if !checkpoint
                && file
                    .set_len(original)
                    .and_then(|_| file.sync_all())
                    .is_err()
            {
                self.poisoned = true;
            }
            // Successful rollback changes ctime even though bytes are restored.
            if !checkpoint && !self.poisoned {
                self.segment_identity = file.metadata().ok().map(|meta| files::identity(&meta));
                self.poisoned = self.segment_identity.is_none();
            }
            return Err(error);
        }
        // All subsequent outcomes are committed. Never turn a post-commit
        // cancellation or directory-sync warning into a retry-as-failure.
        self.segment_identity = file.metadata().ok().map(|meta| files::identity(&meta));
        self.poisoned = self.segment_identity.is_none();
        drop(file);
        drop(temporary);
        // Unlinking the temporary hard link updates ctime on the published inode.
        if checkpoint {
            self.segment_identity = fs::symlink_metadata(&final_path)
                .ok()
                .map(|meta| files::identity(&meta));
            self.poisoned = self.segment_identity.is_none();
        }
        let sync = hook(Phase::AfterPublish)
            .and_then(|_| hook(Phase::DirectorySync))
            .map_err(|e| Error::io("post-commit directory sync", e))
            .and_then(|_| files::sync(&self.path));
        self.epoch = Some(body.metadata.epoch);
        self.sequence = sequence;
        self.previous = digest;
        self.segment = Some(name);
        self.segment_bytes = if checkpoint {
            bytes.len() as u64
        } else {
            original + bytes.len() as u64
        };
        self.segment_records = if checkpoint {
            1
        } else {
            self.segment_records + 1
        };
        if checkpoint {
            self.last_checkpoint = Instant::now();
        }
        let durable = sync.is_ok();
        let mut warning = sync
            .err()
            .map(|e| format!("Record committed, directory durability unconfirmed: {e}"));
        if self.poisoned {
            warning = Some("Record committed; journal identity unavailable. A new session is required for subsequent writes".into());
        }
        if checkpoint && durable && !cancel.load(Ordering::Acquire) {
            let prune = hook(Phase::BeforePrune)
                .map_err(|e| Error::io("checkpoint cleanup", e))
                .and_then(|_| self.prune(config, cancel));
            if let Err(error) = prune {
                warning = Some(format!(
                    "Record is durable; older-generation cleanup deferred: {error}"
                ));
            }
        }
        let usage_bytes = match files::usage(&self.root, &AtomicBool::new(false)) {
            Ok(bytes) => bytes,
            Err(error) => {
                warning=Some(format!("Record committed; current storage usage unavailable (reported value is an upper bound): {error}"));
                before_usage.saturating_add(bytes.len() as u64)
            }
        };
        Ok(Commit {
            durable,
            committed_unix_ms: now(),
            sequence,
            usage_bytes,
            warning,
        })
    }
    fn reserve(&self, extra: u64, config: &Config, cancel: &AtomicBool) -> Result<(), Error> {
        let used = files::usage(&self.root, cancel)?;
        if used
            .checked_add(extra)
            .is_none_or(|bytes| bytes > config.max_bytes)
        {
            return Err(Error::invalid(format!("recovery storage cap reached ({used} bytes retained, {extra} bytes reserved, limit {}); previous checkpoints preserved",config.max_bytes)));
        }
        Ok(())
    }
    fn asset(
        &mut self,
        sample: &Arc<Sample>,
        reserve: u64,
        config: &Config,
        cancel: &AtomicBool,
        hook: &mut impl FnMut(Phase) -> std::io::Result<()>,
    ) -> Result<String, Error> {
        check(cancel)?;
        let key = Arc::as_ptr(sample) as usize;
        if let Some(asset) = self.assets.get(&key).filter(|asset| {
            asset
                .sample
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, sample))
        }) {
            let file = match files::open(
                &self.path.join("assets").join(format!("{}.omat", asset.id)),
                false,
                false,
            ) {
                Ok(file) => file,
                Err(error) => {
                    self.poisoned = true;
                    return Err(error);
                }
            };
            if files::identity(
                &file
                    .metadata()
                    .map_err(|e| Error::io("inspect cached media", e))?,
            ) != asset.identity
            {
                self.poisoned = true;
                return Err(Error::invalid(
                    "cached recovery media changed; open a new session before writing again",
                ));
            }
            return Ok(asset.id.clone());
        }
        let estimate = (sample.data.len() as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add((sample.peaks.len() as u64) * 36))
            .and_then(|n| {
                n.checked_add((sample.name.len() as u64 + sample.path.len() as u64) * 6 + 1024)
            })
            .ok_or_else(|| Error::invalid("media reservation overflow"))?;
        self.reserve(
            estimate
                .checked_add(reserve)
                .ok_or_else(|| Error::invalid("reservation overflow"))?,
            config,
            cancel,
        )?;
        let temporary = files::Temporary(self.path.join("assets").join(format!(
            "pending-{}.omat",
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        project_file::save(
            &temporary.0,
            &Bundle {
                state: (),
                media: vec![sample.clone()],
            },
            Overwrite::Never,
            &Limits::default(),
            cancel,
        )?;
        hook(Phase::AssetWritten).map_err(|e| Error::io("prepare recovery media", e))?;
        let id = files::hex(&files::digest(&temporary.0, ASSET_LIMIT, cancel)?);
        let target = self.path.join("assets").join(format!("{id}.omat"));
        match fs::hard_link(&temporary.0, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if files::hex(&files::digest(&target, ASSET_LIMIT, cancel)?) != id {
                    return Err(Error::invalid(
                        "existing content-addressed media is corrupt",
                    ));
                }
            }
            Err(error) => return Err(Error::io("publish recovery media", error)),
        }
        drop(temporary);
        files::sync(&self.path.join("assets"))?;
        let file = files::open(&target, false, false)?;
        let identity = files::identity(
            &file
                .metadata()
                .map_err(|e| Error::io("inspect published media", e))?,
        );
        self.assets.insert(
            key,
            Asset {
                sample: Arc::downgrade(sample),
                id: id.clone(),
                identity,
            },
        );
        Ok(id)
    }
    fn prune(&mut self, config: &Config, cancel: &AtomicBool) -> Result<(), Error> {
        let segments = segments(&self.path, cancel)?;
        let cutoff = segments.len().saturating_sub(config.retention as usize);
        let mut retained = HashSet::new();
        let mut verified = HashSet::new();
        let mut hash_budget = MAX_STORAGE_BYTES;
        // Verify the generations and sidecars that will remain before removing
        // any older fallback. An externally damaged middle checkpoint must not
        // turn a new successful write into destruction of the last good prior one.
        for path in &segments[cutoff..] {
            let read = journal::read(path, cancel)?;
            if read.warning.is_some() || read.records.is_empty() {
                return Err(Error::invalid(
                    "retained journal has an uncertain tail; media cleanup deferred",
                ));
            }
            for record in read.records {
                verify_assets(
                    &self.path,
                    &record.body.media,
                    cancel,
                    &mut verified,
                    &mut hash_budget,
                )?;
                retained.extend(record.body.media);
            }
        }
        for path in &segments[..cutoff] {
            check(cancel)?;
            fs::remove_file(path).map_err(|e| Error::io("remove old checkpoint generation", e))?;
        }
        files::sync(&self.path)?;
        for path in files::entries(&self.path.join("assets"), FILES_LIMIT, cancel)? {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if files::valid_hash(name) && !retained.contains(name) {
                files::open(&path, false, false)?;
                fs::remove_file(&path)
                    .map_err(|e| Error::io("remove unreferenced recovery media", e))?;
            }
        }
        self.assets.retain(|_, asset| retained.contains(&asset.id));
        files::sync(&self.path.join("assets"))
    }
    pub fn retire_epoch(
        &mut self,
        epoch: u64,
        cancel: &AtomicBool,
    ) -> Result<Option<String>, Error> {
        check(cancel)?;
        let _root_lock = files::root_lock(&self.root)?;
        if self.epoch.is_some_and(|owned| owned != epoch) {
            return Err(Error::invalid("cannot retire a different recovery epoch"));
        }
        if self.retired && !self.path.exists() {
            let pending = self.root.join(format!("retired-{}", self.session));
            let result = if pending.exists() {
                cleanup_session(&self.root, &pending)
            } else {
                files::sync(&self.root)
            };
            return Ok(result
                .err()
                .map(|e| format!("Retirement committed; cleanup deferred: {e}")));
        }
        let warning = if self.retired {
            files::sync(&self.path).err().map(|e| e.to_string())
        } else {
            retire_marker(&self.path, epoch, self.sequence, cancel)?
        };
        self.retired = true;
        self.epoch = Some(epoch);
        if warning.is_some() {
            return Ok(warning);
        }
        Ok(cleanup_session(&self.root, &self.path)
            .err()
            .map(|e| format!("Retirement committed; cleanup deferred: {e}")))
    }
}
fn segments(path: &Path, cancel: &AtomicBool) -> Result<Vec<PathBuf>, Error> {
    let mut segments = Vec::new();
    for entry in files::entries(path, MAX_RETENTION as usize + 32, cancel)? {
        let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.len() == 28
            && name.ends_with(".journal")
            && name.as_bytes()[..20].iter().all(|b| b.is_ascii_digit())
        {
            segments.push(entry);
        }
    }
    Ok(segments)
}

fn valid_session(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 50
        && bytes.starts_with(b"session-")
        && bytes[24] == b'-'
        && bytes[33] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| {
            i < 8 || i == 24 || i == 33 || b.is_ascii_digit() || (b'a'..=b'f').contains(b)
        })
}
fn session_lock(root: &Path, session: &str) -> Result<Option<(PathBuf, files::Lock)>, Error> {
    files::private_dir(root, false)?;
    if !valid_session(session) {
        return Err(Error::invalid("invalid recovery session identity"));
    }
    let path = root.join(session);
    files::private_dir(&path, false)?;
    Ok(files::lock(&path.join("owner.lock"), false)?.map(|lock| (path, lock)))
}
fn retired(path: &Path, cancel: &AtomicBool) -> Result<bool, Error> {
    let marker = path.join("retired.json");
    match fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io("inspect retired session", error)),
        Ok(_) => {
            use std::io::Read;
            check(cancel)?;
            let file = files::open(&marker, false, false)?;
            if file
                .metadata()
                .map_err(|e| Error::io("inspect retirement marker", e))?
                .len()
                > 1024
            {
                return Err(Error::invalid("oversized retirement marker"));
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Retired {
                schema: u32,
                epoch: u64,
                sequence: u64,
            }
            let mut bytes = Vec::new();
            file.take(1025)
                .read_to_end(&mut bytes)
                .map_err(|e| Error::io("read retirement marker", e))?;
            let value: Retired = serde_json::from_slice(&bytes)
                .map_err(|e| Error::invalid(format!("invalid retirement marker: {e}")))?;
            if value.schema != 1 {
                return Err(Error::invalid("unknown retirement marker version"));
            }
            let _ = (value.epoch, value.sequence);
            Ok(true)
        }
    }
}
fn verify_assets(
    path: &Path,
    ids: &[String],
    cancel: &AtomicBool,
    verified: &mut HashSet<String>,
    remaining_bytes: &mut u64,
) -> Result<(), Error> {
    // Every lookup/restore revalidates the intermediate directory. O_NOFOLLOW
    // on each final sidecar alone cannot reject an assets-directory symlink.
    files::private_dir(&path.join("assets"), false)?;
    for id in ids {
        check(cancel)?;
        if verified.contains(id) {
            continue;
        }
        let asset = path.join("assets").join(format!("{id}.omat"));
        let mut file = files::open(&asset, false, false)?;
        let size = file
            .metadata()
            .map_err(|e| Error::io("inspect verification input", e))?
            .len();
        // Charge an attempt before hashing, including malformed sidecars and
        // one byte needed to detect growth. Shared callers cannot multiply the
        // per-file limit by every session/record in a private corrupted store.
        *remaining_bytes = remaining_bytes.checked_sub(size.saturating_add(1))
            .filter(|_| size <= ASSET_LIMIT)
            .ok_or_else(|| Error::invalid("aggregate recovery verification byte budget exhausted; remaining data preserved"))?;
        if files::hex(&files::digest_file(&mut file, size, cancel)?) != *id {
            return Err(Error::invalid(format!(
                "media sidecar {id} has a SHA256 mismatch"
            )));
        }
        verified.insert(id.clone());
    }
    Ok(())
}
/// Full SHA256 avoids exporting the local session name, timestamp or PID.
pub fn session_digest(session:&str)->[u8;32] {
    use sha2::{Digest,Sha256};
    Sha256::digest(session.as_bytes()).into()
}
/// Optional worker-only lookup of one previously confirmed durable record.
/// Unlike discovery this never substitutes a newer sequence. Missing/pruned
/// records return None; corrupt/locked/ambiguous inputs return an explicit error.
pub fn lookup_exact(root:&Path,digest:[u8;32],epoch:u64,sequence:u64,cancel:&AtomicBool)->Result<Option<Candidate>,Error> {
    check(cancel)?;
    match fs::symlink_metadata(root) {
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(Error::io("inspect recovery root",error)),Ok(_)=>{},
    }
    files::private_dir(root,false)?;
    let mut session=None;
    for path in files::entries(root,SESSIONS_LIMIT+1,cancel)? {
        let Some(name)=path.file_name().and_then(|n|n.to_str()).filter(|n|valid_session(n)) else {continue};
        if session_digest(name)==digest {
            if session.is_some(){return Err(Error::invalid("ambiguous recovery session digest; no candidate selected"));}
            session=Some(name.to_owned());
        }
    }
    let Some(session)=session else {return Ok(None)};
    let (path,_lock)=session_lock(root,&session)?.ok_or_else(||Error::invalid("referenced recovery session is still active"))?;
    if retired(&path,cancel)? {return Ok(None);}
    let mut candidate=None;let mut warnings=Vec::new();
    let mut budget=project_file::DEFAULT_PCM_LIMIT + project_file::DEFAULT_METADATA_LIMIT as u64
        + (project_file::CONTAINER_OVERHEAD+1)*project_file::DEFAULT_MEDIA_LIMIT as u64;
    for segment in segments(&path,cancel)? {
        let read=journal::read(&segment,cancel)?;
        if let Some(warning)=read.warning {report(&mut warnings,warning);}
        for record in read.records {
            if record.sequence!=sequence || record.body.metadata.epoch!=epoch {continue;}
            if candidate.is_some(){return Err(Error::invalid("ambiguous recovery record; no candidate selected"));}
            verify_assets(&path,&record.body.media,cancel,&mut HashSet::new(),&mut budget)?;
            candidate=Some(Candidate {session:session.clone(),sequence,metadata:record.body.metadata,
                root:root.into(),segment:segment.file_name().unwrap().to_str().unwrap().into(),digest:record.digest,report:Vec::new()});
        }
    }
    if let Some(candidate)=&mut candidate {candidate.report=warnings;}
    check(cancel)?;Ok(candidate)
}
pub fn discover(root: &Path, cancel: &AtomicBool) -> Result<Inventory, Error> {
    discover_with_budget(root, cancel, MAX_STORAGE_BYTES)
}
fn discover_with_budget(
    root: &Path,
    cancel: &AtomicBool,
    mut hash_budget: u64,
) -> Result<Inventory, Error> {
    check(cancel)?;
    if !root.exists() {
        return Ok(Inventory::default());
    }
    files::private_dir(root, false)?;
    let mut result = Inventory::default();
    match files::usage(root, cancel) {
        Ok(bytes) => result.usage_bytes = bytes,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(error) => report(&mut result.warnings, error.to_string()),
    }
    for path in files::entries(root, SESSIONS_LIMIT + 1, cancel)? {
        let session = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if session == "storage.lock" {
            continue;
        }
        if session.strip_prefix("retired-").is_some_and(valid_session) {
            let cleanup = (|| {
                let _root_lock = files::root_lock(root)?;
                cleanup_session(root, &path)
            })();
            if let Err(error) = cleanup {
                report(
                    &mut result.warnings,
                    format!("Retired session cleanup deferred: {error}"),
                );
            }
            continue;
        }
        if !valid_session(session) {
            report(
                &mut result.warnings,
                "Unrecognized recovery storage entry preserved; it was not opened or removed",
            );
            continue;
        }
        let Some((path, _lock)) = (match session_lock(root, session) {
            Ok(value) => value,
            Err(error) => {
                report(&mut result.warnings, format!("{session}: {error}"));
                continue;
            }
        }) else {
            continue;
        };
        match retired(&path, cancel) {
            Ok(true) => {
                let cleanup = (|| {
                    let _root_lock = files::root_lock(root)?;
                    files::sync(&path)?;
                    cleanup_session(root, &path)
                })();
                if let Err(error) = cleanup {
                    report(
                        &mut result.warnings,
                        format!("Retired {session}: cleanup deferred: {error}"),
                    );
                }
                continue;
            }
            Ok(false) => {}
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => report(
                &mut result.warnings,
                format!("{session}: {error}; recovery records preserved"),
            ),
        }
        let mut warnings = Vec::new();
        if let Err(error) = files::private_dir(&path.join("assets"), false) {
            report(&mut result.warnings, format!("{session}: {error}"));
            continue;
        }
        let session_entries = match files::entries(&path, MAX_RETENTION as usize + 32, cancel) {
            Ok(entries) => entries,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => {
                report(&mut result.warnings, format!("{session}: {error}"));
                continue;
            }
        };
        if session_entries.iter().any(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("pending-"))
        }) {
            report(&mut warnings,"Interrupted uncommitted staging data was ignored; prior committed checkpoints retained");
        }
        let mut chosen = None;
        let mut verified = HashSet::new();
        let mut journals = match segments(&path, cancel) {
            Ok(paths) => paths,
            Err(error) => {
                report(&mut result.warnings, error.to_string());
                continue;
            }
        };
        journals.reverse();
        for journal in journals {
            check(cancel)?;
            let read = match journal::read(&journal, cancel) {
                Ok(read) => read,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    report(&mut warnings, error.to_string());
                    continue;
                }
            };
            if let Some(warning) = read.warning {
                report(&mut warnings, warning);
            }
            for record in read.records.into_iter().rev() {
                match verify_assets(
                    &path,
                    &record.body.media,
                    cancel,
                    &mut verified,
                    &mut hash_budget,
                ) {
                    Ok(()) => {
                        chosen = Some(Candidate {
                            root: root.into(),
                            session: session.into(),
                            segment: journal.file_name().unwrap().to_str().unwrap().into(),
                            sequence: record.sequence,
                            metadata: record.body.metadata,
                            digest: record.digest,
                            report: warnings.clone(),
                        });
                        break;
                    }
                    Err(Error::Cancelled) => return Err(Error::Cancelled),
                    Err(error) => report(
                        &mut warnings,
                        format!("Record {} unavailable: {error}", record.sequence),
                    ),
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        if let Some(candidate) = chosen {
            result.candidates.push(candidate);
        } else {
            report(
                &mut warnings,
                "No usable committed recovery record in this session",
            );
        }
        for warning in warnings {
            report(&mut result.warnings, format!("{session}: {warning}"));
        }
    }
    if let Ok(bytes) = files::usage(root, cancel) {
        result.usage_bytes = bytes;
    }
    check(cancel)?;
    result
        .candidates
        .sort_by_key(|candidate| std::cmp::Reverse(candidate.metadata.captured_unix_ms));
    Ok(result)
}
fn exact_record(
    candidate: &Candidate,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<journal::Record, Error> {
    if retired(path, cancel)? {
        return Err(Error::invalid(
            "this session was deliberately retired after recovery discovery",
        ));
    }
    let read = journal::read(&path.join(&candidate.segment), cancel)?;
    let record = read
        .records
        .into_iter()
        .find(|record| {
            record.sequence == candidate.sequence
                && record.digest == candidate.digest
                && record.body.metadata == candidate.metadata
        })
        .ok_or_else(|| {
            Error::invalid("recovery candidate changed or disappeared; discover it again")
        })?;
    let mut hash_budget = project_file::DEFAULT_PCM_LIMIT
        + project_file::DEFAULT_METADATA_LIMIT as u64
        + (project_file::CONTAINER_OVERHEAD + 1) * project_file::DEFAULT_MEDIA_LIMIT as u64;
    verify_assets(
        path,
        &record.body.media,
        cancel,
        &mut HashSet::new(),
        &mut hash_budget,
    )?;
    Ok(record)
}
pub fn recover<T: DeserializeOwned>(
    candidate: &Candidate,
    cancel: &AtomicBool,
) -> Result<Recovered<T>, Error> {
    recover_with_limits(candidate, cancel, Limits::default())
}
fn recover_with_limits<T: DeserializeOwned>(
    candidate: &Candidate,
    cancel: &AtomicBool,
    mut remaining: Limits,
) -> Result<Recovered<T>, Error> {
    check(cancel)?;
    let (path, _lock) = session_lock(&candidate.root, &candidate.session)?
        .ok_or_else(|| Error::invalid("recovery session is active in another process"))?;
    let record = exact_record(candidate, &path, cancel)?;
    if record.body.media.len() > remaining.max_media {
        return Err(Error::invalid(
            "combined recovery asset count exceeds native project limit",
        ));
    }
    remaining.max_metadata_bytes = remaining
        .max_metadata_bytes
        .checked_sub(journal::json(&record.body.state)?.len())
        .ok_or_else(|| Error::invalid("combined recovery metadata exceeds native project limit"))?;
    let mut media = Vec::with_capacity(record.body.media.len());
    let mut report_rows = candidate.report.clone();
    for id in record.body.media {
        check(cancel)?;
        let asset_path = path.join("assets").join(format!("{id}.omat"));
        let mut file = files::open(&asset_path, false, false)?;
        let identity = files::identity(
            &file
                .metadata()
                .map_err(|e| Error::io("inspect media before decoding", e))?,
        );
        let before = files::digest_file(&mut file, ASSET_LIMIT, cancel)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| Error::io("rewind verified media", e))?;
        let (asset, metadata_bytes): (Bundle<()>, _) = project_file::load_from_file_measured(
            file.try_clone()
                .map_err(|e| Error::io("retain verified media inode", e))?,
            &remaining,
            cancel,
        )?;
        if asset.media.len() != 1
            || before != files::digest_file(&mut file, ASSET_LIMIT, cancel)?
            || files::hex(&before) != id
            || identity
                != files::identity(
                    &file
                        .metadata()
                        .map_err(|e| Error::io("recheck decoded media", e))?,
                )
        {
            return Err(Error::invalid(
                "media sidecar changed while decoding or has invalid shape",
            ));
        }
        let sample = asset.media.into_iter().next().unwrap();
        // The codec rejects each header against the remaining budgets before
        // allocating its metadata/PCM, including repeated references to one ID.
        remaining.max_pcm_bytes -= sample.data.len() as u64 * 4;
        remaining.max_metadata_bytes -= metadata_bytes;
        if Path::new(&sample.path).is_absolute() && !Path::new(&sample.path).is_file() {
            report(&mut report_rows,format!("Original external media is unavailable: {}; verified embedded recovery media was restored",sample.path));
        }
        media.push(sample);
    }
    check(cancel)?;
    let state = serde_json::from_value(record.body.state)
        .map_err(|e| Error::invalid(format!("recovery state validation: {e}")))?;
    Ok(Recovered {
        bundle: Bundle { state, media },
        metadata: record.body.metadata,
        report: report_rows,
    })
}
/// Retire only an explicitly selected, still-identical inactive session.
/// The durable marker commits the decision before bounded cleanup; subsequent
/// discovery can finish cleanup without revisiting an explicit project save.
pub fn discard(candidate: &Candidate, cancel: &AtomicBool) -> Result<Option<String>, Error> {
    check(cancel)?;
    let (path, _lock) = session_lock(&candidate.root, &candidate.session)?
        .ok_or_else(|| Error::invalid("cannot discard an active recovery session"))?;
    exact_record(candidate, &path, cancel)?;
    let _root_lock = files::root_lock(&candidate.root)?;
    let warning = retire_marker(&path, candidate.metadata.epoch, candidate.sequence, cancel)?;
    if warning.is_some() {
        return Ok(warning);
    }
    Ok(cleanup_session(&candidate.root, &path)
        .err()
        .map(|e| format!("Recovery session deletion committed; cleanup deferred: {e}")))
}
fn retire_marker(
    path: &Path,
    epoch: u64,
    sequence: u64,
    cancel: &AtomicBool,
) -> Result<Option<String>, Error> {
    let bytes = journal::json(&serde_json::json!({"schema":1,"epoch":epoch,"sequence":sequence}))?;
    let temporary = files::Temporary(path.join("pending-retired.json"));
    let mut file = files::open(&temporary.0, true, true)?;
    files::write(&mut file, &bytes, cancel)?;
    file.sync_all()
        .map_err(|e| Error::io("sync retirement marker", e))?;
    check(cancel)?;
    fs::hard_link(&temporary.0, path.join("retired.json"))
        .map_err(|e| Error::io("publish retirement marker", e))?;
    drop(file);
    drop(temporary);
    Ok(files::sync(path)
        .err()
        .map(|e| format!("Retirement marker committed; directory durability unconfirmed: {e}")))
}
// Caller holds the root mutation lock throughout this operation.
fn cleanup_session(root: &Path, path: &Path) -> Result<(), Error> {
    let never = AtomicBool::new(false);
    files::private_dir(path, false)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::invalid("invalid cleanup session path"))?;
    let already_renamed = name.strip_prefix("retired-").is_some_and(valid_session);
    let entries = files::entries(path, MAX_RETENTION as usize + 32, &never)?;
    if !retired(path, &never)? {
        // A kill may occur after removing the retirement marker, but only the
        // lock or empty directory can remain at that late cleanup boundary.
        if !already_renamed
            || entries
                .iter()
                .any(|entry| entry.file_name().is_none_or(|name| name != "owner.lock"))
        {
            return Err(Error::invalid(
                "cleanup intent is missing or malformed; unknown data preserved",
            ));
        }
    }
    let asset_dir = path.join("assets");
    let assets = if asset_dir.exists() {
        files::private_dir(&asset_dir, false)?;
        files::entries(&asset_dir, FILES_LIMIT, &never)?
    } else {
        Vec::new()
    };
    for asset in &assets {
        files::open(asset, false, false)?;
    }
    for entry in &entries {
        if *entry != asset_dir {
            files::open(entry, false, false)?;
        }
    }
    let retired_path = if already_renamed {
        path.to_owned()
    } else {
        let destination = root.join(format!("retired-{name}"));
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(Error::invalid(
                "existing retired namespace preserved; cleanup cannot replace it",
            ));
        }
        fs::rename(path, &destination).map_err(|e| Error::io("commit cleanup namespace", e))?;
        files::sync(root)?;
        destination
    };
    let asset_dir = retired_path.join("assets");
    if asset_dir.exists() {
        for asset in files::entries(&asset_dir, FILES_LIMIT, &never)? {
            fs::remove_file(asset).map_err(|e| Error::io("remove retired media", e))?;
        }
        fs::remove_dir(&asset_dir).map_err(|e| Error::io("remove retired asset directory", e))?;
    }
    for entry in files::entries(&retired_path, MAX_RETENTION as usize + 32, &never)? {
        if entry
            .file_name()
            .is_some_and(|name| name != "retired.json" && name != "owner.lock")
        {
            fs::remove_file(entry).map_err(|e| Error::io("remove retired journal", e))?;
        }
    }
    for name in ["retired.json", "owner.lock"] {
        match fs::remove_file(retired_path.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("remove retired marker", e)),
        }
    }
    fs::remove_dir(&retired_path).map_err(|e| Error::io("remove retired session", e))?;
    files::sync(root)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
