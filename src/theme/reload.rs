//! Resolve every theme source off the GUI, publishing only complete valid state.
use super::Theme;
use crossbeam_channel::{bounded, Receiver, Sender};
use egui::{FontData, FontDefinitions, FontFamily};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::Duration;
mod fontconfig;
use fontconfig::Fontconfig;

const INTERVAL: Duration = Duration::from_millis(800);
const TEXT_LIMIT: usize = 64 * 1024;
const FONT_LIMIT: usize = 32 * 1024 * 1024;
pub(super) const SELECTED: &str = "omatainer-selected";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FontSource {
    family: String,
    path: PathBuf,
    index: u32,
}
pub(super) trait Resolver: Send + 'static {
    fn resolve(&mut self) -> Result<FontSource, String>;
}

#[derive(Clone)]
pub(crate) struct Update {
    pub theme: Theme,
    pub fonts: Arc<FontDefinitions>,
}
pub(crate) struct Loader {
    requests: Option<Sender<()>>,
    results: Receiver<Arc<Update>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Loader {
    pub fn start(theme: Theme) -> std::io::Result<Self> {
        Self::with_resolver(theme, Fontconfig::default(), INTERVAL)
    }
    pub(super) fn with_resolver<R: Resolver>(
        theme: Theme,
        resolver: R,
        interval: Duration,
    ) -> std::io::Result<Self> {
        let (requests, jobs) = bounded(1);
        let (output, results) = bounded(1);
        let superseded = results.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::Builder::new()
            .name("omatainer-theme".into())
            .spawn(move || {
                let mut reader = Reader::new(theme, resolver);
                loop {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    if let Some(update) = reader.read() {
                        if let Err(crossbeam_channel::TrySendError::Full(update)) =
                            output.try_send(update)
                        {
                            // One producer and one latest-state slot. Eviction and
                            // large obsolete font buffers are retired on this worker.
                            let _ = superseded.try_recv();
                            let _ = output.try_send(update);
                        }
                    }
                    match jobs.recv_timeout(interval) {
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                        _ => {}
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            results,
            stop,
            worker: Some(worker),
        })
    }
    pub fn poll(&self) -> Option<Arc<Update>> {
        self.results.try_recv().ok()
    }
    #[cfg(test)]
    pub(super) fn request(&self) {
        let _ = self.requests.as_ref().unwrap().try_send(());
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
            // No GUI wait for a slow filesystem. Ownership stays on the worker.
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Identity {
    fn of(meta: &fs::Metadata) -> Result<Self, String> {
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
    fn read(path: &Path) -> Result<Self, String> {
        Self::of(&fs::metadata(path).map_err(|error| error.to_string())?)
    }
}
fn read_file(path: &Path, limit: usize) -> Result<(Vec<u8>, Identity), String> {
    let before = Identity::read(path)?;
    if before.len > limit as u64 {
        return Err(format!("file exceeds {limit} bytes"));
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    if Identity::of(&file.metadata().map_err(|error| error.to_string())?)? != before {
        return Err("file changed while opening".into());
    }
    let mut bytes = Vec::with_capacity(before.len as usize);
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("file exceeds {limit} bytes"));
    }
    if Identity::of(&file.metadata().map_err(|error| error.to_string())?)? != before
        || Identity::read(path)? != before
    {
        return Err("file changed while reading".into());
    }
    Ok((bytes, before))
}
fn read_text(path: &Path) -> Result<String, String> {
    String::from_utf8(read_file(path, TEXT_LIMIT)?.0).map_err(|error| error.to_string())
}

struct Reader<R> {
    current: Arc<Update>,
    resolver: R,
    font_source: Option<(FontSource, Identity)>,
    diagnostics: Vec<String>,
    first: bool,
}
impl<R: Resolver> Reader<R> {
    fn new(theme: Theme, resolver: R) -> Self {
        Self {
            current: Arc::new(Update {
                theme,
                fonts: Arc::new(FontDefinitions::default()),
            }),
            resolver,
            font_source: None,
            diagnostics: Vec::new(),
            first: true,
        }
    }
    fn read(&mut self) -> Option<Arc<Update>> {
        let mut theme = self.current.theme.clone();
        let mut diagnostics = Vec::new();
        match read_text(&theme.path)
            .and_then(|raw| theme.apply_colors(&raw).map_err(|error| error.to_string()))
        {
            Ok(colors) => diagnostics.extend(colors.into_iter().map(|color| color.to_string())),
            Err(error) => diagnostics.push(format!("colors: {error}")),
        }
        // Respect the configured source directory, including private fixtures.
        let shell = theme.path.with_file_name("shell.toml");
        match read_text(&shell).and_then(|raw| {
            raw.parse::<toml::Value>()
                .map_err(|error| error.to_string())
        }) {
            Ok(value) => {
                if let Some(size) = value.get("font").and_then(|font| font.get("base-size")) {
                    let number = size
                        .as_float()
                        .or_else(|| size.as_integer().map(|n| n as f64));
                    match number {
                        Some(size) if size.is_finite() && (4.0..=96.0).contains(&size) => theme.font_size = size as f32,
                        _ => diagnostics.push("font.base-size must be a finite number from 4 to 96; keeping previous size".into()),
                    }
                }
            }
            Err(error) => diagnostics.push(format!("shell: {error}")),
        }
        let mut fonts = self.current.fonts.clone();
        match self.resolver.resolve().and_then(|source| {
            let identity = Identity::read(&source.path)?;
            if self.font_source.as_ref() == Some(&(source.clone(), identity.clone())) {
                return Ok(None);
            }
            let (bytes, identity) = read_file(&source.path, FONT_LIMIT)?;
            ab_glyph::FontRef::try_from_slice_and_index(&bytes, source.index)
                .map_err(|_| "selected font bytes or face index are invalid".to_owned())?;
            let mut definitions = FontDefinitions::default();
            let mut data = FontData::from_owned(bytes);
            data.index = source.index;
            definitions
                .font_data
                .insert(SELECTED.into(), Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                definitions
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, SELECTED.into());
            }
            Ok(Some((source, identity, Arc::new(definitions))))
        }) {
            Ok(Some((source, identity, definitions))) => {
                theme.font = source.family.clone();
                self.font_source = Some((source, identity));
                fonts = definitions;
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(format!("font: {error}; keeping previous font")),
        }
        if diagnostics != self.diagnostics {
            for diagnostic in &diagnostics {
                eprintln!("omatainer theme: {diagnostic}");
            }
            self.diagnostics = diagnostics;
        }
        if self.first || theme != self.current.theme || !Arc::ptr_eq(&fonts, &self.current.fonts) {
            self.first = false;
            self.current = Arc::new(Update { theme, fonts });
            Some(self.current.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
