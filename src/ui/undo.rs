//! Renderer-owned history, with producer gesture IDs and truthful queue status.
use super::*;
use crate::engine::undo::View;

#[derive(Default)]
pub(super) struct History {
    pub open: bool,
    gesture: Cell<Option<u64>>,
    keyboard_gesture: Option<(egui::Id, Key)>,
    view: View,
    seen_failures: u64,
    failure: Option<String>,
    queued: Option<&'static str>,
    refresh_editor_at: Option<u64>,
}

impl History {
    pub fn begin_frame(&mut self, ctx: &egui::Context, handle: &crate::engine::undo::Handle) {
        let focused = ctx.memory(|memory| memory.focused());
        if !ctx.input(|input| input.focused) {
            self.gesture.set(None);
            self.keyboard_gesture = None;
            return;
        }
        if ctx.input(|input| input.pointer.button_pressed(PointerButton::Primary)) {
            self.gesture.set(Some(handle.gesture()));
            self.keyboard_gesture = None;
        } else if !ctx.input(|input| input.pointer.button_down(PointerButton::Primary)) {
            if self.keyboard_gesture.is_some_and(|(owner, key)| {
                focused != Some(owner) || !ctx.input(|input| input.key_down(key))
            }) {
                self.gesture.set(None);
                self.keyboard_gesture = None;
            }
            if let Some(owner) = focused {
                let start = ctx.input(|input| {
                    input.events.iter().find_map(|event| match event {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if !modifiers.ctrl
                            && !modifiers.alt
                            && !modifiers.command
                            && matches!(
                                key,
                                Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
                            ) =>
                        {
                            Some(*key)
                        }
                        _ => None,
                    })
                });
                if let Some(key) = start {
                    self.gesture.set(Some(handle.gesture()));
                    self.keyboard_gesture = Some((owner, key));
                }
            }
        }
    }
    pub fn end_frame(&mut self, ctx: &egui::Context) {
        if self.keyboard_gesture.is_none()
            && !ctx.input(|input| input.pointer.button_down(PointerButton::Primary))
        {
            self.gesture.set(None);
        }
    }
    pub fn wrap(&self, command: Command) -> Command {
        let Some(id) = self.gesture.get() else {
            return command;
        };
        if matches!(
            command,
            Command::SetBpm(_)
                | Command::NudgeBpm(_)
                | Command::Xfader(_)
                | Command::XfaderCurve(_)
                | Command::Master(_)
                | Command::CueMix(_)
                | Command::TrackGain { .. }
                | Command::TrackPan { .. }
                | Command::ClipGain { .. }
                | Command::DeckPitch { .. }
                | Command::DeckGain { .. }
                | Command::DeckEq { .. }
                | Command::DeckFilter { .. }
                | Command::DeckSeek { .. }
                | Command::FxWet { .. }
                | Command::FxMix { .. }
                | Command::FxParam { .. }
        ) {
            Command::Gesture {
                id,
                command: Box::new(command),
            }
        } else {
            command
        }
    }
}

impl App {
    pub(super) fn poll_undo(&mut self) {
        let view = self.engine.undo.view();
        if (view.epoch, view.state) != (self.undo_history.view.epoch, self.undo_history.view.state)
        {
            self.undo_history.refresh_editor_at = Some(self.engine.project.revision());
        }
        if view.failures != self.undo_history.seen_failures {
            self.undo_history.seen_failures = view.failures;
            self.undo_history.failure = view.failure.map(|failure| failure.label().to_owned());
        }
        self.undo_history.view = view;
        if self
            .undo_history
            .refresh_editor_at
            .is_some_and(|revision| self.snap.project_revision >= revision)
        {
            self.undo_history.refresh_editor_at = None;
            if self.undo_history.gesture.get().is_none() {
                if let Some(edit) = &mut self.clip_gain_edit {
                    if let Some(clip) = self
                        .snap
                        .tracks
                        .get(edit.track as usize)
                        .and_then(|track| track.clips.get(edit.scene as usize))
                    {
                        edit.value = clip.gain;
                    }
                }
            }
        }
    }
    pub(super) fn history_action(&mut self, redo: bool) {
        self.undo_history.gesture.set(None);
        self.undo_history.keyboard_gesture = None;
        if self.submit(if redo { Command::Redo } else { Command::Undo }) {
            self.undo_history.queued = Some(if redo { "Redo queued" } else { "Undo queued" });
        }
    }
    pub(super) fn undo_menu(&mut self, ui: &mut Ui) {
        let menu = ui.menu_button(tr!("Edit"), |ui| {
            let view = &self.undo_history.view;
            let undo = view
                .cursor
                .checked_sub(1)
                .and_then(|index| view.items[index]);
            let redo = view.items.get(view.cursor).copied().flatten();
            let label = undo
                .map(|item| format!("Undo {}", item.label()))
                .unwrap_or_else(|| "Undo".into());
            let response = ui.add_enabled(
                    undo.is_some(),
                    egui::Button::new(label).shortcut_text(shortcuts::action_label(self.settings.profile(), shortcuts::Action::Undo)),
                );
            help::annotate(ui, &response, help::Control::Undo);
            if response.clicked() {
                self.history_action(false);
                ui.close();
            }
            let label = redo
                .map(|item| format!("Redo {}", item.label()))
                .unwrap_or_else(|| "Redo".into());
            let response = ui.add_enabled(
                    redo.is_some(),
                    egui::Button::new(label).shortcut_text(shortcuts::action_label(self.settings.profile(), shortcuts::Action::Redo)),
                );
            help::annotate(ui, &response, help::Control::Redo);
            if response.clicked() {
                self.history_action(true);
                ui.close();
            }
            ui.separator();
            let response = ui.button(tr!("History…"));
            help::annotate(ui, &response, help::Control::History);
            if response.clicked() {
                self.undo_history.open = true;
                ui.close();
            }
        });
        help::annotate(ui, &menu.response, help::Control::History);
    }
    pub(super) fn undo_panel(&mut self, ctx: &egui::Context) {
        if !self.undo_history.open && self.undo_history.failure.is_none() {
            return;
        }
        let mut open = self.undo_history.open || self.undo_history.failure.is_some();
        egui::Window::new(tr!("Edit history")).id(egui::Id::new("edit-history"))
            .open(&mut open).default_width(370.0).show(ctx, |ui| {
                if let Some(message) = &self.undo_history.failure {
                    ui.colored_label(self.theme.red, message);
                    let response = ui.button(tr!("Dismiss history message"));
                    help::annotate(ui, &response, help::Control::HistoryDismiss);
                    if response.clicked() { self.undo_history.failure = None; }
                }
                if let Some(queued) = self.undo_history.queued {
                    ui.label(crate::localization::format("{queued}; the list shows renderer-confirmed history.", &[format!("{}", queued)]));
                }
                let view = &self.undo_history.view;
                let can_undo = view.cursor > 0;
                let can_redo = view.items.get(view.cursor).is_some_and(Option::is_some);
                ui.horizontal(|ui| {
                    let enabled = !self.project.committing() && self.project.dialog_is_closed();
                    let response = ui.add_enabled(enabled && can_undo, egui::Button::new(tr!("Undo")));
                    help::annotate(ui, &response, help::Control::Undo);
                    if response.clicked() { self.history_action(false); }
                    let response = ui.add_enabled(enabled && can_redo, egui::Button::new(tr!("Redo")));
                    help::annotate(ui, &response, help::Control::Redo);
                    if response.clicked() { self.history_action(true); }
                });
                let view = &self.undo_history.view;
                ui.label({ let __omatainer_args = (&(view.cursor),&(view.items.iter().flatten().count().saturating_sub(view.cursor)),); crate::localization::format("{} edits applied · {} available to redo", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) });
                ui.label({ let __omatainer_args = (&(view.bytes as f64 / 1048576.0),&(view.budget as f64 / 1048576.0),&(view.retired_bytes as f64 / 1048576.0),); crate::localization::format("History {:.2} MiB / {:.0} MiB · awaiting retirement {:.2} MiB", &[format!("{:.2}", __omatainer_args.0), format!("{:.0}", __omatainer_args.1), format!("{:.2}", __omatainer_args.2)]) });
                ui.label({ let __omatainer_args = (&(view.fixed_bytes as f64 / 1048576.0),); crate::localization::format("Fixed history storage {:.2} MiB", &[format!("{:.2}", __omatainer_args.0)]) });
                ui.label(tr!("Transport and physical gates are not history edits. New and Open begin a new history."));
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (index, item) in view.items.iter().enumerate().filter_map(|(i, item)| item.map(|item| (i,item))) {
                        ui.label({ let __omatainer_args = (&(if index < view.cursor { "Applied" } else { "Redo" }),&(index + 1),&(item.label()),); crate::localization::format("{} {}. {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                    }
                });
            });
        self.undo_history.open &= open;
        if !open {
            self.undo_history.failure = None;
        }
    }
}

#[cfg(test)]
mod tests;
