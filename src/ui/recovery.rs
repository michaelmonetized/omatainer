//! Recovery copies never become the explicitly saved project. Only the worker
//! touches recovery files; GUI state is coalesced into one pending capture view.
use super::*;
use crate::recovery::Candidate;
mod worker;
use worker::{Event, Job, Worker};

#[derive(Default)]
pub(super) struct Recovery {
    worker: Option<Worker>,
    pub open: bool,
    root: Option<PathBuf>,
    message: Option<String>,
    candidates: Vec<Candidate>,
    warnings: Vec<String>,
    preview: Option<worker::Preview>,
    remove: Option<Candidate>,
    pending: Option<u64>,
    epoch: Option<u64>,
    engine_revision: u64,
    view_revision: u64,
    view: Option<project::UiState>,
    config: Option<crate::recovery::Config>,
    saved_path: Option<PathBuf>,
    closing: bool,
    close_shown: bool,
    close_result: Option<Result<Option<String>, String>>,
}
impl Recovery {
    fn request(&mut self, job: Job) {
        if self.pending.is_some() {
            return;
        }
        match self
            .worker
            .as_ref()
            .ok_or("Recovery worker unavailable".to_owned())
            .and_then(|worker| worker.request(job))
        {
            Ok(id) => {
                self.pending = Some(id);
            }
            Err(error) => self.message = Some(error),
        }
    }
    fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        self.pending = None;
    }
}
fn default_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/state"))
        })
        .ok_or("Recovery needs an absolute XDG_STATE_HOME or HOME directory")?;
    Ok(base.join("omatainer/recovery"))
}
impl App {
    pub(super) fn recovery_toolbar_text(&self) -> String {
        let Some(status) = self.recovery.worker.as_ref().map(Worker::status) else {
            return if self.recovery.message.is_some() {
                "Recovery unavailable"
            } else {
                "Recovery"
            }
            .into();
        };
        if status.warning {
            return "Recovery warning".into();
        }
        match &status.durable {
            Some(durable) if Some(durable.epoch) == self.recovery.epoch => format!(
                "Recovery · {}",
                durable_age(worker::unix_ms(), durable.unix_ms)
            ),
            _ => "Recovery · pending".into(),
        }
    }
    pub(super) fn start_default_recovery(&mut self) {
        match default_path() {
            Ok(path) => self.start_recovery(path),
            Err(error) => {
                self.recovery.message = Some(error);
                self.recovery.open = true;
            }
        }
    }
    pub(super) fn start_recovery(&mut self, root: PathBuf) {
        self.recovery.root = Some(root.clone());
        match Worker::start(root, self.engine.project.clone()) {
            Ok(worker) => self.recovery.worker = Some(worker),
            Err(error) => {
                self.recovery.message = Some(format!("Recovery worker unavailable: {error}"));
                self.recovery.open = true;
            }
        }
    }
    pub(super) fn suspend_recovery(&mut self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.recovery.worker.as_ref().map(|worker| {
            worker.pause(true);
            worker.capture_fence()
        })
    }
    pub(super) fn sync_recovery(&mut self) {
        if self.recovery.worker.is_none() {
            return;
        }
        let paused = self.recovery_project_busy() || self.recovery.closing;
        if paused {
            self.suspend_recovery();
            return;
        }
        let checkpoint = self.engine.undo.checkpoint();
        let revision = self.engine.project.revision();
        let view = self.project_view();
        let config = self.settings.profile().recovery.clone();
        let path = self.recovery_project_path();
        let changed = self.recovery.epoch != Some(checkpoint.epoch)
            || self.recovery.engine_revision != revision
            || self.recovery.view.as_ref() != Some(&view)
            || self.recovery.config.as_ref() != Some(&config)
            || self.recovery.saved_path != path;
        if changed {
            if self.recovery.epoch != Some(checkpoint.epoch)
                || self.recovery.view.as_ref() != Some(&view)
            {
                self.recovery.view_revision = self.recovery.view_revision.saturating_add(1);
            }
            let identities = self.project_watch_identities();
            self.recovery
                .worker
                .as_ref()
                .unwrap()
                .update(worker::Update {
                    epoch: checkpoint.epoch,
                    view_revision: self.recovery.view_revision,
                    view: view.clone(),
                    identities,
                    saved_path: path.clone(),
                    config: config.clone(),
                });
            self.recovery.epoch = Some(checkpoint.epoch);
            self.recovery.engine_revision = revision;
            self.recovery.view = Some(view);
            self.recovery.config = Some(config);
            self.recovery.saved_path = path;
        }
        self.recovery.worker.as_ref().unwrap().pause(false);
    }
    pub(super) fn poll_recovery(&mut self) {
        while let Some(output) = self.recovery.worker.as_ref().and_then(Worker::poll) {
            if output.id != 0 && self.recovery.pending != Some(output.id) {
                continue;
            }
            if output.id != 0 {
                self.recovery.pending = None;
            }
            match output.event {
                Event::Listed(Ok(inventory)) => {
                    self.recovery.candidates = inventory.candidates;
                    self.recovery.warnings = inventory.warnings;
                    if output.id == 0
                        && (!self.recovery.candidates.is_empty()
                            || !self.recovery.warnings.is_empty())
                    {
                        self.recovery.open = true;
                    }
                }
                Event::Listed(Err(error)) => {
                    self.recovery.message = Some(error);
                    if output.id == 0 {
                        self.recovery.open = true;
                    }
                }
                Event::Inspected(result) => match result {
                    Ok(preview) => self.recovery.preview = Some(preview),
                    Err(error) => {
                        self.recovery.message = Some(format!(
                            "Preview refused: {error}. Current project is unchanged."
                        ))
                    }
                },
                Event::Removed(result) => {
                    self.recovery.message = Some(match result {
                        Ok(None) => {
                            "Recovery session deleted. Explicit project files were not changed."
                                .into()
                        }
                        Ok(Some(warning)) => {
                            format!("Recovery deletion committed with a warning: {warning}")
                        }
                        Err(error) => format!("Recovery deletion failed: {error}"),
                    });
                    self.recovery.preview = None;
                    self.recovery.request(Job::List);
                }
                Event::Retired(result) => self.recovery.close_result = Some(result),
            }
        }
    }
    pub(super) fn recovery_ui(&mut self, ctx: &egui::Context) {
        if !self.recovery.open {
            return;
        }
        let mut open = true;
        let mut job = None;
        let mut restore = None;
        let mut cancel = false;
        let busy = self.recovery.pending.is_some() || self.recovery_project_busy();
        let status = self.recovery.worker.as_ref().map(Worker::status);
        egui::Window::new("Autosave and recovery").open(&mut open).default_width(570.0).show(ctx, |ui| {
            ui.label("Recovery keeps batched edit-state copies, including untitled projects. It does not save or overwrite your explicit .omat file.");
            ui.label("Dirty state is normally captured about every 2 seconds. Capture, queue pressure and storage I/O can increase the loss window; only the confirmed durable time below is evidence of recovery coverage.");
            if let Some(path) = &self.recovery.root { ui.label(format!("Storage: {}", path.display())); }
            if let Some(status) = &status {
                let durable = match &status.durable {
                    Some(durable) if Some(durable.epoch) == self.recovery.epoch => format!("Last confirmed durable state: {} · revision {} · record {}", durable_age(worker::unix_ms(), durable.unix_ms), durable.revision, durable.sequence),
                    Some(_) => "The prior document has a durable copy; this document has no confirmed durable record yet.".into(),
                    None => "No confirmed durable record for this session yet.".into(),
                };
                ui.label(durable).help(ui, HelpControl::RecoveryStatus);
                ui.label(format!("Observed storage use: {:.1} MiB · {}", status.usage_bytes as f64 / crate::recovery::MIB as f64, status.message));
                let unsaved = status.durable.as_ref().is_none_or(|durable| Some(durable.epoch) != self.recovery.epoch || durable.revision != self.engine.project.revision() || durable.view_revision != self.recovery.view_revision);
                if unsaved { ui.label("Newer current edits are not yet confirmed durable."); }
                if status.busy { ui.spinner(); }
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Refresh recovery list")).help(ui, HelpControl::RecoveryRefresh).clicked() { job = Some(Job::List); }
                if ui.add_enabled(!busy && self.recovery.worker.is_some(), egui::Button::new("Journal current edits now")).help(ui, HelpControl::RecoveryNow).clicked() {
                    if let Some(worker) = &self.recovery.worker { worker.force(); }
                }
                if ui.add_enabled(self.recovery.pending.is_some(), egui::Button::new("Cancel recovery operation")).help(ui, HelpControl::RecoveryCancel).clicked() { cancel = true; }
            });
            if let Some(message) = &self.recovery.message { ui.label(message); }
            for warning in &self.recovery.warnings { ui.label(warning); }
            ui.separator();
            ui.label("Recoverable sessions (active sessions stay locked and are not offered)");
            if self.recovery.candidates.is_empty() { ui.label("No inactive recovery candidates found."); }
            let scroll = egui::ScrollArea::vertical().id_salt("recovery-candidates").max_height(180.0).show(ui, |ui| {
                for candidate in &self.recovery.candidates {
                    ui.push_id((&candidate.session, candidate.sequence), |ui| {
                        ui.label(format!("{} · record {} · revision {}", candidate.session, candidate.sequence, candidate.metadata.revision));
                        ui.label(candidate.metadata.saved_path.as_ref().map(|path| format!("Original saved path: {}", path.display())).unwrap_or("Untitled project".into()));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!busy, egui::Button::new("Preview recovery")).help(ui, HelpControl::RecoveryPreview).clicked() { job = Some(Job::Inspect(candidate.clone())); }
                            if ui.add_enabled(!busy, egui::Button::new("Delete this recovery session…")).help(ui, HelpControl::RecoveryDelete).clicked() { self.recovery.remove = Some(candidate.clone()); }
                        });
                    });
                }
            });
            accessibility::scrollbars(ui, "Recovery candidates", &scroll);
            if let Some(preview) = &self.recovery.preview {
                ui.separator(); ui.heading("Recovery preview");
                ui.label(format!("{} notes · {} embedded media · {} BPM", preview.notes, preview.media, preview.bpm));
                ui.label("Restore opens playback stopped as an unsaved untitled copy. Your current unsaved changes get the ordinary Save / Discard / Cancel decision. The original saved file is never overwritten.");
                for report in &preview.report { ui.label(report); }
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new("Restore as untitled copy")).help(ui, HelpControl::RecoveryRestore).clicked() { restore = Some(preview.candidate.clone()); }
                    if ui.button("Close recovery preview").help(ui, HelpControl::RecoveryCancel).clicked() { cancel = true; }
                });
            }
        });
        if !open {
            cancel = true;
        }
        if cancel {
            self.recovery.cancel();
            self.recovery.preview = None;
        }
        self.recovery.open = open;
        if let Some(job) = job {
            self.recovery.request(job);
        }
        if let Some(candidate) = restore {
            self.recovery.preview = None;
            self.recovery.open = false;
            self.request_recovery_restore(candidate);
        }
        if let Some(candidate) = self.recovery.remove.clone() {
            keyboard::block_for_dialog(ctx);
            let modal = egui::Modal::new(egui::Id::new("recovery-delete")).show(ctx, |ui| {
                ui.heading("Delete every recovery generation in this session?");
                ui.label(format!("Session {}. This permanently deletes its journal, checkpoints and media sidecars. Explicit saved project files are unaffected.", candidate.session));
                if ui.button("Delete recovery session").help(ui, HelpControl::RecoveryDelete).clicked() { self.recovery.request(Job::Remove(candidate.clone())); self.recovery.remove = None; }
                if ui.button("Cancel deletion").help(ui, HelpControl::RecoveryCancel).clicked() { self.recovery.remove = None; }
            });
            if modal.should_close() {
                self.recovery.remove = None;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    pub(super) fn begin_recovery_close(&mut self, ctx: &egui::Context) {
        if self.recovery.worker.is_none() {
            self.finish_project_close(ctx);
            return;
        }
        self.recovery.cancel();
        self.suspend_recovery();
        self.recovery.closing = true;
        self.recovery.close_result = None;
        self.recovery.request(Job::Retire);
        if self.recovery.pending.is_none() {
            self.recovery.close_result = Some(Err(self
                .recovery
                .message
                .clone()
                .unwrap_or("Recovery close unavailable".into())));
        }
    }
    pub(super) fn cancel_recovery_close(&mut self) {
        if !self.recovery.closing {
            return;
        }
        self.recovery.cancel();
        self.recovery.closing = false;
        self.recovery.close_shown = false;
        self.recovery.close_result = None;
        if let Some(worker) = &self.recovery.worker {
            worker.resume_after_close();
        }
    }
    pub(super) fn recovery_close_ui(&mut self, ctx: &egui::Context) {
        if !self.recovery.closing {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let ready = matches!(self.recovery.close_result, Some(Ok(None)));
        let mut keep = false;
        let mut close = false;
        let mut retry = false;
        if !ready || self.recovery.close_shown {
            self.recovery.close_shown = true;
            egui::Window::new("Finishing recovery before exit").collapsible(false).show(ctx, |ui| {
                ui.label("Your Save / Discard decision was accepted. Retiring this recovery epoch prevents it from being offered as interrupted work after an intentional exit.");
                match &self.recovery.close_result {
                    None => { ui.label("Waiting for recovery retirement…"); },
                    Some(Ok(None)) => { ui.label("Recovery retirement confirmed durable."); },
                    Some(Ok(Some(warning))) => { ui.label(format!("Retirement committed, durability uncertain: {warning}")); },
                    Some(Err(error)) => { ui.label(format!("Retirement failed: {error}. Recovery may be offered at next startup.")); },
                }
                if ui.add_enabled(self.recovery.pending.is_none(), egui::Button::new("Retry recovery retirement")).help(ui, HelpControl::RecoveryRetry).clicked() { retry = true; }
                let response = ui.button("Keep working").help(ui, HelpControl::RecoveryKeepWorking);
                keep = response.clicked() || response.is_pointer_button_down_on();
                close = ui.button("Close and retain possible recovery").help(ui, HelpControl::RecoveryClose).clicked();
            });
        }
        if keep {
            self.cancel_project_close();
        } else if ready || close {
            self.recovery.closing = false;
            self.finish_project_close(ctx);
        } else if retry {
            self.recovery.close_result = None;
            self.recovery.request(Job::Retire);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(25));
    }
}

fn durable_age(now: u64, written: u64) -> String {
    if now < written {
        "clock moved backward; age unavailable".into()
    } else {
        format!("{} s ago", (now - written) / 1000)
    }
}
#[cfg(test)]
mod tests;
