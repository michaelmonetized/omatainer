//! One shared renderer with named panel arrangements and native secondary windows.
use super::*;
use crate::panel_workspace::{Book, Place, Workspace};
use crate::preferences::workspaces::{Config, Entry, Layout, Panel};
use crate::property_fields as fields;
use std::collections::{BTreeMap, BTreeSet};

const STUDIO_PANELS: [(&str, &str); 7] = [
    ("library", "Library"),
    ("decks", "Decks"),
    ("session", "Session"),
    ("arrangement", "Arrangement"),
    ("midi", "MIDI"),
    ("mixer", "Mixer"),
    ("sampler", "Sampler"),
];

/// Build task layouts around an application-owned main surface.
/// Args: none. Returns six independent presets whose main surface cannot be hidden.
fn studio_presets() -> Vec<(&'static str, Workspace)> {
    [
        (
            "Library",
            vec![("decks", Place::Bottom), ("mixer", Place::Right)],
        ),
        (
            "Decks",
            vec![("library", Place::Bottom), ("sampler", Place::Right)],
        ),
        (
            "Session",
            vec![("library", Place::Left), ("mixer", Place::Right)],
        ),
        (
            "Arrangement",
            vec![("library", Place::Left), ("session", Place::Bottom)],
        ),
        (
            "MIDI",
            vec![("session", Place::Bottom), ("mixer", Place::Right)],
        ),
        (
            "Mixer",
            vec![("library", Place::Left), ("session", Place::Bottom)],
        ),
    ]
    .into_iter()
    .map(|(task, panels)| (task, Workspace::preset(&panels)))
    .collect()
}

/// Resolve the protected main surface for a task.
/// Args: saved task name. Returns its registered stable panel identity.
fn studio_center(task: &str) -> &'static str {
    match task {
        "Library" => "library",
        "Session" => "session",
        "Arrangement" => "arrangement",
        "MIDI" => "midi",
        "Mixer" => "mixer",
        _ => "decks",
    }
}

#[derive(Default)]
struct WindowInput {
    touch: touch::Input,
    keys: keyboard::ShortcutFocus,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) panel_keyboard: panel_keyboard::Controls,
    requested_focus: Option<String>,
    bound: Option<(String, Layout)>,
    closed: BTreeSet<Panel>,
    pub(super) sizes: BTreeMap<Panel, Vec2>,
    inputs: BTreeMap<Panel, WindowInput>,
    blocked: bool,
    studio_binding: Option<(String, Vec<(Vec<String>, usize, Place)>)>,
    pub(super) center: Option<Rect>,
}

/// Edit a named layout in the existing preference draft.
/// Takes UI, workspace data, name buffer and notice; returns no value and never saves implicitly.
pub(super) fn edit(
    ui: &mut Ui,
    config: &mut Config,
    name: &mut String,
    message: &mut String,
    sizes: &BTreeMap<Panel, Vec2>,
) {
    ui.heading(tr!("Workspaces"));
    ui.label(tr!("Select a workspace, configure its panels, then Preview and Apply to save and recall it. Cancel retains the applied layout."));
    ui.horizontal_wrapped(|ui| {
        for item in config.saved.keys() {
            ui.radio_value(&mut config.active, item.clone(), item);
        }
    });
    let label = ui.label(tr!("New workspace name"));
    ui.add(egui::TextEdit::singleline(name).id_salt("new-workspace-name"))
        .labelled_by(label.id)
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "New workspace name")
        });
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr!("Copy workspace")).clicked() {
            match config.copy(name) {
                Ok(()) => message.clear(),
                Err(error) => *message = error,
            }
        }
        if ui
            .add_enabled(
                config.saved.len() > 1,
                egui::Button::new(tr!("Delete workspace from draft")),
            )
            .clicked()
        {
            match config.remove_current() {
                Ok(()) => message.clear(),
                Err(error) => *message = error,
            }
        }
        if ui.button(tr!("Reset workspace layouts")).clicked() {
            *config = Config::default();
        }
    });
    let Some(layout) = config.saved.get_mut(&config.active) else {
        return;
    };
    if ui.button(tr!("Use current window sizes")).clicked() {
        for entry in &mut layout.panels {
            if let Some(size) = sizes.get(&entry.panel) {
                entry.window_size = [size.x.clamp(240.0, 4096.0), size.y.clamp(240.0, 4096.0)];
            }
        }
    }
    let mut movement = None;
    for (index, entry) in layout.panels.iter_mut().enumerate() {
        ui.push_id(entry.panel, |ui| {
            let panel = entry.panel.label();
            accessibility::group(ui, panel, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(
                        &mut entry.visible,
                        crate::localization::format("Show {0}", &[panel.into()]),
                    );
                    if ui
                        .add_enabled(
                            index > 0,
                            egui::Button::new(crate::localization::format(
                                "Move {0} up",
                                &[panel.into()],
                            )),
                        )
                        .clicked()
                    {
                        movement = Some((index, index - 1));
                    }
                    if ui
                        .add_enabled(
                            index < 3,
                            egui::Button::new(crate::localization::format(
                                "Move {0} down",
                                &[panel.into()],
                            )),
                        )
                        .clicked()
                    {
                        movement = Some((index, index + 1));
                    }
                    ui.checkbox(
                        &mut entry.detached,
                        crate::localization::format("Separate {0} window", &[panel.into()]),
                    );
                });
                let mut automatic = entry.height == 0.0;
                if ui
                    .checkbox(
                        &mut automatic,
                        crate::localization::format("Automatic {0} height", &[panel.into()]),
                    )
                    .changed()
                {
                    entry.height = if automatic { 0.0 } else { 320.0 };
                }
                if !automatic {
                    number(
                        ui,
                        &format!("{panel} panel height"),
                        &mut entry.height,
                        64.0,
                        2048.0,
                    );
                }
                if entry.detached {
                    number(
                        ui,
                        &format!("{panel} window width"),
                        &mut entry.window_size[0],
                        240.0,
                        4096.0,
                    );
                    number(
                        ui,
                        &format!("{panel} window height"),
                        &mut entry.window_size[1],
                        240.0,
                        4096.0,
                    );
                }
            });
        });
    }
    if let Some((from, to)) = movement {
        layout.panels.swap(from, to);
    }
    if let Err(error) = config.validate() {
        ui.colored_label(ui.visuals().warn_fg_color, error);
    }
    ui.label(tr!("Window positions follow your desktop. Saved window sizes are limited to the current monitor. Closing a panel window returns its controls to the main window for this session."));
}

/// Keep layout dimensions editable through keyboard and native accessibility values.
/// Takes UI, label, finite value and range; returns no value and updates only the draft.
fn number(ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32) {
    ui.horizontal(|ui| {
        let text = ui.label(label);
        let response = ui
            .add(egui::DragValue::new(value).range(min..=max).suffix(" pt"))
            .labelled_by(text.id);
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_label(label));
    });
}

impl App {
    /// Report whether this profile is showing a saved studio task.
    /// Args: none. Returns true for the task layouts and false for the preserved classic layout.
    pub(super) fn studio_workspace_active(&self) -> bool {
        !self.settings.profile().workspaces.studio.active.is_empty()
    }

    /// Select and save task layouts without changing musical or device settings.
    /// Args: toolbar UI. Returns nothing; disk writes use the existing conflict-checked worker.
    pub(super) fn workspace_task_controls(&mut self, ui: &mut Ui) {
        let active = self.settings.profile().workspaces.studio.active.clone();
        let title = if active.is_empty() {
            "Workspace: Classic".into()
        } else {
            format!("Workspace: {active}")
        };
        let mut select = None;
        let mut save = false;
        let editable = !self.settings.busy() && !self.settings.open && !self.project.committing();
        ui.add_enabled_ui(editable, |ui| {
            ui.menu_button(title, |ui| {
                if fields::tab(ui, "Classic", active.is_empty()).clicked() {
                    select = Some(String::new());
                    ui.close();
                }
                for (name, _) in studio_presets() {
                    if fields::tab(ui, name, active == name).clicked() {
                        select = Some(name.into());
                        ui.close();
                    }
                }
                ui.separator();
                save = fields::action(
                    ui,
                    "Save workspace layouts",
                    !self.settings.blocked && self.settings.worker.is_some(),
                )
                .clicked();
            });
        });
        if let Some(task) = select {
            let profile = self
                .settings
                .applied
                .profiles
                .get_mut(&self.settings.applied.active)
                .unwrap();
            if let Some((_, preset)) = studio_presets().into_iter().find(|(name, _)| *name == task)
            {
                profile
                    .workspaces
                    .studio
                    .tasks
                    .entry(task.clone())
                    .or_insert(preset);
            }
            profile.workspaces.studio.active = task;
            if let Some(draft) = self
                .settings
                .draft
                .profiles
                .get_mut(&self.settings.applied.active)
            {
                draft.workspaces.studio = profile.workspaces.studio.clone();
            }
        }
        if save {
            self.settings
                .request(crate::preferences::worker::Job::Save {
                    preferences: self.settings.applied.clone(),
                    revision: self.settings.revision.clone(),
                });
        }
    }

    /// Reveal a requested editor in the active task instead of opening a duplicate window.
    /// Args: registered panel ID. Returns nothing and leaves the task's main surface protected.
    pub(super) fn reveal_studio_panel(&mut self, panel: &str) {
        if !self.studio_workspace_active() {
            return;
        }
        let profile = self
            .settings
            .applied
            .profiles
            .get_mut(&self.settings.applied.active)
            .unwrap();
        let book = &mut profile.workspaces.studio;
        if studio_center(&book.active) == panel {
            return;
        }
        self.workspace.requested_focus = Some(panel.into());
        if let Some(layout) = book.tasks.get_mut(&book.active) {
            layout.restore(panel);
            if !layout
                .groups
                .iter()
                .any(|group| group.panels.iter().any(|id| id == panel))
            {
                layout.move_panel(panel, Place::Right);
            }
            for group in &mut layout.groups {
                if let Some(index) = group.panels.iter().position(|id| id == panel) {
                    group.active = index;
                }
            }
            layout.focus = Some(panel.into());
        }
    }

    /// Retire GUI holds before a task or a panel tab moves.
    /// Args: native context and current book. Returns nothing and preserves controller-held ownership.
    fn bind_studio_workspace(&mut self, ctx: &egui::Context, book: &Book) {
        let placements = book
            .tasks
            .get(&book.active)
            .map(|layout| {
                layout
                    .groups
                    .iter()
                    .map(|group| (group.panels.clone(), group.active, group.place))
                    .collect()
            })
            .unwrap_or_default();
        let binding = (book.active.clone(), placements);
        if self.workspace.studio_binding.as_ref() != Some(&binding) {
            self.release_workspace_panel(Panel::Decks);
            self.release_workspace_panel(Panel::Sampler);
            self.release_workspace_midi();
            self.touch_input.retire();
            self.command_palette.open = false;
            accessibility::cancel_editor(ctx);
            self.workspace.inputs.clear();
            self.workspace.studio_binding = Some(binding);
        }
    }

    /// Render real app panels around a protected task surface.
    /// Args: native context. Returns nothing; every panel keeps the same App and command target.
    fn studio_workspace_ui(&mut self, ctx: &egui::Context) {
        let mut book = self.settings.profile().workspaces.studio.clone();
        let presets = studio_presets();
        let center = studio_center(&book.active);
        let registry: Vec<_> = STUDIO_PANELS
            .iter()
            .copied()
            .filter(|(id, _)| *id != center)
            .collect();
        if !presets.iter().any(|(name, _)| *name == book.active) {
            book.active = "Decks".into();
        }
        if !book.tasks.contains_key(&book.active) {
            if let Some((_, preset)) = presets.iter().find(|(name, _)| *name == book.active) {
                book.tasks.insert(book.active.clone(), preset.clone());
            }
        }
        let identities: Vec<_> = registry.iter().map(|(id, _)| *id).collect();
        book.tasks
            .get_mut(&book.active)
            .unwrap()
            .normalize(&identities);
        if let Some(panel) = self.workspace.requested_focus.take() {
            book.tasks.get_mut(&book.active).unwrap().request_focus(
                ctx,
                "omatainer-studio",
                &panel,
            );
        }
        self.bind_studio_workspace(ctx, &book);
        let theme = self.theme.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme.bg).inner_margin(6.0))
            .show(ctx, |ui| {
                self.workspace_status(ui);
                let blocked = self.project.committing()
                    || !self.project.dialog_is_closed()
                    || self.settings.busy()
                    || self.settings.open;
                ui.add_enabled_ui(!blocked, |ui| {
                    let active = book.active.clone();
                    let layout = book.tasks.get_mut(&active).unwrap();
                    ui.horizontal_wrapped(|ui| {
                        ui.menu_button("Panels", |ui| {
                            for &(id, label) in &registry {
                                let hidden = layout.hidden.contains_key(id);
                                if fields::action(
                                    ui,
                                    &format!(
                                        "{} {label}",
                                        if hidden { "Restore" } else { "Focus" }
                                    ),
                                    true,
                                )
                                .clicked()
                                {
                                    layout.restore(id);
                                    for group in &mut layout.groups {
                                        if let Some(index) =
                                            group.panels.iter().position(|panel| panel == id)
                                        {
                                            group.active = index;
                                        }
                                    }
                                    layout.request_focus(ctx, "omatainer-studio", id);
                                    ui.close();
                                }
                            }
                            if fields::action(ui, "Restore all panels", true).clicked() {
                                for &(id, _) in &registry {
                                    layout.restore(id);
                                }
                            }
                            if fields::action(ui, "Reset this task", true).clicked() {
                                if let Some((_, preset)) =
                                    presets.iter().find(|(name, _)| *name == active)
                                {
                                    *layout = preset.clone();
                                    layout.normalize(&identities);
                                }
                            }
                            if fields::action(ui, "Arrange panels…", true).clicked() {
                                self.workspace.panel_keyboard.begin();
                                ui.close();
                            }
                        });
                        ui.weak("Right-click a panel tab to move, group, float or close it.");
                    });
                    let floating: BTreeSet<_> = layout
                        .groups
                        .iter()
                        .filter(|group| group.place == Place::Floating)
                        .flat_map(|group| group.panels.iter().cloned())
                        .collect();
                    let remaining = layout.show(ui, "omatainer-studio", &registry, |ui, panel| {
                        if floating.contains(panel) {
                            self.workspace_status(ui);
                        }
                        self.studio_panel(ui, panel)
                    });
                    self.workspace.center = Some(remaining);
                    ui.allocate_ui_at_rect(remaining.shrink(4.0), |ui| {
                        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                        self.studio_panel(ui, center);
                    });
                });
            });
        self.panel_keyboard_ui(ctx, &mut book, &registry);
        self.bind_studio_workspace(ctx, &book);
        let profile = self
            .settings
            .applied
            .profiles
            .get_mut(&self.settings.applied.active)
            .unwrap();
        profile.workspaces.studio = book.clone();
        if !self.settings.open && !self.settings.busy() {
            if let Some(draft) = self
                .settings
                .draft
                .profiles
                .get_mut(&self.settings.applied.active)
            {
                draft.workspaces.studio = book;
            }
        }
    }

    /// Draw one registered panel with bounded scrolling.
    /// Args: available UI and stable panel ID. Returns nothing and never creates another document owner.
    fn studio_panel(&mut self, ui: &mut Ui, panel: &str) {
        let theme = self.theme.clone();
        let height = ui.available_height().max(80.0);
        let width = ui.available_width();
        let output = egui::ScrollArea::both()
            .id_salt(("studio-panel", panel))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(width);
                if panel != "library" && self.engine.safe_mode() {
                    ui.disable();
                }
                match panel {
                    "library" => self.crate_row(ui, &theme),
                    "decks" => self.workspace_panel(ui, &theme, Panel::Decks, height, false),
                    "sampler" => self.workspace_panel(ui, &theme, Panel::Sampler, height, false),
                    "session" => self.workspace_panel(ui, &theme, Panel::Session, height, false),
                    "arrangement" => {
                        if !self.arrangement.open {
                            self.open_arrangement();
                        }
                        self.arrangement_panel(ui);
                    }
                    "midi" => self.piano_roll_panel(ui),
                    "mixer" => self.mixer_panel(ui),
                    _ => {}
                }
            });
        accessibility::scrollbars(ui, &format!("{panel} workspace panel"), &output);
    }

    /// Recall the applied layout and retire local holds before moving their controls.
    /// Takes the root context; returns a current layout without changing MIDI or project data.
    fn workspace_layout(&mut self, ctx: &egui::Context) -> Layout {
        let config = &self.settings.profile().workspaces;
        let layout = config
            .current()
            .cloned()
            .unwrap_or_else(|| Config::default().current().unwrap().clone());
        let binding = (config.active.clone(), layout.clone());
        if self.workspace.bound.as_ref() != Some(&binding) {
            if self.workspace.bound.is_some() {
                self.release_workspace_panel(Panel::Sampler);
                self.release_workspace_panel(Panel::Decks);
                self.touch_input.retire();
                self.command_palette.open = false;
                accessibility::cancel_editor(ctx);
                self.workspace.inputs.clear();
            }
            self.workspace.bound = Some(binding);
            self.workspace.closed.clear();
            self.workspace.sizes.clear();
        }
        layout
    }

    /// Release GUI-owned inputs for a panel before it closes or moves.
    /// Takes its persistent panel identity; returns no value and preserves independent MIDI owners.
    pub(super) fn release_workspace_panel(&mut self, panel: Panel) {
        match panel {
            Panel::Sampler => {
                for pad in 0..16 {
                    self.set_pad_input(pad, u8::MAX, false);
                }
            }
            Panel::Decks => {
                self.release_cue_inputs();
                self.release_censor_inputs();
                self.release_pitch_inputs();
                for deck in 0..DECKS {
                    self.send(Command::DeckTouch {
                        deck: deck as u8,
                        on: false,
                    });
                }
            }
            _ => {}
        }
    }

    /// Draw the named layout while retaining the app's global safety and transport bars.
    /// Takes the root native context; returns no value and routes every panel into its chosen window.
    pub(super) fn workspace_ui(&mut self, ctx: &egui::Context) {
        if self.studio_workspace_active() {
            self.studio_workspace_ui(ctx);
            return;
        }
        if self.workspace.studio_binding.take().is_some() {
            self.release_workspace_panel(Panel::Decks);
            self.release_workspace_panel(Panel::Sampler);
            self.release_workspace_midi();
            self.touch_input.retire();
            self.command_palette.open = false;
            accessibility::cancel_editor(ctx);
            self.workspace.inputs.clear();
            self.workspace.center = None;
        }
        let layout = self.workspace_layout(ctx);
        self.workspace.blocked = keyboard::dialogs_block_input(ctx)
            || self.library_layout.open
            || self.library_backup.open
            || self.settings.open
            || self.audio_settings.open
            || self.command_palette.open;
        let t = self.theme.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg).inner_margin(6.0))
            .show(ctx, |ui| {
                if self.project.committing() || !self.project.dialog_is_closed() {
                    ui.disable();
                }
                let h = ui.available_height();
                let w = ui.available_width();
                let surface = egui::ScrollArea::both()
                    .id_salt("performance-surface")
                    .animated(!t.reduced_motion)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(w);
                        let occupied: f32 = layout
                            .panels
                            .iter()
                            .filter(|entry| {
                                entry.visible
                                    && entry.panel != Panel::Decks
                                    && (!entry.detached
                                        || self.workspace.closed.contains(&entry.panel))
                            })
                            .map(|entry| self.workspace_height(entry, &t, 320.0) + 4.0)
                            .sum();
                        for entry in &layout.panels {
                            if !entry.visible
                                || entry.detached && !self.workspace.closed.contains(&entry.panel)
                            {
                                continue;
                            }
                            let height =
                                self.workspace_height(entry, &t, (h - occupied).max(200.0));
                            self.workspace_panel(ui, &t, entry.panel, height, entry.height != 0.0);
                            ui.add_space(4.0);
                        }
                    });
                accessibility::scrollbars(ui, "Performance surface", &surface);
            });
        for entry in &layout.panels {
            if entry.visible && entry.detached && !self.workspace.closed.contains(&entry.panel) {
                self.workspace_window(ctx, entry);
            }
        }
    }

    /// Derive an automatic or saved height using the current scalable controls.
    /// Takes panel entry, theme and remaining deck height; returns a bounded content height in points.
    fn workspace_height(&self, entry: &Entry, t: &Theme, decks: f32) -> f32 {
        if entry.height != 0.0 {
            return entry.height;
        }
        match entry.panel {
            Panel::Decks => decks,
            Panel::Sampler => 118.0_f32.max(t.target_size(32.0) * 5.0 + 20.0),
            Panel::Library => 108.0_f32.max(t.target_size(26.0) * 4.0 + 24.0),
            Panel::Session => {
                70.0 + 26.0
                    + 48.0
                    + t.target_size(26.0) * SCENES as f32
                    + 4.0 * (SCENES as f32 + 2.0)
            }
        }
    }

    /// Render the existing panel controls through the same shared App and command port.
    /// Takes UI, theme, panel and height; returns no value and preserves controller targeting.
    fn workspace_panel(&mut self, ui: &mut Ui, t: &Theme, panel: Panel, height: f32, scroll: bool) {
        ui.push_id(("workspace-panel", panel), |ui| {
            let mut draw = |ui: &mut Ui| {
                ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
                    if panel != Panel::Library && self.engine.safe_mode() {
                        ui.disable();
                    }
                    match panel {
                        Panel::Decks => self.scratch_row(ui, t),
                        Panel::Sampler => {
                            accessibility::scope(ui, "Sampler", |ui| self.sampler_row(ui, t));
                        }
                        Panel::Library => self.crate_row(ui, t),
                        Panel::Session => {
                            if self.snap.fx_view >= 0 {
                                self.fx_row(ui, t);
                            } else {
                                self.sequencer_row(ui, t);
                            }
                        }
                    }
                });
            };
            if scroll {
                let output = egui::ScrollArea::both()
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, draw);
                accessibility::scrollbars(ui, &format!("{} panel", panel.label()), &output);
            } else {
                draw(ui);
            }
        });
    }

    /// Keep transport, safety state and output warnings visible above a detached panel.
    /// Takes its window UI; returns no value and submits ordinary shared commands.
    fn workspace_status(&mut self, ui: &mut Ui) {
        scale_status::show(ui, &self.snap);
        let status = self.engine.cmd.performance().status();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.engine.safe_mode() && !status.recovery,
                    egui::Button::new(tr!("Play / stop session")),
                )
                .clicked()
            {
                self.send(Command::TogglePlay);
            }
            if ui.button(tr!("Stop session")).clicked() {
                self.send(Command::Stop);
            }
            if ui.button(tr!("Mute output…")).clicked() {
                self.performance_panel.silence();
            }
        });
        ui.strong(if status.output_muted {
            tr!("Output muted")
        } else if status.recovery {
            tr!("Controls paused")
        } else if status.protected {
            tr!("PERFORMANCE · protected")
        } else {
            tr!("Performance mode off")
        });
        if self.engine.safe_mode() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                tr!("Safe mode: performance controls are disabled"),
            );
        }
        if self.engine.output_info().is_none() {
            ui.colored_label(ui.visuals().warn_fg_color, tr!("Audio output unavailable"));
        }
        if self.snap.audio.device_lost > 0 || self.snap.audio.backend_errors > 0 {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                tr!("Audio device error; inspect Setup in the main window"),
            );
        }
        if let Some(error) = self.submission_error.get() {
            ui.colored_label(ui.visuals().warn_fg_color, error.to_string());
        }
        ui.label(crate::localization::format(
            "Controller target: deck {0}, track {1}",
            &[
                format!("{}", (b'A' + self.snap.selected_deck as u8) as char),
                (self.snap.selected_track + 1).to_string(),
            ],
        ));
        ui.separator();
    }

    /// Show a native child viewport or the backend's embedded-window fallback.
    /// Takes the root context and saved panel entry; returns no value and returns closed controls to the root.
    fn workspace_window(&mut self, root: &egui::Context, entry: &Entry) {
        let panel = entry.panel;
        let title = crate::localization::format("Omatainer · {0}", &[panel.label().into()]);
        let root_monitor = root
            .input(|input| input.viewport().monitor_size)
            .filter(|size| size.is_finite() && size.x > 0.0 && size.y > 0.0)
            .unwrap_or_else(|| root.screen_rect().size());
        let size = self
            .workspace
            .sizes
            .get(&panel)
            .copied()
            .unwrap_or_else(|| {
                Vec2::new(entry.window_size[0], entry.window_size[1]).min(root_monitor)
            });
        let builder = egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id(crate::APPLICATION_ID)
            .with_inner_size(size)
            .with_min_inner_size([240.0, 240.0])
            .with_clamp_size_to_monitor_size(true)
            .with_drag_and_drop(false);
        let id = egui::ViewportId::from_hash_of(("workspace-window", panel));
        let mut close = false;
        root.show_viewport_immediate(id, builder, |ctx, class| {
            let t = self.theme.clone();
            if class == egui::ViewportClass::Embedded {
                let mut open = true;
                let output = egui::Window::new(&title)
                    .id(egui::Id::new(("workspace-window", panel)))
                    .open(&mut open)
                    .default_size(size)
                    .resizable(true)
                    .show(ctx, |ui| {
                        self.workspace_status(ui);
                        if ui.button(tr!("Return panel to main window")).clicked() {
                            close = true;
                        }
                        let h = self.workspace_height(entry, &t, ui.available_height().max(200.0));
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if self.workspace.blocked
                                    || self.project.committing()
                                    || !self.project.dialog_is_closed()
                                {
                                    ui.disable();
                                }
                                self.workspace_panel(ui, &t, panel, h, entry.height != 0.0);
                            });
                    });
                if let Some(output) = output {
                    self.workspace
                        .sizes
                        .insert(panel, output.response.rect.size());
                }
                close |= !open;
            } else {
                {
                    let input = self.workspace.inputs.entry(panel).or_default();
                    std::mem::swap(&mut self.touch_input, &mut input.touch);
                    std::mem::swap(&mut self.shortcut_focus, &mut input.keys);
                }
                self.touch_input.begin(ctx);
                self.shortcut_focus.begin_frame(ctx);
                accessibility::begin_frame(ctx);
                let requested = ctx.input(|input| input.viewport().close_requested());
                egui::TopBottomPanel::top("workspace-status").show(ctx, |ui| {
                    self.workspace_status(ui);
                    if ui.button(tr!("Return panel to main window")).clicked() {
                        close = true;
                    }
                });
                close |= requested;
                egui::CentralPanel::default().show(ctx, |ui| {
                    if self.workspace.blocked
                        || self.project.committing()
                        || !self.project.dialog_is_closed()
                    {
                        ui.disable();
                    }
                    let h = self.workspace_height(entry, &t, ui.available_height().max(200.0));
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.workspace_panel(ui, &t, panel, h, entry.height != 0.0)
                        });
                });
                self.performance_decision(ctx);
                accessibility::numeric_editor(ctx);
                self.command_palette_ui(ctx);
                self.touch_input_ui(ctx);
                if self.workspace.blocked || close {
                    keyboard::block_for_dialog(ctx);
                }
                self.handle_keys(ctx);
                self.finish_touch_input(ctx);
                accessibility::finish_frame(ctx);
                let actual = ctx.screen_rect().size();
                let monitor = ctx
                    .input(|input| input.viewport().monitor_size)
                    .or(Some(root_monitor));
                if actual.is_finite() && actual.x > 0.0 && actual.y > 0.0 {
                    let bounded = monitor
                        .filter(|size| size.is_finite() && size.x > 0.0 && size.y > 0.0)
                        .map_or(actual, |size| actual.min(size));
                    if bounded != actual {
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(bounded));
                    }
                    self.workspace.sizes.insert(panel, bounded);
                }
                {
                    let input = self.workspace.inputs.get_mut(&panel).unwrap();
                    std::mem::swap(&mut self.touch_input, &mut input.touch);
                    std::mem::swap(&mut self.shortcut_focus, &mut input.keys);
                }
            }
        });
        if close {
            self.command_palette.close_for_viewport(id);
            self.release_workspace_panel(panel);
            self.workspace.closed.insert(panel);
            self.workspace.inputs.remove(&panel);
            root.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests;
