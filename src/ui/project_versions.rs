use super::*;
use crate::project_versions::{Entry, Review};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(test)]
mod tests;
pub(super) mod worker;
use worker::{Event, Job, Record, Task, Worker};

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    root: String,
    loaded_root: Option<PathBuf>,
    name: String,
    notes: String,
    branch: String,
    entries: Vec<Entry>,
    selected: Vec<String>,
    compared: Option<(Record, u64, project::UiState)>,
    prune: Option<Review>,
    recoveries: Vec<crate::project_versions::cleanup::Saved>,
    archive: String,
    recordings: String,
    renders: String,
    compact_destination: String,
    compacted: Option<(String, Arc<AtomicBool>)>,
    compaction_description: String,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    message: String,
    error: Option<String>,
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(c) = &self.active {
            c.store(true, Ordering::Release);
        }
        if let Some((_, cancel)) = &self.compacted {
            cancel.store(true, Ordering::Release);
        }
    }
    fn discard_compaction(&mut self) {
        self.compaction_description.clear();
        if let Some((_, cancel)) = self.compacted.take() {
            cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn poll(&mut self) {
        while let Some(event) = self
            .worker
            .as_ref()
            .and_then(|worker| worker.events.try_recv().ok())
        {
            let cancelled = self
                .active
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Acquire));
            if let Event::CompactExpired(id) = &event {
                if self
                    .compacted
                    .as_ref()
                    .is_some_and(|(review, _)| review == id)
                {
                    self.compacted = None;
                    self.compaction_description.clear();
                    self.message =
                        "Compaction review cancelled; original archive preserved.".into();
                }
                continue;
            }
            let active = self.active.take();
            match event {
                Event::StorageInspected(message) if !cancelled => {
                    self.message = message;
                }
                Event::CompactReviewed(id, message) if !cancelled => {
                    self.compacted = Some((id, active.unwrap()));
                    self.compaction_description = message.clone();
                    self.message = message;
                }
                Event::Published(entries, message) => {
                    if let Some(entries) = entries {
                        self.entries = entries;
                    }
                    self.message = message;
                    self.discard_compaction();
                    self.prune = None;
                    self.recoveries.clear();
                    self.compared = None;
                    self.selected
                        .retain(|id| self.entries.iter().any(|e| &e.id == id));
                }
                Event::Listed(entries) if !cancelled => {
                    self.entries = entries;
                    self.selected.clear();
                    self.message =
                        "Select a version to compare, or select versions to preview pruning."
                            .into();
                }
                Event::CleanupListed(recoveries) if !cancelled => {
                    self.recoveries = recoveries;
                    self.message = "Cleanup keeps unused native files in recovery storage. Restore the newest cleanup first while its version index and retained dependencies remain unchanged.".into();
                }
                Event::Compared(record, revision, view, text) if !cancelled => {
                    self.compared = Some((record, revision, view));
                    self.message = text;
                }
                Event::PruneReviewed(review) if !cancelled => {
                    self.message = format!("Remove {} named version references. {} audio files and {} revision files become unreferenced and will enter recoverable storage. Files referenced by retained versions stay. Quarantine retains disk bytes.\n", review.entries.len(), review.orphaned_audio.len(), review.orphaned_revisions.len());
                    for e in &review.entries {
                        self.message.push_str(&format!("Version: {}\n", e.name));
                    }
                    for hash in &review.orphaned_audio {
                        self.message
                            .push_str(&format!("Unreferenced audio: {hash}.omat\n"));
                    }
                    for id in &review.orphaned_revisions {
                        self.message
                            .push_str(&format!("Unreferenced revision: {id}.omat\n"));
                    }
                    self.prune = Some(review);
                }
                Event::Failed(error) => {
                    self.error = Some(error);
                    self.prune = None;
                    self.compared = None;
                    self.discard_compaction();
                }
                _ => {
                    self.message = "Version operation cancelled before publication.".into();
                    self.compared = None;
                    self.prune = None;
                    self.discard_compaction();
                }
            }
        }
    }
    fn start(&mut self, engine: &Engine, task: Task) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<Arc<AtomicBool>, String> {
            let independent = matches!(
                task,
                Task::InspectStorage { .. }
                    | Task::ReviewCompact { .. }
                    | Task::ApplyCompact { .. }
            );
            let root = (!self.root.trim().is_empty())
                .then(|| std::path::absolute(self.root.trim()).map_err(|e| e.to_string()))
                .transpose()?;
            if !independent && root.is_none() {
                return Err("Choose a version storage folder".into());
            }
            if !independent
                && !matches!(task, Task::List | Task::ListCleanup | Task::Snapshot { .. })
                && self.loaded_root != root
            {
                return Err("Refresh the chosen version folder before using its versions".into());
            }
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let cancel = work.cancel();
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone())?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(Job {
                    root: root.clone(),
                    task,
                    work,
                })
                .map_err(|_| "Version worker unavailable")?;
            if !independent {
                self.loaded_root = root;
            }
            Ok(cancel)
        })();
        match result {
            Ok(cancel) => {
                self.active = Some(cancel);
                self.error = None;
                self.message = "Preparing version operation…".into();
            }
            Err(error) => self.error = Some(error),
        }
    }
}
fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) -> bool {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    response.changed()
}
impl App {
    pub(super) fn project_versions_ui(&mut self, ctx: &egui::Context) {
        if !self.project_versions.open {
            return;
        }
        let mut open = true;
        let mut restore = None;
        let enabled = !self.project.busy()
            && !self.project.committing()
            && !self.engine.cmd.performance().protected()
            && !self.project_import.busy()
            && !self.templates.busy();
        let revision = self.engine.project.revision();
        let view = self.project_view();
        let identities = self.project_watch_identities();
        if self.project_versions.archive.is_empty() {
            if let Some(path) = self.recovery_project_path() {
                self.project_versions.archive = path.display().to_string();
            }
        }
        let cache = self
            .library_metadata
            .tag_recovery_root
            .as_ref()
            .map(|p| p.with_extension("analysis"));
        let recovery = self.support_recovery_root();
        egui::Window::new(tr!("Named project versions")).id(egui::Id::new("Named project versions")).open(&mut open).default_width(650.0).vscroll(true).show(ctx, |ui| {
            let p = &mut self.project_versions;
            ui.add_enabled_ui(enabled && !p.busy(), |ui| {
                if field(ui, tr!("Version storage folder"), &mut p.root, 4096) { p.entries.clear(); p.selected.clear(); p.compared = None; p.prune = None; p.recoveries.clear(); p.loaded_root = None; }
                ui.label(tr!("Choose a dedicated folder. Named snapshots share unchanged decoded audio. The current project save path stays separate."));
                if ui.button(tr!("Refresh named versions")).clicked() { p.compared = None; p.prune = None; p.start(&self.engine, Task::List); }
                field(ui, tr!("Version name"), &mut p.name, 256);
                field(ui, tr!("Revision notes"), &mut p.notes, 4096);
                if ui.button(tr!("Save named snapshot")).clicked() { p.compared = None; p.prune = None; p.start(&self.engine, Task::Snapshot { name: p.name.clone(), notes: p.notes.clone(), view: view.clone(), identities: identities.clone() }); }
                egui::ScrollArea::vertical().id_salt("version-list").max_height(180.0).show(ui, |ui| {
                    for e in &p.entries {
                        let mut selected = p.selected.contains(&e.id);
                        if ui.checkbox(&mut selected, { let __omatainer_args = (&(e.name),&(&e.id[..8]),); crate::localization::format("Version: {} [{}]", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) }).changed() { p.compared = None; p.prune = None; if selected { p.selected.push(e.id.clone()); } else { p.selected.retain(|id| id != &e.id); } }
                        ui.label(crate::localization::timestamp(e.captured_unix_ms.saturating_mul(1_000_000)));
                        ui.label(&e.notes);
                    }
                });
                let selected = (p.selected.len() == 1).then(|| p.entries.iter().find(|e| p.selected.contains(&e.id)).cloned()).flatten();
                if ui.add_enabled(selected.is_some(), egui::Button::new(tr!("Compare selected version"))).clicked() {
                    let e = selected.unwrap();
                    p.compared = None;
                    p.prune = None;
                    p.start(&self.engine, Task::Compare { entry: e, revision, view: view.clone() });
                }
                if p.compared.as_ref().is_some_and(|(_,r,v)| *r != revision || *v != view) { p.compared = None; p.message = "Current project changed. Compare again before restoring or branching.".into(); }
                if ui.add_enabled(p.compared.is_some(), egui::Button::new(tr!("Restore compared version as unsaved copy"))).clicked() { restore = p.compared.take().map(|(r,_,_)| r); }
                field(ui, tr!("New branch project path"), &mut p.branch, 4096);
                if ui.add_enabled(p.compared.is_some(), egui::Button::new(tr!("Branch compared version into new project"))).clicked() {
                    match std::path::absolute(p.branch.trim()) { Ok(path) => p.start(&self.engine, Task::Branch { record: p.compared.as_ref().unwrap().0.clone(), path }), Err(e) => p.error = Some(e.to_string()) }
                }
                if ui.button(tr!("Preview pruning and unused assets")).clicked() { p.compared = None; p.start(&self.engine, Task::PreviewPrune { ids: p.selected.clone() }); }
                if ui.add_enabled(p.prune.is_some(), egui::Button::new(tr!("Apply reviewed pruning"))).clicked() { let review = p.prune.take().unwrap(); p.start(&self.engine, Task::Prune { review }); }
                if ui.button(tr!("Inspect cleanup recovery")).clicked() { p.start(&self.engine, Task::ListCleanup); }
                let mut restore_cleanup = None;
                for recovery in &p.recoveries {
                    ui.label(format!("{} · {} versions · {} quarantined files · {} bytes{}", recovery.id, recovery.versions, recovery.files, recovery.bytes, if recovery.pending { " · incomplete" } else if recovery.restored { " · restored" } else { "" }));
                    if ui.add_enabled(!recovery.restored, egui::Button::new(format!("Restore cleanup {}", recovery.id))).clicked() { restore_cleanup = Some(recovery.id.clone()); }
                }
                if let Some(id) = restore_cleanup { p.start(&self.engine, Task::RestoreCleanup { id }); }
                ui.separator();
                ui.label(tr!("Project storage"));
                if field(ui, tr!("Native archive to inspect or compact"), &mut p.archive, 4096) { p.discard_compaction(); }
                field(ui, tr!("Original recording folder (optional)"), &mut p.recordings, 4096);
                field(ui, tr!("Rendered audio folder (optional)"), &mut p.renders, 4096);
                if ui.button(tr!("Inspect project storage")).clicked() {
                    let roots = (|| -> Result<crate::project_versions::storage::Roots, String> {
                        let absolute = |value: &str| (!value.trim().is_empty()).then(|| std::path::absolute(value.trim()).map_err(|e| e.to_string())).transpose();
                        Ok(crate::project_versions::storage::Roots { archive: absolute(&p.archive)?, recordings: absolute(&p.recordings)?, renders: absolute(&p.renders)?, cache: cache.clone(), recovery: recovery.clone() })
                    })();
                    match roots { Ok(roots) => p.start(&self.engine, Task::InspectStorage { roots }), Err(e) => p.error = Some(e) }
                }
                if field(ui, tr!("New compacted archive path"), &mut p.compact_destination, 4096) { p.discard_compaction(); }
                if ui.add_enabled(!p.archive.trim().is_empty() && !p.compact_destination.trim().is_empty(), egui::Button::new(tr!("Review compacted archive copy"))).clicked() {
                    p.discard_compaction();
                    let paths = std::path::absolute(p.archive.trim()).and_then(|source| std::path::absolute(p.compact_destination.trim()).map(|destination| (source, destination)));
                    match paths { Ok((source,destination)) => p.start(&self.engine, Task::ReviewCompact { source, destination }), Err(e) => p.error = Some(e.to_string()) }
                }
                let reviewed = p.compacted.as_ref().filter(|(_,c)| !c.load(Ordering::Acquire)).map(|(id,_)| id.clone());
                if reviewed.is_some() { ui.label(&p.compaction_description); }
                if ui.add_enabled(reviewed.is_some(), egui::Button::new(tr!("Save reviewed compacted copy"))).clicked() { p.start(&self.engine, Task::ApplyCompact { id: reviewed.unwrap() }); }
                if ui.add_enabled(p.compacted.is_some(), egui::Button::new(tr!("Cancel compaction review"))).clicked() { p.discard_compaction(); p.message = "Compaction review cancelled; no archive was written.".into(); }
            });
            egui::ScrollArea::vertical().id_salt("version-report").max_height(260.0).show(ui, |ui| { ui.label(&p.message); });
            if let Some(e) = &p.error { ui.colored_label(Color32::LIGHT_RED, e); }
            if ui.add_enabled(p.busy(), egui::Button::new(tr!("Cancel version operation"))).clicked() { p.cancel(); }
            if p.busy() { ctx.request_repaint_after(Duration::from_millis(20)); }
        });
        self.project_versions.open = open;
        if let Some(record) = restore {
            self.restore_named_version(record);
        }
    }
}
