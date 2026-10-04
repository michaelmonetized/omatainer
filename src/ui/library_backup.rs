//! One cancellable backup job; captured catalogs leave the GUI in its mailbox.
use super::*;
use crate::{engine::performance::WorkPermit, library::backup, project_file::SaveOutcome};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc,
};

enum Action {
    Export {
        catalog: Arc<crate::library::Catalog>,
        destination: PathBuf,
        limit: Option<u64>,
    },
    Inspect {
        path: PathBuf,
    },
    Restore {
        path: PathBuf,
        destination: PathBuf,
    },
}
struct Task {
    action: Action,
    work: Arc<WorkPermit>,
    progress: Arc<Progress>,
}
struct Progress {
    files: AtomicUsize,
    total: AtomicUsize,
    bytes: AtomicU64,
}
impl Progress {
    /// Retain small progress counters for GUI reads.
    /// Takes the latest worker counts; never sends a catalog or file buffer.
    fn update(&self, progress: backup::Progress) {
        self.files.store(progress.files, Ordering::Relaxed);
        self.total.store(progress.total, Ordering::Relaxed);
        self.bytes.store(progress.bytes, Ordering::Relaxed);
    }
}
enum Receipt {
    Inspected(backup::Summary),
    Published(backup::Published),
}
struct Worker {
    jobs: mpsc::SyncSender<Task>,
    results: mpsc::Receiver<Result<Receipt, String>>,
}
impl Worker {
    /// Start a bounded backup owner on first use.
    /// Returns one request slot and one small result slot, or the thread error.
    fn start() -> Result<Self, String> {
        let (jobs, pending) = mpsc::sync_channel::<Task>(1);
        let (done, results) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("omatainer-library-backup".into())
            .spawn(move || {
                while let Ok(task) = pending.recv() {
                    let cancel = task.work.cancel();
                    let progress = |value| task.progress.update(value);
                    let result = match &task.action {
                        Action::Export {
                            catalog,
                            destination,
                            limit,
                        } => backup::export(
                            catalog,
                            destination,
                            *limit,
                            &cancel,
                            Some(&task.work),
                            progress,
                        )
                        .map(Receipt::Published),
                        Action::Inspect { path } => {
                            backup::inspect(path, &cancel, progress).map(Receipt::Inspected)
                        }
                        Action::Restore { path, destination } => {
                            backup::restore(path, destination, &cancel, Some(&task.work), progress)
                                .map(Receipt::Published)
                        }
                    };
                    drop(task);
                    if done.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs, results })
    }
}
pub(super) struct Panel {
    pub open: bool,
    destination: String,
    source: String,
    restore_destination: String,
    collect: bool,
    limit_gib: u32,
    worker: Option<Worker>,
    active: Option<(Arc<WorkPermit>, Arc<Progress>)>,
    restored: Option<PathBuf>,
    message: String,
    revision: u64,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            destination: String::new(),
            source: String::new(),
            restore_destination: String::new(),
            collect: false,
            limit_gib: 64,
            worker: None,
            active: None,
            restored: None,
            message: String::new(),
            revision: 0,
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Panel {
    /// Cancel only this panel's admitted optional work.
    /// Returns immediately; the worker owns read cleanup and final catalog release.
    fn cancel(&self) {
        if let Some((work, _)) = &self.active {
            work.cancel().store(true, Ordering::Release);
        }
    }
    /// Retire activation IDs when a pathname or copy grant changes.
    /// Takes the current editor; saturates at a disabled terminal revision.
    fn edited(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
    /// Admit one job and move its captured snapshot to the worker.
    /// `action` is complete and `performance` owns cancellation; returns submission status.
    fn submit(
        &mut self,
        action: Action,
        performance: &crate::engine::performance::Handle,
    ) -> Result<(), String> {
        if self.active.is_some() {
            return Err("A library backup job is already running".into());
        }
        let next = self
            .revision
            .checked_add(1)
            .ok_or("Library backup request counter exhausted")?;
        let work = Arc::new(performance.optional_work().map_err(|e| e.to_string())?);
        if self.worker.is_none() {
            self.worker = Some(Worker::start()?);
        }
        let progress = Arc::new(Progress {
            files: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
        });
        self.worker
            .as_ref()
            .unwrap()
            .jobs
            .try_send(Task {
                action,
                work: work.clone(),
                progress: progress.clone(),
            })
            .map_err(|_| "Library backup worker is unavailable")?;
        self.active = Some((work, progress));
        self.revision = next;
        self.restored = None;
        self.message = "Library backup job queued".into();
        Ok(())
    }
    /// Receive one actual job outcome without waiting on filesystem work.
    /// Retains a published path for explicit catalog import and reports durability warnings.
    fn poll(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let result = match worker.results.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) if self.active.is_some() => {
                Err("Library backup worker disconnected".into())
            }
            Err(_) => return,
        };
        self.active = None;
        self.message = match result {
            Err(error) => error,
            Ok(Receipt::Inspected(summary)) => format!("Verified backup: {} tracks, {} crates, {} collected tracks, {} bytes of unique media", summary.tracks, summary.crates, summary.collected, summary.bytes),
            Ok(Receipt::Published(published)) => {
                let warning = match published.outcome { SaveOutcome::Durable => String::new(), SaveOutcome::CommittedButDirectorySyncFailed(error) => format!("; published, but directory sync failed: {error}") };
                let message = format!("Published {}: {} tracks, {} crates, {} collected tracks, {} unique media bytes{}", published.catalog.display(), published.summary.tracks, published.summary.crates, published.summary.collected, published.summary.bytes, warning);
                if published.catalog.file_name().is_some_and(|name| name == "library.json") { self.restored = Some(published.catalog); }
                message
            }
        };
    }
}
/// Draw one bounded native pathname field with an accessible name.
/// Takes its label and draft text; leaves filesystem validation to the worker.
fn path_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .id_salt(label)
            .char_limit(4096)
            .desired_width(460.0),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, response.enabled(), label)
    });
    help::annotate(ui, &response, HelpControl::LibraryBackupPath);
    response.changed()
}
impl App {
    /// Poll backup progress independently of the window's visibility.
    /// Requests another GUI frame only while its bounded worker is active.
    pub(super) fn poll_library_backup(&mut self, ctx: &egui::Context) {
        self.library_backup.poll();
        if self.library_backup.active.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Draw consistent catalog export, verified restore and explicit import.
    /// Takes the native context; no file read, write, catalog clone or join occurs on this thread.
    pub(super) fn library_backup_ui(&mut self, ctx: &egui::Context) {
        if !self.library_backup.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let busy = self.library_backup.active.is_some();
        let allowed = !self.engine.safe_mode()
            && !self.project.committing()
            && !self.library_closing()
            && !self.engine.cmd.performance().protected()
            && self.library_backup.revision != u64::MAX;
        let export_allowed = allowed
            && self.library_metadata.ready()
            && !self.library_metadata.active()
            && self.library_metadata.durable;
        let mut open = true;
        egui::Window::new("Back up and restore DJ library").id(egui::Id::new("library-backup")).open(&mut open).show(ctx, |ui| {
            ui.label("Export waits for a saved catalog. Its captured snapshot stays fixed while you keep working.");
            ui.add_enabled_ui(!busy, |ui| {
                if path_field(ui, "New backup directory", &mut self.library_backup.destination) { self.library_backup.edited(); }
                let response = ui.checkbox(&mut self.library_backup.collect, "I authorize copying all local catalog music into this backup");
                accessibility::button(ui, &response, "Authorize local music collection", Some(self.library_backup.collect));
                help::annotate(ui, &response, HelpControl::LibraryBackupCollect);
                if response.changed() { self.library_backup.edited(); }
                if self.library_backup.collect {
                    ui.horizontal(|ui| {
                        ui.label("Collection limit (GiB)");
                        let before = self.library_backup.limit_gib;
                        let response = ui.add(egui::DragValue::new(&mut self.library_backup.limit_gib).range(1..=1024));
                        accessibility::numeric(ui, &response, "Library collection limit", self.library_backup.limit_gib as f32, 1.0, 1024.0, 1.0, " GiB").map(|value| self.library_backup.limit_gib = value.round() as u32);
                        help::annotate(ui, &response, HelpControl::LibraryBackupCollect);
                        if before != self.library_backup.limit_gib { self.library_backup.edited(); }
                    });
                    ui.label("8 GiB per file. Duplicate bytes count toward this limit and share one backup asset. Provider music is not collected.");
                }
                ui.push_id(self.library_backup.revision, |ui| {
                    let response = ui.add_enabled(export_allowed, egui::Button::new("Export library snapshot"));
                    help::annotate(ui, &response, HelpControl::LibraryBackupExport);
                    if response.clicked() {
                        let catalog = self.library_metadata.catalog.clone();
                        let count = catalog.tracks.len();
                        let revision = catalog.crates.revision();
                        let action = Action::Export { catalog, destination: PathBuf::from(self.library_backup.destination.trim()), limit: self.library_backup.collect.then_some(self.library_backup.limit_gib as u64 * 1024 * 1024 * 1024) };
                        self.library_backup.message = match self.library_backup.submit(action, self.engine.cmd.performance()) { Ok(()) => format!("Captured {} saved tracks at crate revision {}", count, revision), Err(error) => error };
                    }
                });
                if !export_allowed { ui.label("Finish the current library save and leave protection before exporting."); }
                ui.separator();
                if path_field(ui, "Existing backup directory", &mut self.library_backup.source) { self.library_backup.edited(); }
                let response = ui.push_id(("verify-backup", self.library_backup.revision), |ui| ui.add_enabled(allowed, egui::Button::new("Verify backup"))).inner;
                help::annotate(ui, &response, HelpControl::LibraryBackupRestore);
                if response.clicked() {
                    let action = Action::Inspect { path: PathBuf::from(self.library_backup.source.trim()) };
                    if let Err(error) = self.library_backup.submit(action, self.engine.cmd.performance()) { self.library_backup.message = error; }
                }
                if path_field(ui, "New restore directory", &mut self.library_backup.restore_destination) { self.library_backup.edited(); }
                let response = ui.push_id(("restore-backup", self.library_backup.revision), |ui| ui.add_enabled(allowed, egui::Button::new("Restore to new directory"))).inner;
                help::annotate(ui, &response, HelpControl::LibraryBackupRestore);
                if response.clicked() {
                    let action = Action::Restore { path: PathBuf::from(self.library_backup.source.trim()), destination: PathBuf::from(self.library_backup.restore_destination.trim()) };
                    if let Err(error) = self.library_backup.submit(action, self.engine.cmd.performance()) { self.library_backup.message = error; }
                }
                ui.label("Restore preserves IDs and preparation. It never overwrites a directory or activates the catalog. Import rejects conflicting identities, paths and crates before changing your live library.");
            });
            if let Some((_, progress)) = &self.library_backup.active {
                ui.label(format!("{} / {} tracks; {} bytes read", progress.files.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed), progress.bytes.load(Ordering::Relaxed)));
                if ui.button("Cancel library backup job").help(ui, HelpControl::LibraryBackupCancel).clicked() { self.library_backup.cancel(); }
            }
            if let Some(path) = self.library_backup.restored.clone() {
                let response = ui.add_enabled(!busy && allowed && self.library_metadata.ready() && !self.library_metadata.active(), egui::Button::new("Import restored catalog"));
                help::annotate(ui, &response, HelpControl::LibraryImport);
                if response.clicked() {
                    self.status = if self.library_metadata.import(path) { "Restored library import queued; inspect catalog save status".into() } else { "Restored library import is unavailable or pending".into() };
                }
                ui.label(self.library_metadata.label());
            }
            ui.label(&self.library_backup.message);
        });
        self.library_backup.open = open;
    }
}

#[cfg(test)]
mod tests;
