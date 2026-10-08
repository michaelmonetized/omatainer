use super::*;
use crate::engine::note_variation::{self, Expression, Group, GroupKind, Properties, Velocity};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping { Independent, Linked, Exclusive }
struct Settings {
    velocity: u8,
    chance: f64,
    range: bool,
    minimum: u8,
    maximum: u8,
    grouping: Grouping,
    expression: Expression,
}
impl Default for Settings {
    fn default() -> Self {
        Self { velocity: 90, chance: 100.0, range: false, minimum: 64, maximum: 127, grouping: Grouping::Independent, expression: Expression::PolyPressure }
    }
}
#[derive(Default)]
pub(super) struct Controls { settings: Settings, seed: Seed }
impl Controls {
    pub(super) fn busy(&self) -> bool { self.seed.busy() }
    fn apply(&self, draft: &mut Draft, clear: bool) -> Result<bool, String> {
        if !draft.tools.editing() { return Err("Keep or restore the transformation preview before changing note choices".into()); }
        let selected: Vec<_> = draft.notes.iter().enumerate().filter(|(_, note)| draft.selected.contains(&note.id)).map(|(index, _)| index).collect();
        if selected.is_empty() { return Err("Select notes for the velocity and chance lanes".into()); }
        let identity = selected.iter().map(|&index| draft.notes[index].id).min().unwrap();
        let group = match self.settings.grouping {
            Grouping::Independent => None,
            Grouping::Linked => Some(Group { identity, kind: GroupKind::Linked }),
            Grouping::Exclusive => Some(Group { identity, kind: GroupKind::Exclusive }),
        };
        let properties = (!clear).then_some(Properties {
            chance: (self.settings.chance * 100.0).round() as u16,
            velocity: self.settings.range.then_some(Velocity { minimum: self.settings.minimum, maximum: self.settings.maximum }),
            group,
            expression: self.settings.expression,
        });
        let mut next = draft.notes.clone();
        for index in selected { next[index].variation = properties; }
        note_variation::validate(&next)?;
        if next == draft.notes { return Ok(false); }
        draft.notes = next;
        draft.dirty = true;
        Ok(true)
    }
    fn apply_velocity(&self, draft: &mut Draft) -> Result<bool, String> {
        if !draft.tools.editing() { return Err("Keep or restore the transformation preview before changing note velocity".into()); }
        if !draft.notes.iter().any(|note|draft.selected.contains(&note.id)) { return Err("Select notes for the velocity lane".into()); }
        let mut changed = false;
        for note in &mut draft.notes {
            if draft.selected.contains(&note.id) && note.vel != self.settings.velocity {
                note.vel = self.settings.velocity;
                changed = true;
            }
        }
        draft.dirty |= changed;
        Ok(changed)
    }
}

pub(super) fn show(ui: &mut Ui, draft: &mut Draft, theme: &Theme) -> Result<bool, String> {
    let mut controls = std::mem::take(&mut draft.variation);
    let mut action = None;
    let mut velocity = false;
    ui.collapsing("Note velocity and chance", |ui| {
        ui.label("Choices repeat from the saved project seed. Linked notes play together; exclusive weights select at most one note. Groups require the same onset.");
        ui.horizontal_wrapped(|ui| {
            let mut base = f64::from(controls.settings.velocity);
            if number(ui, "Lane note velocity", &mut base, 1.0, 127.0) { controls.settings.velocity = base.round() as u8; }
            velocity = button(ui, "Set selected note velocity").clicked();
        });
        ui.horizontal_wrapped(|ui| {
            number(ui, "Note chance percent", &mut controls.settings.chance, 0.0, 100.0);
            let response = ui.checkbox(&mut controls.settings.range, "Vary note velocity");
            accessibility::button(ui, &response, "Vary note velocity", Some(controls.settings.range));
            let mut minimum = f64::from(controls.settings.minimum);
            let mut maximum = f64::from(controls.settings.maximum);
            if number(ui, "Minimum note velocity", &mut minimum, 1.0, 127.0) { controls.settings.minimum = minimum.round() as u8; }
            if number(ui, "Maximum note velocity", &mut maximum, 1.0, 127.0) { controls.settings.maximum = maximum.round() as u8; }
        });
        ui.horizontal_wrapped(|ui| {
            let grouping = egui::ComboBox::from_id_salt("note-choice-group")
                .selected_text(match controls.settings.grouping { Grouping::Independent => "Independent", Grouping::Linked => "Linked", Grouping::Exclusive => "Exclusive" })
                .show_ui(ui, |ui| { for (kind, label) in [(Grouping::Independent, "Independent"), (Grouping::Linked, "Linked"), (Grouping::Exclusive, "Exclusive")] { ui.selectable_value(&mut controls.settings.grouping, kind, label); } });
            grouping.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Note probability group"));
            let mut mpe = controls.settings.expression != Expression::PolyPressure;
            let response = ui.checkbox(&mut mpe, "Explicit MPE expression ownership");
            accessibility::button(ui, &response, "Explicit MPE expression ownership", Some(mpe));
            if response.changed() { controls.settings.expression = if mpe { Expression::Lower(15) } else { Expression::PolyPressure }; }
            if mpe {
                let mut upper = matches!(controls.settings.expression, Expression::Upper(_));
                ui.checkbox(&mut upper, "Upper MPE zone");
                let mut members = match controls.settings.expression { Expression::Lower(n) | Expression::Upper(n) => f64::from(n), Expression::PolyPressure => 15.0 };
                number(ui, "MPE member channels", &mut members, 1.0, 15.0);
                controls.settings.expression = if upper { Expression::Upper(members.round() as u8) } else { Expression::Lower(members.round() as u8) };
            }
        });
        ui.horizontal_wrapped(|ui| {
            if button(ui, "Set selected note choices").clicked() { action = Some(false); }
            if button(ui, "Clear selected note choices").clicked() { action = Some(true); }
        });
        let rows = egui::ScrollArea::vertical().id_salt("note-choice-lanes").max_height(180.0).show_rows(ui, 44.0, draft.notes.len(), |ui, range| {
            for index in range {
                let note = &draft.notes[index];
                let properties = note.variation.unwrap_or_default();
                let bounds = properties.velocity.unwrap_or(Velocity { minimum: note.vel, maximum: note.vel });
                let label = format!("Note {} velocity lane: {}–{}; chance lane: {:.2}%; {}", index + 1, bounds.minimum, bounds.maximum, f64::from(properties.chance) / 100.0, match properties.group.map(|g|g.kind) { None => "independent", Some(GroupKind::Linked) => "linked", Some(GroupKind::Exclusive) => "exclusive" });
                let response = ui.push_id(note.id, |ui| ui.selectable_label(draft.selected.contains(&note.id), &label)).inner;
                accessibility::button(ui, &response, &label, Some(draft.selected.contains(&note.id)));
                let id = note.id;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(460.0), 14.0), egui::Sense::hover());
                let velocity = egui::Rect::from_min_max(egui::pos2(rect.left() + rect.width() * f32::from(bounds.minimum) / 127.0, rect.top()), egui::pos2(rect.left() + rect.width() * f32::from(bounds.maximum) / 127.0, rect.top() + 5.0));
                ui.painter().rect_filled(velocity, 0.0, theme.blue);
                let chance = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + 8.0), egui::vec2(rect.width() * f32::from(properties.chance) / f32::from(note_variation::CERTAIN), 5.0));
                ui.painter().rect_filled(chance, 0.0, theme.green);
                if response.clicked() { draft.select(id, ui.input(|i|i.modifiers.shift)); }
            }
        });
        accessibility::scrollbars(ui, "Note velocity and chance lanes", &rows);
    });
    let result = if velocity { controls.apply_velocity(draft) } else { action.map_or(Ok(false), |clear| controls.apply(draft, clear)) };
    draft.variation = controls;
    result
}

struct SeedWorker {
    receiver: mpsc::Receiver<Result<(Box<crate::engine::arrangement::edit::Request>, Ack), String>>,
    cancel: Arc<AtomicBool>,
}
struct Seed {
    text: String,
    initialized: bool,
    worker: Option<SeedWorker>,
    pending: Option<Ack>,
    message: String,
}
impl Default for Seed {
    fn default() -> Self { Self { text: "1".into(), initialized: false, worker: None, pending: None, message: String::new() } }
}
impl Drop for Seed {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker { worker.cancel.store(true, Ordering::Release); }
        if let Some(ack) = &self.pending { ack.cancel(); }
    }
}
impl Seed {
    fn busy(&self) -> bool { self.worker.is_some() || self.pending.is_some() }
    fn begin(&mut self, engine: &Engine) -> Result<(), String> {
        if self.busy() { return Err("A note-seed change is already pending".into()); }
        let seed = self.text.parse::<u64>().map_err(|_|"Use a whole note seed from 0 through 18446744073709551615")?;
        let project = engine.project.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("note-seed-prepare".into()).spawn(move || {
            let result = project.capture(&worker_cancel).map_err(|error|error.to_string()).and_then(|captured|crate::engine::arrangement::edit::Request::prepare_seed(captured, seed, &worker_cancel));
            let _ = sender.send(result);
        }).map_err(|error|format!("Note seed preparation could not start: {error}"))?;
        self.worker = Some(SeedWorker { receiver, cancel });
        self.message = "Preparing the saved seed and song choices…".into();
        Ok(())
    }
    fn poll(&mut self, engine: &Engine) -> Result<(), String> {
        if let Some(worker) = &self.worker {
            let result = match worker.receiver.try_recv() { Ok(result) => Some(result), Err(mpsc::TryRecvError::Empty) => None, Err(mpsc::TryRecvError::Disconnected) => Some(Err("Note seed worker disconnected; the draft is retained".into())) };
            if let Some(result) = result {
                let worker = self.worker.take().unwrap();
                if worker.cancel.load(Ordering::Acquire) { self.message = "Note seed change cancelled; the saved seed is retained".into(); return Ok(()); }
                let (request, ack) = result?;
                engine.send(Command::ArrangementEdit(request)).map_err(|error|error.to_string())?;
                self.pending = Some(ack);
            }
        }
        if let Some(ack) = &self.pending {
            match ack.state() {
                Outcome::Pending => {},
                Outcome::Applied => { self.pending = None; self.message = "Saved note seed applied with one Undo".into(); },
                Outcome::Rejected => { self.pending = None; return Err("The project changed or playback is active; the seed draft is retained".into()); },
                Outcome::Cancelled => { self.pending = None; self.message = "Note seed change cancelled".into(); },
            }
        }
        Ok(())
    }
}

pub(super) fn show_seed(ui: &mut Ui, draft: &mut Draft, engine: &Engine, snap: &crate::engine::Snapshot) -> Result<(), String> {
    let seed = &mut draft.variation.seed;
    if !seed.initialized { seed.text = snap.note_seed.to_string(); seed.initialized = true; }
    let mut result = Ok(());
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Saved note seed: {}", snap.note_seed));
        let response = ui.text_edit_singleline(&mut seed.text);
        response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Note variation seed"));
        let stopped = !snap.playing && !snap.recording && snap.count_in_remaining == 0.0 && !snap.decks.iter().any(|deck|deck.playing || deck.touching);
        ui.add_enabled_ui(stopped && !seed.busy(), |ui| {
            if button(ui, "Apply saved note seed").clicked() { result = seed.begin(engine); }
            if button(ui, "Next note variation").clicked() { seed.text = snap.note_seed.wrapping_add(1).to_string(); result = seed.begin(engine); }
        });
        if button(ui, "Restore seed draft").clicked() { seed.text = snap.note_seed.to_string(); }
        if seed.busy() && button(ui, "Cancel note seed change").clicked() {
            if let Some(worker) = &seed.worker { worker.cancel.store(true, Ordering::Release); }
            if let Some(ack) = &seed.pending { ack.cancel(); }
        }
    });
    if !seed.message.is_empty() { let response = ui.label(&seed.message); accessibility::status(ui, &response, &seed.message); }
    result
}

pub(super) fn poll(draft: &mut Draft, engine: &Engine) -> Result<(), String> { draft.variation.seed.poll(engine) }

#[cfg(test)]
mod tests;
