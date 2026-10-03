//! Independent native template copies, hardware review and portable migration.
use super::*;
use crate::{
    portable_project as data, project_dependencies as dependencies, project_template as model,
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[cfg(test)]
mod tests;
pub(super) mod worker;

pub(crate) struct StartupSession {
    pub initial: crate::engine::InitialSession,
    pub view: project::UiState,
    pub metadata: Option<model::Metadata>,
}
/// Prepare the chosen startup session before opening any audio or MIDI backend.
/// `choice` selects empty or a validated native template; Demo returns no replacement.
pub(crate) fn startup_session(
    choice: &model::Startup,
    cancel: &AtomicBool,
) -> Result<Option<StartupSession>, String> {
    match choice {
        model::Startup::Demo => Ok(None),
        model::Startup::Empty => Ok(Some(StartupSession {
            initial: crate::engine::InitialSession::Empty,
            view: project::UiState::default(),
            metadata: None,
        })),
        model::Startup::Template { path } => {
            let (bundle, _) = worker::load(path, cancel)?;
            if bundle.state.metadata.kind != model::Kind::Project {
                return Err("Startup requires a project template".into());
            }
            let mut state = bundle.state.document.engine;
            state.validate_processor_storage(48_000)?;
            let layout = state
                .session
                .as_mut()
                .ok_or_else(|| "Template session identities are missing".to_string())?;
            layout.namespace = crate::engine::midi_edit::NoteId::new().words();
            layout.generation = layout
                .generation
                .checked_add(1)
                .ok_or_else(|| "Template generation exhausted".to_string())?;
            let mut view = bundle.state.document.view;
            view.deck_identities = std::array::from_fn(|_| None);
            Ok(Some(StartupSession {
                initial: crate::engine::InitialSession::Project {
                    state,
                    media: bundle.media,
                },
                view,
                metadata: Some(bundle.state.metadata),
            }))
        }
    }
}

use worker::{Job, Kind, Record, ResultData, Worker};

#[derive(Default)]
pub(super) struct Templates {
    pub open: bool,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    record: Option<Record>,
    archive: Option<(PathBuf, Arc<data::Manifest>)>,
    name: String,
    new_path: String,
    source_path: String,
    backup_path: String,
    archive_path: String,
    destination: String,
    pub hardware: Option<(model::Metadata, Option<model::Target>)>,
    message: String,
    pub(in crate::ui) error: Option<String>,
}
impl Drop for Templates {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Templates {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn loaded(&mut self, metadata: model::Metadata, target: Option<model::Target>) {
        self.hardware = Some((metadata, target));
        self.open = true;
    }
    fn start(&mut self, engine: &Engine, kind: Kind) {
        if self.busy() {
            return;
        }
        let result = (|| {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|error| error.to_string())?;
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone())?);
            }
            let cancel = work.cancel();
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(Job { kind, work })
                .map_err(|_| "Template worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.error = None;
            self.message = "Preparing template…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    fn complete(&mut self, path: PathBuf, outcome: crate::project_file::SaveOutcome) {
        self.message = format!(
            "Completed: {}. The current session and source template were preserved.",
            path.display()
        );
        if let crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(warning) = outcome
        {
            self.message
                .push_str(&format!(" Published, but folder sync failed: {warning}"));
        }
        self.error = None;
    }
}
fn field(ui: &mut Ui, label: &str, text: &mut String, limit: usize) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .char_limit(limit),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    help::annotate(ui, &response, HelpControl::TemplatePath);
}
fn hardware(ui: &mut Ui, metadata: &model::Metadata, engine: &Engine) {
    ui.label(format!("Template: {}", metadata.name));
    if let model::Kind::Track { bus } = &metadata.kind {
        ui.label(format!("Exact scene-bus alias: {bus}"));
    }
    ui.label(format!(
        "Requested audio backend: {} · output: {}",
        metadata
            .hardware
            .audio
            .backend
            .as_deref()
            .unwrap_or("System default"),
        metadata
            .hardware
            .audio
            .device
            .as_deref()
            .unwrap_or("System default")
    ));
    ui.label("Hardware references retain exact names and optional MIDI IDs. Missing and ambiguous destinations are never substituted. Current hardware remains on the active profile until explicitly reviewed and applied. Review audio activation in Preferences; its backend preview checks the actual host.");
    ui.label(format!(
        "Requested MIDI input policy: {:?}",
        metadata.hardware.midi_inputs
    ));
    if let crate::preferences::MidiInputs::Selected(names) = &metadata.hardware.midi_inputs {
        let discovery = engine.midi.policy_status();
        for name in names {
            let availability = match discovery.as_ref().filter(|status| {
                !status.pending() && status.error.is_none() && !status.available_truncated
            }) {
                Some(status)
                    if status
                        .available_inputs
                        .iter()
                        .any(|available| available == name) =>
                {
                    "available by exact policy name"
                }
                Some(_) => "unavailable",
                None => "unverified; discovery unavailable or incomplete",
            };
            ui.label(format!("MIDI input policy alias: {name} · {availability}"));
        }
    }
    let status = engine.midi.routing_status();
    for route in &metadata.hardware.routing.routes {
        for (direction, endpoint) in route
            .inputs
            .iter()
            .map(|input| ("input", &input.port))
            .chain(route.output.iter().map(|output| ("output", output)))
        {
            let matches = status
                .as_ref()
                .filter(|status| !status.pending && status.error.is_none() && !status.truncated)
                .map(|status| {
                    let ports = if direction == "input" {
                        &status.inputs
                    } else {
                        &status.outputs
                    };
                    ports
                        .iter()
                        .filter(|port| {
                            endpoint.matches(&port.name, port.id.as_deref().unwrap_or(""))
                        })
                        .count()
                });
            let availability = match matches {
                Some(1) => "available",
                Some(0) => "unavailable",
                Some(_) => "ambiguous",
                None => "unverified; MIDI discovery is unavailable, pending or incomplete",
            };
            ui.label(format!(
                "MIDI track {} {direction}: {} [{}] · {availability}",
                route.track + 1,
                endpoint.name,
                endpoint.id.as_deref().unwrap_or("exact name only")
            ));
        }
    }
}
impl App {
    pub(super) fn poll_templates(&mut self) {
        let received = self
            .templates
            .worker
            .as_ref()
            .map(|worker| worker.events.try_recv());
        if matches!(
            received,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected))
        ) {
            self.templates.cancel();
            self.templates.active = None;
            self.templates.worker = None;
            self.templates.error =
                Some("Template worker disconnected. The current session was preserved.".into());
        }
        if let Some(result) = received.and_then(Result::ok) {
            let cancelled = self
                .templates
                .active
                .take()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire));
            match result {
                Ok(ResultData::Saved(path, outcome)) => self.templates.complete(path, outcome),
                Ok(ResultData::Imported(path, outcome)) => {
                    self.templates.source_path = path.display().to_string();
                    self.templates.record = None;
                    self.templates.complete(path, outcome);
                }
                Ok(_) if cancelled => {
                    self.templates.error =
                        Some("Template operation cancelled before publication".into())
                }
                Ok(ResultData::Inspected(record)) => {
                    self.templates.record = Some(record);
                    self.templates.message = "Template inspected. Its source file remains separate from the live project.".into();
                }
                Ok(ResultData::Archive(path, manifest)) => {
                    self.templates.archive = Some((path, manifest));
                    self.templates.message = "Archive metadata reviewed; import verifies the native template and payload before publication.".into();
                }
                Err(error) => self.templates.error = Some(error),
            }
        }
    }
    pub(super) fn templates_ui(&mut self, ctx: &egui::Context) {
        if !self.templates.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut panel = std::mem::take(&mut self.templates);
        let mut open = true;
        let mut close = false;
        let mut use_template = None;
        let mut review_hardware = None;
        egui::Window::new("Project and track templates").open(&mut open).default_width(760.0).max_height((ctx.screen_rect().height()-80.0).max(200.0)).vscroll(true).show(ctx, |ui| {
            ui.label("Save reusable native projects or one track's devices, mixer and named routing. Templates include embedded audio. Project copies open stopped and unsaved. Track configuration preserves clips and other tracks, stops playback and starts fresh undo history. Source files are never live save destinations.");
            ui.add_enabled_ui(!panel.busy() && !self.project.committing(), |ui| {
                field(ui, "Template name", &mut panel.name, 80); field(ui, "New template file (.omtemplate)", &mut panel.new_path, 4096);
                for (label, track) in [("Save project template", None), ("Save selected track configuration", Some(self.snap.selected_track))] {
                    if ui.button(label).help(ui, HelpControl::TemplateSave).clicked() {
                        let view = self.project_view();
                        panel.start(&self.engine, Kind::Save { path: PathBuf::from(&panel.new_path), name: panel.name.clone(), track, hardware: model::Hardware::capture(self.settings.profile()), view });
                    }
                }
                ui.separator(); field(ui, "Template to inspect", &mut panel.source_path, 4096);
                if ui.button("Inspect template").help(ui, HelpControl::TemplateInspect).clicked() { panel.start(&self.engine, Kind::Inspect(PathBuf::from(&panel.source_path))); }
                if let Some(record) = panel.record.clone().filter(|record| record.path == Path::new(&panel.source_path)) {
                    hardware(ui, &record.metadata, &self.engine);
                    ui.label(format!("{} embedded samples · {} device/preset records · {} unresolved sources", record.manifest.media.len(), record.manifest.devices.len(), record.manifest.unresolved.len()));
                    let label = if matches!(record.metadata.kind, model::Kind::Project) { "Create project from template" } else { "Apply to selected track" };
                    if ui.button(label).help(ui, HelpControl::TemplateUse).clicked() { use_template = Some(record.clone()); }
                    if ui.button("Duplicate template to new file").help(ui, HelpControl::TemplateDuplicate).clicked() { panel.start(&self.engine, Kind::Duplicate { record: record.clone(), path: PathBuf::from(&panel.new_path), name: panel.name.clone() }); }
                    if ui.button("Review template hardware in Preferences").help(ui, HelpControl::TemplateHardware).clicked() {
                        let target = matches!(record.metadata.kind, model::Kind::Track {..}).then(|| self.snap.session.as_ref().and_then(|layout| model::Target::capture(layout, self.snap.selected_track))).flatten();
                        review_hardware = Some((record.metadata.clone(), target));
                    }
                    field(ui, "New template backup archive (.ompack)", &mut panel.backup_path, 4096);
                    if ui.button("Back up inspected template").help(ui, HelpControl::TemplateBackup).clicked() { panel.start(&self.engine, Kind::Backup { record, path: PathBuf::from(&panel.backup_path) }); }
                }
                if let Some((metadata, target)) = &panel.hardware { hardware(ui, metadata, &self.engine); if ui.button("Review loaded template hardware").help(ui, HelpControl::TemplateHardware).clicked() { review_hardware = Some((metadata.clone(), *target)); } }
                ui.separator(); field(ui, "Template archive to import", &mut panel.archive_path, 4096);
                if ui.button("Review template archive").help(ui, HelpControl::TemplateBackup).clicked() { panel.start(&self.engine, Kind::ReviewArchive(PathBuf::from(&panel.archive_path))); }
                if let Some((archive, manifest)) = panel.archive.clone().filter(|(path,_)| path == Path::new(&panel.archive_path)) {
                    ui.label(format!("{} · {} embedded samples · {} device/preset records", manifest.application, manifest.media.len(), manifest.devices.len()));
                    field(ui, "New imported template folder", &mut panel.destination, 4096);
                    if ui.button("Import portable template").help(ui, HelpControl::TemplateBackup).clicked() { panel.start(&self.engine, Kind::Import { archive, destination: PathBuf::from(&panel.destination), reviewed: manifest }); }
                }
            });
            if panel.busy() { if ui.button("Cancel template operation").help(ui, HelpControl::TemplateCancel).clicked() { panel.cancel(); } ctx.request_repaint_after(Duration::from_millis(20)); }
            if !panel.message.is_empty() { ui.label(&panel.message); }
            if let Some(error) = &panel.error { ui.colored_label(self.theme.red, error); }
            if ui.button("Close templates").help(ui, HelpControl::TemplateCancel).clicked() { close = true; }
        });
        if !open
            || close
            || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            panel.cancel();
            if !panel.busy() {
                panel.open = false;
            }
        }
        self.templates = panel;
        if let Some(record) = use_template {
            self.use_native_template(record);
        }
        if let Some((metadata, target)) = review_hardware {
            self.review_template_hardware(&metadata, target);
        }
    }
}
