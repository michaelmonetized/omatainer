//! Local support inspection and explicit safe-mode restart. No uploader exists.
use super::*;
use crate::support::{
    self as model,
    operations::{Job, Preview, ResultValue, Selection, Worker},
};
use std::sync::atomic::AtomicBool;

pub(super) struct Panel {
    pub open: bool,
    client: Option<model::worker::Client>,
    root: Option<PathBuf>,
    worker: Option<Worker>,
    preview: Option<Preview>,
    previous: Vec<model::storage::Previous>,
    found: Option<crate::recovery::Candidate>,
    selection: Selection,
    consent: bool,
    path: String,
    message: String,
    next_sample: Instant,
    seen_backend_errors: u64,
    seen_audio_generation: u64,
    seen_recovery_failures: u64,
    last_recovery: Option<([u8; 32], u64)>,
    pub restart: Arc<AtomicBool>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            client: None,
            root: None,
            worker: None,
            preview: None,
            previous: Vec::new(),
            found: None,
            selection: Selection::default(),
            consent: false,
            path: "support-report.omasupport.json".into(),
            message: String::new(),
            next_sample: Instant::now(),
            seen_backend_errors: 0,
            seen_audio_generation: 0,
            seen_recovery_failures: 0,
            last_recovery: None,
            restart: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Panel {
    #[cfg(test)]
    pub(super) fn message_for_native_test(&self) -> &str {
        &self.message
    }
    fn request(&mut self, job: Job) {
        match self
            .worker
            .as_mut()
            .ok_or_else(|| "Support file worker unavailable".to_string())
            .and_then(|worker| worker.request(job))
        {
            Ok(()) => self.message = "Support work pending; no upload is performed.".into(),
            Err(error) => self.message = error,
        }
    }
    fn poll(&mut self) {
        let Some(done) = self.worker.as_mut().and_then(Worker::poll) else {
            return;
        };
        let _claim = match done.work.as_ref().map(|work| work.commit()).transpose() {
            Ok(claim) => claim,
            Err(error) => {
                self.message = format!("Support result cancelled: {error}");
                return;
            }
        };
        match done.result {
            Ok(ResultValue::Preview(preview))=>{self.preview=Some(preview);self.consent=false;self.found=None;self.message="Review the exact redacted JSON below before exporting locally. Nothing is uploaded.".into();},
            Ok(ResultValue::Previous(inventory))=>{
                self.message=format!("{} prior reports; {} active runs skipped; {} unreadable; {} incomplete. An unclean exit does not identify its cause.",inventory.previous.len(),inventory.skipped_active,inventory.unreadable,inventory.incomplete);
                self.previous=inventory.previous;
            },
            Ok(ResultValue::Exported(model::storage::Published::Durable))=>self.message="Local support report exported and synced. No upload occurred.".into(),
            Ok(ResultValue::Exported(model::storage::Published::CommittedWarning))=>self.message="Local support report committed, but directory durability could not be confirmed. No upload occurred; do not retry as an unsaved failure.".into(),
            Ok(ResultValue::Recovery(candidate))=>{
                self.message=if candidate.is_some(){"Exact retained recovery record verified. Open its recovery preview before deciding whether to restore."}else{"The exact referenced recovery was retired or pruned. No newer record was substituted; use Autosave and recovery to inspect other copies."}.into();
                self.found=candidate;
            },
            Err(error)=>self.message=error.to_string(),
        }
    }
}
impl App {
    pub(crate) fn initialize_support(
        &mut self,
        client: Option<model::worker::Client>,
        root: PathBuf,
        restart: Arc<AtomicBool>,
    ) {
        self.support.client = client;
        self.support.root = Some(root);
        self.support.restart = restart;
        match Worker::new(self.engine.cmd.performance().clone()) {
            Ok(worker) => self.support.worker = Some(worker),
            Err(_) => {
                self.support.message =
                    "Support file worker unavailable; the session remains usable.".into()
            }
        }
    }
    pub(super) fn observe_support(&mut self) {
        let Some(client) = &self.support.client else {
            return;
        };
        if let Some(reference) = self.support_recovery_reference() {
            let identity = (reference.session, reference.sequence);
            if self.support.last_recovery != Some(identity) {
                client.port.recovery(reference);
                self.support.last_recovery = Some(identity);
            }
        }
        if let Some((count, class)) = self.support_recovery_failure() {
            if count > self.support.seen_recovery_failures {
                client
                    .port
                    .event(model::Code::RecoveryWriteFailed, Some(class));
                self.support.seen_recovery_failures = count;
            }
        }
        if Instant::now() < self.support.next_sample {
            return;
        }
        self.support.next_sample = Instant::now() + std::time::Duration::from_secs(1);
        let audio = self.engine.cmd.audio_metrics();
        let midi = self.engine.midi.input_stats();
        if audio.backend_errors > self.support.seen_backend_errors {
            client.port.event(
                model::Code::AudioBackendFailed,
                Some(model::FailureClass::Unavailable),
            );
            self.support.seen_backend_errors = audio.backend_errors;
        }
        if let Some(status) = self.engine.audio_handle().map(|handle| handle.status()) {
            if status.generation != self.support.seen_audio_generation
                && status.phase == crate::engine::audio::owner::Phase::Offline
                && !self.engine.safe_mode()
            {
                client.port.event(
                    model::Code::AudioOutputOffline,
                    Some(model::FailureClass::Unavailable),
                );
            }
            self.support.seen_audio_generation = status.generation;
        }
        client.port.sample(model::Sample {
            elapsed_ms: 0,
            audio: audio.into(),
            commands: self.engine.cmd.queue_pressure().into(),
            midi: [
                midi.received,
                midi.queued,
                midi.dispatched,
                midi.coalesced,
                midi.dropped,
                midi.resets,
                midi.oversized,
                midi.disconnected,
            ],
            ui_update_ns: self.diagnostics.ui_update_ns,
        });
        client.port.routes(
            Some(model::Route::requested(&self.settings.profile().audio)),
            self.engine
                .output_info()
                .map(|info| model::Route::active(&info.plan)),
        );
    }
    pub(super) fn support_ui(&mut self, ctx: &egui::Context) {
        self.support.poll();
        if self.engine.safe_mode() {
            egui::TopBottomPanel::top("safe-mode-banner").show(ctx,|ui|{
                ui.horizontal_wrapped(|ui|{
                    ui.strong("SAFE MODE · Audio and MIDI offline · Engine controls disabled");
                    ui.label("Project Open, recovery and Save remain available. Saved settings are unchanged.");
                    if ui.add_enabled(!self.project.committing()&&!self.recovery_project_busy(),egui::Button::new("Restart normally")).help(ui,HelpControl::SupportRestart).clicked(){self.request_normal_restart();}
                });
            });
        }
        if !self.support.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let busy = self.support.worker.as_ref().is_some_and(Worker::busy);
        let mut open = true;
        let mut job = None;
        let mut preview_recovery = None;
        let recovery_root = self.support_recovery_root();
        egui::Window::new("Support and crash reports").id(egui::Id::new("support-window")).open(&mut open).default_width(760.0).default_height(730.0).vscroll(true).show(ctx,|ui|{
            ui.label("Local-only diagnostics. No upload client exists. Review the exact JSON, export it, and choose separately whether to share that file.");
            ui.label("Always excluded: media/PCM/waveforms, projects/notes, paths/titles, device and MIDI port names, raw errors/panic payloads, environment/credentials and core dumps. Audio plugin hosting is unavailable in this build.");
            ui.push_id("support-live-status",|ui|{
                if let Some(client)=&self.support.client {
                    let view=client.view();
                    ui.label(format!("Run {} · {} structured events · {} counter samples · {} observations dropped before collection",view.report.run.hex(),view.report.events.len(),view.report.samples.len(),view.dropped_observations));
                    if let Some(failure)=view.storage_failure {ui.label(format!("Persistent support storage unavailable ({failure:?}); in-memory inspection still works."));}
                    else if view.committed_warning {ui.label("Latest report committed; durability warning. Prior confirmed time remains authoritative.");}
                    else if let Some(time)=view.durable_unix_ms {ui.label(format!("Last confirmed report persistence: {time} ms since Unix epoch. Crash evidence can lag live state by five seconds or storage delay."));}
                } else {ui.label("Support collector unavailable; no persistent or current report is claimed.");}
            });
            ui.add_enabled_ui(!busy,|ui|{
                let before=self.support.selection;
                ui.horizontal_wrapped(|ui|{
                    ui.checkbox(&mut self.support.selection.routes,"Audio routing").help(ui,HelpControl::SupportCategories);
                    ui.checkbox(&mut self.support.selection.events,"Structured events").help(ui,HelpControl::SupportCategories);
                    ui.checkbox(&mut self.support.selection.performance,"Performance counters").help(ui,HelpControl::SupportCategories);
                    ui.checkbox(&mut self.support.selection.recovery,"Opaque recovery references").help(ui,HelpControl::SupportCategories);
                });
                if before!=self.support.selection{self.support.preview=None;self.support.consent=false;}
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(self.support.client.is_some(),egui::Button::new("Inspect current report")).help(ui,HelpControl::SupportInspect).clicked(){job=Some(Job::Preview(self.support.client.as_ref().unwrap().view().report.clone(),self.support.selection));}
                    if ui.button("Find previous runs").help(ui,HelpControl::SupportPrevious).clicked(){if let Some(root)=&self.support.root{job=Some(Job::Previous(root.clone()));}}
                });
                ui.push_id("previous-support-reports",|ui|{
                    for (index,previous) in self.support.previous.iter().enumerate(){
                        let report=&previous.report;
                        if ui.button(format!("Inspect {:?} run {} · {}",report.exit,&report.run.hex()[..8],report.started_unix_ms)).help(ui,HelpControl::SupportPrevious).clicked(){job=Some(Job::Preview(Arc::new(report.clone()),self.support.selection));}
                        let _=index;
                    }
                });
                let label=ui.label("Support report file");
                let field=ui.text_edit_singleline(&mut self.support.path).labelled_by(label.id);
                field.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Support report file"));
                field.help(ui,HelpControl::SupportPath);
                if ui.add_enabled(!self.support.path.trim().is_empty(),egui::Button::new("Reopen support report")).help(ui,HelpControl::SupportReopen).clicked(){job=Some(Job::Reopen(PathBuf::from(self.support.path.trim())));}
            });
            ui.push_id("support-outcome",|ui|{ui.label(&self.support.message);});
            if busy {
                ui.spinner();
                if ui.button("Cancel support work").help(ui,HelpControl::SupportCancel).clicked(){if let Some(worker)=&self.support.worker{worker.cancel();}}
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
            if let Some(preview)=&self.support.preview {
                ui.label(format!("Reviewed report: {:?} · {} bytes. Unknown cause stays unclean; an observed Rust panic is not proof of an audio/device fault.",preview.report.exit,preview.json.len()));
                ui.push_id("support-json",|ui|{
                    let output=egui::ScrollArea::vertical().id_salt("json-scroll").max_height(220.0).show_rows(ui,16.0,preview.lines.len(),|ui,rows|{
                        for row in rows {ui.monospace(&preview.json[preview.lines[row].clone()]);}
                    });
                    accessibility::scrollbars(ui,"Support report text",&output);
                });
                ui.add_enabled_ui(!busy,|ui|{
                    ui.checkbox(&mut self.support.consent,"I reviewed this report and want to export it locally").help(ui,HelpControl::SupportConsent);
                    if ui.add_enabled(self.support.consent&&!self.support.path.trim().is_empty(),egui::Button::new("Export reviewed support report")).help(ui,HelpControl::SupportExport).clicked(){job=Some(Job::Export(PathBuf::from(self.support.path.trim()),preview.report.clone()));}
                    for reference in &preview.report.recovery {
                        if ui.button(format!("Find exact recovery record {}",reference.sequence)).help(ui,HelpControl::SupportRecovery).clicked(){
                            if let Some(root)=&recovery_root {job=Some(Job::Recovery(root.clone(),reference.clone()));}
                            else {self.support.message="Recovery storage is unavailable; no original project was opened.".into();}
                        }
                    }
                });
            }
            if let Some(candidate)=&self.support.found {
                if ui.add_enabled(!busy&&!self.recovery_project_busy(),egui::Button::new("Preview linked recovery")).help(ui,HelpControl::SupportRecovery).clicked(){preview_recovery=Some(candidate.clone());}
            }
        });
        if let Some(job) = job {
            self.support.request(job);
        }
        let opening_recovery = preview_recovery.is_some();
        if let Some(candidate) = preview_recovery {
            self.preview_support_recovery(candidate);
            self.support.open = false;
        }
        if self.support.open && !open {
            if let Some(worker) = &self.support.worker {
                worker.cancel();
            }
        }
        self.support.open = open && !opening_recovery;
    }
}

#[cfg(test)]
mod tests;
