//! Explicit widget metadata, offline reference, and renderer-observed lessons.
use super::*;
mod catalogue;
mod lessons;
pub(crate) use catalogue::{Control, Topic};
use lessons::{Lesson, Observation};
const CONTEXT: &str = "omatainer-help-context";
#[derive(Clone, Copy)]
struct Context {
    control: Control,
    frame: u64,
    focused: bool,
}

/// Attach only at the actual handler. Labels are deliberately not parsed.
pub(super) fn annotate(ui: &Ui, response: &egui::Response, control: Control) {
    let definition = control.definition();
    rich_tooltip(response, || {
        vec![format!(
            "{}\n{}\n{}\nHelp: {}",
            definition.title,
            definition.units,
            definition.purpose,
            definition.topic.title()
        )]
    });
    describe(ui, response, control);
}

/// Use with a single rich tooltip when the control also exposes dynamic data.
pub(super) fn describe(ui: &Ui, response: &egui::Response, control: Control) {
    let definition = control.definition();
    ui.ctx().accesskit_node_builder(response.id, |node| {
        let existing = node.description().unwrap_or_default().to_owned();
        node.set_description(format!(
            "{existing} {}. {}. {}",
            definition.title, definition.units, definition.purpose
        ));
    });
    if !matches!(
        control,
        Control::Help
            | Control::HelpTopic
            | Control::HelpSearch
            | Control::HelpReference
            | Control::HelpShortcuts
            | Control::LessonStart
            | Control::LessonNext
            | Control::LessonCancel
            | Control::LessonPhysical
    ) && (response.has_focus() || response.hovered())
    {
        let frame = ui.ctx().cumulative_frame_nr();
        let focused = response.has_focus();
        ui.data_mut(|data| {
            let key = egui::Id::new(CONTEXT);
            let previous = data.get_temp::<Context>(key);
            if focused || !previous.is_some_and(|old| old.frame == frame && old.focused) {
                data.insert_temp(
                    key,
                    Context {
                        control,
                        frame,
                        focused,
                    },
                );
            }
        });
    }
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(response.id.with("help-control"), control));
}

/// Paint informational text without registering a hit-test widget or Area.
/// egui 0.32's Tooltip Area/labels can otherwise hide the next pad beneath a
/// long tooltip, even when the Area itself has `interactable(false)`.
pub(super) fn rich_tooltip(response: &egui::Response, paragraphs: impl FnOnce() -> Vec<String>) {
    if !egui::Tooltip::should_show_tooltip(response) {
        return;
    }
    let paragraphs = paragraphs();
    let ctx = &response.ctx;
    let style = ctx.style();
    let screen = ctx.screen_rect();
    let margin = Vec2::new(8.0, 6.0);
    let width = style
        .spacing
        .tooltip_width
        .min((screen.width() - 2.0 * margin.x).max(20.0));
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            response.id.with("offline-help-tooltip"),
        ))
        .with_clip_rect(screen);
    let font = egui::TextStyle::Body.resolve(&style);
    let color = style.visuals.text_color();
    let galleys: Vec<_> = paragraphs
        .iter()
        .map(|text| painter.layout(text.clone(), font.clone(), color, width))
        .collect();
    let size = Vec2::new(
        galleys.iter().map(|g| g.size().x).fold(0.0, f32::max),
        galleys.iter().map(|g| g.size().y).sum::<f32>()
            + 4.0 * galleys.len().saturating_sub(1) as f32,
    ) + 2.0 * margin;
    let position = (response.rect.left_bottom() + Vec2::new(0.0, 4.0))
        .min(screen.max - size)
        .max(screen.min);
    let rect = Rect::from_min_size(position, size);
    painter.rect_filled(rect, 4.0, style.visuals.window_fill());
    painter.rect_stroke(
        rect,
        4.0,
        style.visuals.window_stroke(),
        egui::StrokeKind::Inside,
    );
    let mut position = rect.min + margin;
    for galley in galleys {
        let height = galley.size().y;
        painter.galley(position, galley, color);
        position.y += height + 4.0;
    }
}

pub(super) fn tooltip_text(control: Control) -> String {
    let d = control.definition();
    format!("{} · {}\n{}", d.title, d.units, d.purpose)
}

pub(super) trait ContextHelp {
    fn help(self, ui: &Ui, control: Control) -> Self;
    fn help_detail(self, ui: &Ui, control: Control, detail: &str) -> Self;
}
impl ContextHelp for egui::Response {
    fn help_detail(self, ui: &Ui, control: Control, detail: &str) -> Self {
        rich_tooltip(&self, || vec![detail.to_owned(), tooltip_text(control)]);
        describe(ui, &self, control);
        ui.ctx().accesskit_node_builder(self.id, |node| {
            let previous = node.description().unwrap_or_default().to_owned();
            node.set_description(format!("{previous} {detail}"));
        });
        self
    }
    fn help(self, ui: &Ui, control: Control) -> Self {
        annotate(ui, &self, control);
        self
    }
}

pub(super) struct Help {
    topic: Topic,
    filter: String,
    context: Option<Control>,
    lesson: Option<Lesson>,
}
#[cfg(test)]
impl Help {
    pub(super) fn evidence(&self) -> Option<(usize, bool, bool)> {
        self.lesson
            .as_ref()
            .map(|lesson| (lesson.step, lesson.ready, lesson.complete()))
    }
}
impl Default for Help {
    fn default() -> Self {
        Self {
            topic: Topic::Setup,
            filter: String::new(),
            context: None,
            lesson: None,
        }
    }
}
impl App {
    pub(super) fn help_panel(&mut self, ctx: &egui::Context) {
        let project = self.project_help_state();
        let history = self.engine.undo.view();
        let observation = Observation {
            snap: &self.snap,
            history: &history,
            receipts: self
                .loads
                .each_ref()
                .map(|load| load.as_ref().and_then(|load| load.receipt.as_ref())),
            edit: self.clip_gain_edit,
            diagnostics: self.diagnostics.open,
            midi: self.midi_open,
            midi_dispatched: self.engine.midi.input_stats().dispatched,
            audio_running: self.engine.output_info().is_some(),
            audio_pending_restart: self.settings.pending_restart(),
            project: &project,
        };
        if let Some(lesson) = &mut self.help.lesson {
            lesson.observe(&observation);
        }
        if !self.keys_open {
            return;
        }
        if let Some(context) = ctx.data(|data| data.get_temp::<Context>(egui::Id::new(CONTEXT))) {
            self.help.context = Some(context.control);
        }
        let mut open = self.keys_open;
        egui::Window::new("Help and lessons").open(&mut open).default_pos(Pos2::new(30.0, 60.0)).default_width(590.0).default_height(620.0).show(ctx, |ui| {
            let content = egui::ScrollArea::vertical().id_salt("offline-help-content").max_height(ui.available_height().max(160.0)).show(ui, |ui| {
            ui.label("Offline · implementation-specific · hardware checks remain your observations");
            if let Some(control) = self.help.context {
                let definition = control.definition();
                ui.heading(format!("Control: {}", definition.title));
                ui.label(definition.units);
                ui.label(definition.purpose);
                if ui.button(format!("Read: {}", definition.topic.title())).help(ui, Control::HelpTopic).clicked() { self.help.topic = definition.topic; }
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| { for topic in Topic::ALL {
                if ui.selectable_label(self.help.topic == topic, topic.title()).help(ui, Control::HelpTopic).clicked() { self.help.topic = topic; }
            }});
            ui.heading(self.help.topic.title());
            ui.label(self.help.topic.text());
            if !lessons::steps(self.help.topic).is_empty() && ui.button("Start this lesson").help(ui, Control::LessonStart).clicked() {
                self.help.lesson = Some(Lesson::new(self.help.topic, &observation));
            }
            if let Some(lesson) = &mut self.help.lesson {
                ui.separator();
                ui.heading(format!("Lesson: {}", lesson.topic.title()));
                if lesson.invalidated {
                    ui.label("Lesson target or document/history identity changed. Restart the lesson to capture a fresh target; earlier progress cannot certify the new document.");
                } else if lesson.complete() {
                    ui.label("Lesson complete: the required software states were observed. Physical checks, where requested, are self-reported only.");
                } else {
                    ui.label(format!("Step {} of {}", lesson.step + 1, lesson.steps().len()));
                    ui.label(lesson.steps()[lesson.step]);
                    if lesson.needs_physical_ack() {
                        ui.checkbox(&mut lesson.physical_ack, "I personally verified this physical check").help(ui, Control::LessonPhysical);
                    }
                    ui.label(if lesson.ready { "Observed — ready for Next" } else { "Waiting for the stated evidence; queued or rejected actions do not count" });
                    if ui.add_enabled(lesson.ready, egui::Button::new("Next lesson step")).help(ui, Control::LessonNext).clicked() { lesson.next(); }
                }
                if ui.button("Cancel lesson").help(ui, Control::LessonCancel).clicked() { self.help.lesson = None; }
            }
            ui.separator();
            let shortcuts = ui.collapsing("Active shortcuts and accessible input", |ui| shortcuts::show_help_with(ui, self.settings.profile()));
            annotate(ui, &shortcuts.header_response, Control::HelpShortcuts);
            ui.separator();
            ui.label("Control reference");
            ui.add(egui::TextEdit::singleline(&mut self.help.filter).hint_text("Search help controls")).help(ui, Control::HelpSearch).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search help controls"));
            let filter = self.help.filter.to_lowercase();
            for &control in Control::ALL {
                let definition = control.definition();
                if !filter.is_empty() && !format!("{} {} {}", definition.title, definition.units, definition.purpose).to_lowercase().contains(&filter) { continue; }
                let entry = ui.collapsing(format!("Reference: {}", definition.title), |ui| { ui.label(definition.units); ui.label(definition.purpose); });
                annotate(ui, &entry.header_response, Control::HelpReference);
            }
            });
            accessibility::scrollbars(ui, "Help content", &content);
        });
        self.keys_open = open;
    }
}

#[cfg(test)]
mod tests;
