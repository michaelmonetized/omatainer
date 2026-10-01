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

/// Keep every bundled fallback available to both UI text families. Some
/// transport symbols (for example ⇄) exist in Hack but not Ubuntu/emoji fonts.
/// Preserve each family's established order and append only missing fallbacks.
pub(crate) fn fallback_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallbacks: Vec<_> = fonts.families.values().flatten().cloned().collect();
    for family in fonts.families.values_mut() {
        for fallback in &fallbacks {
            if !family.contains(fallback) {
                family.push(fallback.clone());
            }
        }
    }
    fonts
}

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
enum Job {
    Wake,
    Force(u64),
}
pub(crate) struct Forced {
    pub generation: u64,
    pub result: Result<Arc<Update>, String>,
}
pub(crate) struct Loader {
    requests: Option<Sender<Job>>,
    results: Receiver<Arc<Update>>,
    forced_results: Receiver<Forced>,
    forced_busy: AtomicBool,
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
        let (forced_output, forced_results) = bounded(1);
        let superseded = results.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::Builder::new()
            .name("omatainer-theme".into())
            .spawn(move || {
                let mut reader = Reader::new(theme, resolver);
                let mut next = Job::Wake;
                loop {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    match next {
                        Job::Force(generation) => {
                            // Supersede only automatic candidates from before this
                            // transaction. A later valid recovery remains publishable.
                            let _ = superseded.try_recv();
                            let result = reader.force();
                            if stopped.load(Ordering::Acquire) {
                                break;
                            }
                            // One forced request stays busy until the GUI takes
                            // its result, so this lane cannot evict a receipt.
                            let _ = forced_output.try_send(Forced { generation, result });
                        }
                        Job::Wake => {
                            if let Some(update) = reader.read() {
                                if let Err(crossbeam_channel::TrySendError::Full(update)) =
                                    output.try_send(update)
                                {
                                    // Automatic updates coalesce and retire on this worker.
                                    let _ = superseded.try_recv();
                                    let _ = output.try_send(update);
                                }
                            }
                        }
                    }
                    next = match jobs.recv_timeout(interval) {
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                        Ok(job) => job,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => Job::Wake,
                    };
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            results,
            forced_results,
            forced_busy: AtomicBool::new(false),
            stop,
            worker: Some(worker),
        })
    }
    pub fn poll(&self) -> Option<Arc<Update>> {
        self.results.try_recv().ok()
    }
    pub fn force(&self, generation: u64) -> Result<(), &'static str> {
        if self.forced_busy.swap(true, Ordering::AcqRel) {
            return Err("theme worker already has an outstanding reload");
        }
        if self
            .requests
            .as_ref()
            .is_none_or(|send| send.try_send(Job::Force(generation)).is_err())
        {
            self.forced_busy.store(false, Ordering::Release);
            return Err("theme worker request slot is unavailable");
        }
        Ok(())
    }
    pub fn poll_forced(&self) -> Option<Forced> {
        let result = self.forced_results.try_recv().ok()?;
        self.forced_busy.store(false, Ordering::Release);
        Some(result)
    }
    #[cfg(test)]
    pub(super) fn request(&self) {
        let _ = self.requests.as_ref().unwrap().try_send(Job::Wake);
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
    strict_after_failure: bool,
}
impl<R: Resolver> Reader<R> {
    fn new(theme: Theme, resolver: R) -> Self {
        Self {
            current: Arc::new(Update {
                theme,
                fonts: Arc::new(fallback_fonts()),
            }),
            resolver,
            font_source: None,
            diagnostics: Vec::new(),
            first: true,
            strict_after_failure: false,
        }
    }
    fn sample(
        &mut self,
        force: bool,
    ) -> (
        Theme,
        Arc<FontDefinitions>,
        Option<(FontSource, Identity)>,
        Vec<String>,
    ) {
        let mut font_source = self.font_source.clone();
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
            if !force && self.font_source.as_ref() == Some(&(source.clone(), identity.clone())) {
                return Ok(None);
            }
            let (bytes, identity) = read_file(&source.path, FONT_LIMIT)?;
            ab_glyph::FontRef::try_from_slice_and_index(&bytes, source.index)
                .map_err(|_| "selected font bytes or face index are invalid".to_owned())?;
            let mut definitions = fallback_fonts();
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
                font_source = Some((source, identity));
                fonts = definitions;
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(format!("font: {error}; keeping previous font")),
        }
        (theme, fonts, font_source, diagnostics)
    }
    fn report(&mut self, diagnostics: Vec<String>) {
        if diagnostics != self.diagnostics {
            for diagnostic in &diagnostics {
                eprintln!("omatainer theme: {diagnostic}");
            }
            self.diagnostics = diagnostics;
        }
    }
    fn force(&mut self) -> Result<Arc<Update>, String> {
        let (theme, fonts, font_source, diagnostics) = self.sample(true);
        if !diagnostics.is_empty() {
            let failure = diagnostics.join("; ");
            self.report(diagnostics);
            // An explicit failed transaction must not be followed by a partial
            // automatic publication of that same invalid resource bundle.
            self.strict_after_failure = true;
            return Err(failure);
        }
        self.report(diagnostics);
        self.strict_after_failure = false;
        // The GUI may cancel this ticket before applying it. Keep the normal
        // watcher able to publish the valid bundle once on its next check. It
        // shares these font definitions, so an already-applied force does not
        // cause a second font-byte installation.
        self.first = true;
        self.font_source = font_source;
        self.current = Arc::new(Update { theme, fonts });
        Ok(self.current.clone())
    }
    fn read(&mut self) -> Option<Arc<Update>> {
        let (theme, fonts, font_source, diagnostics) = self.sample(false);
        let valid = diagnostics.is_empty();
        self.report(diagnostics);
        if self.strict_after_failure && !valid {
            return None;
        }
        if valid {
            self.strict_after_failure = false;
        }
        self.font_source = font_source;
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
