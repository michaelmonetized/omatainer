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
    shortcut_capture: Option<String>,
    pub(super) workspace_open: bool,
    workspace_name: String,
    roots: String,
    midi_names: String,
    pub rescan: bool,
    pub(super) routing_pending: bool,
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
    #[cfg(test)]
    pub(super) fn with_worker_for_test(path:PathBuf) -> Self {
        let home=path.parent().unwrap().to_path_buf();
        Self::from_startup(Startup{path,preferences:model::Preferences::defaults(&home),home,revision:None,blocked:false,diagnostic:None},true,None)
    }

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
            shortcut_capture: None,
            workspace_open: false,
            workspace_name: String::new(),
            roots: String::new(),
            midi_names: String::new(),
            rescan: false,
            routing_pending: false,
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
    pub(super) fn request(&mut self, job: Job) {
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
            Event::ShortcutsImported(bundle) => {
                let profile = self.draft.profiles.get_mut(&self.edited).expect("selected draft profile");
                self.message = match bundle.apply(profile) {
                    Ok(()) => { self.preview = None; "Bindings imported into draft. Preview and Apply to save them.".into() },
                    Err(error) => error,
                };
            }
            Event::ShortcutsExported(saved) => {
                self.message = saved.warning.unwrap_or_else(|| "Bindings exported. Current settings are unchanged.".into());
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
        self.apply_automation();
        self.library_layout.apply(self.settings.profile().library_layout.clone());
        self.waveform.settings = self.settings.profile().waveforms;
        if self.settings.profile().startup.scan_library {
            self.scan_library();
        }
    }
    pub(super) fn review_template_hardware(&mut self, metadata: &crate::project_template::Metadata, target: Option<crate::project_template::Target>) {
        let result = (|| {
            metadata.validate()?;
            if self.settings.busy() || self.settings.draft != self.settings.applied {
                return Err("Finish or cancel the existing preference draft before reviewing template hardware".into());
            }
            let mut profile = self.settings.profile().clone();
            if matches!(metadata.kind, crate::project_template::Kind::Track { .. }) {
                let slot = target.and_then(|target| self.snap.session.as_ref().and_then(|layout| target.resolve(layout)))
                    .ok_or_else(|| "The template target was replaced or deleted; inspect and select it again".to_string())?;
                profile.audio = metadata.hardware.audio.clone();
                profile.midi_inputs = metadata.hardware.midi_inputs.clone();
                profile.midi_routing.routes.retain(|route| usize::from(route.track) != slot);
                for route in &metadata.hardware.routing.routes {
                    let mut route = route.clone(); route.track = slot as u8;
                    profile.midi_routing.routes.push(route);
                }
                profile.midi_routing.enabled |= metadata.hardware.routing.enabled;
            } else { metadata.hardware.apply(&mut profile); }
            profile.validate()?;
            self.settings.edited = self.settings.draft.active.clone();
            self.settings.draft.profiles.insert(self.settings.edited.clone(), profile);
            self.settings.editor_strings();
            self.settings.preview = None;
            self.settings.open = true;
            self.settings.message = "Template hardware is a draft. Preview exact endpoints, then Apply deliberately. Cancel preserves the active profile and all connections.".into();
            Ok::<(), String>(())
        })();
        if let Err(error) = result { self.templates.error = Some(error); }
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
        self.theme.configure_display(&appearance);
        ctx.set_zoom_factor(appearance.scale);
        self.theme.apply(ctx);
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
            self.apply_automation();
            if old.library_layout != self.settings.profile().library_layout || old_profile != self.settings.applied.active { self.library_layout.apply(self.settings.profile().library_layout.clone()); }
            if old_profile != self.settings.applied.active || (old.waveforms != self.settings.profile().waveforms && self.waveform.settings == old.waveforms) { self.waveform.settings = self.settings.profile().waveforms; }
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
            if old.midi_learn != self.settings.profile().midi_learn {
                if let Err(error)=self.engine.cmd.midi_learn().configure(self.settings.profile().midi_learn.clone()) {self.settings.message.push_str(&format!(" Saved MIDI assignments are not applied: {error}"));}
            }
            if old.midi_routing != self.settings.profile().midi_routing {
                self.settings.routing_pending=true;
            }
        }
        self.poll_library_layout_save();
        if self.settings.routing_pending && !self.engine.midi.connections_busy() && self.engine.midi.policy_status().is_none_or(|s|!s.pending()) {
            self.settings.routing_pending=false;
            match self.engine.midi.configure_routing(self.settings.profile().midi_routing.clone()) {
                Ok(generation)=>self.settings.message.push_str(&format!(" MIDI routing request {generation} queued.")),
                Err(error)=>self.settings.message.push_str(&format!(" Saved MIDI routing is not applied: {error}")),
            }
        }
        if self.settings.rescan && !self.library_scan.active() && self.library_metadata.ready() && !self.library_metadata.active() && !self.engine.cmd.performance().protected() && !self.project.committing() {
            self.settings.rescan = false;
            self.scan_library();
        }
        if self.settings.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
    pub(super) fn preferences_ui(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            self.settings.shortcut_capture = None;
            return;
        }
        keyboard::block_for_dialog(ctx);
        if let Some(id) = self.settings.shortcut_capture.clone() {
            let captured = ctx.input(|input| input.events.iter().find_map(|event| match event {
                egui::Event::Key { key, modifiers, pressed: true, repeat: false, .. } => Some((*key, *modifiers)),
                _ => None,
            }));
            if self.settings.busy() || self.engine.safe_mode() { self.settings.shortcut_capture = None; }
            else if let Some((key, modifiers)) = captured {
                self.settings.shortcut_capture = None;
                if key == Key::Escape { self.settings.message = "Shortcut capture cancelled; bindings are unchanged.".into(); }
                else if modifiers.mac_cmd { self.settings.message = "This Linux profile accepts Ctrl, Alt and Shift modifiers.".into(); }
                else {
                    let shortcut = model::Shortcut { key: key.name().into(), ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift };
                    match shortcut.validate() {
                        Ok(()) => {
                            self.settings.draft.profiles.get_mut(&self.settings.edited).unwrap().shortcuts.insert(id, Some(shortcut));
                            self.settings.preview = None;
                            self.settings.message = "Shortcut captured into draft. Resolve any collisions, then Preview and Apply.".into();
                        },
                        Err(error) => self.settings.message = error,
                    }
                }
            }
        }
        if self.engine.safe_mode() {
            let mut open=true;
            egui::Window::new(tr!("Preferences and profiles")).id(egui::Id::new("Preferences and profiles")).open(&mut open).show(ctx,|ui|{
                ui.label(tr!("Safe mode keeps saved preferences unchanged and does not apply audio, MIDI, theme or startup settings."));
                ui.label({ let __omatainer_args = (&(self.settings.applied.active),); crate::localization::format("Saved active profile: {}", &[format!("{}", __omatainer_args.0)]) });
                if let Some(notice)=&self.settings.startup_notice {ui.label(notice);}
                ui.label(tr!("Use Restart normally after saving your project to edit and apply profiles."));
            });
            self.settings.open=open;return;
        }
        let state = &mut self.settings;
        let mut open = true;
        let mut discard = false;
        egui::Window::new(tr!("Preferences and profiles")).id(egui::Id::new("preferences-window"))
            .open(&mut open).default_width(680.0).default_height(620.0).vscroll(true).hscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
                if self.project.committing() || !self.project.dialog_is_closed() { ui.disable(); }
                ui.label(tr!("Apply saves preferences. MIDI, folders, appearance and shortcuts follow that save. Audio can be applied explicitly in Audio devices, or after restart."));
                if ui.button(tr!("Audio devices and latency")).help(ui, HelpControl::AudioDevices).clicked() { self.audio_settings.open = true; }
                if let Some(path) = &state.path { ui.label({ let __omatainer_args = (&(path.display()),); crate::localization::format("Preferences file: {}", &[format!("{}", __omatainer_args.0)]) }); }
                if let Some(info) = self.engine.output_info() {
                    ui.label({ let __omatainer_args = (&(info.plan.device),&(info.plan.rate),&(info.format),&(info.plan.route()),); crate::localization::format("Running: {} · {} Hz · {} · {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });
                } else { ui.label(tr!("Audio device unavailable in this session")); }
                if state.pending_restart() { ui.colored_label(ui.visuals().warn_fg_color, tr!("Saved audio differs from running intent; use Audio devices or restart")); }
                let network=self.automation_network.status();
                if network.pending {ui.label(tr!("OSC configuration is pending"));ctx.request_repaint_after(std::time::Duration::from_millis(20));}
                if let Some(error)=&network.error {ui.colored_label(ui.visuals().warn_fg_color,error);}
                if network.error.is_some() || network.applied!=Some(state.profile().automation) {
                    if ui.button(tr!("Retry saved OSC settings")).help(ui,HelpControl::AutomationRetry).clicked() {
                        if let Err(error)=self.automation_network.configure(state.profile().automation) {state.message=error;}
                    }
                }
                ui.label(&state.message);
                if !state.message.is_empty() && !state.busy() && ui.button(tr!("Dismiss preferences notice")).help(ui, HelpControl::PreferenceNotice).clicked() { state.message.clear(); }

                if let Some(policy) = self.engine.midi.policy_status() {
                    ui.label({ let __omatainer_args = (&(policy.requested),&(policy.requested_policy),); crate::localization::format("MIDI requested generation {}: {:?}", &[format!("{}", __omatainer_args.0), format!("{:?}", __omatainer_args.1)]) });
                    ui.label({ let __omatainer_args = (&(policy.applied),&(policy.applied_policy),); crate::localization::format("MIDI applied generation {:?}: {:?}", &[format!("{:?}", __omatainer_args.0), format!("{:?}", __omatainer_args.1)]) });
                    if policy.pending() {ui.label(tr!("MIDI policy pending; existing permitted connections remain live."));ctx.request_repaint_after(std::time::Duration::from_millis(50));}
                    if let Some(error)=&policy.error {ui.colored_label(ui.visuals().warn_fg_color,crate::localization::format("MIDI: {error}", &[format!("{}", error)]));}
                    if !policy.missing_names.is_empty(){ui.label({ let __omatainer_args = (&(policy.missing_names.join(", ")),); crate::localization::format("Missing MIDI inputs: {}", &[format!("{}", __omatainer_args.0)]) });}
                    if policy.requested_policy.as_ref()!=&state.profile().midi_inputs {ui.colored_label(ui.visuals().warn_fg_color,tr!("Saved MIDI input policy differs from the live request."));}
                    if ui.add_enabled(!policy.pending() && self.engine.midi.connections_available(),egui::Button::new(tr!("Retry saved MIDI policy"))).help(ui, HelpControl::PreferenceMidiRetry).clicked(){
                        match self.engine.midi.configure_inputs(state.profile().midi_inputs.clone()) { Ok(generation)=>state.message=format!("MIDI request {generation} queued"),Err(error)=>state.message=error.to_string() }
                    }
                } else {ui.label(tr!("MIDI input manager unavailable; saved policy will be used after restart."));}
                if let Some(routing)=self.engine.midi.routing_status(){
                    if routing.pending{ui.label(tr!("MIDI routing is pending; cancel in the MIDI panel."));ctx.request_repaint_after(std::time::Duration::from_millis(50));}
                    if let Some(error)=&routing.error{ui.colored_label(ui.visuals().warn_fg_color,error);}
                    if routing.applied.as_ref()!=&state.profile().midi_routing{ui.colored_label(ui.visuals().warn_fg_color,tr!("Saved MIDI routing differs from the applied route."));}
                    if ui.add_enabled(!routing.pending&&!state.routing_pending,egui::Button::new(tr!("Retry saved MIDI routing"))).help(ui,HelpControl::MidiRouteEnable).clicked(){
                        state.routing_pending=true;
                    }
                }
                if state.busy() {
                    if ui.button(tr!("Cancel pending preferences operation")).help(ui, HelpControl::PreferenceCancel).clicked() { state.worker.as_ref().unwrap().cancel(); }
                }
                ui.add_enabled_ui(!state.busy(), |ui| {
                    egui::ScrollArea::vertical().id_salt("preferences-body").max_height(470.0).show(ui, |ui| {
                        let previous = state.edited.clone();
                        let profile_combo = egui::ComboBox::from_label(tr!("Edit profile")).selected_text(&state.edited).show_ui(ui, |ui| {
                            for name in state.draft.profiles.keys() { ui.selectable_value(&mut state.edited, name.clone(), name).help(ui, HelpControl::PreferenceProfile); }
                        });
                        help::annotate(ui, &profile_combo.response, HelpControl::PreferenceProfile);
                        if previous != state.edited { state.editor_strings(); state.shortcut_capture = None; }
                        ui.horizontal_wrapped(|ui| {
                            ui.label({ let __omatainer_args = (&(state.draft.active),); crate::localization::format("Active profile on Apply: {}", &[format!("{}", __omatainer_args.0)]) });
                            if ui.button(tr!("Use this profile")).help(ui, HelpControl::PreferenceActivate).clicked() { state.draft.active = state.edited.clone(); state.preview = None; }
                        });
                        text(ui, "New profile name", &mut state.profile_name, HelpControl::PreferenceRename);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr!("Duplicate profile")).help(ui, HelpControl::PreferenceClone).clicked() {
                                let name = state.profile_name.trim().to_owned();
                                if name.is_empty() || state.draft.profiles.contains_key(&name) { state.message = "Choose a new unique profile name.".into(); }
                                else if let Some(profile) = state.draft.profiles.get(&state.edited).cloned() {
                                    state.draft.profiles.insert(name.clone(), profile); state.edited = name; state.preview = None;
                                }
                            }
                            if ui.button(tr!("Rename profile")).help(ui, HelpControl::PreferenceRename).clicked() {
                                let name = state.profile_name.trim().to_owned();
                                if name.is_empty() || state.draft.profiles.contains_key(&name) { state.message = "Choose a new unique profile name.".into(); }
                                else if let Some(profile) = state.draft.profiles.remove(&state.edited) {
                                    if state.draft.active == state.edited { state.draft.active = name.clone(); }
                                    state.draft.profiles.insert(name.clone(), profile); state.edited = name; state.preview = None;
                                }
                            }
                            if ui.add_enabled(state.draft.profiles.len()>1 && state.edited != state.draft.active, egui::Button::new(tr!("Delete profile"))).help(ui, HelpControl::PreferenceDelete).clicked() {
                                state.draft.profiles.remove(&state.edited); state.edited = state.draft.active.clone(); state.editor_strings(); state.preview = None;
                            }
                            if ui.button(tr!("Reset profile to defaults")).help(ui, HelpControl::PreferenceReset).clicked() {
                                state.draft.profiles.insert(state.edited.clone(), model::Profile::defaults(&state.home)); state.editor_strings(); state.preview = None;
                            }
                        });
                        if let Some(profile) = state.draft.profiles.get_mut(&state.edited) {
                            if ui.button(tr!("Configure panel layout")).help(ui,HelpControl::Preferences).clicked() {state.workspace_open=!state.workspace_open;}
                            if state.workspace_open {
                                let before=profile.workspaces.clone();
                                workspace::edit(ui,&mut profile.workspaces,&mut state.workspace_name,&mut state.message,&self.workspace.sizes);
                                if before!=profile.workspaces {state.preview=None;}
                            }
                            ui.separator(); ui.heading(tr!("Saved audio and calibration settings"));
                            audio_settings::edit_profile(ui, &mut profile.audio, state.inventory.as_ref());
                            ui.heading(tr!("MIDI inputs"));
                            if let Some(policy)=self.engine.midi.policy_status(){
                                ui.label({ let __omatainer_args = (&(policy.available_inputs.join(", ")),); crate::localization::format("Discovered MIDI names: {}", &[format!("{}", __omatainer_args.0)]) });
                                if policy.available_truncated {ui.label(tr!("Device preview is truncated; exact-name selection is still supported."));}
                            }
                            let mut mode = match profile.midi_inputs { model::MidiInputs::All=>0, model::MidiInputs::Selected(_)=>1, model::MidiInputs::Disabled=>2 };
                            let midi_combo = egui::ComboBox::from_label(tr!("MIDI input policy")).selected_text(["All discovered inputs","Selected exact names","Disabled"][mode]).show_ui(ui, |ui| {
                                for (i,name) in ["All discovered inputs","Selected exact names","Disabled"].iter().enumerate() { ui.selectable_value(&mut mode,i,*name).help(ui, HelpControl::PreferenceMidiPolicy); }
                            });
                            help::annotate(ui, &midi_combo.response, HelpControl::PreferenceMidiPolicy);
                            if mode == 1 { multiline(ui, "Selected MIDI input names (one per line)", &mut state.midi_names, HelpControl::PreferenceMidiNames); }
                            profile.midi_inputs = match mode { 0=>model::MidiInputs::All,1=>model::MidiInputs::Selected(state.midi_names.lines().filter(|s|!s.is_empty()).map(str::to_owned).collect()),_=>model::MidiInputs::Disabled };
                            midi_routing::edit(ui,&mut profile.midi_routing,self.engine.midi.routing_status().as_deref());
                            ui.heading(tr!("Automation and remote control"));
                            ui.checkbox(&mut profile.automation.enabled,tr!("Enable loopback OSC")).help(ui,HelpControl::AutomationOscEnable);
                            ui.horizontal(|ui| {
                                let label=ui.label(tr!("OSC port (0 = automatic)"));
                                let response=ui.add(egui::DragValue::new(&mut profile.automation.port).range(0..=65535)).labelled_by(label.id);
                                ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("OSC port (0 = automatic)"));
                                help::annotate(ui,&response,HelpControl::AutomationOscPort);
                            });
                            ui.label(tr!("Apply saves the listener intent. The actual listener and access token are shown in Automation. Cancel keeps the current listener."));
                            ui.heading(tr!("Library folders"));
                            multiline(ui, "Library folders (one absolute path per line)", &mut state.roots, HelpControl::PreferenceLibraryRoots);
                            profile.library_roots = state.roots.lines().filter(|s| !s.is_empty()).map(PathBuf::from).collect();
                            ui.heading(tr!("Appearance"));
                            ui.horizontal_wrapped(|ui| {
                                ui.label(tr!("Interface language"));
                                for locale in crate::localization::Locale::ALL { ui.radio_value(&mut profile.appearance.locale, locale, locale.name()).help(ui, HelpControl::PreferenceLanguage); }
                            });
                            ui.label(tr!("Language changes after Apply. Cancel preserves the current language."));
                            ui.checkbox(&mut profile.appearance.follow_theme,tr!("Follow desktop theme and font")).help(ui, HelpControl::PreferenceTheme);
                            let mut custom = profile.appearance.font_size.is_some();
                            if ui.checkbox(&mut custom,tr!("Override theme font size")).help(ui, HelpControl::PreferenceFont).changed() { profile.appearance.font_size = custom.then_some(12.0); }
                            if let Some(size) = &mut profile.appearance.font_size { float_control(ui,"Font size",size,8.0,48.0,1.0," pt",HelpControl::PreferenceFont); }
                            float_control(ui,"UI scale",&mut profile.appearance.scale,0.5,3.0,0.05,"×",HelpControl::PreferenceScale);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(tr!("Contrast"));
                                for (mode,label) in [(crate::theme::Contrast::Theme,"Desktop colors"),(crate::theme::Contrast::Dark,"High contrast dark"),(crate::theme::Contrast::Light,"High contrast light")] { ui.radio_value(&mut profile.appearance.contrast,mode,label).help(ui,HelpControl::DisplayContrast); }
                            });
                            ui.checkbox(&mut profile.appearance.reduced_motion,tr!("Reduce decorative motion")).help(ui,HelpControl::DisplayMotion);
                            float_control(ui,"Waveform contrast",&mut profile.appearance.waveform_contrast,1.0,3.0,0.1,"",HelpControl::DisplayWaveform);
                            float_control(ui,"Level contrast",&mut profile.appearance.level_contrast,1.0,3.0,0.1,"",HelpControl::DisplayLevel);
                            ui.label(tr!("Appearance changes preserve audio. Playing, queued and active states also use text or shapes. Controls keep a readable minimum as UI scale decreases."));
                            ui.heading(tr!("Startup"));
                            let mut choice = match profile.startup.session { crate::project_template::Startup::Demo => 0, crate::project_template::Startup::Empty => 1, crate::project_template::Startup::Template { .. } => 2 };
                            let before = choice;
                            ui.horizontal(|ui| {
                                ui.label(tr!("Startup session"));
                                for (index, label) in ["Demo session", "Empty session", "Project template"].into_iter().enumerate() { ui.radio_value(&mut choice, index, label).help(ui, HelpControl::TemplateStartup); }
                            });
                            if choice != before { profile.startup.session = match choice { 0 => crate::project_template::Startup::Demo, 1 => crate::project_template::Startup::Empty, _ => crate::project_template::Startup::Template { path: PathBuf::new() } }; }
                            if let crate::project_template::Startup::Template { path } = &mut profile.startup.session {
                                let mut path_text = path.to_string_lossy().into_owned();
                                ui.label(tr!("Startup template file"));
                                let response = ui.add(egui::TextEdit::singleline(&mut path_text).char_limit(4096)).help(ui, HelpControl::TemplateStartup);
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Startup template file"));
                                if response.changed() { *path = PathBuf::from(path_text); }
                            }
                            ui.label(tr!("Startup templates retain creative state. Hardware stays on the active profile until explicitly reviewed and applied in Templates → Review hardware."));
                            ui.checkbox(&mut profile.startup.performance_mode,tr!("Enable performance protection on startup")).help(ui, HelpControl::PerformanceMode);
                            ui.checkbox(&mut profile.startup.scan_library,tr!("Scan library on startup")).help(ui, HelpControl::PreferenceStartup);
                            ui.checkbox(&mut profile.startup.show_help,tr!("Open help on startup")).help(ui, HelpControl::PreferenceStartup);
                            ui.checkbox(&mut profile.startup.show_midi,tr!("Open MIDI panel on startup")).help(ui, HelpControl::PreferenceStartup);
                            ui.collapsing("Keyboard shortcuts", |ui| {
                                ui.checkbox(&mut profile.shortcuts_enabled,tr!("Enable performance shortcuts")).help(ui, HelpControl::PreferenceShortcut);
                                ui.label(tr!("Navigation, focused controls and reserved project keys remain available. Empty key disables this shortcut."));
                                ui.label(tr!("Text editing owns typing keys. Editor actions change creative edits; performance actions control playback. Logical keys follow the active keyboard layout; unsupported non-Latin keys use egui's physical-key fallback."));
                                if ui.button(tr!("Reset all shortcut bindings")).help(ui, HelpControl::PreferenceShortcut).clicked() {
                                    profile.shortcuts.clear(); profile.shortcuts_enabled = true; state.shortcut_capture = None;
                                }
                                if let Some(id) = &state.shortcut_capture {
                                    ui.label(crate::localization::format("Press the new shortcut for {0}; Escape cancels.", &[id.clone()]));
                                }
                                if let Err(error) = shortcuts::validate(profile) { ui.colored_label(ui.visuals().warn_fg_color, error); }
                                for binding in shortcuts::BINDINGS {
                                    accessibility::group(ui, binding.description, |ui| {
                                    ui.push_id(binding.id(), |ui| {
                                        ui.label(binding.description);
                                        ui.label(crate::localization::text_dynamic(binding.action.context()));
                                        let mut value = binding.effective(profile).unwrap_or(model::Shortcut {key:String::new(),ctrl:false,shift:false,alt:false});
                                        let before = value.clone();
                                        ui.horizontal(|ui| {
                                            text(ui, &format!("{} key",binding.description), &mut value.key, HelpControl::PreferenceShortcut);
                                            ui.checkbox(&mut value.ctrl,tr!("Ctrl")).help(ui, HelpControl::PreferenceShortcut);ui.checkbox(&mut value.alt,tr!("Alt")).help(ui, HelpControl::PreferenceShortcut);ui.checkbox(&mut value.shift,tr!("Shift")).help(ui, HelpControl::PreferenceShortcut);
                                            if ui.button(tr!("Capture key")).help(ui, HelpControl::PreferenceShortcut).clicked() { state.shortcut_capture = Some(binding.id().into()); }
                                            if ui.button(tr!("Default")).help(ui, HelpControl::PreferenceShortcut).clicked() { profile.shortcuts.remove(binding.id()); }
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
                        ui.label(tr!("Export contains only these settings. No credentials, environment, runtime tokens or device connection handles. Device names and library paths remain explicit and may need changing on another machine. Export never overwrites an existing file."));
                        ui.horizontal(|ui| {
                            if ui.button(tr!("Import into draft")).help(ui, HelpControl::PreferenceImport).clicked() { state.request(Job::Import(PathBuf::from(&state.file_path))); }
                            if ui.button(tr!("Export draft")).help(ui, HelpControl::PreferenceExport).clicked() { state.request(Job::Export{path:PathBuf::from(&state.file_path),preferences:state.draft.clone()}); }
                            if ui.button(tr!("Reload saved preferences")).help(ui, HelpControl::PreferenceReload).clicked() { state.request(Job::Reload); }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr!("Import bindings into draft")).help(ui, HelpControl::PreferenceImport).clicked() { state.request(Job::ImportShortcuts(PathBuf::from(&state.file_path))); }
                            if ui.button(tr!("Export bindings only")).help(ui, HelpControl::PreferenceExport).clicked() { state.request(Job::ExportShortcuts { path: PathBuf::from(&state.file_path), profile: state.draft.profiles[&state.edited].clone() }); }
                        });
                        ui.label(tr!("Binding exports include only shortcut overrides and the performance shortcut switch for the edited profile. Import changes the draft; Cancel preserves saved bindings. Export never replaces an existing file."));
                        if state.blocked {
                            ui.colored_label(ui.visuals().warn_fg_color,tr!("Saving is blocked to preserve an unreadable or changed preferences file."));
                            if ui.button(tr!("Preserve old file and reset all preferences")).help(ui, HelpControl::PreferenceRecover).clicked() { state.request(Job::Reset(model::Preferences::defaults(&state.home))); }
                        }
                        if let Some((preview, plan, paths)) = &state.preview {
                            if preview == &state.draft {
                                ui.heading({ let __omatainer_args = (&(preview.active),); crate::localization::format("Preview: {}", &[format!("{}", __omatainer_args.0)]) });
                                match plan {
                                    Ok(plan)=> {ui.label({ let __omatainer_args = (&(plan.device),&(plan.rate),&(plan.channels),&(plan.route()),); crate::localization::format("Saved output proposal: {} · {} Hz · {} channels · {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });if let Some(warning)=&plan.warning{ui.label(warning);}},
                                    Err(error)=> {ui.colored_label(ui.visuals().warn_fg_color,crate::localization::format("Audio unavailable: {error}", &[format!("{}", error)]));},
                                }
                                let midi = &preview.current().unwrap().midi_inputs;
                                ui.label(crate::localization::format("MIDI policy: {midi:?}", &[format!("{:?}", midi)]));
                                let routes=&preview.current().unwrap().midi_routing;
                                ui.label({ let __omatainer_args = (&(routes.enabled),&(routes.routes.len()),); crate::localization::format("Explicit track routing: {} · {} configured tracks", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) });
                                for route in &routes.routes {ui.label({ let __omatainer_args = (&(route.track+1),&(route.inputs),&(route.output),&(route.output_channel.map(|ch|ch+1)),&(route.thru),&(route.filter),); crate::localization::format("Track {}: inputs {:?} · output {:?} · channel {:?} · live thru {} · filters {:?}", &[format!("{}", __omatainer_args.0), format!("{:?}", __omatainer_args.1), format!("{:?}", __omatainer_args.2), format!("{:?}", __omatainer_args.3), format!("{}", __omatainer_args.4), format!("{:?}", __omatainer_args.5)]) });}
                                if let Some(status)=self.engine.midi.policy_status() {
                                    if status.pending() || status.error.is_some() {ui.colored_label(ui.visuals().warn_fg_color,tr!("MIDI discovery is pending or failed; availability cannot be fully verified yet."));}
                                    if let model::MidiInputs::Selected(names)=midi {
                                        let missing=names.iter().filter(|name|!status.available_inputs.contains(name)).cloned().collect::<Vec<_>>();
                                        if !missing.is_empty(){ui.colored_label(ui.visuals().warn_fg_color,{ let __omatainer_args = (&(missing.join(", ")),&(if status.available_truncated{" (preview is truncated)"}else{""}),); crate::localization::format("Not in current MIDI discovery preview: {}{}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) });}
                                    }
                                } else {ui.colored_label(ui.visuals().warn_fg_color,tr!("MIDI manager unavailable; this selection can only apply after restart."));}
                                ui.label(tr!("Permitted live MIDI connections remain connected; excluded sources are released by the MIDI worker after Apply."));
                                for path in paths { ui.label(path); }
                                let current = preview.current().unwrap();
                                ui.label({ let __omatainer_args = (&(current.appearance.scale*100.0),&(current.appearance.follow_theme),&(current.appearance.font_size),&(current.shortcuts_enabled),&(current.startup.scan_library),&(current.startup.show_help),&(current.startup.show_midi),); crate::localization::format("Appearance: scale {:.0}%, theme {}, font {:?}; shortcuts {}; startup scan {}, help {}, MIDI {}", &[format!("{:.0}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{:?}", __omatainer_args.2), format!("{}", __omatainer_args.3), format!("{}", __omatainer_args.4), format!("{}", __omatainer_args.5), format!("{}", __omatainer_args.6)]) });
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr!("Preview changes")).help(ui, HelpControl::PreferencePreview).clicked() { state.request(Job::Preview(state.draft.clone())); }
                        let ready = state.preview.as_ref().is_some_and(|(draft,plan,_)| draft==&state.draft && (plan.is_ok() || draft.current().map(|p| &p.audio)==state.applied.current().map(|p| &p.audio)));
                        if ui.add_enabled(ready && !state.blocked,egui::Button::new(tr!("Apply and save"))).help(ui, HelpControl::PreferenceApply).clicked() {
                            state.request(Job::Save {preferences:state.draft.clone(),revision:state.revision.clone()});
                        }
                        if ui.button(tr!("Cancel changes")).help(ui, HelpControl::PreferenceCancel).clicked() { discard = true; }
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
            state.shortcut_capture = None;
            state.open = false;
        }
    }
}
fn text(ui: &mut Ui, name: &str, value: &mut String, control: HelpControl) -> bool {
    let label = ui.label(crate::localization::text_dynamic(name));
    ui.text_edit_singleline(value)
        .labelled_by(label.id)
        .help(ui, control).changed()
}
fn multiline(ui: &mut Ui, name: &str, value: &mut String, control: HelpControl) {
    let label = ui.label(crate::localization::text_dynamic(name));
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
    let name = crate::localization::text_dynamic(name);
    let original = *value;
    let response = ui.add(egui::Slider::new(value, min..=max).text(name).custom_formatter(|v, _| crate::localization::number(v, 2)).custom_parser(crate::localization::parse_number));
    if let Some(alternate) =
        accessibility::numeric(ui, &response, name, original, min, max, step, unit)
    {
        *value = alternate;
    }
    help::annotate(ui, &response, control);
}
#[cfg(test)]
mod tests;
