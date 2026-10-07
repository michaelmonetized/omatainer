use super::*;
use crate::engine::deck_pads::{Mode, Press, Release};

pub(super) struct Inputs {
    source: u64,
    pub owners: [[[Option<egui::ViewportId>; 5]; 8]; DECKS],
    media: [u64; DECKS],
    epoch: u64,
}
impl Inputs {
    /// Prepare independent native pad ownership before rendering.
    /// Takes no arguments; returns fixed mouse, Space, Enter, assistive and touch owners for every deck pad.
    pub fn new() -> Self {
        Self {
            source: crate::engine::midi::next_source_id(),
            owners: [[[None; 5]; 8]; DECKS],
            media: [0; DECKS],
            epoch: 0,
        }
    }
}

/// Keep every local input route independent of controller MIDI addresses.
/// Takes deck, zero-based fixed slot and input route; returns a stable native raw key.
fn key(deck: u8, pad: u8, route: usize) -> u32 {
    0x4000_0000 | u32::from(deck) * 64 | u32::from(pad) * 8 | route as u32
}

impl App {
    /// Apply one native pad input without sharing another route's release.
    /// Takes exact deck, slot, route, gate, velocity, shift and viewport; records ownership only after admission.
    pub(super) fn set_deck_pad_input(
        &mut self,
        deck: u8,
        pad: u8,
        route: usize,
        on: bool,
        pressure: f32,
        shifted: bool,
        viewport: egui::ViewportId,
    ) {
        if deck >= 2 || pad >= 8 || route >= 5 {
            return;
        }
        let owner = self.deck_pad_inputs.owners[usize::from(deck)][usize::from(pad)][route];
        if on && owner.is_some() || !on && owner != Some(viewport) {
            return;
        }
        let source = self.deck_pad_inputs.source;
        let key = key(deck, pad, route);
        let command = if on {
            Command::DeckPadPress(Press {
                source,
                key,
                deck,
                id: pad + 1,
                mode: None,
                pressure,
                shifted,
            })
        } else {
            Command::DeckPadRelease(Release { source, key })
        };
        if !self.submit(command) {
            return;
        }
        self.deck_pad_inputs.owners[usize::from(deck)][usize::from(pad)][route] =
            on.then_some(viewport);
    }

    /// Release native holds at focus, media, safety and project boundaries.
    /// Takes the current viewport and blocking state; releases original admitted keys even when their widgets disappeared.
    pub(super) fn guard_deck_pad_inputs(&mut self, ctx: &egui::Context, blocked: bool) {
        let epoch = self.engine.cmd.performance().input_epoch();
        let changed = epoch != self.deck_pad_inputs.epoch;
        let focused = ctx.input(|i| i.focused);
        let viewport = ctx.viewport_id();
        for deck in 0..DECKS {
            let media = self.snap.decks[deck].media_key;
            let replaced = media != self.deck_pad_inputs.media[deck];
            for pad in 0..8 {
                for route in 0..5 {
                    let Some(owner) = self.deck_pad_inputs.owners[deck][pad][route] else {
                        continue;
                    };
                    let ended = owner == viewport
                        && (!focused
                            || match route {
                                0 => !ctx.input(|i| i.pointer.primary_down()),
                                1 => !ctx.input(|i| i.key_down(Key::Space)),
                                2 => !ctx.input(|i| i.key_down(Key::Enter)),
                                _ => false,
                            });
                    if blocked || changed || replaced || ended {
                        self.set_deck_pad_input(
                            deck as u8, pad as u8, route, false, 0.0, false, owner,
                        );
                    }
                }
            }
            self.deck_pad_inputs.media[deck] = media;
        }
        self.deck_pad_inputs.epoch = epoch;
    }

    /// Draw a shared per-deck performance pad surface.
    /// Takes current deck snapshot, theme and cell size; submits mode, parameters and captured pad gestures through ordinary admission.
    pub(super) fn deck_pad_grid(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        deck: usize,
        snap: &crate::engine::DeckSnap,
        cell: f32,
    ) {
        let mode = Mode::from_index(snap.controls.pad_mode).unwrap_or(Mode::HotCue);
        ui.push_id(("deck-pad-mode", deck), |ui| {
            let chooser = egui::ComboBox::from_id_salt("mode")
                .selected_text(mode.label())
                .width((cell + 4.0) * 4.0)
                .show_ui(ui, |ui| {
                    for choice in Mode::ALL {
                        let [r, g, b] = choice.color();
                        let response = ui.selectable_label(
                            mode == choice,
                            RichText::new(choice.label()).color(Color32::from_rgb(r, g, b)),
                        );
                        accessibility::button(
                            ui,
                            &response,
                            &format!("Pad mode {}", choice.label()),
                            Some(mode == choice),
                        );
                        if response.clicked() {
                            self.send(Command::DeckControl {
                                source: self.deck_pad_inputs.source,
                                deck: deck as u8,
                                control: crate::engine::deck_controls::Control::PadMode {
                                    mode: choice.index(),
                                },
                            });
                            ui.close();
                        }
                    }
                })
                .response;
            accessibility::button(ui, &chooser, "Pad mode", None);
            accessibility::status(ui, &chooser, mode.label());
            ui.horizontal(|ui| {
                for (up, label) in [(false, "−"), (true, "+")] {
                    let response = ui.small_button(label);
                    accessibility::button(
                        ui,
                        &response,
                        if up {
                            "Pad parameter right"
                        } else {
                            "Pad parameter left"
                        },
                        None,
                    );
                    if response.clicked() {
                        self.send(Command::DeckPadParameter {
                            source: self.deck_pad_inputs.source,
                            deck: deck as u8,
                            up,
                            shifted: ui.input(|i| i.modifiers.shift),
                        });
                    }
                }
                ui.small(parameter(mode, snap, self.snap.sampler_bank));
            });
            for row in 0..2 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                    for column in 0..4 {
                        let pad = row * 4 + column;
                        let (name, color, on) = presentation(mode, deck, pad, snap, &self.snap);
                        let label = if mode == Mode::HotCue {
                            cue_editor::label(pad, snap.cue_styles[pad])
                        } else {
                            format!("{} pad {}: {}", mode.label(), pad + 1, name)
                        };
                        let text = if mode == Mode::HotCue
                            && snap.cue_styles[pad].name.as_str().is_empty()
                        {
                            (pad + 1).to_string()
                        } else {
                            format!("{}\n{}", pad + 1, cue_editor::short_name(&name, 8))
                        };
                        let response = sq_btn(ui, t, &text, on, color, cell);
                        accessibility::button(ui, &response, &label, Some(on));
                        let description = if mode == Mode::HotCue {
                            cue_editor::description(snap, pad)
                        } else {
                            format!("{} · {} · fixed pad ID {}", mode.label(), name, pad + 1)
                        };
                        accessibility::status(ui, &response, &description);
                        help::annotate(ui, &response, HelpControl::HotCue);
                        self.deck_pad_gate(ui, deck as u8, pad as u8, &response, mode);
                    }
                });
            }
        });
    }

    /// Interpret independent native pointer, keyboard and assistive pad gestures.
    /// Takes this visible pad and current mode; captures holds on press and retires them outside the original rectangle.
    fn deck_pad_gate(&mut self, ui: &Ui, deck: u8, pad: u8, response: &egui::Response, mode: Mode) {
        let enabled = response.enabled() && !self.engine.safe_mode() && !self.project.committing();
        let mouse = touch::register(
            ui,
            response,
            touch::Target::DeckPad { deck, pad },
            response.rect,
            enabled,
        );
        let viewport = ui.ctx().viewport_id();
        let focused = ui.input(|i| i.focused);
        let shifted = ui.input(|i| i.modifiers.shift);
        if mouse.released || !mouse.down || !focused || !enabled {
            self.set_deck_pad_input(deck, pad, 0, false, 0.0, false, viewport);
        }
        if mouse.pressed && focused && enabled && mouse.starts_here(response) {
            response.request_focus();
            self.set_deck_pad_input(deck, pad, 0, true, 1.0, shifted, viewport);
            if !mouse.down {
                self.set_deck_pad_input(deck, pad, 0, false, 0.0, false, viewport);
            }
        }
        for (route, key) in [(1, Key::Space), (2, Key::Enter)] {
            let (pressed, down) = ui.input(|i| (i.events.iter().any(|event| matches!(event, egui::Event::Key { key: pressed_key, pressed: true, repeat: false, .. } if *pressed_key == key)), i.key_down(key)));
            if !down || !response.has_focus() || !focused || !enabled {
                self.set_deck_pad_input(deck, pad, route, false, 0.0, false, viewport);
            }
            if pressed
                && response.has_focus()
                && focused
                && enabled
                && !ui.input(|i| i.modifiers.ctrl || i.modifiers.alt || i.modifiers.command)
            {
                self.set_deck_pad_input(deck, pad, route, true, 1.0, shifted, viewport);
            }
        }
        let hotcue_actions = [
            "Set or jump to cue",
            "Delete cue",
            "Edit cue names and colors",
            "Press pad",
            "Release pad",
        ];
        let pad_actions = ["Trigger pad", "Press pad", "Release pad"];
        let action = accessibility::actions(
            ui,
            response,
            if mode == Mode::HotCue {
                &hotcue_actions
            } else {
                &pad_actions
            },
        );
        let click = ui.input(|i| {
            i.num_accesskit_action_requests(response.id, egui::accesskit::Action::Click) > 0
        });
        if enabled && focused {
            if mode == Mode::HotCue && action == Some(2) {
                self.open_cue_editor(usize::from(deck));
            } else if mode == Mode::HotCue && action == Some(1) {
                self.send(Command::DeckHotCue {
                    deck,
                    pad,
                    del: true,
                });
            } else if action == Some(if mode == Mode::HotCue { 3 } else { 1 }) {
                self.set_deck_pad_input(deck, pad, 3, true, 1.0, shifted, viewport);
            } else if action == Some(if mode == Mode::HotCue { 4 } else { 2 }) {
                self.set_deck_pad_input(deck, pad, 3, false, 0.0, false, viewport);
            } else if action == Some(0) || click {
                self.set_deck_pad_input(deck, pad, 3, true, 1.0, shifted, viewport);
                self.set_deck_pad_input(deck, pad, 3, false, 0.0, false, viewport);
            }
        } else {
            self.set_deck_pad_input(deck, pad, 3, false, 0.0, false, viewport);
        }
        if self.deck_pad_inputs.owners[usize::from(deck)][usize::from(pad)]
            .iter()
            .any(Option::is_some)
        {
            active_mark(ui.painter(), response.rect, self.theme.fg);
        }
    }
}

/// Describe the actual parameter changed by the adjacent controls.
/// Takes mode, applied deck state and sampler bank; returns a concise native label including shifted behavior.
fn parameter(mode: Mode, snap: &crate::engine::DeckSnap, bank: usize) -> String {
    match mode {
        Mode::Roll | Mode::AutoLoop => format!(
            "Size ×{} · Shift: move",
            2_f32.powi(i32::from(snap.controls.roll_scale))
        ),
        Mode::Slice => format!(
            "Slice ÷{} · Shift: {} beats",
            1 << snap.controls.slice_quant,
            1 << snap.controls.slice_domain
        ),
        Mode::Sampler | Mode::VelocitySampler => format!("Bank {}", bank + 1),
        Mode::SavedLoop | Mode::ManualLoop => "Length · Shift: move".into(),
        Mode::HotCue => "Move loop".into(),
    }
}

/// Present fixed pad identities with current names, colors and active states.
/// Takes mode, zero-based pad, deck and global snapshot; returns its visible description and applied state.
fn presentation(
    mode: Mode,
    deck: usize,
    pad: usize,
    snap: &crate::engine::DeckSnap,
    global: &crate::engine::Snapshot,
) -> (String, Color32, bool) {
    let [r, g, b] = mode.color();
    let mut color = Color32::from_rgb(r, g, b);
    let (name, on) = match mode {
        Mode::HotCue => {
            if let Some([r, g, b]) = snap.cue_styles[pad].color {
                color = Color32::from_rgb(r, g, b);
            }
            (
                snap.cue_styles[pad].name.as_str().to_owned(),
                snap.hotcues[pad],
            )
        }
        Mode::Roll | Mode::AutoLoop => {
            let beats = 2_f32.powi(pad as i32 - 5 + i32::from(snap.controls.roll_scale));
            let rate = f64::from(snap.source_sample_rate.max(1));
            let start = snap.loop_start / rate;
            let length = snap
                .grid
                .and_then(|grid| grid.advance(start, f64::from(beats)))
                .map_or(
                    f64::from(beats) * 60.0 / f64::from(snap.bpm.max(1.0)) * rate,
                    |end| (end - start) * rate,
                );
            (
                format!("{beats} beats"),
                if mode == Mode::Roll {
                    snap.controls.roll == Some(pad as u8)
                } else {
                    snap.loop_on && (snap.loop_len - length).abs() <= 1.0
                },
            )
        }
        Mode::Slice => (
            format!("Slice {}", pad + 1),
            snap.controls.slice == Some(pad as u8),
        ),
        Mode::Sampler | Mode::VelocitySampler => {
            let slot = deck * 8 + pad;
            let name = global
                .sampler_instances
                .get(global.sampler_bank)
                .and_then(|bank| bank.data.audio[slot].as_ref())
                .map(|sample| sample.name.clone())
                .unwrap_or_else(|| "Empty".into());
            (name, global.surfaces.sampler_playing[slot])
        }
        Mode::SavedLoop => {
            let slot = snap.saved_loops.slots[pad];
            if let Some([r, g, b]) = slot.and_then(|slot| slot.style.color) {
                color = Color32::from_rgb(r, g, b);
            }
            (
                slot.map(|slot| {
                    if slot.style.name.as_str().is_empty() {
                        format!("Loop {}", pad + 1)
                    } else {
                        slot.style.name.as_str().into()
                    }
                })
                .unwrap_or_else(|| "Save loop".into()),
                slot.is_some(),
            )
        }
        Mode::ManualLoop => (
            [
                "Select", "Loop", "Save", "Previous", "In", "Out", "Loop", "Next",
            ][pad]
                .into(),
            matches!(pad, 1 | 6) && snap.loop_on,
        ),
    };
    (name, color, on)
}

#[cfg(test)]
mod tests;
