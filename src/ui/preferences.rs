use super::*;
use crate::engine::audio::config::{Inventory, Plan};
use crate::preferences::{
    self as model, storage,
    worker::{Event, Job, Startup, Worker},
};

pub(super) struct Settings {
    pub open: bool,
    pub applied: model::Preferences,
    pub draft: model::Preferences,
    pub message: String,
    pub startup_notice: Option<String>,
    pub path: Option<PathBuf>,
    pub worker: Option<Worker>,
    pub revision: Option<storage::Revision>,
    pub blocked: bool,
    pub preview: Option<(model::Preferences, Result<Plan, String>, Vec<String>)>,
    pub inventory: Option<Inventory>,
    pub running_audio: model::Audio,
    home: PathBuf,
    edited: String,
    profile_name: String,
    file_path: String,
    roots: String,
    midi_names: String,
    pub rescan: bool,
    pub theme_update: Option<Arc<crate::theme::reload::Update>>,
}
impl Default for Settings {
    fn default() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into()));
        Self::from_startup(
            Startup {
                path: PathBuf::new(),
                preferences: model::Preferences::defaults(&home),
                home,
                revision: None,
                blocked: false,
                diagnostic: None,
            },
            false,
            None,
        )
    }
}
impl Settings {
    fn from_startup(
        start: Startup,
        start_worker: bool,
        running_audio: Option<model::Audio>,
    ) -> Self {
        let worker = start_worker
            .then(|| Worker::start(start.path.clone()))
            .transpose();
        let message = match &worker {
            Err(error) => format!("Preferences worker unavailable: {error}"),
            Ok(_) => start.diagnostic.clone().unwrap_or_default(),
        };
        let mut state = Self {
            open: false,
            edited: start.preferences.active.clone(),
            profile_name: String::new(),
            running_audio: running_audio
                .unwrap_or_else(|| start.preferences.current().unwrap().audio.clone()),
            applied: start.preferences.clone(),
            draft: start.preferences,
            startup_notice: start.diagnostic,
            message,
            path: (!start.path.as_os_str().is_empty()).then_some(start.path),
            worker: worker.ok().flatten(),
            revision: start.revision,
            blocked: start.blocked,
            preview: None,
            inventory: None,
            home: start.home,
            file_path: String::new(),
            roots: String::new(),
            midi_names: String::new(),
            rescan: false,
            theme_update: None,
        };
        state.editor_strings();
        state
    }
    pub fn profile(&self) -> &model::Profile {
        self.applied.current().expect("validated settings")
    }
    pub fn pending_restart(&self) -> bool {
        !self.profile().audio.output_eq(&self.running_audio)
    }
    pub fn busy(&self) -> bool {
        self.worker.as_ref().is_some_and(Worker::busy)
    }
    fn request(&mut self, job: Job) {
        match self
            .worker
            .as_mut()
            .ok_or_else(|| "Preferences worker unavailable".into())
            .and_then(|worker| worker.request(job))
        {
            Ok(()) => self.message = "Preferences operation pending…".into(),
            Err(error) => self.message = error,
        }
    }
    fn editor_strings(&mut self) {
        if let Some(profile) = self.draft.profiles.get(&self.edited) {
            self.roots = profile
                .library_roots
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n");
            self.midi_names = match &profile.midi_inputs {
                model::MidiInputs::Selected(names) => names.join("\n"),
                _ => String::new(),
            };
        }
    }
    fn poll(&mut self) -> bool {
        let Some(event) = self.worker.as_mut().and_then(Worker::poll) else {
            return false;
        };
        match event {
            Event::Preview {
                preferences,
                inventory,
                plan,
                paths,
            } => {
                self.inventory = inventory.ok();
                self.preview = Some((preferences, plan, paths));
                self.message =
                    "Preview ready. Confirm Apply to save and change live settings.".into();
            }
            Event::Saved { preferences, saved } => {
                self.applied = preferences.clone();
                self.draft = preferences;
                self.edited = self.draft.active.clone();
                self.editor_strings();
                self.revision = saved.revision;
                self.blocked = self.revision.is_none();
                self.preview = None;
                self.message = saved.warning.unwrap_or_else(|| "Preferences saved. MIDI application is acknowledged below; use Audio devices to confirm a live output change, or restart.".into());
                return true;
            }
            Event::Imported(preferences, migrated) => {
                self.draft = preferences;
                self.edited = self.draft.active.clone();
                self.editor_strings();
                self.preview = None;
                self.message = if migrated {
                    "Imported and migrated into draft; preview before applying."
                } else {
                    "Imported into draft; current settings are unchanged."
                }
                .into();
            }
            Event::Reloaded(loaded) => {
                self.draft = loaded.preferences;
                self.edited = self.draft.active.clone();
                self.editor_strings();
                self.preview = None;
                self.revision = loaded.revision;
                self.blocked = false;
                self.message =
                    "Reloaded into draft. Preview and Apply to change this session.".into();
            }
            Event::Exported(saved) => {
                self.message = saved.warning.unwrap_or_else(|| {
                    "Preferences exported. Current settings and active profile unchanged.".into()
                })
            }
            Event::Failed(error) => self.message = format!("Preferences failed: {error}"),
            Event::Cancelled => self.message = "Cancelled; current settings are unchanged.".into(),
        }
        false
    }
}
impl App {
    pub(crate) fn initialize_preferences(
        &mut self,
        ctx: &egui::Context,
        startup: Startup,
        running_audio: model::Audio,
    ) {
        self.settings = Settings::from_startup(startup, !self.engine.safe_mode(), Some(running_audio));
        if self.engine.safe_mode() {
            self.settings.message = "Safe mode: saved preferences are available for inspection but not applied. Audio/MIDI, external theme and startup library scan remain disabled until an explicit normal restart.".into();
            return;
        }
        if let Some(worker) = &mut self.settings.worker { worker.set_performance(self.engine.cmd.performance().clone()); }
        self.initialize_project_panels(
            self.settings.profile().startup.show_help,
            self.settings.profile().startup.show_midi,
        );
        self.apply_appearance(ctx);
        if self.settings.profile().startup.scan_library {
            self.scan_library();
        }
    }
    pub(super) fn apply_appearance(&mut self, ctx: &egui::Context) {
        if self.engine.safe_mode() { self.theme=Theme::default(); self.theme.apply(ctx); ctx.set_zoom_factor(1.0); return; }
        let appearance = self.settings.profile().appearance.clone();
        if appearance.follow_theme {
            if let Some(update) = &self.settings.theme_update {
                self.theme = update.theme.clone();
                if !self
                    .theme_fonts
                    .as_ref()
                    .is_some_and(|fonts| Arc::ptr_eq(fonts, &update.fonts))
                {
                    ctx.set_fonts((*update.fonts).clone());
                    self.theme_fonts = Some(update.fonts.clone());
                }
            } else {
                self.theme = Theme::default();
            }
        } else {
            self.theme = Theme::default();
            if self.theme_fonts.take().is_some() {
                ctx.set_fonts(crate::theme::reload::fallback_fonts());
            }
        }
        if let Some(size) = appearance.font_size {
            self.theme.font_size = size;
        }
        self.theme.apply(ctx);
        ctx.set_zoom_factor(appearance.scale);
    }
    pub(super) fn poll_preferences(&mut self, ctx: &egui::Context) {
        if self.engine.safe_mode() { return; }
        if self.engine.cmd.performance().protected() {
            if self.settings.busy() { self.settings.message = "Preferences work is cancelled/deferred by performance protection. A committed file remains saved; its live application waits until protection is deliberately left.".into(); }
            return;
        }
        let old = self.settings.profile().clone();let old_profile=self.settings.applied.active.clone();
        if self.settings.poll() {
            self.apply_appearance(ctx);
            if old.library_roots != self.settings.profile().library_roots || old_profile!=self.settings.applied.active {
                self.library_scan.cancel();
                self.library_metadata.cancel_scan();
                self.settings.rescan = true;
            }
            if old.midi_inputs != self.settings.profile().midi_inputs {
                match self
                    .engine
                    .midi
                    .configure_inputs(self.settings.profile().midi_inputs.clone())
                {
                    Ok(generation) => self
                        .settings
                        .message
                        .push_str(&format!(" MIDI request {generation} queued.")),
                    Err(error) => self
                        .settings
                        .message
                        .push_str(&format!(" Saved MIDI policy is not applied: {error}")),
                }
            }
        }
        if self.settings.rescan && !self.library_scan.active() && self.library_metadata.ready() && !self.library_metadata.active() {
            self.settings.rescan = false;
            self.scan_library();
        }
        if self.settings.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
    pub(super) fn preferences_ui(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        if self.engine.safe_mode() {
            let mut open=true;
            egui::Window::new("Preferences and profiles").open(&mut open).show(ctx,|ui|{
                ui.label("Safe mode keeps saved preferences unchanged and does not apply audio, MIDI, theme or startup settings.");
                ui.label(format!("Saved active profile: {}",self.settings.applied.active));
                if let Some(notice)=&self.settings.startup_notice {ui.label(notice);}
                ui.label("Use Restart normally after saving your project to edit and apply profiles.");
            });
            self.settings.open=open;return;
        }
        let state = &mut self.settings;
        let mut open = true;
        let mut discard = false;
        egui::Window::new("Preferences and profiles").id(egui::Id::new("preferences-window"))
            .open(&mut open).default_width(680.0).default_height(620.0).show(ctx, |ui| {
                if self.project.committing() || !self.project.dialog_is_closed() { ui.disable(); }
                ui.label("Apply saves preferences. MIDI, folders, appearance and shortcuts follow that save. Audio can be applied explicitly in Audio devices, or after restart.");
                if ui.button("Audio devices and latency").help(ui, HelpControl::AudioDevices).clicked() { self.audio_settings.open = true; }
                if let Some(path) = &state.path { ui.label(format!("Preferences file: {}", path.display())); }
                if let Some(info) = self.engine.output_info() {
                    ui.label(format!("Running: {} · {} Hz · {} · {}", info.plan.device, info.plan.rate, info.format, info.plan.route()));
                } else { ui.label("Audio device unavailable in this session"); }
                if state.pending_restart() { ui.colored_label(Color32::YELLOW, "Saved audio differs from running intent; use Audio devices or restart"); }
                ui.label(&state.message);
                if !state.message.is_empty() && !state.busy() && ui.button("Dismiss preferences notice").help(ui, HelpControl::PreferenceNotice).clicked() { state.message.clear(); }

                if let Some(policy) = self.engine.midi.policy_status() {
                    ui.label(format!("MIDI requested generation {}: {:?}",policy.requested,policy.requested_policy));
                    ui.label(format!("MIDI applied generation {:?}: {:?}",policy.applied,policy.applied_policy));
                    if policy.pending() {ui.label("MIDI policy pending; existing permitted connections remain live.");ctx.request_repaint_after(std::time::Duration::from_millis(50));}
                    if let Some(error)=&policy.error {ui.colored_label(Color32::YELLOW,format!("MIDI: {error}"));}
                    if !policy.missing_names.is_empty(){ui.label(format!("Missing MIDI inputs: {}",policy.missing_names.join(", ")));}
                    if policy.requested_policy.as_ref()!=&state.profile().midi_inputs {ui.colored_label(Color32::YELLOW,"Saved MIDI input policy differs from the live request.");}
                    if ui.add_enabled(!policy.pending() && self.engine.midi.connections_available(),egui::Button::new("Retry saved MIDI policy")).help(ui, HelpControl::PreferenceMidiRetry).clicked(){
                        match self.engine.midi.configure_inputs(state.profile().midi_inputs.clone()) { Ok(generation)=>state.message=format!("MIDI request {generation} queued"),Err(error)=>state.message=error.to_string() }
                    }
                } else {ui.label("MIDI input manager unavailable; saved policy will be used after restart.");}
                if state.busy() {
                    if ui.button("Cancel pending preferences operation").help(ui, HelpControl::PreferenceCancel).clicked() { state.worker.as_ref().unwrap().cancel(); }
                }
                ui.add_enabled_ui(!state.busy(), |ui| {
                    egui::ScrollArea::vertical().id_salt("preferences-body").max_height(470.0).show(ui, |ui| {
                        let previous = state.edited.clone();
                        let profile_combo = egui::ComboBox::from_label("Edit profile").selected_text(&state.edited).show_ui(ui, |ui| {
                            for name in state.draft.profiles.keys() { ui.selectable_value(&mut state.edited, name.clone(), name).help(ui, HelpControl::PreferenceProfile); }
                        });
                        help::annotate(ui, &profile_combo.response, HelpControl::PreferenceProfile);
                        if previous != state.edited { state.editor_strings(); }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("Active profile on Apply: {}", state.draft.active));
                            if ui.button("Use this profile").help(ui, HelpControl::PreferenceActivate).clicked() { state.draft.active = state.edited.clone(); state.preview = None; }
                        });
                        text(ui, "New profile name", &mut state.profile_name, HelpControl::PreferenceRename);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Duplicate profile").help(ui, HelpControl::PreferenceClone).clicked() {
                                let name = state.profile_name.trim().to_owned();
                                if name.is_empty() || state.draft.profiles.contains_key(&name) { state.message = "Choose a new unique profile name.".into(); }
                                else if let Some(profile) = state.draft.profiles.get(&state.edited).cloned() {
                                    state.draft.profiles.insert(name.clone(), profile); state.edited = name; state.preview = None;
                                }
                            }
                            if ui.button("Rename profile").help(ui, HelpControl::PreferenceRename).clicked() {
                                let name = state.profile_name.trim().to_owned();
                                if name.is_empty() || state.draft.profiles.contains_key(&name) { state.message = "Choose a new unique profile name.".into(); }
                                else if let Some(profile) = state.draft.profiles.remove(&state.edited) {
                                    if state.draft.active == state.edited { state.draft.active = name.clone(); }
                                    state.draft.profiles.insert(name.clone(), profile); state.edited = name; state.preview = None;
                                }
                            }
                            if ui.add_enabled(state.draft.profiles.len()>1 && state.edited != state.draft.active, egui::Button::new("Delete profile")).help(ui, HelpControl::PreferenceDelete).clicked() {
                                state.draft.profiles.remove(&state.edited); state.edited = state.draft.active.clone(); state.editor_strings(); state.preview = None;
                            }
                            if ui.button("Reset profile to defaults").help(ui, HelpControl::PreferenceReset).clicked() {
                                state.draft.profiles.insert(state.edited.clone(), model::Profile::defaults(&state.home)); state.editor_strings(); state.preview = None;
                            }
                        });
                        if let Some(profile) = state.draft.profiles.get_mut(&state.edited) {
                            ui.separator(); ui.heading("Saved audio and calibration settings");
                            audio_settings::edit_profile(ui, &mut profile.audio, state.inventory.as_ref());
                            ui.heading("MIDI inputs");
                            if let Some(policy)=self.engine.midi.policy_status(){
                                ui.label(format!("Discovered MIDI names: {}",policy.available_inputs.join(", ")));
                                if policy.available_truncated {ui.label("Device preview is truncated; exact-name selection is still supported.");}
                            }
                            let mut mode = match profile.midi_inputs { model::MidiInputs::All=>0, model::MidiInputs::Selected(_)=>1, model::MidiInputs::Disabled=>2 };
                            let midi_combo = egui::ComboBox::from_label("MIDI input policy").selected_text(["All discovered inputs","Selected exact names","Disabled"][mode]).show_ui(ui, |ui| {
                                for (i,name) in ["All discovered inputs","Selected exact names","Disabled"].iter().enumerate() { ui.selectable_value(&mut mode,i,*name).help(ui, HelpControl::PreferenceMidiPolicy); }
                            });
                            help::annotate(ui, &midi_combo.response, HelpControl::PreferenceMidiPolicy);
                            if mode == 1 { multiline(ui, "Selected MIDI input names (one per line)", &mut state.midi_names, HelpControl::PreferenceMidiNames); }
                            profile.midi_inputs = match mode { 0=>model::MidiInputs::All,1=>model::MidiInputs::Selected(state.midi_names.lines().filter(|s|!s.is_empty()).map(str::to_owned).collect()),_=>model::MidiInputs::Disabled };
                            ui.heading("Library folders");
                            multiline(ui, "Library folders (one absolute path per line)", &mut state.roots, HelpControl::PreferenceLibraryRoots);
                            profile.library_roots = state.roots.lines().filter(|s| !s.is_empty()).map(PathBuf::from).collect();
                            ui.heading("Appearance");
                            ui.checkbox(&mut profile.appearance.follow_theme,"Follow desktop theme and font").help(ui, HelpControl::PreferenceTheme);
                            let mut custom = profile.appearance.font_size.is_some();
                            if ui.checkbox(&mut custom,"Override theme font size").help(ui, HelpControl::PreferenceFont).changed() { profile.appearance.font_size = custom.then_some(12.0); }
                            if let Some(size) = &mut profile.appearance.font_size { float_control(ui,"Font size",size,8.0,48.0,1.0," pt",HelpControl::PreferenceFont); }
                            float_control(ui,"UI scale",&mut profile.appearance.scale,0.5,3.0,0.05,"×",HelpControl::PreferenceScale);
                            ui.heading("Startup");
                            ui.checkbox(&mut profile.startup.performance_mode,"Enable performance protection on startup").help(ui, HelpControl::PerformanceMode);
                            ui.checkbox(&mut profile.startup.scan_library,"Scan library on startup").help(ui, HelpControl::PreferenceStartup);
                            ui.checkbox(&mut profile.startup.show_help,"Open help on startup").help(ui, HelpControl::PreferenceStartup);
                            ui.checkbox(&mut profile.startup.show_midi,"Open MIDI panel on startup").help(ui, HelpControl::PreferenceStartup);
                            ui.collapsing("Keyboard shortcuts", |ui| {
                                ui.checkbox(&mut profile.shortcuts_enabled,"Enable performance shortcuts").help(ui, HelpControl::PreferenceShortcut);
                                ui.label("Navigation, focused controls and reserved project keys remain available. Empty key disables this shortcut.");
                                for binding in shortcuts::BINDINGS {
                                    accessibility::group(ui, binding.description, |ui| {
                                    ui.push_id(binding.id(), |ui| {
                                        ui.label(binding.description);
                                        let mut value = binding.effective(profile).unwrap_or(model::Shortcut {key:String::new(),ctrl:false,shift:false,alt:false});
                                        let before = value.clone();
                                        ui.horizontal(|ui| {
                                            text(ui, &format!("{} key",binding.description), &mut value.key, HelpControl::PreferenceShortcut);
                                            ui.checkbox(&mut value.ctrl,"Ctrl").help(ui, HelpControl::PreferenceShortcut);ui.checkbox(&mut value.alt,"Alt").help(ui, HelpControl::PreferenceShortcut);ui.checkbox(&mut value.shift,"Shift").help(ui, HelpControl::PreferenceShortcut);
                                            if ui.button("Default").help(ui, HelpControl::PreferenceShortcut).clicked() { profile.shortcuts.remove(binding.id()); }
                                        });
                                        if value != before { profile.shortcuts.insert(binding.id().into(),(!value.key.is_empty()).then_some(value)); }
                                    });
                                    });
                                }
                            });
                            ui.collapsing("Autosave and recovery limits", |ui| recovery_settings::edit(ui, &mut profile.recovery))
                                .header_response.help(ui, HelpControl::RecoverySettings);
                        }
                        ui.separator();
                        text(ui, "Preferences import or export path", &mut state.file_path, HelpControl::PreferenceFilePath);
                        ui.label("Export contains only these settings. No credentials, environment, runtime tokens or device connection handles. Device names and library paths remain explicit and may need changing on another machine. Export never overwrites an existing file.");
                        ui.horizontal(|ui| {
                            if ui.button("Import into draft").help(ui, HelpControl::PreferenceImport).clicked() { state.request(Job::Import(PathBuf::from(&state.file_path))); }
                            if ui.button("Export draft").help(ui, HelpControl::PreferenceExport).clicked() { state.request(Job::Export{path:PathBuf::from(&state.file_path),preferences:state.draft.clone()}); }
                            if ui.button("Reload saved preferences").help(ui, HelpControl::PreferenceReload).clicked() { state.request(Job::Reload); }
                        });
                        if state.blocked {
                            ui.colored_label(Color32::YELLOW,"Saving is blocked to preserve an unreadable or changed preferences file.");
                            if ui.button("Preserve old file and reset all preferences").help(ui, HelpControl::PreferenceRecover).clicked() { state.request(Job::Reset(model::Preferences::defaults(&state.home))); }
                        }
                        if let Some((preview, plan, paths)) = &state.preview {
                            if preview == &state.draft {
                                ui.heading(format!("Preview: {}",preview.active));
                                match plan {
                                    Ok(plan)=> {ui.label(format!("Saved output proposal: {} · {} Hz · {} channels · {}",plan.device,plan.rate,plan.channels,plan.route()));if let Some(warning)=&plan.warning{ui.label(warning);}},
                                    Err(error)=> {ui.colored_label(Color32::YELLOW,format!("Audio unavailable: {error}"));},
                                }
                                let midi = &preview.current().unwrap().midi_inputs;
                                ui.label(format!("MIDI policy: {midi:?}"));
                                if let Some(status)=self.engine.midi.policy_status() {
                                    if status.pending() || status.error.is_some() {ui.colored_label(Color32::YELLOW,"MIDI discovery is pending or failed; availability cannot be fully verified yet.");}
                                    if let model::MidiInputs::Selected(names)=midi {
                                        let missing=names.iter().filter(|name|!status.available_inputs.contains(name)).cloned().collect::<Vec<_>>();
                                        if !missing.is_empty(){ui.colored_label(Color32::YELLOW,format!("Not in current MIDI discovery preview: {}{}",missing.join(", "),if status.available_truncated{" (preview is truncated)"}else{""}));}
                                    }
                                } else {ui.colored_label(Color32::YELLOW,"MIDI manager unavailable; this selection can only apply after restart.");}
                                ui.label("Permitted live MIDI connections remain connected; excluded sources are released by the MIDI worker after Apply.");
                                for path in paths { ui.label(path); }
                                let current = preview.current().unwrap();
                                ui.label(format!("Appearance: scale {:.0}%, theme {}, font {:?}; shortcuts {}; startup scan {}, help {}, MIDI {}",current.appearance.scale*100.0,current.appearance.follow_theme,current.appearance.font_size,current.shortcuts_enabled,current.startup.scan_library,current.startup.show_help,current.startup.show_midi));
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Preview changes").help(ui, HelpControl::PreferencePreview).clicked() { state.request(Job::Preview(state.draft.clone())); }
                        let ready = state.preview.as_ref().is_some_and(|(draft,plan,_)| draft==&state.draft && (plan.is_ok() || draft.current().map(|p| &p.audio)==state.applied.current().map(|p| &p.audio)));
                        if ui.add_enabled(ready && !state.blocked,egui::Button::new("Apply and save")).help(ui, HelpControl::PreferenceApply).clicked() {
                            state.request(Job::Save {preferences:state.draft.clone(),revision:state.revision.clone()});
                        }
                        if ui.button("Cancel changes").help(ui, HelpControl::PreferenceCancel).clicked() { discard = true; }
                    });
                });
            });
        if !open || discard {
            if state.busy() {
                state.worker.as_ref().unwrap().cancel();
            }
            state.draft = state.applied.clone();
            state.edited = state.draft.active.clone();
            state.editor_strings();
            state.preview = None;
            state.open = false;
        }
    }
}
fn text(ui: &mut Ui, name: &str, value: &mut String, control: HelpControl) -> bool {
    let label = ui.label(name);
    ui.text_edit_singleline(value)
        .labelled_by(label.id)
        .help(ui, control).changed()
}
fn multiline(ui: &mut Ui, name: &str, value: &mut String, control: HelpControl) {
    let label = ui.label(name);
    ui.add(
        egui::TextEdit::multiline(value)
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id).help(ui, control);
}
pub(super) fn float_control(
    ui: &mut Ui,
    name: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &str,
    control: HelpControl,
) {
    let original = *value;
    let response = ui.add(egui::Slider::new(value, min..=max).text(name));
    if let Some(alternate) =
        accessibility::numeric(ui, &response, name, original, min, max, step, unit)
    {
        *value = alternate;
    }
    help::annotate(ui, &response, control);
}
#[cfg(test)]
mod tests;
