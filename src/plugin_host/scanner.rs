use super::*;
use std::{
    collections::HashSet,
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const MAX_CATALOG: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub path: PathBuf,
    pub binary: Option<BinaryIdentity>,
    pub classes: Vec<Class>,
    pub failure: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub schema: u32,
    pub roots: Vec<PathBuf>,
    pub blacklist: HashSet<PathBuf>,
    pub records: Vec<Record>,
}
impl Default for Catalog {
    fn default() -> Self {
        let mut roots = vec!["/usr/lib/vst3".into(), "/usr/local/lib/vst3".into()];
        if let Some(home) = std::env::var_os("HOME") {
            roots.insert(0, PathBuf::from(home).join(".vst3"));
        }
        Self {
            schema: 1,
            roots,
            blacklist: HashSet::new(),
            records: Vec::new(),
        }
    }
}
impl Catalog {
    pub(super) fn validate(&self) -> Result<(), String> {
        let path = |p: &PathBuf| {
            p.is_absolute()
                && p.as_os_str().len() <= 4096
                && !p
                    .components()
                    .any(|v| matches!(v, std::path::Component::ParentDir))
        };
        if self.schema != 1
            || self.roots.len() > 64
            || self.blacklist.len() > 4096
            || self.records.len() > 4096
            || self.roots.iter().chain(&self.blacklist).any(|p| !path(p))
        {
            return Err("Plugin catalog schema or path limits are invalid".into());
        }
        let mut seen = HashSet::new();
        for r in &self.records {
            if !path(&r.path)
                || !seen.insert(&r.path)
                || r.classes.len() > 64
                || r.failure.as_ref().is_some_and(|f| f.len() > 4096)
                || r.classes.is_empty() && r.failure.is_none()
            {
                return Err("Invalid or duplicate plugin record".into());
            }
            for c in &r.classes {
                c.validate()?;
            }
            if let Some(b) = &r.binary {
                if b.bundle != r.path
                    || b.sha256.len() != 64
                    || !b.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || b.arch.len() > 64
                    || !path(&b.binary)
                {
                    return Err("Invalid cached plugin binary identity".into());
                }
            } else if !r.classes.is_empty() {
                return Err("Plugin classes have no qualified binary".into());
            }
        }
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn read(path: &Path) -> Result<(Catalog, Option<[u8; 32]>), String> {
    match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => {
            let meta = file.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > MAX_CATALOG as u64 {
                return Err("Plugin catalog is not a bounded regular file".into());
            }
            let mut bytes = Vec::new();
            file.take(MAX_CATALOG as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > MAX_CATALOG {
                return Err("Plugin catalog exceeds 16 MiB".into());
            }
            let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            catalog.validate()?;
            Ok((catalog, Some(digest(&bytes))))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Catalog::default(), None)),
        Err(e) => Err(e.to_string()),
    }
}
fn save(path: &Path, catalog: &Catalog, previous: Option<[u8; 32]>) -> Result<[u8; 32], String> {
    catalog.validate()?;
    let bytes = serde_json::to_vec(catalog).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CATALOG {
        return Err("Plugin catalog exceeds 16 MiB; narrow the scan roots".into());
    }
    let (_, current) = read(path)?;
    if previous != current {
        return Err("Plugin catalog changed in another session; reload before publishing".into());
    }
    let parent = path.parent().ok_or("Plugin catalog has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = parent.join(format!(
        ".plugins-{}-{}.partial",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staging)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        let (_, current) = read(path)?;
        if current != previous {
            return Err("Plugin catalog changed before publication".into());
        }
        std::fs::rename(&staging, path).map_err(|e| e.to_string())?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(digest(&bytes))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staging);
    }
    result
}
pub(super) fn candidates(roots: &[PathBuf], cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
    let mut found = HashSet::new();
    let mut visited = 0;
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut walk = walkdir::WalkDir::new(root)
            .follow_links(false)
            .max_depth(16)
            .into_iter();
        while let Some(entry) = walk.next() {
            if cancel.load(Ordering::Acquire) {
                return Err("Plugin scan cancelled".into());
            }
            visited += 1;
            if visited > 100_000 {
                return Err(
                    "Plugin traversal exceeds 100000 entries; narrow the scan roots".into(),
                );
            }
            let entry = entry.map_err(|e| format!("Plugin scan path cannot be read: {e}"))?;
            if entry.file_type().is_dir() && entry.path().extension().is_some_and(|s| s == "vst3") {
                found.insert(entry.path().to_path_buf());
                walk.skip_current_dir();
                if found.len() > 4096 {
                    return Err("Plugin scan exceeds 4096 bundles".into());
                }
            }
        }
    }
    let mut paths = found.into_iter().collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}
fn probe(exe: &Path, binary: BinaryIdentity, cancel: &AtomicBool) -> Result<Vec<Class>, String> {
    let mut process = process::Process::start(exe)?;
    match process.exchange(
        &Request::Probe {
            binary: binary.clone(),
        },
        cancel,
        Duration::from_secs(10),
    )? {
        Response::Classes { classes } => {
            if classes.is_empty() || classes.len() > 64 {
                return Err("Plugin probe returned invalid class count".into());
            }
            let mut ids = HashSet::new();
            for class in &classes {
                class.validate()?;
                if class.info.path != binary.bundle || !ids.insert(&class.info.uid) {
                    return Err("Plugin probe returned mismatched or duplicate identities".into());
                }
            }
            Ok(classes)
        }
        _ => Err("Plugin probe returned an unexpected response".into()),
    }
}
enum Event {
    LoadFailed(String),
    Loaded(Catalog, Option<[u8; 32]>),
    Progress(usize, usize),
    Saved(Catalog, [u8; 32]),
    Failed(String),
}
struct Work {
    cancel: Arc<AtomicBool>,
    rx: crossbeam_channel::Receiver<Event>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub(crate) struct Browser {
    pub open: bool,
    pub catalog: Catalog,
    pub message: String,
    pub roots: String,
    pub selected: Option<(usize, usize)>,
    path: Option<PathBuf>,
    revision: Option<[u8; 32]>,
    work: Option<Work>,
    blocked: bool,
    performance: Option<crate::engine::performance::Handle>,
}
impl Browser {
    pub fn set_performance(&mut self, handle: crate::engine::performance::Handle) {
        self.performance = Some(handle);
    }
    fn admit(&mut self) -> Result<Option<crate::engine::performance::WorkPermit>, String> {
        self.performance
            .as_ref()
            .map(|h| h.optional_work())
            .transpose()
            .map_err(|e| e.to_string())
    }
    pub fn reload(&mut self) {
        if self.busy() {
            return;
        }
        let Some(path) = self.path.take() else {
            return;
        };
        self.initialize(path);
    }
    pub fn initialize(&mut self, path: PathBuf) {
        if self.path.is_some() {
            return;
        }
        self.path = Some(path.clone());
        self.launch(None, move |_, tx| match read(&path) {
            Ok((c, r)) => {
                let _ = tx.send(Event::Loaded(c, r));
            }
            Err(e) => {
                let _ = tx.send(Event::LoadFailed(e));
            }
        });
    }
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    fn launch(
        &mut self,
        permit: Option<crate::engine::performance::WorkPermit>,
        job: impl FnOnce(Arc<AtomicBool>, crossbeam_channel::Sender<Event>) + Send + 'static,
    ) {
        if self.work.is_some() {
            return;
        }
        let cancel = permit
            .as_ref()
            .map(|p| p.cancel())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let c = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(4);
        match std::thread::Builder::new()
            .name("vst3-catalog".into())
            .spawn(move || {
                let _permit = permit;
                job(c, tx)
            }) {
            Ok(join) => {
                self.work = Some(Work {
                    cancel,
                    rx,
                    join: Some(join),
                })
            }
            Err(e) => self.message = e.to_string(),
        }
    }
    pub fn cancel(&self) {
        if let Some(w) = &self.work {
            w.cancel.store(true, Ordering::Release);
        }
    }
    pub fn poll(&mut self) {
        if let Some(w) = &self.work {
            while let Ok(e) = w.rx.try_recv() {
                match e {
                    Event::Loaded(c, r) => {
                        self.roots = c
                            .roots
                            .iter()
                            .map(|p| p.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.catalog = c;
                        self.revision = r;
                        self.blocked = false;
                        self.message =
                            "Plugin catalog loaded; scan installed bundles to qualify them.".into();
                    }
                    Event::Progress(n, total) => {
                        self.message =
                            format!("Scanning plugin {n}/{total}; known plugins remain available")
                    }
                    Event::Saved(c, r) => {
                        self.catalog = c;
                        self.revision = Some(r);
                        self.message="Plugin catalog saved. Failed binaries remain quarantined until retry or a changed installation.".into();
                    }
                    Event::LoadFailed(e) => {
                        self.blocked = true;
                        self.message = e;
                    }
                    Event::Failed(e) => {
                        self.message = e;
                    }
                }
            }
            if w.join.as_ref().is_some_and(|j| j.is_finished()) {
                if let Some(mut w) = self.work.take() {
                    if w.join.take().unwrap().join().is_err() {
                        self.message =
                            "Plugin catalog worker panicked; previous catalog retained".into();
                    }
                }
            }
        }
    }
    pub fn scan(&mut self, retry: bool) {
        if self.busy() || self.blocked {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        let mut catalog = self.catalog.clone();
        catalog.roots = self
            .roots
            .lines()
            .filter(|p| !p.trim().is_empty())
            .map(|p| PathBuf::from(p.trim()))
            .collect();
        if let Err(e) = catalog.validate() {
            self.message = e;
            return;
        }
        let previous = self.revision;
        let permit = match self.admit() {
            Ok(p) => p,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        self.launch(permit, move |cancel, tx| {
            let result = (|| {
                let exe = executable()?;
                let mut discovery = process::Process::start(&exe)?;
                let paths = match discovery.exchange(
                    &Request::Paths {
                        roots: catalog.roots.clone(),
                    },
                    &cancel,
                    Duration::from_secs(10),
                )? {
                    Response::Paths { paths } => paths,
                    _ => return Err("Invalid plugin path inventory".into()),
                };
                drop(discovery);
                let total = paths.len();
                for (index, path) in paths.into_iter().enumerate() {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Plugin scan cancelled; previous catalog retained".into());
                    }
                    let _ = tx.try_send(Event::Progress(index + 1, total));
                    if catalog.blacklist.contains(&path) {
                        continue;
                    }
                    let identity: Result<BinaryIdentity, String> = (|| {
                        let mut p = process::Process::start(&exe)?;
                        match p.exchange(
                            &Request::Inspect { path: path.clone() },
                            &cancel,
                            Duration::from_secs(10),
                        )? {
                            Response::Identity { binary } if binary.bundle == path => Ok(binary),
                            _ => Err("Invalid plugin binary inspection".into()),
                        }
                    })();
                    let old = catalog.records.iter().position(|r| r.path == path);
                    if old.is_some_and(|i| {
                        identity
                            .as_ref()
                            .is_ok_and(|b| catalog.records[i].binary.as_ref() == Some(b))
                            && (!retry || catalog.records[i].failure.is_none())
                    }) {
                        continue;
                    }
                    let (binary, classes, failure) = match identity {
                        Ok(binary) => match probe(&exe, binary.clone(), &cancel) {
                            Ok(classes) => (Some(binary), classes, None),
                            Err(error) => (
                                Some(binary),
                                Vec::new(),
                                Some(error.chars().take(4096).collect()),
                            ),
                        },
                        Err(error) => (None, Vec::new(), Some(error.chars().take(4096).collect())),
                    };
                    let record = Record {
                        path,
                        binary,
                        classes,
                        failure,
                    };
                    if let Some(i) = old {
                        catalog.records[i] = record;
                    } else {
                        if catalog.records.len() >= 4096 {
                            return Err("Retained plugin catalog exceeds 4096 bundles".into());
                        }
                        catalog.records.push(record);
                    }
                    if serde_json::to_vec(&catalog)
                        .map_err(|e| e.to_string())?
                        .len()
                        > MAX_CATALOG
                    {
                        return Err("Plugin catalog exceeds 16 MiB; narrow the scan roots".into());
                    }
                }
                if cancel.load(Ordering::Acquire) {
                    return Err("Plugin scan cancelled; previous catalog retained".into());
                }
                let revision = save(&path, &catalog, previous)?;
                Ok((catalog, revision))
            })();
            let event = match result {
                Ok((c, r)) => Event::Saved(c, r),
                Err(e) => Event::Failed(e),
            };
            let _ = tx.send(event);
        });
    }
    pub fn blacklist(&mut self, path: PathBuf, blocked: bool) {
        if self.busy() || self.blocked {
            return;
        }
        let Some(storage) = self.path.clone() else {
            return;
        };
        let mut catalog = self.catalog.clone();
        if blocked {
            catalog.blacklist.insert(path);
        } else {
            catalog.blacklist.remove(&path);
        }
        let previous = self.revision;
        let permit = match self.admit() {
            Ok(p) => p,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        self.launch(permit, move |_, tx| {
            let e = match save(&storage, &catalog, previous) {
                Ok(r) => Event::Saved(catalog, r),
                Err(e) => Event::Failed(e),
            };
            let _ = tx.send(e);
        });
    }
}
#[cfg(test)]
mod tests;

fn executable() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("OMATAINER_TEST_BIN") {
        return Ok(path.into());
    }
    std::env::current_exe().map_err(|e| e.to_string())
}
