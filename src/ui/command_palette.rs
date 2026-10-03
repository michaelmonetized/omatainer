//! The binding registry also supplies explicit, searchable native commands.
use super::*;

#[derive(Default)]
pub(super) struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    focus: bool,
    composing: bool,
}

impl Palette {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
        self.composing = false;
    }
}

/// Choose a free palette chord without taking over a saved user binding.
/// Takes the active profile; returns the first available Ctrl+Shift chord, or menu-only access.
pub(super) fn chord(profile: &crate::preferences::Profile) -> Option<Key> {
    [Key::P, Key::K, Key::F3].into_iter().find(|key| !shortcuts::BINDINGS.iter().any(|binding| {
        binding.effective(profile).is_some_and(|value| value.ctrl && value.shift && !value.alt && Key::from_name(&value.key) == Some(*key))
    }))
}

impl App {
    pub(super) fn command_palette_ui(&mut self, ctx: &egui::Context) {
        if !self.command_palette.open { return; }
        keyboard::block_for_dialog(ctx);
        let profile = self.settings.profile().clone();
        let project_ready = !self.project.committing() && self.project.dialog_is_closed();
        let palette = &mut self.command_palette;
        let was_composing = palette.composing;
        let ime_event = ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Ime(_))));
        ctx.input(|input| for event in &input.events {
            if let egui::Event::Ime(event) = event {
                match event {
                    egui::ImeEvent::Preedit(text) => palette.composing = !text.is_empty(),
                    egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => palette.composing = false,
                    _ => {},
                }
            }
        });
        let editing_ime = was_composing || palette.composing || ime_event;
        let keyboard_focused = ctx.input(|input| input.focused);
        let (up, down, enter, escape) = ctx.input_mut(|input| (
            keyboard_focused && !editing_ime && input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            keyboard_focused && !editing_ime && input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            keyboard_focused && !editing_ime && input.consume_key(egui::Modifiers::NONE, Key::Enter),
            input.consume_key(egui::Modifiers::NONE, Key::Escape),
        ));
        let mut selected = None;
        let mut preferences = false;
        egui::Modal::new(egui::Id::new("command-palette")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width()-64.0).clamp(180.0, 560.0));
            ui.heading(tr!("Commands"));
            ui.label(tr!("Search, use Up/Down, then Enter to run the selected command. Escape closes. Typing never plays music."));
            let search = ui.add(egui::TextEdit::singleline(&mut palette.query).id_salt("command-search").char_limit(256).hint_text(tr!("Search commands")));
            search.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search commands"));
            help::annotate(ui, &search, HelpControl::PreferenceShortcut);
            if palette.focus { search.request_focus(); palette.focus = false; }
            if search.changed() { palette.selected = 0; }
            let query = crate::localization::search_key(palette.query.trim());
            let matches: Vec<_> = shortcuts::BINDINGS.iter().enumerate().filter_map(|(index, binding)| {
                if shortcuts::BINDINGS[..index].iter().any(|other| other.action == binding.action) { return None; }
                let labels = shortcuts::BINDINGS.iter().filter(|other| other.action == binding.action)
                    .filter_map(|other| other.effective(&profile).map(|value| value.label())).collect::<Vec<_>>().join(" / ");
                let description = crate::localization::text_dynamic(binding.description);
                let context = crate::localization::text_dynamic(binding.action.context());
                let searchable = format!("{description} {context} {labels}");
                crate::localization::search_key(&searchable).contains(&query).then_some((binding, labels))
            }).collect();
            palette.selected = palette.selected.min(matches.len().saturating_sub(1));
            if up { palette.selected = palette.selected.saturating_sub(1); }
            if down { palette.selected = (palette.selected+1).min(matches.len().saturating_sub(1)); }
            if matches.is_empty() { ui.label(tr!("No matching commands")); }
            egui::ScrollArea::vertical().id_salt("command-results").max_height((ctx.screen_rect().height()*0.5).max(100.0)).show(ui, |ui| {
                for (index, (binding, labels)) in matches.iter().enumerate() {
                    ui.push_id(binding.id(), |ui| {
                        let enabled = project_ready && (!self.engine.safe_mode() || binding.action.context() == "Navigation");
                        let response = ui.add_enabled(enabled, egui::Button::selectable(index == palette.selected, crate::localization::text_dynamic(binding.description)))
                            .help(ui, HelpControl::PreferenceShortcut);
                        if response.clicked() { selected = Some(binding.action); }
                        if index == palette.selected && (up || down) { response.scroll_to_me(Some(Align::Center)); }
                        ui.label(format!("{} · {}", crate::localization::text_dynamic(binding.action.context()),
                            if !profile.shortcuts_enabled { tr!("Shortcuts disabled") } else if labels.is_empty() { tr!("Unbound") } else { labels }));
                    });
                }
            });
            if enter && !search.changed() { selected = matches.get(palette.selected).map(|(binding,_)| binding.action); }
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr!("Edit shortcuts in Preferences")).help(ui, HelpControl::PreferenceShortcut).clicked() { preferences = true; }
                if ui.button(tr!("Close commands")).help(ui, HelpControl::PreferenceShortcut).clicked() { palette.open = false; }
            });
        });
        if escape { selected = None; preferences = false; palette.open = false; }
        if self.project.committing() || !self.project.dialog_is_closed() { selected = None; preferences = false; }
        if let Some(action) = selected { palette.open = false; self.dispatch_shortcut(action); }
        if preferences { self.command_palette.open = false; self.settings.open = true; }
    }
}
