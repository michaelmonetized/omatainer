//! Selection-aware track properties retain renderer-owned commands and history.
use super::*;
use crate::engine::session::{Axis, Reference};
use crate::property_fields::{self as fields, FieldState};

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    selection: Option<Vec<(usize, Reference)>>,
    locked: bool,
    message: String,
    editing: Option<Vec<(usize, Reference)>>,
    gain_draft: Option<f32>,
    pan_draft: Option<f32>,
    gain_focused: bool,
    pan_focused: bool,
}

impl App {
    /// Show the same track inspector used by the workspace mixer panel.
    /// Args: native context. Returns nothing; edits use the existing typed command port.
    pub(super) fn mixer_ui(&mut self, ctx: &egui::Context) {
        if !self.mixer.open || self.studio_workspace_active() {
            return;
        }
        let mut open = true;
        egui::Window::new("Track mixer")
            .id(egui::Id::new("track-mixer"))
            .open(&mut open)
            .default_width(420.0)
            .vscroll(true)
            .max_height(self.theme.window_height(ctx))
            .show(ctx, |ui| self.mixer_panel(ui));
        self.mixer.open = open;
    }

    /// Draw track selection and its real gain, pan, mute and solo properties.
    /// Args: available panel UI. Returns nothing; saved project values remain engine-owned.
    pub(super) fn mixer_panel(&mut self, ui: &mut Ui) {
        let Some(layout) = self.snap.session.clone() else {
            ui.label("No session tracks");
            return;
        };
        if self.mixer.selection.as_ref().is_some_and(|selection| {
            selection
                .iter()
                .any(|(slot, target)| !layout.resolves(Axis::Track, *slot, *target))
        }) {
            self.mixer.selection = Some(Vec::new());
            self.mixer.message =
                "Selected tracks changed. Select tracks again before editing.".into();
        }
        let mut selected = self.mixer.selection.clone().unwrap_or_else(|| {
            layout
                .reference(Axis::Track, self.snap.selected_track)
                .map(|target| vec![(self.snap.selected_track, target)])
                .unwrap_or_default()
        });
        fields::section(ui, "Tracks", |ui| {
            ui.horizontal_wrapped(|ui| {
                for slot in layout.track_order.iter().copied().map(usize::from) {
                    let item = &layout.tracks[slot];
                    let chosen = selected.iter().any(|(track, _)| *track == slot);
                    let response = ui
                        .push_id(item.id, |ui| fields::tab(ui, &item.name, chosen))
                        .inner;
                    accessibility::button(
                        ui,
                        &response,
                        &format!("Inspect track {}: {}", slot + 1, item.name),
                        Some(chosen),
                    );
                    if response.clicked() {
                        if ui.input(|input| input.modifiers.command || input.modifiers.shift) {
                            if chosen {
                                selected.retain(|(track, _)| *track != slot);
                            } else if let Some(target) = layout.reference(Axis::Track, slot) {
                                selected.push((slot, target));
                            }
                        } else {
                            selected = layout
                                .reference(Axis::Track, slot)
                                .map(|target| vec![(slot, target)])
                                .unwrap_or_default();
                        }
                        self.mixer.selection = Some(selected.clone());
                        self.mixer.message.clear();
                        self.send(Command::Select {
                            track: slot,
                            scene: self.snap.selected_scene,
                        });
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if fields::action(ui, "Select all tracks", true).clicked() {
                    selected = layout
                        .track_order
                        .iter()
                        .copied()
                        .map(usize::from)
                        .filter_map(|slot| {
                            layout
                                .reference(Axis::Track, slot)
                                .map(|target| (slot, target))
                        })
                        .collect();
                    self.mixer.selection = Some(selected.clone());
                }
                if fields::action(ui, "Clear track selection", !selected.is_empty()).clicked() {
                    selected.clear();
                    self.mixer.selection = Some(Vec::new());
                }
                ui.checkbox(&mut self.mixer.locked, "Lock track inspector");
            });
        });
        if !self.mixer.message.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.mixer.message);
        }
        if self.mixer.editing.as_ref() != Some(&selected) || self.mixer.locked {
            self.mixer.editing = Some(selected.clone());
            self.mixer.gain_draft = None;
            self.mixer.pan_draft = None;
            self.mixer.gain_focused = false;
            self.mixer.pan_focused = false;
        }
        if selected.is_empty() {
            ui.label("Select a track to inspect its properties. Shift-click adds tracks.");
            return;
        }
        let tracks: Vec<_> = selected
            .iter()
            .filter_map(|(slot, _)| self.snap.tracks.get(*slot))
            .map(|track| (track.gain, track.pan, track.mute, track.solo))
            .collect();
        let Some(first) = tracks.first() else {
            return;
        };
        let mut gain = self.mixer.gain_draft.unwrap_or(first.0 * 100.0);
        let mut pan = self.mixer.pan_draft.unwrap_or(first.1 * 100.0);
        let gain_mixed =
            self.mixer.gain_draft.is_none() && tracks.iter().any(|track| track.0 != first.0);
        let pan_mixed =
            self.mixer.pan_draft.is_none() && tracks.iter().any(|track| track.1 != first.1);
        let muted = tracks.iter().all(|track| track.2);
        let soloed = tracks.iter().all(|track| track.3);
        let locked = self.mixer.locked
            || self.engine.safe_mode()
            || self.project.committing()
            || !self.project.dialog_is_closed();
        let error = self.submission_error.get().map(|error| error.to_string());
        fields::section(
            ui,
            &format!("Track properties · {} selected", tracks.len()),
            |ui| {
                let enter = ui.input(|input| input.key_pressed(Key::Enter));
                if locked {
                    ui.label("Track properties are locked");
                }
                let gain_field = fields::number(
                    ui,
                    "track-inspector-gain",
                    "Track gain",
                    &mut gain,
                    0.0..=150.0,
                    "%",
                    Some(100.0),
                    FieldState {
                        mixed: gain_mixed,
                        locked,
                        error: error.as_deref(),
                    },
                );
                let pan_field = fields::number(
                    ui,
                    "track-inspector-pan",
                    "Track pan",
                    &mut pan,
                    -100.0..=100.0,
                    "%",
                    Some(0.0),
                    FieldState {
                        mixed: pan_mixed,
                        locked,
                        error: None,
                    },
                );
                ui.ctx()
                    .accesskit_node_builder(gain_field.response.id, |node| {
                        node.set_label("Track gain")
                    });
                ui.ctx()
                    .accesskit_node_builder(pan_field.response.id, |node| {
                        node.set_label("Track pan")
                    });
                if gain_field.changed {
                    self.mixer.gain_draft = Some(gain);
                }
                if pan_field.changed {
                    self.mixer.pan_draft = Some(pan);
                }
                let explicit_commit = |response: &egui::Response| {
                    ui.input(|input| {
                        input
                            .accesskit_action_requests(
                                response.id,
                                egui::accesskit::Action::SetValue,
                            )
                            .next()
                            .is_some()
                    }) || (enter && response.has_focus())
                };
                let gain_focused = gain_field.response.has_focus();
                let pan_focused = pan_field.response.has_focus();
                let commit_gain = !locked
                    && !gain_field.cancelled
                    && self.mixer.gain_draft.is_some()
                    && (gain_field.committed
                        || explicit_commit(&gain_field.response)
                        || (self.mixer.gain_focused && !gain_focused));
                let commit_pan = !locked
                    && !pan_field.cancelled
                    && self.mixer.pan_draft.is_some()
                    && (pan_field.committed
                        || explicit_commit(&pan_field.response)
                        || (self.mixer.pan_focused && !pan_focused));
                self.mixer.gain_focused = gain_focused;
                self.mixer.pan_focused = pan_focused;
                if gain_field.cancelled || commit_gain {
                    self.mixer.gain_draft = None;
                }
                if pan_field.cancelled || commit_pan {
                    self.mixer.pan_draft = None;
                }
                for (slot, _) in &selected {
                    if commit_gain {
                        self.send(Command::TrackGain {
                            track: *slot as u8,
                            value: gain / 100.0,
                        });
                    }
                    if commit_pan {
                        self.send(Command::TrackPan {
                            track: *slot as u8,
                            value: (pan / 100.0 + 1.0) * 0.5,
                        });
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    let mute = fields::action(
                        ui,
                        if muted {
                            "Unmute selected tracks"
                        } else {
                            "Mute selected tracks"
                        },
                        !locked,
                    )
                    .clicked();
                    let solo = fields::action(
                        ui,
                        if soloed {
                            "Unsolo selected tracks"
                        } else {
                            "Solo selected tracks"
                        },
                        !locked,
                    )
                    .clicked();
                    for (slot, _) in &selected {
                        if mute && self.snap.tracks[*slot].mute == muted {
                            self.send(Command::Mute { track: *slot as u8 });
                        }
                        if solo && self.snap.tracks[*slot].solo == soloed {
                            self.send(Command::Solo { track: *slot as u8 });
                        }
                    }
                });
            },
        );
    }
}

#[cfg(test)]
mod tests;
