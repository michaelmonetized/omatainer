//! Inspect dependencies and explicitly review source aliases for embedded audio.
use super::*;
use crate::project_dependencies as data;
use std::sync::atomic::{AtomicBool, Ordering};
mod worker;
use worker::{Event, Job, Kind, ResultData, Review, Scope, Worker};

#[derive(Default)]
pub(super) struct Dependencies {
    pub open: bool,
    pub origins: Vec<data::Origin>,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    review: Option<Review>,
    search: Option<data::Search>,
    choices: Vec<Option<usize>>,
    asset: usize,
    device: usize,
    missing: usize,
    roots: String,
    message: String,
    error: Option<String>,
    confirm_discard: bool,
    discard_when_settled: bool,
}
impl Drop for Dependencies { fn drop(&mut self) { self.cancel(); } }
impl Dependencies {
    fn busy(&self) -> bool { self.active.is_some() }
    pub(super) fn blocks_close(&self) -> bool { self.busy() || self.choices.iter().any(Option::is_some) }
    /// Keep source choices when a project replacement is requested.
    /// Returns whether replacement is blocked and opens explicit keep/discard review.
    pub(super) fn guard_replacement(&mut self) -> bool {
        if !self.blocks_close() { return false; }
        self.open = true; self.confirm_discard = true; true
    }
    pub(super) fn cancel(&self) { if let Some(cancel) = &self.active { cancel.store(true, Ordering::Release); } }
    pub(super) fn install_origins(&mut self, origins: Vec<data::Origin>) {
        self.cancel(); self.origins = origins; self.review = None; self.search = None; self.choices.clear();
        self.confirm_discard = false; self.discard_when_settled = false; self.message.clear(); self.error = None;
    }
    fn start(&mut self, engine: &Engine, kind: Kind) {
        if self.busy() { return; }
        let result = (|| {
            let work = engine.cmd.performance().optional_work().map_err(|e| e.to_string())?;
            if self.worker.is_none() { self.worker = Some(Worker::start(engine.project.clone())?); }
            let cancel = work.cancel();
            self.worker.as_ref().unwrap().jobs.try_send(Job { kind, origins: self.origins.clone(), work }).map_err(|_| "Dependency worker is busy or disconnected")?;
            self.active = Some(cancel); self.error = None; self.message = "Checking project dependencies…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result { self.error = Some(error); }
    }
    fn current(&self, engine: &Engine, scope: &Scope) -> bool {
        engine.project.revision() == scope.revision
            && engine.snapshot().session.as_ref().is_some_and(|layout| layout.namespace == scope.namespace)
    }
}
pub(super) fn source_name(source: &LibSource) -> String {
    match source {
        LibSource::File(path) => path.display().to_string(),
        LibSource::Removable { volume_id, relative_path } => format!("Volume {volume_id} / {}", relative_path.display()),
        LibSource::Builtin(_) => "Embedded factory audio".into(),
        LibSource::Provider { provider, media_id } => format!("{provider}: {media_id}"),
    }
}
fn picker(ui: &mut Ui, label: &str, selected: &mut usize, length: usize) {
    if length == 0 { return; }
    let mut display = selected.saturating_add(1).min(length);
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(&mut display).range(1..=length));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, label));
        if let Some(value) = accessibility::numeric(ui, &response, label, display as f32, 1.0, length as f32, 1.0, "") { display = value.round() as usize; }
        *selected = display.saturating_sub(1).min(length - 1);
    });
}
impl App {
    pub(super) fn poll_dependencies(&mut self) {
        let received = self.dependencies.worker.as_ref().map(|worker| worker.events.try_recv());
        if matches!(received, Some(Err(crossbeam_channel::TryRecvError::Disconnected))) {
            self.dependencies.cancel(); self.dependencies.active = None; self.dependencies.worker = None;
            self.dependencies.error = Some("Dependency worker disconnected; current source aliases were preserved.".into());
        }
        let event = received.and_then(Result::ok);
        if let Some(Event { baseline, result }) = event {
            let cancelled = self.dependencies.active.take().is_some_and(|cancel| cancel.load(Ordering::Acquire));
            if cancelled { self.dependencies.error = Some("Dependency operation cancelled; current source aliases were preserved.".into()); }
            else if baseline != self.dependencies.origins { self.dependencies.error = Some("Project source aliases changed during this operation. Check dependencies again.".into()); }
            else {
                match result {
                    Ok(ResultData::Inspected(review)) | Ok(ResultData::Searched(review, _)) if !self.dependencies.current(&self.engine, &review.scope) => {
                        self.dependencies.error = Some("Project changed before the dependency result arrived. Check dependencies again.".into());
                    }
                    Ok(ResultData::Inspected(review)) => {
                        self.dependencies.choices = vec![None; review.inventory.assets.len()];
                        self.dependencies.review = Some(review); self.dependencies.search = None;
                        self.dependencies.message = "Dependency inspection complete. Embedded audio remains in the project.".into(); self.dependencies.error = None;
                    }
                    Ok(ResultData::Searched(review, search)) => {
                        self.dependencies.choices = vec![None; review.inventory.assets.len()];
                        self.dependencies.review = Some(review); self.dependencies.search = Some(search);
                        self.dependencies.message = "Search complete. Choose each replacement explicitly before applying the batch.".into(); self.dependencies.error = None;
                    }
                    Ok(ResultData::Verified(scope, selected, keys)) => {
                        let result = (|| {
                            let work = self.engine.cmd.performance().optional_work().map_err(|e| e.to_string())?;
                            if work.cancelled() || !self.dependencies.current(&self.engine, &scope) { return Err("Project or protection changed before relink; current aliases were preserved".into()); }
                            let mut origins = self.dependencies.origins.clone();
                            origins.retain(|origin| keys.contains(&origin.key));
                            for origin in selected {
                                origins.retain(|old| old.key != origin.key); origins.push(origin);
                            }
                            data::validate_origins(&origins)?;
                            self.dependencies.origins = origins; self.dependencies.choices.fill(None);
                            self.dependencies.review = None; self.dependencies.search = None;
                            self.project.local_edits += 1;
                            self.dependencies.message = "Project sources relinked. Save the project to keep these references.".into();
                            Ok::<_, String>(())
                        })();
                        self.dependencies.error = result.err();
                    }
                    Err(error) => self.dependencies.error = Some(error),
                }
            }
        } else if self.dependencies.active.is_some() && self.dependencies.worker.as_ref().is_some_and(|worker| worker.events.is_empty() && !self.engine.cmd.is_connected()) {
            self.dependencies.cancel(); self.dependencies.active = None;
            self.dependencies.error = Some("Renderer disconnected. Current source aliases were preserved.".into());
        }
        if self.dependencies.discard_when_settled && !self.dependencies.busy() {
            self.dependencies.choices.clear(); self.dependencies.search = None; self.dependencies.open = false; self.dependencies.discard_when_settled = false;
        }
    }
    pub(super) fn dependencies_ui(&mut self, ctx: &egui::Context) {
        if !self.dependencies.open { return; }
        keyboard::block_for_dialog(ctx);
        let mut editor = std::mem::take(&mut self.dependencies);
        let mut open = true; let mut close = false;
        egui::Window::new("Project dependencies").open(&mut open).default_width(650.0).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
            if let Some(picture)=&self.video.clip { ui.label(format!("External picture reference: {}. Open Video to verify its source and frame metadata; native projects retain this reference. Portable export requires clearing it in a saved copy.",picture.path.display())); }
            ui.label("Native projects retain embedded audio and unavailable device state. Relink verifies decoded audio and updates this project's source references.");
            ui.add_enabled_ui(!editor.busy() && !self.project.committing(), |ui| {
                if ui.add_enabled(!editor.choices.iter().any(Option::is_some), egui::Button::new("Check dependencies")).help(ui, HelpControl::DependenciesInspect).clicked() { editor.start(&self.engine, Kind::Inspect); }
                if let Some(review) = editor.review.clone() {
                    ui.label(format!("{} embedded assets · {} unavailable devices · {} sources without embedded audio", review.inventory.assets.len(), review.inventory.devices.len(), review.inventory.missing.len()));
                    picker(ui, "Project asset", &mut editor.asset, review.inventory.assets.len());
                    if let Some(asset) = review.inventory.assets.get(editor.asset) {
                        ui.label(&asset.name); ui.label(format!("Embedded audio: {} frames · {} references", asset.frames, asset.uses));
                        if let Some(source) = &asset.source { ui.label(source_name(source)); }
                        match &asset.availability {
                            data::Availability::Builtin => { ui.label("Embedded audio has no external source path."); }
                            data::Availability::Verified => { ui.label("Source audio matches the embedded project audio."); }
                            data::Availability::Unresolved(reason) => { ui.colored_label(self.theme.red, format!("Unresolved source: {reason}")); }
                        }
                        if let Some(search) = &editor.search {
                            if let Some(candidates) = search.matches.get(editor.asset) {
                                ui.label(format!("{} verified replacement candidates", candidates.len()));
                                egui::ScrollArea::vertical().id_salt("dependency-candidates").max_height(180.0).show_rows(ui, ui.spacing().interact_size.y, candidates.len(), |ui, rows| {
                                    for index in rows {
                                        let candidate = &candidates[index];
                                        if ui.button(format!("Use replacement {}: {}", index + 1, source_name(&candidate.location.source))).help(ui, HelpControl::DependenciesChoose).clicked() { editor.choices[editor.asset] = Some(index); }
                                    }
                                });
                                if editor.choices.get(editor.asset).copied().flatten().is_some() && ui.button("Keep current source reference").help(ui, HelpControl::DependenciesChoose).clicked() { editor.choices[editor.asset] = None; }
                            }
                        }
                    }
                    picker(ui, "Source without embedded audio", &mut editor.missing, review.inventory.missing.len());
                    if let Some(missing) = review.inventory.missing.get(editor.missing) {
                        ui.colored_label(self.theme.red, format!("{} · {}", missing.placement, missing.source));
                        if let Some(hash) = missing.file_hash { ui.label(format!("Retained file SHA-256: {}", hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>())); }
                        ui.label("No rendered audio is retained in this slot. Restore its source through the library and retry in Edit banks; its saved source and controls remain intact.");
                    }
                    picker(ui, "Unavailable device", &mut editor.device, review.inventory.devices.len());
                    if let Some(device) = review.inventory.devices.get(editor.device) {
                        ui.label(format!("{} · {}", device.placement, device.identifier));
                        ui.label(format!("Retained state: schema {:?}, {} bytes · requested enabled: {}", device.state_schema, device.state_bytes, device.requested_on));
                        if let Some(fallbacks) = device.rendered_fallbacks {
                            ui.label(format!("Unavailable instrument is silent; {fallbacks} clips have embedded rendered audio for playback. Notes and automation remain editable."));
                        }
                        ui.label("Processing is bypassed until a compatible device returns. Its position and saved state remain in the project.");
                    }
                    ui.label("Search folders, one absolute path per line");
                    let response = ui.add(egui::TextEdit::multiline(&mut editor.roots).desired_rows(3).desired_width(f32::INFINITY).char_limit(256 * 1024));
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Project dependency search folders"));
                    help::annotate(ui, &response, HelpControl::DependenciesSearch);
                    if ui.add_enabled(!editor.choices.iter().any(Option::is_some), egui::Button::new("Search moved sources")).help(ui, HelpControl::DependenciesSearch).clicked() {
                        let roots = editor.roots.lines().filter(|line| !line.trim().is_empty()).map(|line| PathBuf::from(line.trim())).collect();
                        editor.start(&self.engine, Kind::Search { review: review.clone(), roots });
                    }
                    if let Some(search) = &editor.search {
                        ui.label(format!("{} entries · {} files · {} unsupported/non-audio skipped · {}", search.entries, search.files, search.skipped_unsupported, if search.complete { "Complete search of supported audio in selected folders" } else { "Partial search; additional matches may exist" }));
                        for warning in &search.warnings { ui.label(warning); }
                        let choices: Vec<_> = editor.choices.iter().enumerate().filter_map(|(asset, &choice)| choice.map(|choice|
                            (review.inventory.assets[asset].clone(), search.matches[asset][choice].clone()))).collect();
                        if ui.add_enabled(!choices.is_empty(), egui::Button::new("Apply reviewed relinks")).help(ui, HelpControl::DependenciesApply).clicked() {
                            editor.start(&self.engine, Kind::Verify { review, choices });
                        }
                    }
                }
            });
            if editor.busy() {
                if ui.button("Cancel dependency operation").help(ui, HelpControl::DependenciesCancel).clicked() { editor.cancel(); }
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
            }
            if !editor.message.is_empty() { ui.label(&editor.message); }
            if let Some(error) = &editor.error { ui.colored_label(self.theme.red, error); }
            if ui.button("Close dependency report").help(ui, HelpControl::DependenciesClose).clicked() { close = true; }
        });
        if !open || close || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            if editor.blocks_close() { editor.confirm_discard = true; } else { editor.open = false; }
        }
        if editor.confirm_discard {
            egui::Window::new("Unapplied source choices").collapsible(false).show(ctx, |ui| {
                ui.label("Keep reviewing or discard the unapplied choices. Completed relinks remain in the project.");
                if ui.button("Keep source review").help(ui, HelpControl::DependenciesClose).clicked() { editor.confirm_discard = false; }
                if ui.button("Discard source review").help(ui, HelpControl::DependenciesClose).clicked() { editor.cancel(); editor.confirm_discard = false; editor.discard_when_settled = true; }
            });
        }
        self.dependencies = editor;
    }
}

#[cfg(test)]
mod tests;
