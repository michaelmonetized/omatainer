//! Layout edits preview locally and save through the existing preference owner.
use super::*;
use crate::preferences::library_layout::{Column, Config, Density, Sort};
pub(super) mod sort;
#[cfg(test)]
mod tests;

pub(super) struct Layouts {
    pub open: bool,
    pub live: Config,
    draft: Config,
    name: String,
    message: String,
    saving: bool,
}
impl Default for Layouts {
    fn default() -> Self {
        let live = Config::default();
        Self {
            open: false,
            draft: live.clone(),
            live,
            name: String::new(),
            message: String::new(),
            saving: false,
        }
    }
}
impl Layouts {
    /// Adopt a successfully applied profile's library layouts.
    /// Takes validated saved configuration; replaces the browser and editor presentation.
    pub fn apply(&mut self, config: Config) {
        self.live = config.clone();
        self.draft = config;
    }
}
/// Edit one distinct primary or secondary ordering column.
/// Takes a slot, selected sort and excluded column; updates only a valid column and direction.
fn sort_editor(ui: &mut Ui, label: &str, selected: &mut Option<Sort>, excluded: Option<Column>) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = egui::ComboBox::from_id_salt(label)
            .selected_text(selected.map_or("Manual order / none", |sort| sort.column.label()))
            .show_ui(ui, |ui| {
                ui.selectable_value(selected, None, "Manual order / none");
                for column in Column::ALL {
                    if Some(column) != excluded {
                        let value = Some(Sort {
                            column,
                            descending: selected
                                .is_some_and(|sort| sort.column == column && sort.descending),
                        });
                        ui.selectable_value(selected, value, column.label());
                    }
                }
            })
            .response;
        accessibility::button(ui, &response, label, None);
        help::annotate(ui, &response, HelpControl::LibrarySort);
        if let Some(sort) = selected {
            let response = ui.checkbox(&mut sort.descending, "Descending");
            accessibility::button(
                ui,
                &response,
                &format!("{label} descending"),
                Some(sort.descending),
            );
            help::annotate(ui, &response, HelpControl::LibrarySort);
        }
    });
}
impl App {
    /// Toggle a header sort without changing source selection.
    /// Takes its column and secondary modifier; updates the live layout and a clean matching editor draft.
    pub(super) fn sort_library_column(&mut self, column: Column, secondary: bool) {
        let state = &mut self.library_layout;
        let before = state.live.clone();
        let layout = state.live.current_mut();
        if secondary && layout.primary.is_some_and(|sort| sort.column != column) {
            layout.secondary = Some(Sort {
                column,
                descending: layout
                    .secondary
                    .is_some_and(|sort| sort.column == column && !sort.descending),
            });
        } else {
            layout.primary = Some(Sort {
                column,
                descending: layout
                    .primary
                    .is_some_and(|sort| sort.column == column && !sort.descending),
            });
            if layout.secondary.is_some_and(|sort| sort.column == column) {
                layout.secondary = None;
            }
        }
        if state.draft == before {
            state.draft = state.live.clone();
        }
        state.message = "Sort preview changed. Save layouts to keep it in this profile.".into();
    }
    /// Save complete library presentation without overwriting another preference draft.
    /// Takes the validated layout editor; queues an exact-revision preference write and applies only its saved result.
    fn save_library_layout(&mut self) {
        let result = (|| {
            self.library_layout.draft.validate()?;
            if self.engine.safe_mode() || self.engine.cmd.performance().protected() {
                return Err("Leave protection deliberately before saving layouts".into());
            }
            if self.settings.busy()
                || self.settings.blocked
                || self.settings.draft != self.settings.applied
            {
                return Err("Finish or discard the Preferences draft before saving layouts".into());
            }
            if self.settings.worker.is_none() {
                return Err(
                    "The preference owner is unavailable; layout preview remains local".into(),
                );
            }
            let mut next = self.settings.applied.clone();
            next.profiles.get_mut(&next.active).unwrap().library_layout =
                self.library_layout.draft.clone();
            next.validate()?;
            self.settings
                .request(crate::preferences::worker::Job::Save {
                    preferences: next,
                    revision: self.settings.revision.clone(),
                });
            self.library_layout.saving = self.settings.busy();
            self.library_layout.message = self.settings.message.clone();
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.library_layout.message = error;
        }
    }
    /// Finish a layout save with the preference owner's actual outcome.
    /// Takes current worker state; retains local previews on failure and exposes durability warnings unchanged.
    pub(super) fn poll_library_layout_save(&mut self) {
        if self.library_layout.saving && !self.settings.busy() {
            self.library_layout.saving = false;
            self.library_layout.message = self.settings.message.clone();
        }
    }
    /// Draw saved library layout, column and ordering controls.
    /// Takes the GUI context; edits a bounded draft with explicit preview, save and discard actions.
    pub(super) fn library_layout_ui(&mut self, ctx: &egui::Context) {
        if !self.library_layout.open {
            return;
        }
        let can_save = !self.settings.busy()
            && !self.settings.blocked
            && self.settings.draft == self.settings.applied
            && self.settings.worker.is_some()
            && !self.engine.safe_mode()
            && !self.engine.cmd.performance().protected();
        let saved = self.settings.profile().library_layout.clone();
        let state = &mut self.library_layout;
        let mut open = state.open;
        let mut save = false;
        let mut retry_artwork = false;
        egui::Window::new("Library layouts").id(egui::Id::new("library-layout-window")).open(&mut open).default_width(640.0).show(ctx, |ui| {
            ui.label("Columns and ordering belong to the active profile. Preview keeps the current track selected. Save layouts writes the complete layout list.");
            if state.live != saved { ui.label("Unsaved library layout preview"); }
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("saved-library-layout").selected_text(&state.draft.active).show_ui(ui, |ui| {
                    for layout in &state.draft.layouts { ui.selectable_value(&mut state.draft.active, layout.name.clone(), &layout.name); }
                }).response.help(ui, HelpControl::LibraryLayouts);
                let name = ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("New layout name").char_limit(80).desired_width(180.0));
                name.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "New library layout name"));
                if ui.button("Copy layout").help(ui, HelpControl::LibraryLayouts).clicked() {
                    let mut next=state.draft.clone(); let mut copy=next.current().clone(); copy.name=state.name.trim().into(); next.active=copy.name.clone(); next.layouts.push(copy);
                    match next.validate() { Ok(()) => { state.draft=next; state.name.clear(); state.message.clear(); }, Err(error) => state.message=error }
                }
                if ui.add_enabled(state.draft.layouts.len()>1, egui::Button::new("Delete layout")).help(ui, HelpControl::LibraryLayouts).clicked() {
                    state.draft.layouts.retain(|layout|layout.name!=state.draft.active); state.draft.active=state.draft.layouts[0].name.clone();
                }
            });
            let layout=state.draft.current_mut();
            ui.horizontal(|ui| {
                ui.label("Row density");
                for density in [Density::Compact, Density::Comfortable, Density::Artwork] {
                    ui.selectable_value(&mut layout.density, density, density.label()).help(ui, HelpControl::LibraryDensity);
                }
            });
            sort_editor(ui, "Primary sort", &mut layout.primary, None);
            if layout.primary.is_none() { layout.secondary=None; }
            ui.add_enabled_ui(layout.primary.is_some(), |ui|sort_editor(ui, "Secondary sort", &mut layout.secondary, layout.primary.map(|sort|sort.column)));
            ui.label("Manual order uses saved direct crate membership; All tracks uses its published catalog order. Equal sort values keep that order. Missing metadata stays last in either direction.");
            egui::ScrollArea::vertical().id_salt("library-layout-columns").max_height(330.0).show(ui, |ui| {
                let mut move_column = None;
                let total=layout.columns.len();
                let visible=layout.columns.iter().filter(|spec|spec.visible).count();
                for (index,spec) in layout.columns.iter_mut().enumerate() {
                    ui.push_id(spec.column, |ui| ui.horizontal(|ui| {
                        let response=ui.add_enabled(!spec.visible || visible>1, egui::Checkbox::new(&mut spec.visible, spec.column.label()));
                        accessibility::button(ui, &response, &format!("Show {} column", spec.column.label()), Some(spec.visible));
                        help::annotate(ui, &response, HelpControl::LibraryColumns);
                        let earlier=ui.add_enabled(index>0, egui::Button::new("↑"));
                        accessibility::button(ui,&earlier,&format!("Move {} column earlier",spec.column.label()),None);
                        help::annotate(ui,&earlier,HelpControl::LibraryColumns);
                        if earlier.clicked() { move_column=Some((index,index-1)); }
                        let later=ui.add_enabled(index+1<total, egui::Button::new("↓"));
                        accessibility::button(ui,&later,&format!("Move {} column later",spec.column.label()),None);
                        help::annotate(ui,&later,HelpControl::LibraryColumns);
                        if later.clicked() { move_column=Some((index,index+1)); }
                        let response=ui.add(egui::DragValue::new(&mut spec.width).range(36.0..=1024.0).speed(2.0).suffix(" pt"));
                        if let Some(value)=accessibility::numeric(ui, &response, &format!("{} column width", spec.column.label()), spec.width,36.0,1024.0,1.0," pt") { spec.width=value; }
                        help::annotate(ui, &response, HelpControl::LibraryColumns);
                    }));
                }
                if let Some((from,to))=move_column { layout.columns.swap(from,to); }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Preview layout").help(ui, HelpControl::LibraryLayouts).clicked() {
                    match state.draft.validate() { Ok(()) => { state.live=state.draft.clone(); state.message="Layout preview applied. Save layouts keeps it after restart.".into(); }, Err(error)=>state.message=error }
                }
                if ui.add_enabled(can_save, egui::Button::new("Save layouts")).help(ui, HelpControl::LibraryLayouts).clicked() { save=true; }
                if ui.button("Discard layout preview").help(ui, HelpControl::LibraryLayouts).clicked() { state.apply(saved.clone()); state.message="Saved library layouts restored.".into(); }
            });
            if ui.button("Retry artwork").help(ui,HelpControl::LibraryDensity).clicked() { retry_artwork=true; }
            if !can_save { ui.label("Saving waits for the preference owner, a clean Preferences draft and deliberate exit from protection."); }
            if !state.message.is_empty() { ui.label(&state.message); }
        });
        state.open = open;
        if retry_artwork {
            self.library_artwork.retry();
        }
        if save {
            self.save_library_layout();
        }
    }
}
