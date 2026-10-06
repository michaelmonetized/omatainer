//! Named definitions save independently of reviewed live port assignments.
use super::*;
use crate::engine::midi::{
    learn::{self, Endpoint},
    presets::{self, Preset},
};
use crate::preferences::worker::Job;

#[derive(Default)]
pub(super) struct Panel {
    target: Option<Endpoint>,
    selected: Option<String>,
    name: String,
    revision_hint: String,
    path: String,
    token: u64,
    pending: Option<(u64, crate::preferences::Preferences)>,
    imported: Option<(Preset, crate::preferences::Preferences)>,
    review: Option<Review>,
    message: String,
}
struct Review {
    origin: crate::preferences::Preferences,
    port: Endpoint,
    source: u64,
    revision: u64,
    config: learn::Config,
    label: String,
}

impl App {
    fn midi_preset_editable(&self) -> Result<(), String> {
        if self.engine.safe_mode()
            || self.engine.cmd.performance().protected()
            || self.project.committing()
        {
            return Err(
                "Leave protection and finish project work before editing MIDI presets".into(),
            );
        }
        if self.settings.busy()
            || self.settings.blocked
            || self.settings.worker.is_none()
            || self.settings.draft != self.settings.applied
        {
            return Err(
                "Finish or discard preference edits before saving or transferring MIDI presets"
                    .into(),
            );
        }
        Ok(())
    }
    /// Queue a bank change through the sole preference owner.
    /// Takes a complete candidate bank; retains runtime assignments and applies only the durable saved result.
    fn save_midi_preset_bank(&mut self, bank: Vec<Preset>) -> Result<(), String> {
        self.midi_preset_editable()?;
        presets::validate_bank(&bank)?;
        let mut next = self.settings.applied.clone();
        next.profiles.get_mut(&next.active).unwrap().midi_presets = bank;
        next.validate()?;
        self.settings.worker.as_mut().unwrap().request(Job::Save {
            preferences: next,
            revision: self.settings.revision.clone(),
        })?;
        self.settings.message = "Saving MIDI preset bank; wait for the preferences receipt. Active assignments are unchanged.".into();
        Ok(())
    }
    fn midi_preset_port(&self, view: &learn::View) -> Result<(Endpoint, u64), String> {
        let port = self
            .midi_learn
            .presets
            .target
            .as_ref()
            .ok_or("Select the exact destination MIDI port")?;
        let mut devices = view
            .devices
            .iter()
            .filter(|device| &device.endpoint == port);
        let source = devices
            .next()
            .ok_or("Selected MIDI port is disconnected")?
            .source;
        if devices.next().is_some() {
            return Err("Selected MIDI port is ambiguous".into());
        }
        Ok((port.clone(), source))
    }
    /// Prepare a visible port replacement without applying it.
    /// Takes an optional preset (none restores factory defaults); captures the exact profile, port and mapping revision.
    fn review_midi_preset(&mut self, preset: Option<Preset>) -> Result<(), String> {
        self.midi_preset_editable()?;
        let view = self.engine.cmd.midi_learn().view();
        if view.armed || view.capture.is_some() {
            return Err("Finish or cancel MIDI capture before reviewing a preset".into());
        }
        let (port, source) = self.midi_preset_port(&view)?;
        let config = match &preset {
            Some(preset) => preset.target(&port, &view.config)?,
            None => presets::defaults(&port, &view.config),
        };
        config.validate()?;
        self.midi_learn.presets.review = Some(Review {
            origin: self.settings.applied.clone(),
            port,
            source,
            revision: view.revision,
            config,
            label: preset.map_or_else(|| "Factory defaults".into(), |preset| preset.name),
        });
        Ok(())
    }
    /// Commit exactly the reviewed live replacement.
    /// Takes no arguments; refuses stale profile, destination, reconnect or input revision and preserves existing mappings.
    fn confirm_midi_preset(&mut self) -> Result<(), String> {
        self.midi_preset_editable()?;
        let review = self
            .midi_learn
            .presets
            .review
            .take()
            .ok_or("Review a MIDI preset first")?;
        if self.settings.applied != review.origin
            || self.midi_learn.presets.target.as_ref() != Some(&review.port)
        {
            return Err("Preset profile or selected destination changed; review again".into());
        }
        self.engine.cmd.midi_learn().configure_reviewed(
            review.revision,
            review.source,
            &review.port,
            review.config,
        )?;
        self.midi_learn.presets.message = "Preset applied for this run. Save MIDI assignments to retain its exact port assignments.".into();
        Ok(())
    }
    /// Start a cancellable portable import owned by this editor and profile.
    /// Takes the entered absolute path; queues a read without changing the bank or active mappings.
    fn import_midi_preset(&mut self) -> Result<(), String> {
        self.midi_preset_editable()?;
        let panel = &mut self.midi_learn.presets;
        panel.token = panel
            .token
            .checked_add(1)
            .ok_or("Preset import identity exhausted")?;
        let token = panel.token;
        self.settings
            .worker
            .as_mut()
            .unwrap()
            .request(Job::ImportMidiPreset {
                path: PathBuf::from(&panel.path),
                token,
            })?;
        panel.pending = Some((token, self.settings.applied.clone()));
        panel.imported = None;
        panel.message = "Reading preset; no mappings have changed.".into();
        Ok(())
    }
    /// Deliver a read only to its unchanged originating editor and profile.
    /// Takes no arguments; cancelled, closed or stale replies never become reviewable imports.
    pub(super) fn poll_midi_preset_import(&mut self) {
        let panel = &mut self.midi_learn.presets;
        if !self.midi_open {
            panel.imported = None;
            panel.review = None;
        }
        if !self.midi_open && panel.pending.is_some() {
            if let Some(worker) = &self.settings.worker {
                worker.cancel();
            }
            panel.pending = None;
            panel.message = "Closed MIDI editor; pending preset import discarded.".into();
        }
        if let Some((token, preset)) = self.settings.midi_preset_import.take() {
            match panel.pending.take() {
                Some((expected, origin))
                    if token == expected
                        && origin == self.settings.applied
                        && self.settings.draft == origin
                        && self.midi_open =>
                {
                    panel.name = preset.name.clone();
                    panel.imported = Some((preset, origin));
                    panel.message = "Imported definition ready for review. Save it to the bank before explicitly loading it to a port.".into();
                }
                _ => {
                    panel.message =
                        "Preset import owner changed; current bank and mappings retained.".into()
                }
            }
        } else if !self.settings.busy() {
            panel.pending = None;
        }
    }
    fn adopt_midi_preset(&mut self) -> Result<(), String> {
        let (mut preset, origin) = self
            .midi_learn
            .presets
            .imported
            .clone()
            .ok_or("Import and review a preset first")?;
        if self.settings.applied != origin || self.settings.draft != origin {
            return Err("Import profile changed; read the preset again".into());
        }
        preset.name = self.midi_learn.presets.name.clone();
        let name = preset.name.clone();
        let mut bank = self.settings.profile().midi_presets.clone();
        bank.push(preset);
        self.save_midi_preset_bank(bank)?;
        self.midi_learn.presets.selected = Some(name);
        self.midi_learn.presets.imported = None;
        Ok(())
    }
    /// Render portable preset banking and explicit factory override review.
    /// Takes the MIDI panel and editing permission; filesystem work stays on the preference worker.
    pub(super) fn midi_presets_ui(&mut self, ui: &mut Ui, allowed: bool) {
        egui::CollapsingHeader::new("MIDI mapping presets").id_salt("midi_presets").show(ui, |ui| {
            ui.label("Named presets contain factory overrides. Other controls keep their factory behavior. Port IDs are assigned only when you load.");
            let view = self.engine.cmd.midi_learn().view();
            let bank = self.settings.profile().midi_presets.clone();
            let mut action = None;
            ui.add_enabled_ui(allowed, |ui| {
                let panel = &mut self.midi_learn.presets;
                let before = panel.target.clone();
                let destination = egui::ComboBox::from_label("Preset destination port").selected_text(panel.target.as_ref().map_or("Select exact port".into(), |port|format!("{} / {}",port.name,port.id))).show_ui(ui, |ui| {
                    if ui.selectable_value(&mut panel.target, None, "Select exact port").clicked() { ui.close(); }
                    for device in &view.devices { if ui.selectable_value(&mut panel.target, Some(device.endpoint.clone()), format!("{} / {}",device.endpoint.name,device.endpoint.id)).clicked() { ui.close(); } }
                });
                accessibility::button(ui, &destination.response, "Preset destination port", None);
                if before != panel.target { panel.review = None; }
                let before = panel.selected.clone();
                let saved = egui::ComboBox::from_label("Saved MIDI preset").selected_text(panel.selected.as_deref().unwrap_or("Select preset")).show_ui(ui, |ui| {
                    if ui.selectable_value(&mut panel.selected, None, "Select preset").clicked() { ui.close(); }
                    for preset in &bank { if ui.selectable_value(&mut panel.selected, Some(preset.name.clone()), &preset.name).clicked() { ui.close(); } }
                });
                accessibility::button(ui, &saved.response, "Saved MIDI preset", None);
                if before != panel.selected { panel.review = None; }
                preferences::text(ui, "MIDI preset name", &mut panel.name, HelpControl::MidiPresets);
                preferences::text(ui, "Device revision hint (optional)", &mut panel.revision_hint, HelpControl::MidiPresets);
                ui.horizontal_wrapped(|ui| {
                    for (id, label) in [(0,"Save port as preset"),(1,"Review preset load"),(2,"Duplicate preset"),(3,"Rename preset"),(4,"Delete preset"),(5,"Review factory defaults")] {
                        if ui.button(label).help(ui, HelpControl::MidiPresets).clicked() { action = Some(id); }
                    }
                });
                preferences::text(ui, "MIDI preset import or export path", &mut panel.path, HelpControl::PreferenceFilePath);
                ui.horizontal(|ui| {
                    if ui.button("Import MIDI preset").help(ui, HelpControl::PreferenceImport).clicked() { action = Some(6); }
                    if ui.button("Export MIDI preset").help(ui, HelpControl::PreferenceExport).clicked() { action = Some(7); }
                    if ui.add_enabled(panel.pending.is_some(), egui::Button::new("Cancel preset import")).clicked() {
                        if let Some(worker) = &self.settings.worker { worker.cancel(); }
                        panel.pending = None; panel.imported = None;
                    }
                });
                if let Some((preset, _)) = &panel.imported {
                    ui.label(format!("Import review: {} · device {} · revision {} · {} factory overrides",preset.name,preset.device_hint,preset.revision_hint,preset.bindings.len()));
                    for binding in &preset.bindings { ui.label(format!("Channel {} {:?} {} → {:?}, deck/track {}, target {}", binding.ch+1,binding.kind,binding.data,binding.action,binding.deck+1,binding.extra)); }
                    if ui.button("Save imported preset to bank").clicked() { action = Some(8); }
                    if ui.button("Discard imported preset").clicked() { panel.imported = None; }
                }
                if let Some(review) = &panel.review {
                    ui.label(format!("Replace {} / {} overrides with {}. Other ports and the saved bank stay intact.",review.port.name,review.port.id,review.label));
                    for mapping in review.config.mappings.iter().filter(|row|row.endpoint==review.port) { ui.label(format!("{:?}",mapping.binding)); }
                    if ui.button("Apply reviewed MIDI preset").clicked() { action = Some(9); }
                    if ui.button("Cancel MIDI preset review").clicked() { panel.review = None; }
                }
            });
            if let Some(action) = action {
                let result = (|| {
                    let selected = || bank.iter().find(|preset|Some(&preset.name)==self.midi_learn.presets.selected.as_ref()).cloned().ok_or_else(||"Select a saved MIDI preset".to_string());
                    match action {
                        0 => { self.midi_preset_editable()?; let (port,_) = self.midi_preset_port(&view)?; let preset = Preset::capture(self.midi_learn.presets.name.clone(),self.midi_learn.presets.revision_hint.clone(),&port,&view.config)?; let name=preset.name.clone();let mut next=bank;next.push(preset);self.save_midi_preset_bank(next)?;self.midi_learn.presets.selected=Some(name);Ok(()) },
                        1 => { let preset=selected()?;self.review_midi_preset(Some(preset)) },
                        2 | 3 => { let mut preset=selected()?;let old=preset.name.clone();preset.name=self.midi_learn.presets.name.clone();let name=preset.name.clone();let mut next=bank;if action==3 {next.retain(|preset|preset.name!=old);}next.push(preset);self.save_midi_preset_bank(next)?;self.midi_learn.presets.selected=Some(name);Ok(()) },
                        4 => { let preset=selected()?;let mut next=bank;next.retain(|item|item.name!=preset.name);self.save_midi_preset_bank(next)?;self.midi_learn.presets.selected=None;Ok(()) },
                        5 => self.review_midi_preset(None),
                        6 => self.import_midi_preset(),
                        7 => { self.midi_preset_editable()?;let preset=selected()?;self.settings.worker.as_mut().unwrap().request(Job::ExportMidiPreset {path:PathBuf::from(&self.midi_learn.presets.path),preset}) },
                        8 => self.adopt_midi_preset(),
                        9 => self.confirm_midi_preset(),
                        _ => unreachable!(),
                    }
                })();
                if let Err(error) = result { self.midi_learn.presets.message = error; }
            }
            ui.label(&self.midi_learn.presets.message);
        });
    }
}

#[cfg(test)]
mod tests;
