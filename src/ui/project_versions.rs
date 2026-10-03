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
    fn cancel(&self) {
        if let Some(c) = &self.active {
            c.store(true, Ordering::Release);
        }
    }
    pub(super) fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            while let Ok(event) = worker.events.try_recv() {
                let cancelled = self
                    .active
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::Acquire));
                self.active = None;
                match event {
                    Event::Published(entries, message) => {
                        if let Some(entries) = entries {
                            self.entries = entries;
                        }
                        self.message = message;
                        self.prune = None;
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
                    Event::Compared(record, revision, view, text) if !cancelled => {
                        self.compared = Some((record, revision, view));
                        self.message = text;
                    }
                    Event::PruneReviewed(review) if !cancelled => {
                        self.message = format!("Prune {} named versions. {} audio files and {} revision files become unreferenced and will be removed. Files referenced by retained versions stay.\n", review.entries.len(), review.orphaned_audio.len(), review.orphaned_revisions.len());
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
                    }
                    _ => {
                        self.message = "Version operation cancelled before publication.".into();
                        self.compared = None;
                        self.prune = None;
                    }
                }
            }
        }
    }
    fn start(&mut self, engine: &Engine, task: Task) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<Arc<AtomicBool>, String> {
            if self.root.trim().is_empty() {
                return Err("Choose a version storage folder".into());
            }
            let root = std::path::absolute(self.root.trim()).map_err(|e| e.to_string())?;
            if !matches!(task, Task::List | Task::Snapshot { .. })
                && self.loaded_root.as_ref() != Some(&root)
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
            self.loaded_root = Some(root);
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
        egui::Window::new("Named project versions").open(&mut open).default_width(650.0).show(ctx, |ui| {
            let p = &mut self.project_versions;
            ui.add_enabled_ui(enabled && !p.busy(), |ui| {
                if field(ui, "Version storage folder", &mut p.root, 4096) { p.entries.clear(); p.selected.clear(); p.compared = None; p.prune = None; p.loaded_root = None; }
                ui.label("Choose a dedicated folder. Named snapshots share unchanged decoded audio. The current project save path stays separate.");
                if ui.button("Refresh named versions").clicked() { p.compared = None; p.prune = None; p.start(&self.engine, Task::List); }
                field(ui, "Version name", &mut p.name, 256);
                field(ui, "Revision notes", &mut p.notes, 4096);
                if ui.button("Save named snapshot").clicked() { p.compared = None; p.prune = None; p.start(&self.engine, Task::Snapshot { name: p.name.clone(), notes: p.notes.clone(), view: view.clone(), identities: identities.clone() }); }
                egui::ScrollArea::vertical().id_salt("version-list").max_height(180.0).show(ui, |ui| {
                    for e in &p.entries {
                        let mut selected = p.selected.contains(&e.id);
                        if ui.checkbox(&mut selected, format!("Version: {} [{}]", e.name, &e.id[..8])).changed() { p.compared = None; p.prune = None; if selected { p.selected.push(e.id.clone()); } else { p.selected.retain(|id| id != &e.id); } }
                        ui.label(&e.notes);
                    }
                });
                let selected = (p.selected.len() == 1).then(|| p.entries.iter().find(|e| p.selected.contains(&e.id)).cloned()).flatten();
                if ui.add_enabled(selected.is_some(), egui::Button::new("Compare selected version")).clicked() {
                    let e = selected.unwrap();
                    p.compared = None;
                    p.prune = None;
                    p.start(&self.engine, Task::Compare { entry: e, revision, view: view.clone() });
                }
                if p.compared.as_ref().is_some_and(|(_,r,v)| *r != revision || *v != view) { p.compared = None; p.message = "Current project changed. Compare again before restoring or branching.".into(); }
                if ui.add_enabled(p.compared.is_some(), egui::Button::new("Restore compared version as unsaved copy")).clicked() { restore = p.compared.take().map(|(r,_,_)| r); }
                field(ui, "New branch project path", &mut p.branch, 4096);
                if ui.add_enabled(p.compared.is_some(), egui::Button::new("Branch compared version into new project")).clicked() {
                    match std::path::absolute(p.branch.trim()) { Ok(path) => p.start(&self.engine, Task::Branch { record: p.compared.as_ref().unwrap().0.clone(), path }), Err(e) => p.error = Some(e.to_string()) }
                }
                if ui.button("Preview pruning and unused assets").clicked() { p.compared = None; p.start(&self.engine, Task::PreviewPrune { ids: p.selected.clone() }); }
                if ui.add_enabled(p.prune.is_some(), egui::Button::new("Apply reviewed pruning")).clicked() { let review = p.prune.take().unwrap(); p.start(&self.engine, Task::Prune { review }); }
            });
            egui::ScrollArea::vertical().id_salt("version-report").max_height(260.0).show(ui, |ui| { ui.label(&p.message); });
            if let Some(e) = &p.error { ui.colored_label(Color32::LIGHT_RED, e); }
            if ui.add_enabled(p.busy(), egui::Button::new("Cancel version operation")).clicked() { p.cancel(); }
            if p.busy() { ctx.request_repaint_after(Duration::from_millis(20)); }
        });
        self.project_versions.open = open;
        if let Some(record) = restore {
            self.restore_named_version(record);
        }
    }
}
