//! Native keyboard commands for the existing workspace layout.
use super::*;
use crate::panel_workspace::{Book, Place};
use crate::property_fields as fields;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DRAFT: AtomicU64 = AtomicU64::new(1);

pub(super) struct Controls {
    pub open: bool,
    draft_instance: u64,
    panel: String,
    tab_with: String,
    loaded: Option<(String, String, String)>,
    size: [f32; 2],
    extent: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            open: false,
            draft_instance: 0,
            panel: "midi".into(),
            tab_with: String::new(),
            loaded: None,
            size: [640.0, 480.0],
            extent: 288.0,
        }
    }
}

impl Controls {
    /// Begin a fresh native size draft without reusing canceled text ownership.
    /// Args: none. Returns no value; document, workspace and panel identities stay unchanged.
    pub(super) fn begin(&mut self) {
        self.draft_instance = NEXT_DRAFT.fetch_add(1, Ordering::Relaxed);
        self.loaded = None;
        self.open = true;
    }
}

impl App {
    /// Open native layout commands using the existing task workspaces.
    /// Args: none. Returns no value; Classic enters the existing Decks task.
    pub(super) fn open_panel_keyboard(&mut self) {
        self.workspace.panel_keyboard.begin();
        if !self.studio_workspace_active() {
            self.settings
                .applied
                .profiles
                .get_mut(&self.settings.applied.active)
                .unwrap()
                .workspaces
                .studio
                .active = "Decks".into();
        }
    }

    /// Draw keyboard access to the existing panel placement and request APIs.
    /// Args: context, current preference book and known registry. Returns no value; no artwork or audio history is changed.
    pub(super) fn panel_keyboard_ui(
        &mut self,
        ctx: &egui::Context,
        book: &mut Book,
        registry: &[(&str, &str)],
    ) {
        if !self.workspace.panel_keyboard.open {
            return;
        }
        let mut state = std::mem::take(&mut self.workspace.panel_keyboard);
        let mut open = true;
        let mut close = false;
        let profile = self.settings.applied.active.clone();
        let task = book.active.clone();
        let identities: Vec<_> = registry.iter().map(|entry| entry.0).collect();
        let Some(layout) = book.tasks.get_mut(&task) else {
            self.workspace.panel_keyboard = state;
            return;
        };
        layout.normalize(&identities);
        if !identities.contains(&state.panel.as_str()) {
            state.panel = identities.first().copied().unwrap_or_default().into();
        }
        let enabled = !self.project.committing()
            && self.project.dialog_is_closed()
            && !self.settings.busy()
            && !self.settings.open;
        egui::Window::new("Panel layout").id(egui::Id::new("keyboard-panel-layout")).open(&mut open)
            .resizable(true).default_width(480.0).show(ctx, |ui| {
            keyboard::block_for_dialog(ctx);
            keyboard_authoring::form(ui,egui::Id::new(("panel-layout",&profile,&task,state.draft_instance)),|ui| {
            ui.label(format!("{task} workspace. The main surface stays visible. Layouts belong to Preferences."));
            ui.add_enabled_ui(enabled,|ui| {
                let chooser=egui::ComboBox::from_label("Panel to arrange")
                    .selected_text(registry.iter().find(|entry|entry.0==state.panel).map_or("Choose panel",|entry|entry.1))
                    .show_ui(ui,|ui| { for &(id,label) in registry {
                        ui.selectable_value(&mut state.panel,id.to_owned(),label);
                    }});
                accessibility::focus(ui,&chooser.response);
                let identity=(profile.clone(),task.clone(),state.panel.clone());
                let group=layout.groups.iter().find(|group|group.panels.contains(&state.panel)).cloned()
                    .or_else(||layout.hidden.get(&state.panel).cloned());
                if state.loaded.as_ref()!=Some(&identity) {
                    if let Some(group)=&group { state.size=[group.rect[2],group.rect[3]];state.extent=group.extent; }
                    state.loaded=Some(identity);
                }
                let visible=layout.groups.iter().any(|group|group.panels.contains(&state.panel));
                let floating=visible && group.as_ref().is_some_and(|group|group.place==Place::Floating);
                keyboard_authoring::target(ui, &state.panel, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if keyboard_authoring::action(ui,"Focus panel",visible).clicked() {
                            layout.request_focus(ctx,"omatainer-studio",&state.panel);
                        }
                        let visibility=keyboard_authoring::native(ui,keyboard_authoring::property(ui,"visibility"),|ui|
                            ui.button(if visible { "Hide panel" } else { "Restore panel" }));
                        accessibility::focus(ui,&visibility);
                        if visibility.clicked() {
                            if visible { layout.close(&state.panel); } else {
                                layout.restore(&state.panel); layout.normalize(&identities);
                                layout.request_focus(ctx,"omatainer-studio",&state.panel);
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (label,place) in [("Move panel left",Place::Left),("Move panel right",Place::Right),
                            ("Move panel top",Place::Top),("Move panel bottom",Place::Bottom),("Float panel",Place::Floating)] {
                            if keyboard_authoring::action(ui,label,true).clicked() {
                                layout.move_panel(&state.panel,place);layout.normalize(&identities);
                                layout.request_focus(ctx,"omatainer-studio",&state.panel);state.loaded=None;
                            }
                        }
                    });
                    let targets:Vec<_>=registry.iter().copied().filter(|entry|entry.0!=state.panel && layout.groups.iter().any(|group|group.panels.iter().any(|id|id==entry.0))).collect();
                    if !targets.iter().any(|entry|entry.0==state.tab_with) { state.tab_with=targets.first().map_or("",|entry|entry.0).into(); }
                    let target=egui::ComboBox::from_label("Group with panel")
                        .selected_text(targets.iter().find(|entry|entry.0==state.tab_with).map_or("Choose visible panel",|entry|entry.1))
                        .show_ui(ui,|ui| {for &(id,label) in &targets {ui.selectable_value(&mut state.tab_with,id.to_owned(),label);}});
                    accessibility::focus(ui,&target.response);
                    if keyboard_authoring::action(ui,"Join panel tabs",!state.tab_with.is_empty()).clicked() {
                        if layout.tab_with(&state.panel,&state.tab_with) {layout.normalize(&identities);layout.request_focus(ctx,"omatainer-studio",&state.panel);state.loaded=None;}
                    }
                    if floating {
                        let width=fields::number(ui,keyboard_authoring::property(ui,"width"),"Floating panel width (px)",&mut state.size[0],160.0..=4096.0," px",None,fields::FieldState::default());
                        let height=fields::number(ui,keyboard_authoring::property(ui,"height"),"Floating panel height (px)",&mut state.size[1],120.0..=4096.0," px",None,fields::FieldState::default());
                        if width.changed || height.changed { state.loaded=Some((profile.clone(),task.clone(),state.panel.clone())); }
                        let valid=fields::number_is_valid(&width) && fields::number_is_valid(&height)
                            && state.size.iter().all(|value|value.is_finite());
                        if keyboard_authoring::action(ui,"Apply floating panel size",valid).clicked() && valid {
                            layout.request_floating_size(ctx,"omatainer-studio",&state.panel,Vec2::new(state.size[0],state.size[1]));
                        }
                    } else {
                        let size=fields::number(ui,keyboard_authoring::property(ui,"dock-extent"),"Dock size (px)",&mut state.extent,80.0..=1200.0," px",None,fields::FieldState {locked:!visible,..Default::default()});
                        if keyboard_authoring::action(ui,"Apply dock size",visible && fields::number_is_valid(&size)).clicked() && fields::number_is_valid(&size) {
                            if let Some(group)=layout.groups.iter_mut().find(|group|group.panels.contains(&state.panel)) {group.extent=state.extent;}
                        }
                    }
                });
            });
            ui.label("Size changes use Apply. Cancel drops unapplied size text. Save workspace layouts in the Workspace menu to keep applied changes after reopening.");
            if keyboard_authoring::action(ui,"Cancel size draft",true).clicked() {close=true;}
            });
        });
        state.open = open && !close;
        self.workspace.panel_keyboard = state;
    }
}
