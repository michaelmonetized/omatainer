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
    discovery_complete: bool,
    discovery_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    preview_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
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
                durable_age(worker::unix_ms(), durable.captured_unix_ms)
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
        match Worker::start_with_scan(
            root,
            self.engine.project.clone(),
            self.engine.cmd.performance(),
            !self.engine.safe_mode(),
        ) {
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
                || self.recovery.saved_path != path
                || self.recovery.config.as_ref() != Some(&config)
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
        use std::sync::atomic::Ordering;
        let expired_list = self
            .recovery
            .discovery_cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        let expired_preview = self
            .recovery
            .preview_cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        if expired_list || expired_preview {
            self.recovery.preview = None;
            self.recovery.preview_cancel = None;
            if expired_list {
                self.recovery.candidates.clear();
                self.recovery.discovery_complete = false;
                self.recovery.discovery_cancel = None;
            }
            self.recovery.message = Some("Recovery inspection expired when performance protection changed. Leave protection and Refresh recovery list for a fresh verified selection.".into());
        }
        while let Some(output) = self.recovery.worker.as_ref().and_then(Worker::poll) {
            if output.id != 0 && self.recovery.pending != Some(output.id) {
                // Cancellation cannot undo an already committed deletion. Its
                // result remains truthful even after a newer UI job was chosen.
                if let Event::Removed(Ok(warning)) = output.event {
                    self.recovery.message = Some(format!("Earlier recovery deletion committed before cancellation. Explicit project files were not changed. {}", warning.unwrap_or_default()));
                    self.recovery.candidates.clear();
                    self.recovery.preview = None;
                    self.recovery.discovery_cancel = None;
                    self.recovery.preview_cancel = None;
                    self.recovery.discovery_complete = false;
                }
                continue;
            }
            if output.id != 0 {
                self.recovery.pending = None;
            }
            match output.event {
                Event::Listed(result, work) => {
                    let publication = work.as_ref().map(|work| work.commit()).transpose();
                    match publication {
                        Err(error) => {
                            self.recovery.message = Some(format!("Recovery scan deferred or cancelled: {error}. Leave protection and Refresh recovery list; copies may exist."));
                            self.recovery.open = true;
                        }
                        Ok(_publication) => match result {
                            Ok(inventory) => {
                                self.recovery.discovery_complete = work.is_some();
                                self.recovery.discovery_cancel =
                                    work.as_ref().map(|work| work.cancel());
                                self.recovery.candidates = inventory.candidates;
                                self.recovery.warnings = inventory.warnings;
                                if output.id == 0
                                    && (!self.recovery.candidates.is_empty()
                                        || !self.recovery.warnings.is_empty())
                                {
                                    self.recovery.open = true;
                                }
                            }
                            Err(error) => {
                                self.recovery.message = Some(error);
                                if output.id == 0 {
                                    self.recovery.open = true;
                                }
                            }
                        },
                    }
                }
                Event::Inspected(result) => match result {
                    Ok((preview, work)) => match work.commit() {
                        Ok(_publication) => {
                            self.recovery.preview_cancel = Some(work.cancel());
                            self.recovery.preview = Some(preview);
                        }
                        Err(error) => {
                            self.recovery.message = Some(format!(
                                "Recovery preview was cancelled by performance protection: {error}"
                            ))
                        }
                    },
                    Err(error) => {
                        self.recovery.message = Some(format!(
                            "Preview refused: {error}. Current project is unchanged."
                        ))
                    }
                },
                Event::Removed(result) => {
                    let outcome = match result {
                        Ok(None) => {
                            "Recovery session deleted. Explicit project files were not changed."
                                .into()
                        }
                        Ok(Some(warning)) => {
                            format!("Recovery deletion committed with a warning: {warning}")
                        }
                        Err(error) => format!("Recovery deletion failed: {error}"),
                    };
                    self.recovery.preview = None;
                    self.recovery.candidates.clear();
                    self.recovery.discovery_cancel = None;
                    self.recovery.preview_cancel = None;
                    self.recovery.discovery_complete = false;
                    self.recovery.message = None;
                    self.refresh_recovery();
                    // A later discovery refusal cannot erase the filesystem
                    // operation's already committed outcome or cleanup warning.
                    let refresh_notice = self.recovery.message.take();
                    self.recovery.message = Some(match refresh_notice {
                        Some(notice) => format!("{outcome} {notice}"),
                        None => outcome,
                    });
                }
                Event::Retired(result) => self.recovery.close_result = Some(result),
            }
        }
    }
    #[cfg(test)]
    pub(super) fn force_support_recovery_test(&self) {self.recovery.worker.as_ref().unwrap().force();}
    #[cfg(test)]
    pub(super) fn stop_support_recovery_test(&mut self) {if let Some(worker)=self.recovery.worker.take(){worker.stop_for_test();}}
    pub(super) fn support_recovery_failure(&self)->Option<(u64,crate::support::FailureClass)> {let status=self.recovery.worker.as_ref()?.status();Some((status.write_failures,status.last_write_failure?))}
    pub(super) fn support_recovery_root(&self)->Option<PathBuf> {self.recovery.root.clone()}
    pub(super) fn support_recovery_reference(&self)->Option<crate::support::RecoveryRef> {
        let durable=self.recovery.worker.as_ref()?.status().durable.clone()?;
        Some(crate::support::RecoveryRef{session:durable.session,epoch:durable.epoch,sequence:durable.sequence,revision:durable.revision,view_revision:durable.view_revision,captured_unix_ms:durable.captured_unix_ms,committed_unix_ms:durable.committed_unix_ms})
    }
    pub(super) fn preview_support_recovery(&mut self,candidate:crate::recovery::Candidate) {
        self.recovery.open=true;
        match self.engine.cmd.performance().optional_work() {
            Ok(work)=>self.recovery.request(Job::Inspect(candidate,work)),
            Err(error)=>self.recovery.message=Some(format!("Recovery preview not accepted: {error}")),
        }
    }
    fn refresh_recovery(&mut self) {
        match self.engine.cmd.performance().optional_work() {
            Ok(work) => self.recovery.request(Job::List(work)),
            Err(error) => self.recovery.message = Some(format!("Recovery discovery deferred: {error}. Copies may exist; leave protection and Refresh to verify.")),
        }
    }
    pub(super) fn recovery_ui(&mut self, ctx: &egui::Context) {
        if !self.recovery.open {
            return;
        }
        let mut open = true;
        let mut refresh = false;
        let mut restore = None;
        let mut inspect = None;
        let mut cancel = false;
        let busy = self.recovery.pending.is_some() || self.recovery_project_busy();
        let status = self.recovery.worker.as_ref().map(Worker::status);
        egui::Window::new(tr!("Autosave and recovery")).id(egui::Id::new("Autosave and recovery")).open(&mut open).default_width(570.0).default_height(700.0).vscroll(true).show(ctx, |ui| {
            ui.label(tr!("Recovery keeps batched edit-state copies, including untitled projects. It does not save or overwrite your explicit .omat file."));
            ui.label(tr!("Dirty state is normally captured about every 2 seconds. Capture, queue pressure and storage I/O can increase the loss window; only the confirmed durable time below is evidence of recovery coverage."));
            if let Some(path) = &self.recovery.root { ui.label({ let __omatainer_args = (&(path.display()),); crate::localization::format("Storage: {}", &[format!("{}", __omatainer_args.0)]) }); }
            // Dynamic labels/spinners stay in one scope. Their auto-ID count
            // must not change the identity of a previously exposed action.
            ui.push_id("recovery-status", |ui| {
            if let Some(status) = &status {
                let durable = match &status.durable {
                    Some(durable) if Some(durable.epoch) == self.recovery.epoch => format!("Confirmed durable captured state: {} · revision {} · record {}", durable_age(worker::unix_ms(), durable.captured_unix_ms), durable.revision, durable.sequence),
                    Some(_) => "The prior document has a durable copy; this document has no confirmed durable record yet.".into(),
                    None => "No confirmed durable record for this session yet.".into(),
                };
                ui.label(durable).help(ui, HelpControl::RecoveryStatus);
                if let Some(durable) = &status.durable {
                    ui.label({ let __omatainer_args = (&(durable_age(worker::unix_ms(), durable.committed_unix_ms)),); crate::localization::format("Durable commit acknowledgment: {}", &[format!("{}", __omatainer_args.0)]) });
                }
                ui.label(match status.usage_bytes {
                    Some(bytes) => { let __omatainer_args = (&(bytes as f64 / crate::recovery::MIB as f64),); crate::localization::format("Last write storage accounting: {:.1} MiB of logical files (may be an upper bound; warnings apply)", &[format!("{:.1}", __omatainer_args.0)]) },
                    None => tr!("Current recovery storage usage has not been measured by a successful write.").into(),
                });
                ui.label(&status.message);
                let unsaved = status.durable.as_ref().is_none_or(|durable| Some(durable.epoch) != self.recovery.epoch || durable.revision != self.engine.project.revision() || durable.view_revision != self.recovery.view_revision);
                if unsaved { ui.label(tr!("Newer current state or recovery metadata is not yet confirmed durable.")); }
                if status.busy { ui.spinner(); }
            }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(tr!("Refresh recovery list"))).help(ui, HelpControl::RecoveryRefresh).clicked() { refresh = true; }
                if ui.add_enabled(!busy && self.recovery.worker.is_some(), egui::Button::new(tr!("Journal current edits now"))).help(ui, HelpControl::RecoveryNow).clicked() {
                    if let Some(worker) = &self.recovery.worker { worker.force(); }
                }
                if ui.add_enabled(self.recovery.pending.is_some(), egui::Button::new(tr!("Cancel recovery operation"))).help(ui, HelpControl::RecoveryCancel).clicked() { cancel = true; }
            });
            ui.push_id("recovery-message", |ui| {
                if let Some(message) = &self.recovery.message { ui.label(message); }
            });
            show_report(ui, "Recovery discovery report", &self.recovery.warnings);
            ui.separator();
            ui.push_id("recovery-candidate-panel", |ui| {
            ui.label(tr!("Recoverable sessions (active sessions stay locked and are not offered)"));
            if self.recovery.candidates.is_empty() { ui.label(if self.recovery.discovery_complete { tr!("No inactive recovery candidates found.") } else { tr!("Recovery candidates have not been verified. Copies may exist; use Refresh recovery list in Studio.") }); }
            let scroll = egui::ScrollArea::vertical().id_salt("recovery-candidates").max_height(180.0).show(ui, |ui| {
                for candidate in &self.recovery.candidates {
                    ui.push_id((&candidate.session, candidate.sequence), |ui| {
                        ui.label({ let __omatainer_args = (&(candidate.session),&(candidate.sequence),&(candidate.metadata.revision),); crate::localization::format("{} · record {} · revision {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                        ui.label(candidate.metadata.saved_path.as_ref().map(|path| { let __omatainer_args = (&(path.display()),); crate::localization::format("Original saved path: {}", &[format!("{}", __omatainer_args.0)]) }).unwrap_or(tr!("Untitled project").into()));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!busy, egui::Button::new(tr!("Preview recovery"))).help(ui, HelpControl::RecoveryPreview).clicked() { inspect = Some(candidate.clone()); }
                            if ui.add_enabled(!busy, egui::Button::new(tr!("Delete this recovery session…"))).help(ui, HelpControl::RecoveryDelete).clicked() { self.recovery.remove = Some(candidate.clone()); }
                        });
                    });
                }
            });
            accessibility::scrollbars(ui, "Recovery candidates", &scroll);
            });
            if let Some(preview) = &self.recovery.preview {
                ui.separator(); ui.heading(tr!("Recovery preview"));
                ui.label({ let __omatainer_args = (&(preview.notes),&(preview.media),&(preview.bpm),); crate::localization::format("{} notes · {} embedded media · {} BPM", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                ui.label(tr!("Restore opens playback stopped as an unsaved untitled copy. Your current unsaved changes get the ordinary Save / Discard / Cancel decision. The original saved file is never overwritten."));
                show_report(ui, "Recovery preview report", &preview.report);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(tr!("Restore as untitled copy"))).help(ui, HelpControl::RecoveryRestore).clicked() { restore = Some(preview.candidate.clone()); }
                    if ui.button(tr!("Close recovery preview")).help(ui, HelpControl::RecoveryCancel).clicked() { cancel = true; }
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
        if let Some(candidate) = inspect {
            match self.engine.cmd.performance().optional_work() {
                Ok(work) => self.recovery.request(Job::Inspect(candidate, work)),
                Err(error) => {
                    self.recovery.message =
                        Some(format!("Recovery preview was not accepted: {error}"))
                }
            }
        }
        if refresh {
            self.refresh_recovery();
        }
        if let Some(candidate) = restore {
            self.recovery.preview = None;
            self.recovery.open = false;
            self.request_recovery_restore(candidate);
        }
        if let Some(candidate) = self.recovery.remove.clone() {
            keyboard::block_for_dialog(ctx);
            let modal = egui::Modal::new(egui::Id::new("recovery-delete")).show(ctx, |ui| {
                ui.heading(tr!("Delete every recovery generation in this session?"));
                ui.label({ let __omatainer_args = (&(candidate.session),); crate::localization::format("Session {}. This permanently deletes its journal, checkpoints and media sidecars. Explicit saved project files are unaffected.", &[format!("{}", __omatainer_args.0)]) });
                if ui.button(tr!("Delete recovery session")).help(ui, HelpControl::RecoveryDelete).clicked() { match self.engine.cmd.performance().optional_work() {
                    Ok(work) => self.recovery.request(Job::Remove(candidate.clone(), work)),
                    Err(error) => self.recovery.message = Some(format!("Recovery deletion was not accepted: {error}")),
                } self.recovery.remove = None; }
                if ui.button(tr!("Cancel deletion")).help(ui, HelpControl::RecoveryCancel).clicked() { self.recovery.remove = None; }
            });
            if modal.should_close() {
                self.recovery.remove = None;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    pub(super) fn restored_recovery_report(&mut self, report: Vec<String>) -> usize {
        let count = report.len();
        if count > 0 {
            self.recovery.warnings = report;
            self.recovery.open = true;
        }
        count
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
            egui::Window::new(tr!("Finishing recovery before exit")).id(egui::Id::new("Finishing recovery before exit")).collapsible(false).show(ctx, |ui| {
                ui.label(tr!("Your Save / Discard decision was accepted. Retiring this recovery epoch prevents it from being offered as interrupted work after an intentional exit."));
                match &self.recovery.close_result {
                    None => { ui.label(tr!("Waiting for recovery retirement…")); },
                    Some(Ok(None)) => { ui.label(tr!("Recovery retirement confirmed durable.")); },
                    Some(Ok(Some(warning))) => { ui.label(crate::localization::format("Retirement committed, durability uncertain: {warning}", &[format!("{}", warning)])); },
                    Some(Err(error)) => { ui.label(crate::localization::format("Retirement failed: {error}. Recovery may be offered at next startup.", &[format!("{}", error)])); },
                }
                if ui.add_enabled(self.recovery.pending.is_none(), egui::Button::new(tr!("Retry recovery retirement"))).help(ui, HelpControl::RecoveryRetry).clicked() { retry = true; }
                let response = ui.button(tr!("Keep working")).help(ui, HelpControl::RecoveryKeepWorking);
                keep = response.clicked() || response.is_pointer_button_down_on();
                close = ui.button(tr!("Close and retain possible recovery")).help(ui, HelpControl::RecoveryClose).clicked();
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

fn show_report(ui: &mut Ui, name: &str, rows: &[String]) {
    // One parent auto-ID whether the report is absent, short or scrollable.
    ui.push_id(name, |ui| {
        if rows.is_empty() {
            return;
        }
        ui.label({ let __omatainer_args = (&(rows.len()),); crate::localization::format("{name}: {} notices", &[format!("{}", name), format!("{}", __omatainer_args.0)]) });
        let scroll = egui::ScrollArea::vertical()
            .id_salt(name)
            .max_height(100.0)
            .show(ui, |ui| {
                for row in rows {
                    ui.label(row);
                }
            });
        accessibility::scrollbars(ui, name, &scroll);
    });
}
