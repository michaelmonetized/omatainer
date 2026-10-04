use super::*;
use crate::library::{
    crates::{CrateId, Edit},
    smart_crates::{Combine, Condition, NumberField, Rule, TextField, TextMatch},
};
use library_metadata::CollectionAction;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Weak,
};

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    selected: Option<CrateId>,
    rule: Rule,
    active: Option<Preview>,
    reviewed: Option<Reviewed>,
    message: String,
    next_preview: u64,
}
struct Reviewed {
    id: u64,
    rule: Rule,
    crate_id: CrateId,
    revision: u64,
    rows: Weak<Vec<LibItem>>,
    catalog: Weak<crate::library::Catalog>,
    count: usize,
    titles: Vec<String>,
}
struct Preview {
    reviewed: Reviewed,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<(usize, Vec<String>), String>>,
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Release);
        }
    }
}
impl Panel {
    /// Retire any preview that no longer describes this exact draft and catalog.
    /// Takes current immutable rows and catalog; stale results can never authorize Save.
    fn poll(&mut self, rows: &Arc<Vec<LibItem>>, catalog: &Arc<crate::library::Catalog>) {
        let current = |reviewed: &Reviewed| {
            self.selected.as_ref() == Some(&reviewed.crate_id)
                && self.rule == reviewed.rule
                && reviewed.rows.as_ptr() == Arc::as_ptr(rows)
                && reviewed.catalog.as_ptr() == Arc::as_ptr(catalog)
        };
        if self
            .reviewed
            .as_ref()
            .is_some_and(|reviewed| !self.open || !current(reviewed))
        {
            self.reviewed = None;
        }
        let Some(active) = &self.active else { return };
        if !self.open || !current(&active.reviewed) {
            active.cancel.store(true, Ordering::Release);
        }
        let result = match active.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Smart crate preview worker stopped".into())
            }
        };
        let active = self.active.take().unwrap();
        if active.cancel.load(Ordering::Acquire) {
            self.message = "Smart crate preview cancelled; nothing saved".into();
            return;
        }
        match result {
            Ok((count, titles)) => {
                let mut reviewed = active.reviewed;
                reviewed.count = count;
                reviewed.titles = titles;
                self.message = format!("Preview: {count} matching tracks");
                self.reviewed = Some(reviewed);
            }
            Err(error) => self.message = error,
        }
    }
}

impl App {
    /// Preview a bounded smart rule on an optional worker.
    /// Takes the current draft; captures stable crate identity and exact metadata without publishing an edit.
    fn preview_smart_crate(&mut self) {
        let panel = &mut self.smart_crates;
        if panel.active.is_some() {
            return;
        }
        panel.reviewed = None;
        let Some(crate_id) = panel.selected.clone() else {
            return;
        };
        let compiled = match panel.rule.compile() {
            Ok(rule) => rule,
            Err(error) => {
                panel.message = error;
                return;
            }
        };
        let permit = match self.engine.cmd.performance().optional_work() {
            Ok(permit) => permit,
            Err(error) => {
                panel.message = error.to_string();
                return;
            }
        };
        let rows = self.library.clone();
        let catalog = self.library_metadata.catalog.clone();
        let Some(id) = panel.next_preview.checked_add(1) else {
            panel.message = "Smart crate preview identity exhausted; restart before editing".into();
            return;
        };
        panel.next_preview = id;
        let reviewed = Reviewed {
            id,
            rule: panel.rule.clone(),
            crate_id,
            revision: catalog.crates.revision(),
            rows: Arc::downgrade(&rows),
            catalog: Arc::downgrade(&catalog),
            count: 0,
            titles: vec![],
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let (done, result) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("omatainer-smart-preview".into())
            .spawn(move || {
                let mut count = 0;
                let mut titles = Vec::new();
                for (index, item) in rows.iter().enumerate() {
                    if index % 128 == 0 && (cancelled.load(Ordering::Acquire) || permit.cancelled())
                    {
                        let _ =
                            done.send(Err("Smart crate preview cancelled; nothing saved".into()));
                        return;
                    }
                    let Some(track) = catalog.track(&item.source) else {
                        continue;
                    };
                    if track.versions[track.current].fingerprint != item.fingerprint {
                        continue;
                    }
                    let key = crate::musical_key::effective(Some(&track.versions[track.current]), &item.key, track.locks.metadata).0;
                    if compiled.matches(crate::library::search::Row {
                        title: &item.title,
                        artist: &item.artist,
                        key: &key,
                        bpm: item.bpm.value(),
                        seconds: item.length,
                        played: item.last_play.is_some(),
                        annotations: &track.annotations,
                    }) {
                        count += 1;
                        if titles.len() < 8 {
                            titles.push(item.title.chars().take(128).collect::<String>());
                        }
                    }
                }
                let _ = done.send(Ok((count, titles)));
            }) {
            Ok(_) => {
                panel.active = Some(Preview {
                    reviewed,
                    cancel,
                    result,
                });
                panel.message = "Preparing smart crate preview…".into();
            }
            Err(error) => panel.message = format!("Smart crate preview unavailable: {error}"),
        }
    }

    /// Edit typed automatic crate conditions through the native window.
    /// Takes the frame context; Save publishes only the exact successfully previewed draft.
    pub(super) fn smart_crates_ui(&mut self, ctx: &egui::Context) {
        let selected = self.library_crates.selected.clone();
        if self.smart_crates.selected != selected {
            self.smart_crates.selected = selected.clone();
            self.smart_crates.rule = selected
                .as_ref()
                .and_then(|id| self.library_metadata.catalog.crates.node(id))
                .and_then(|node| node.smart_rule.clone())
                .unwrap_or_default();
            self.smart_crates.reviewed = None;
            self.smart_crates.message.clear();
        }
        self.smart_crates
            .poll(&self.library, &self.library_metadata.catalog);
        if !self.smart_crates.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let available = self.library_metadata.ready()
            && self.library_crates.pending.is_none()
            && self.project.dialog_is_closed()
            && !self.project.committing()
            && !self.library_closing()
            && !self.engine.cmd.performance().protected();
        let mut open = true;
        let mut preview = false;
        let mut action = None;
        let panel = &mut self.smart_crates;
        egui::Window::new(tr!("Smart crate rules")).id(egui::Id::new("smart-crate-rules-window")).open(&mut open).default_width(660.0)
            .vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
            let name = selected.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)).map(|node|node.name.as_str()).unwrap_or("Select an empty named crate first");
            ui.heading(name);
            ui.label(tr!("Membership follows current track metadata automatically. Audio and manual preparation stay intact."));
            let previous = panel.rule.clone();
            ui.add_enabled_ui(available && panel.active.is_none(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr!("Match"));
                    ui.selectable_value(&mut panel.rule.combine, Combine::All, tr!("All conditions")).help(ui, HelpControl::SmartCrates);
                    ui.selectable_value(&mut panel.rule.combine, Combine::Any, tr!("Any condition")).help(ui, HelpControl::SmartCrates);
                });
                let mut remove = None;
                for (index, condition) in panel.rule.conditions.iter_mut().enumerate() {
                    ui.push_id(index, |ui| { ui.horizontal_wrapped(|ui| {
                        condition_ui(ui, condition, index);
                        if ui.button(tr!("Remove condition")).help(ui, HelpControl::SmartCrates).clicked() { remove = Some(index); }
                    }); });
                }
                if let Some(index) = remove { panel.rule.conditions.remove(index); }
                if ui.add_enabled(panel.rule.conditions.len() < crate::library::smart_crates::MAX_CONDITIONS, egui::Button::new(tr!("Add condition"))).help(ui, HelpControl::SmartCrates).clicked() {
                    panel.rule.conditions.push(Rule::default().conditions.remove(0));
                }
            });
            if previous != panel.rule { panel.reviewed = None; }
            ui.label(&panel.message);
            if let Some(reviewed) = &panel.reviewed { for title in &reviewed.titles { ui.label(title); } }
            ui.push_id(("smart-preview-actions", &selected, self.library_metadata.catalog.crates.revision()), |ui| { ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(available && selected.is_some() && panel.active.is_none(), egui::Button::new(tr!("Preview smart crate"))).help(ui, HelpControl::SmartCrates).clicked() { preview = true; }
                ui.push_id(panel.active.as_ref().map(|active|active.reviewed.id), |ui| { if ui.add_enabled(panel.active.is_some(), egui::Button::new(tr!("Cancel smart crate preview"))).help(ui, HelpControl::SmartCrates).clicked() {
                    if let Some(active) = &panel.active { active.cancel.store(true, Ordering::Release); }
                }
                });
                ui.push_id(panel.reviewed.as_ref().map(|reviewed|reviewed.id), |ui| { if ui.add_enabled(available && panel.reviewed.is_some(), egui::Button::new(tr!("Save previewed smart rule"))).help(ui, HelpControl::SmartCrates).clicked() {
                    let reviewed = panel.reviewed.take().unwrap();
                    action = Some((reviewed.revision, CollectionAction::Edit(Edit::SetSmartRule { id: reviewed.crate_id, rule: Some(reviewed.rule) })));
                }
                });
            }); });
            ui.push_id(("smart-saved-actions", &selected, self.library_metadata.catalog.crates.revision()), |ui| { ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(available && selected.is_some() && panel.active.is_none(), egui::Button::new(tr!("Refresh smart membership"))).help(ui, HelpControl::SmartCrates).clicked() {
                    action = Some((self.library_metadata.catalog.crates.revision(), CollectionAction::Read));
                }
                if ui.add_enabled(available && selected.as_ref().is_some_and(|id|self.library_metadata.catalog.crates.node(id).is_some_and(|node|node.smart_rule.is_some())), egui::Button::new(tr!("Remove smart rule"))).help(ui, HelpControl::SmartCrates).clicked() {
                    panel.reviewed = None;
                    action = Some((self.library_metadata.catalog.crates.revision(), CollectionAction::Edit(Edit::SetSmartRule { id: selected.clone().unwrap(), rule: None })));
                }
            }); });
            ui.label(&self.library_crates.message);
            ui.label(tr!("Ranges include both ends. Unknown BPM and duration do not match. Played means this current track version has confirmed playback. Removing a rule leaves an empty manual crate."));
        });
        self.smart_crates.open = open;
        if preview {
            self.preview_smart_crate();
        }
        if let Some((revision, action)) = action {
            self.submit_crate_edit(revision, action);
        }
    }
}

const FIELDS: [&str; 10] = [
    "Title", "Artist", "Key", "Tag", "Group", "Note", "BPM", "Length", "Rating", "Played",
];

/// Edit one typed condition with native labels and bounded controls.
/// Takes a rule row and its index; changes only the draft and preserves explicit field types.
fn condition_ui(ui: &mut Ui, condition: &mut Condition, index: usize) {
    let mut field = match condition {
        Condition::Text { field, .. } => match field {
            TextField::Title => 0,
            TextField::Artist => 1,
            TextField::Key => 2,
            TextField::Tag => 3,
            TextField::Group => 4,
            TextField::Note => 5,
        },
        Condition::Number { field, .. } => match field {
            NumberField::Bpm => 6,
            NumberField::Length => 7,
            NumberField::Rating => 8,
        },
        Condition::Played { .. } => 9,
    };
    let previous = field;
    let response = egui::ComboBox::from_id_salt("field")
        .selected_text(FIELDS[field])
        .show_ui(ui, |ui| {
            for (value, name) in FIELDS.iter().enumerate() {
                ui.selectable_value(&mut field, value, *name);
            }
        })
        .response
        .help(ui, HelpControl::SmartCrates);
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(format!("Smart condition {} field", index + 1))
    });
    if field != previous {
        *condition = match field {
            0..=5 => Condition::Text {
                field: [
                    TextField::Title,
                    TextField::Artist,
                    TextField::Key,
                    TextField::Tag,
                    TextField::Group,
                    TextField::Note,
                ][field],
                comparison: TextMatch::Equals,
                value: String::new(),
            },
            6..=8 => Condition::Number {
                field: [NumberField::Bpm, NumberField::Length, NumberField::Rating][field - 6],
                minimum: 0.0,
                maximum: if field == 8 { 5.0 } else { 120.0 },
            },
            _ => Condition::Played { value: true },
        };
    }
    match condition {
        Condition::Text {
            comparison, value, ..
        } => {
            ui.selectable_value(comparison, TextMatch::Equals, tr!("Equals"))
                .help(ui, HelpControl::SmartCrates);
            ui.selectable_value(comparison, TextMatch::Contains, tr!("Contains"))
                .help(ui, HelpControl::SmartCrates);
            let response = ui
                .add(
                    egui::TextEdit::singleline(value)
                        .char_limit(256)
                        .desired_width(220.0),
                )
                .help(ui, HelpControl::SmartCrates);
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label(format!("Smart condition {} text", index + 1))
            });
        }
        Condition::Number {
            field,
            minimum,
            maximum,
        } => {
            let (limit, unit, speed) = match field {
                NumberField::Bpm => (1000.0, " BPM", 0.1),
                NumberField::Length => (604800.0, " seconds", 1.0),
                NumberField::Rating => (5.0, " stars", 1.0),
            };
            for (name, value) in [("minimum", minimum), ("maximum", maximum)] {
                let response = ui
                    .add(
                        egui::DragValue::new(value)
                            .range(0.0..=limit)
                            .speed(speed)
                            .suffix(unit),
                    )
                    .help(ui, HelpControl::SmartCrates);
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label(format!("Smart condition {} {name}", index + 1))
                });
            }
        }
        Condition::Played { value } => {
            ui.checkbox(value, tr!("Has played"))
                .help(ui, HelpControl::SmartCrates);
        }
    }
}

#[cfg(test)]
mod tests;
