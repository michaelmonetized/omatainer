use super::*;
use crate::engine::{dj_fx_preset::{Name, Preset, Settings}, dj_fx_recall::{Phase, Request}, session::Axis, surface_controls::Input};
use std::sync::atomic::{AtomicBool, Ordering};
mod worker;
#[cfg(test)]
mod tests;

pub(super) struct Panel {
    path: String,
    name: String,
    names: [String; 2],
    review: Option<Preset>,
    expected: Option<([u64; 2], [Settings; 2])>,
    next_token: u64,
    pending: Option<u64>,
    worker: Option<worker::Worker>,
    active: Option<Arc<AtomicBool>>,
    message: String,
}
impl Default for Panel {
    fn default() -> Self {
        Self { path: String::new(), name: "Performance layout".into(), names: ["Unit A".into(), "Unit B".into()], review: None, expected: None, next_token: 1, pending: None, worker: None, active: None, message: String::new() }
    }
}
impl Drop for Panel { fn drop(&mut self) { if let Some(cancel) = &self.active { cancel.store(true, Ordering::Release); } } }
impl Panel {
    fn busy(&self) -> bool { self.active.is_some() || self.pending.is_some() }
    fn start(&mut self, engine: &Engine, operation: worker::Operation) {
        if self.busy() { return; }
        let result = (|| -> Result<(), String> {
            let path = PathBuf::from(self.path.trim());
            if self.path.trim().is_empty() || path.extension().and_then(|ext| ext.to_str()) != Some("omatfx") { return Err("Choose a .omatfx preset path".into()); }
            let work = engine.cmd.performance().optional_work().map_err(|error| error.to_string())?;
            let cancel = work.cancel();
            if self.worker.is_none() { self.worker = Some(worker::Worker::start()?); }
            self.worker.as_ref().unwrap().jobs.try_send(worker::Job { path, operation, work }).map_err(|error| error.to_string())?;
            self.active = Some(cancel); self.expected = None; self.message = "Working on the preset file; current effects stay in place.".into(); Ok(())
        })();
        if let Err(error) = result { self.message = error; }
    }
    /// Receive file work and renderer-confirmed recall results.
    /// Takes the current snapshot; updates review/status without opening audio, MIDI or an OS window.
    pub(super) fn poll(&mut self, snapshot: &Snapshot) {
        if let Some(worker) = &self.worker {
            if let Ok(event) = worker.events.try_recv() {
                let cancelled = self.active.take().is_some_and(|cancel| cancel.load(Ordering::Acquire));
                match event {
                    worker::Event::Saved(crate::project_file::SaveOutcome::Durable) => self.message = "Saved portable layout. The existing file was not replaced.".into(),
                    worker::Event::Saved(crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(error)) => self.message = format!("The new file was committed, but its folder could not be synced: {error}. Inspect it before retrying."),
                    worker::Event::Inspected(preset) if !cancelled => { self.review = Some(preset); self.expected = None; self.message = "Inspected layout. Check its units and sampler routes, then review recall.".into(); }
                    worker::Event::Inspected(_) => self.message = "Inspection cancelled; current effects are unchanged.".into(),
                    worker::Event::Failed(error) => self.message = error,
                }
            }
        }
        if let Some(token) = self.pending {
            let receipt = snapshot.surfaces.fx_recall;
            if receipt.last_rejected == token { self.pending = None; self.message = "Another recall is in progress; this recall was refused.".into(); }
            else if receipt.token == token {
                match receipt.phase {
                    Phase::Applied => { self.pending = None; self.expected = None; self.names = snapshot.surfaces.fx.map(|unit| unit.name.as_str().to_owned()); self.message = "Recalled both units together. Previous tails were reset after the fade.".into(); }
                    Phase::Stale | Phase::Rejected => { self.pending = None; self.expected = None; self.message = "Recall refused because the reviewed controls or project routes changed. Current effects are preserved; review again.".into(); }
                    _ => {}
                }
            }
        }
    }
}
fn entry(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::TextEdit::singleline(value).hint_text(label));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, response.enabled(), label));
        accessibility::focus(ui, &response);
    });
}
fn route_valid(unit: Settings, snapshot: &Snapshot) -> bool {
    unit.sampler.is_none_or(|reference| snapshot.session.as_ref().is_some_and(|layout| layout.tracks.iter().enumerate().any(|(slot, _)| layout.resolves(Axis::Track, slot, reference))))
}
impl App {
    /// Review, save and recall the native portable FX layout.
    /// Takes the existing DJ FX panel; presents exact settings and explicit tail reset before submitting a fixed atomic renderer command.
    pub(super) fn dj_fx_presets_ui(&mut self, ui: &mut Ui) {
        let mut panel = std::mem::take(&mut self.dj_fx_presets); panel.poll(&self.snap);
        accessibility::scope(ui, "DJ FX presets", |ui| {
            ui.collapsing("Saved layouts", |ui| {
                ui.label("A layout saves both units, in slot order, with their names, routes, timing and parameters. Share the .omatfx file. Importing it never changes effects until you review and recall.");
                ui.add_enabled_ui(!panel.busy(), |ui| {
                    entry(ui, "Preset name", &mut panel.name);
                    entry(ui, "Preset path", &mut panel.path);
                    for bank in 0..2 {
                        ui.horizontal(|ui| {
                            entry(ui, &format!("Unit {} name", (b'A' + bank as u8) as char), &mut panel.names[bank]);
                            let label = format!("Apply Unit {} name", (b'A' + bank as u8) as char);
                            let response = ui.button(&label); accessibility::button(ui, &response, &label, None);
                            if response.clicked() { match Name::new(&panel.names[bank]) { Ok(name) => self.send(Command::Surface(Input::DjFxName { bank: bank as u8, name })), Err(error) => panel.message = error } }
                        });
                    }
                    ui.horizontal(|ui| {
                        let response = ui.button("Save new layout"); accessibility::button(ui, &response, "Save new layout", None);
                        if response.clicked() { match Name::new(&panel.name) { Ok(name) => panel.start(&self.engine, worker::Operation::Save(Preset { version: 1, name, units: self.snap.surfaces.fx.map(Into::into) })), Err(error) => panel.message = error } }
                        let response = ui.button("Inspect layout"); accessibility::button(ui, &response, "Inspect layout", None);
                        if response.clicked() { panel.start(&self.engine, worker::Operation::Inspect); }
                    });
                });
                if let Some(mut preset) = panel.review {
                    ui.heading(format!("Layout · {}", preset.name.as_str()));
                    for bank in 0..2 {
                        let unit = preset.units[bank];
                        ui.label(format!("Unit {} · {} · {:?} · {:?} · {} ms · beat divisor {}", (b'A' + bank as u8) as char, unit.name.as_str(), unit.placement, unit.timing, unit.manual_ms, unit.beats));
                        ui.label(format!("Routes: Deck A {} · Deck B {} · Master {} · Sampler {:?}", unit.assigned[0], unit.assigned[1], unit.master, unit.sampler));
                        for slot in 0..3 { ui.label(format!("Slot {} · {} · enabled {} · wet {:.3} · parameter {:.3}", slot + 1, unit.kinds[slot].name(), unit.on[slot], unit.wet[slot], unit.parameter[slot])); }
                        if unit.sampler.is_some() {
                            let valid = route_valid(unit, &self.snap);
                            ui.label(if valid { "Sampler route belongs to the current project." } else { "This sampler route is from another project or is no longer active. Bind it to a current track or remove the route before review." });
                            let target = self.snap.session.as_ref().and_then(|layout| layout.reference(Axis::Track, self.snap.selected_track));
                            let label = format!("Bind Unit {} to selected sampler track", (b'A' + bank as u8) as char);
                            let response = ui.add_enabled(!panel.busy() && target.is_some(), egui::Button::new(&label)); accessibility::button(ui, &response, &label, None);
                            if response.clicked() { preset.units[bank].sampler = target; panel.expected = None; }
                            let label = format!("Remove Unit {} sampler route", (b'A' + bank as u8) as char);
                            let response = ui.add_enabled(!panel.busy(), egui::Button::new(&label)); accessibility::button(ui, &response, &label, None);
                            if response.clicked() { preset.units[bank].sampler = None; panel.expected = None; }
                        }
                    }
                    panel.review = Some(preset);
                    let ready = preset.units.iter().all(|unit| route_valid(*unit, &self.snap));
                    let response = ui.add_enabled(!panel.busy() && ready && self.snap.session.is_some(), egui::Button::new("Review recall")); accessibility::button(ui, &response, "Review recall", None);
                    if response.clicked() { panel.expected = self.snap.session.as_ref().map(|layout| (layout.namespace, self.snap.surfaces.fx.map(Into::into))); panel.message = "Reviewed. Recall fades to dry for 5 ms, resets previous tails, applies both units together, then fades in for 5 ms. Changed controls or routes refuse recall.".into(); }
                    if let Some((namespace, expected)) = panel.expected {
                        let response = ui.add_enabled(!panel.busy() && ready, egui::Button::new("Recall reviewed layout · reset tails")); accessibility::button(ui, &response, "Recall reviewed layout · reset tails", None);
                        if response.clicked() {
                            let token = panel.next_token; panel.next_token = panel.next_token.wrapping_add(1).max(1);
                            match self.engine.send(Command::Surface(Input::DjFxRecall(Request { token, namespace, expected, desired: preset.units }))) { Ok(_) => { panel.pending = Some(token); panel.message = "Recalling with Reset after fade…".into(); }, Err(error) => panel.message = error.to_string() }
                        }
                    }
                }
                if panel.active.is_some() { let response = ui.button("Cancel preset file work"); accessibility::button(ui, &response, "Cancel preset file work", None); if response.clicked() { if let Some(cancel) = &panel.active { cancel.store(true, Ordering::Release); } } }
                let response = ui.label(&panel.message); accessibility::status(ui, &response, &panel.message);
            });
        });
        self.dj_fx_presets = panel;
    }
}
