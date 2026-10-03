use super::*;
use crate::engine::{
    midi_edit::{Ack, Outcome},
    session,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(test)]
mod tests;
mod worker;
use worker::{Catalog, Event, Job, Worker};

pub(super) struct Panel {
    pub open: bool,
    path: String,
    catalog: Option<Catalog>,
    tracks: Vec<session::Id>,
    scenes: Vec<session::Id>,
    clips: bool,
    devices: bool,
    keep_timing: bool,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    decision: Option<crossbeam_channel::Sender<()>>,
    ready: bool,
    message: String,
    error: Option<String>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            path: String::new(),
            catalog: None,
            tracks: Vec::new(),
            scenes: Vec::new(),
            clips: true,
            devices: true,
            keep_timing: false,
            worker: None,
            active: None,
            pending: None,
            decision: None,
            ready: false,
            message: String::new(),
            error: None,
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    pub(super) fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            while let Ok(event) = worker.events.try_recv() {
                let cancelled = self
                    .active
                    .as_ref()
                    .is_some_and(|cancel| cancel.load(Ordering::Acquire));
                match event {
                    Event::Browsed(catalog) => {
                        self.active = None;
                        if !cancelled {
                            self.tracks = catalog.tracks.iter().map(|(id, _)| *id).collect();
                            self.scenes = catalog.scenes.iter().map(|(id, _)| *id).collect();
                            self.catalog = Some(catalog);
                            self.keep_timing = false;
                            self.message =
                                "Source verified. Select the material to import, then review."
                                    .into();
                        } else {
                            self.message = "Browse cancelled.".into();
                        }
                    }
                    Event::Reviewed(message, ack) => {
                        if cancelled {
                            ack.cancel();
                        } else {
                            self.message = message;
                            self.ready = true;
                        }
                        self.pending = Some(ack);
                    }
                    Event::Finished(ack) => {
                        self.active = None;
                        self.pending = Some(ack);
                        self.ready = false;
                        self.decision = None;
                    }
                    Event::Failed(error) => {
                        self.active = None;
                        self.ready = false;
                        self.decision = None;
                        if let Some(ack) = self.pending.take() {
                            ack.cancel();
                        }
                        self.error = Some(error);
                    }
                }
            }
        }
        if let Some(ack) = &self.pending {
            match ack.state() {
                Outcome::Pending => {}
                Outcome::Applied => {
                    self.pending = None;
                    self.message = "Import applied as one transaction. Undo and Redo are available in History.".into();
                    self.error = None;
                }
                Outcome::Rejected => {
                    self.pending = None;
                    self.error = Some("Import refused because the destination, protection, recording, output rate or undo budget changed. Review again.".into());
                }
                Outcome::Cancelled => {
                    if self.active.is_none() {
                        self.pending = None;
                        self.message = "Import cancelled before application.".into();
                    }
                }
            }
        }
    }
    fn start(&mut self, engine: &Engine, review: bool, revision: u64) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<Arc<AtomicBool>, String> {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let cancel = work.cancel();
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone(), engine.cmd.clone())?);
            }
            let job = if review {
                let catalog = self
                    .catalog
                    .clone()
                    .ok_or("Browse a source project first")?;
                let (decision, waiting) = crossbeam_channel::bounded(1);
                self.decision = Some(decision);
                Job::Review {
                    catalog,
                    selection: session::ImportSelection {
                        tracks: self.tracks.clone(),
                        scenes: self.scenes.clone(),
                        clips: self.clips,
                        devices: self.devices,
                        keep_timing: self.keep_timing,
                    },
                    revision,
                    decision: waiting,
                    work,
                }
            } else {
                self.catalog = None;
                Job::Browse {
                    path: std::path::absolute(self.path.trim()).map_err(|e| e.to_string())?,
                    work,
                }
            };
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(job)
                .map_err(|_| "Project import worker is unavailable")?;
            Ok(cancel)
        })();
        match result {
            Ok(cancel) => {
                self.active = Some(cancel);
                self.ready = false;
                self.error = None;
                self.message = "Preparing project material…".into();
            }
            Err(error) => {
                self.decision = None;
                self.error = Some(error);
            }
        }
    }
}
fn choices(
    ui: &mut Ui,
    items: &[(session::Id, String)],
    selected: &mut Vec<session::Id>,
    prefix: &str,
) {
    for (id, name) in items {
        let mut checked = selected.contains(id);
        if ui
            .checkbox(&mut checked, format!("{prefix}: {name} [{}]", id.0))
            .changed()
        {
            if checked {
                selected.push(*id);
            } else {
                selected.retain(|current| current != id);
            }
        }
    }
}
impl App {
    pub(super) fn project_import_ui(&mut self, ctx: &egui::Context) {
        if !self.project_import.open {
            return;
        }
        let mut open = true;
        let enabled = !self.project.busy()
            && !self.project.committing()
            && !self.engine.cmd.performance().protected();
        let revision = self.snap.project_revision;
        egui::Window::new("Import project material")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                let panel = &mut self.project_import;
                ui.add_enabled_ui(enabled && !panel.busy(), |ui| {
                    ui.label("Source native project path");
                    ui.add(
                        egui::TextEdit::singleline(&mut panel.path)
                            .char_limit(4096)
                            .desired_width(f32::INFINITY),
                    )
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::TextEdit,
                            true,
                            "Source native project path",
                        )
                    });
                    if ui.button("Browse source project").clicked() {
                        panel.start(&self.engine, false, revision);
                    }
                    if let Some(catalog) = &panel.catalog {
                        ui.label(format!(
                            "Source: {} BPM · {} tracks · {} scenes",
                            catalog.bpm,
                            catalog.tracks.len(),
                            catalog.scenes.len()
                        ));
                        egui::ScrollArea::vertical()
                            .id_salt("project-import-selection")
                            .max_height(300.0)
                            .show(ui, |ui| {
                                choices(ui, &catalog.tracks, &mut panel.tracks, "Track");
                                ui.separator();
                                choices(ui, &catalog.scenes, &mut panel.scenes, "Scene");
                            });
                        ui.checkbox(
                            &mut panel.clips,
                            "Include clips and MIDI controller automation",
                        );
                        ui.checkbox(&mut panel.devices, "Include instruments and effects");
                        ui.checkbox(
                            &mut panel.keep_timing,
                            "Keep destination tempo and meter; use source beat positions",
                        );
                        if ui.button("Review selected project material").clicked() {
                            panel.start(&self.engine, true, revision);
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("project-import-report")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.label(&panel.message);
                    });
                if let Some(error) = &panel.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            enabled && panel.ready,
                            egui::Button::new("Apply reviewed project import"),
                        )
                        .clicked()
                    {
                        if let Some(decision) = &panel.decision {
                            if decision.try_send(()).is_ok() {
                                panel.ready = false;
                            }
                        }
                    }
                    if ui
                        .add_enabled(panel.busy(), egui::Button::new("Cancel project import"))
                        .clicked()
                    {
                        panel.cancel();
                        panel.ready = false;
                    }
                });
                if panel.busy() {
                    ctx.request_repaint_after(Duration::from_millis(20));
                }
            });
        self.project_import.open = open;
    }
}
