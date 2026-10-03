//! Review and package captured native projects without changing the live session.
use super::*;
use crate::{portable_project as data, project_dependencies as dependencies};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
pub(super) mod worker;
use worker::{Job, Kind, ResultData, Review, Worker};

#[derive(Default)]
pub(super) struct Portability {
    pub open: bool,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    review: Option<Review>,
    selected: Vec<bool>,
    preview: Option<(PathBuf, Arc<data::Manifest>, Arc<crate::licenses::Catalog>)>,
    asset: usize,
    device: usize,
    notice: usize,
    export_path: String,
    archive_path: String,
    destination: String,
    imported: Option<PathBuf>,
    message: String,
    error: Option<String>,
}
impl Drop for Portability {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Portability {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn reset_review(&mut self) {
        self.cancel();
        self.review = None;
        self.selected.clear();
    }
    fn complete(&mut self, path: PathBuf, outcome: crate::project_file::SaveOutcome) {
        self.message = format!(
            "Completed: {}. Current session and playback were preserved.",
            path.display()
        );
        if let crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(warning) = outcome
        {
            self.message
                .push_str(&format!(" Published, but folder sync failed: {warning}"));
        }
        self.error = None;
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
                .map_err(|e| e.to_string())?;
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone())?);
            }
            let cancel = work.cancel();
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(Job { kind, work })
                .map_err(|_| "Portable project worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.error = None;
            self.message = "Preparing portable project…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
}

fn field(ui: &mut Ui, label: &str, text: &mut String) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .char_limit(4096),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    help::annotate(ui, &response, HelpControl::PortablePath);
}
fn index(ui: &mut Ui, label: &str, selected: &mut usize, length: usize) {
    if length == 0 {
        return;
    }
    let mut value = selected.saturating_add(1).min(length);
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(&mut value).range(1..=length));
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, label));
        if let Some(number) = accessibility::numeric(
            ui,
            &response,
            label,
            value as f32,
            1.0,
            length as f32,
            1.0,
            "",
        ) {
            value = number.round() as usize;
        }
    });
    *selected = value.saturating_sub(1).min(length - 1);
}
fn report(
    ui: &mut Ui,
    manifest: &data::Manifest,
    catalog: &crate::licenses::Catalog,
    asset: &mut usize,
    device: &mut usize,
    notice: &mut usize,
    scope: &str,
) {
    ui.label(format!("Created by: {}", manifest.application));
    ui.label(format!(
        "{} embedded samples · {} devices/presets · {} sources without audio",
        manifest.media.len(),
        manifest.devices.len(),
        manifest.unresolved.len()
    ));
    index(
        ui,
        &format!("Portable {scope} sample"),
        asset,
        manifest.media.len(),
    );
    if let Some(media) = manifest.media.get(*asset) {
        ui.label(format!(
            "{} · {} frames · {} Hz · {} channels",
            media.name, media.frames, media.sample_rate, media.channels
        ));
        ui.label(
            media
                .source
                .as_deref()
                .unwrap_or("Embedded sample; no external source path"),
        );
        ui.label(format!(
            "Collected relative path: {}",
            media.collected.as_deref().unwrap_or("not collected")
        ));
        ui.label(format!("Audio rights note (unverified): {}", media.rights));
    }
    index(
        ui,
        &format!("Portable {scope} device or preset"),
        device,
        manifest.devices.len(),
    );
    if let Some(device) = manifest.devices.get(*device) {
        ui.label(format!("{} · {}", device.placement, device.identifier));
        let hash = device
            .settings_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        ui.label(format!("Saved preset/settings SHA-256: {hash}"));
        ui.label(&device.rights);
        if device.unavailable {
            ui.label("Unavailable device: effect processing is bypassed; instruments are silent unless a clip retains rendered audio. Saved notes, automation and device state remain intact.");
        }
    }
    index(
        ui,
        &format!("Portable {scope} license-sensitive dependency"),
        notice,
        catalog.manifest.entries.len(),
    );
    if let Some(entry) = catalog.manifest.entries.get(*notice) {
        ui.label(format!(
            "Recorded distribution notice: {} · {} · {}",
            entry.name, entry.category, entry.license
        ));
        ui.label(&entry.redistribution);
    }
    ui.label("Full application distribution notices are retained in this archive. These records do not grant rights to user audio or external plugins.");
    if !manifest.unresolved.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt(("portable-unresolved", scope))
            .max_height(90.0)
            .show_rows(
                ui,
                ui.spacing().interact_size.y,
                manifest.unresolved.len(),
                |ui, rows| {
                    for row in rows {
                        ui.label(&manifest.unresolved[row]);
                    }
                },
            );
    }
}
impl App {
    pub(super) fn poll_portability(&mut self) {
        let received = self
            .portability
            .worker
            .as_ref()
            .map(|worker| worker.events.try_recv());
        if matches!(
            received,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected))
        ) {
            self.portability.cancel();
            self.portability.active = None;
            self.portability.worker = None;
            self.portability.error =
                Some("Portable project worker disconnected. Current project was preserved.".into());
        }
        if let Some(result) = received.and_then(Result::ok) {
            let cancelled = self
                .portability
                .active
                .take()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire));
            match result {
                Ok(ResultData::Exported(path, outcome)) => self.portability.complete(path, outcome),
                Ok(ResultData::Imported(path, outcome)) => {
                    self.portability.imported = Some(path.clone());
                    self.portability.complete(path, outcome);
                }
                Ok(_) if cancelled => {
                    self.portability.error =
                        Some("Portable project operation cancelled before publication.".into())
                }
                Ok(ResultData::Inspected(review)) => {
                    if review.revision != self.engine.project.revision()
                        || self
                            .engine
                            .snapshot()
                            .session
                            .as_ref()
                            .is_none_or(|layout| layout.namespace != review.namespace)
                        || self.dependencies.origins != review.origins
                    {
                        self.portability.error =
                            Some("Project changed during inspection; inspect again.".into());
                    } else {
                        self.portability.selected = vec![false; review.inventory.assets.len()];
                        self.portability.review = Some(review);
                        self.portability.message = "Inspection complete. Export captures the reviewed musical revision and includes every embedded sample. Select original files individually.".into();
                        self.portability.error = None;
                    }
                }
                Ok(ResultData::Previewed(path, manifest, catalog)) => {
                    self.portability.preview = Some((path, manifest, catalog));
                    self.portability.message = "Archive metadata reviewed. Import verifies every checksum and the native session before publication.".into();
                    self.portability.error = None;
                }
                Err(error) => self.portability.error = Some(error),
            }
        }
    }
    pub(super) fn portability_ui(&mut self, ctx: &egui::Context) {
        if !self.portability.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut editor = std::mem::take(&mut self.portability);
        let mut open = true;
        let mut close = false;
        egui::Window::new("Portable project").open(&mut open).default_width(720.0).max_height(self.theme.window_height(ctx)).vscroll(true).show(ctx, |ui| {
            ui.label("Package the native session with its embedded audio. Selected original files are copied and deduplicated by checksum. Originals and global settings stay intact. Plugin binaries are not included.");
            ui.add_enabled_ui(!editor.busy() && !self.project.committing(), |ui| {
                if ui.button("Inspect portable dependencies").help(ui, HelpControl::PortableInspect).clicked() { let view = self.project_view(); editor.start(&self.engine, Kind::Inspect(view)); }
                if let Some(review) = editor.review.clone() {
                    report(ui, &review.manifest, &review.catalog, &mut editor.asset, &mut editor.device, &mut editor.notice, "export");
                    if let Some(asset) = review.inventory.assets.get(editor.asset) {
                        let available = asset.source.is_some() && matches!(asset.availability, dependencies::Availability::Verified);
                        ui.add_enabled(available, egui::Checkbox::new(&mut editor.selected[editor.asset], "Collect this original source")).help(ui, HelpControl::PortableCollect);
                        if let dependencies::Availability::Unresolved(reason) = &asset.availability { ui.label(format!("Original source unresolved: {reason}. Embedded audio is still included.")); }
                    }
                    ui.label(format!("{} original sources selected", editor.selected.iter().filter(|&&selected| selected).count()));
                    field(ui, "New archive path (.ompack)", &mut editor.export_path);
                    if ui.button("Export portable project").help(ui, HelpControl::PortableExport).clicked() {
                        let selected = editor.selected.iter().enumerate().filter_map(|(index, &selected)| selected.then_some(index)).collect();
                        let view = self.project_view(); editor.start(&self.engine, Kind::Export { review, selected, view, path: PathBuf::from(&editor.export_path) });
                    }
                }
                ui.separator(); field(ui, "Archive to import", &mut editor.archive_path);
                if ui.button("Review portable archive").help(ui, HelpControl::PortableReview).clicked() { editor.start(&self.engine, Kind::Preview(PathBuf::from(&editor.archive_path))); }
                if let Some((path, manifest, catalog)) = &editor.preview {
                    if path == Path::new(&editor.archive_path) {
                        report(ui, manifest, catalog, &mut editor.asset, &mut editor.device, &mut editor.notice, "import");
                        field(ui, "New imported project folder", &mut editor.destination);
                        if ui.button("Import into new folder").help(ui, HelpControl::PortableImport).clicked() { editor.start(&self.engine, Kind::Import { archive: path.clone(), destination: PathBuf::from(&editor.destination), reviewed: manifest.clone() }); }
                    }
                }
                if let Some(path) = &editor.imported {
                    if ui.button("Open imported project").help(ui, HelpControl::PortableOpen).clicked() { self.open_imported_project(path.clone()); editor.open = false; }
                }
            });
            if editor.busy() {
                if ui.button("Cancel portable operation").help(ui, HelpControl::PortableCancel).clicked() { editor.cancel(); }
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
            }
            if !editor.message.is_empty() { ui.label(&editor.message); }
            if let Some(error) = &editor.error { ui.colored_label(self.theme.red, error); }
            if ui.button("Close portable project").help(ui, HelpControl::PortableCancel).clicked() { close = true; }
        });
        if !open
            || close
            || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            editor.cancel();
            if !editor.busy() {
                editor.open = false;
            }
        }
        self.portability = editor;
    }
}

#[cfg(test)]
mod tests;
