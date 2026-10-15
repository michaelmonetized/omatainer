//! Keyboard entry points reuse the native editors and their guarded commands.
use super::*;
use std::cell::RefCell;

const OWNER: &str = "keyboard-authoring-owner";
const FORM: &str = "keyboard-authoring-form";

/// Draw existing native controls with a stable native auto-ID seed.
/// Args: parent UI, document/property identity and contents. Returns the native contents' result.
pub(super) fn native<R>(ui: &mut Ui, id: egui::Id, draw: impl FnOnce(&mut Ui) -> R) -> R {
    crate::property_fields::native_scope(ui, id, draw).inner
}

fn context_key(ui: &Ui, salt: &str) -> egui::Id {
    egui::Id::new((ui.ctx().viewport_id(), salt))
}

/// Bind one native form to persisted document targets.
/// Args: UI, owner identity and native form. Returns its result and collects only visible validation failures.
pub(super) fn form<R>(ui: &mut Ui, id: egui::Id, draw: impl FnOnce(&mut Ui) -> R) -> R {
    let owner_key = context_key(ui, OWNER);
    let form_key = context_key(ui, FORM);
    let previous = ui.data_mut(|data| {
        let previous = (
            data.get_temp::<egui::Id>(owner_key),
            data.get_temp::<egui::Id>(form_key),
        );
        data.insert_temp(owner_key, id);
        data.insert_temp(form_key, id);
        data.insert_temp(id.with("invalid"), false);
        previous
    });
    let result = native(ui, id, draw);
    ui.data_mut(|data| {
        for (key, value) in [(owner_key, previous.0), (form_key, previous.1)] {
            if let Some(value) = value {
                data.insert_temp(key, value);
            } else {
                data.remove::<egui::Id>(key);
            }
        }
    });
    result
}

/// Bind properties to a selected saved object without changing their form's admission.
/// Args: UI, saved target identity and contents. Returns the native contents' result.
pub(super) fn target<R>(
    ui: &mut Ui,
    target: impl std::hash::Hash,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    let key = context_key(ui, OWNER);
    let previous = ui.data(|data| data.get_temp::<egui::Id>(key));
    let id = previous.unwrap_or(ui.id()).with(target);
    ui.data_mut(|data| data.insert_temp(key, id));
    let result = draw(ui);
    ui.data_mut(|data| {
        if let Some(previous) = previous {
            data.insert_temp(key, previous);
        } else {
            data.remove::<egui::Id>(key);
        }
    });
    result
}

/// Resolve an application's property identity independently of conditional rows.
/// Args: UI and property salt. Returns an ID scoped to the current document and persisted owner.
pub(super) fn property(ui: &Ui, salt: impl std::hash::Hash) -> egui::Id {
    let key = context_key(ui, OWNER);
    ui.data(|data| data.get_temp::<egui::Id>(key))
        .unwrap_or(ui.id())
        .with(salt)
}

/// Read current native form validation.
/// Args: form UI. Returns true when any rendered property has an invalid draft or domain value.
pub(super) fn invalid(ui: &Ui) -> bool {
    let key = context_key(ui, FORM);
    ui.data(|data| {
        data.get_temp::<egui::Id>(key)
            .is_some_and(|id| data.get_temp::<bool>(id.with("invalid")).unwrap_or(false))
    })
}

/// Associate one visible polite error with the actual native widget.
/// Args: UI, native response and error text. Returns no value; no native-reader announcement is inferred.
pub(super) fn error(ui: &mut Ui, response: &egui::Response, text: &str) {
    let key = context_key(ui, FORM);
    let message = native(ui, response.id.with("validation"), |ui| {
        ui.colored_label(ui.visuals().error_fg_color, text)
    });
    ui.ctx().accesskit_node_builder(message.id, |node| {
        node.set_role(egui::accesskit::Role::Alert);
        node.set_label(text);
        node.set_live(egui::accesskit::Live::Polite);
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_invalid(egui::accesskit::Invalid::True);
        node.clear_numeric_value();
        node.set_error_message(message.id.value().into());
        node.push_described_by(message.id.value().into());
    });
    ui.data_mut(|data| {
        if let Some(form) = data.get_temp::<egui::Id>(key) {
            data.insert_temp(form.with("invalid"), true);
        }
    });
}

/// Keep the native decimal limits without rounding through f32 comparisons.
/// Args: displayed value and native decimal range. Returns precise text with the native minimum padding.
fn number_text(value: f64, decimals: std::ops::RangeInclusive<usize>) -> String {
    let (minimum, maximum) = decimals.into_inner();
    for precision in minimum..maximum {
        let text = format!("{value:.precision$}");
        if text.parse::<f64>().ok() == Some(value) {
            return text;
        }
    }
    format!("{value:.maximum$}")
}

/// Draw the existing precise DragValue with public draft observation.
/// Args: UI, property label, value, native range, step, precision and unit. Returns the actual response and accepted-change decision.
pub(super) fn number(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    unit: &str,
) -> (egui::Response, bool) {
    let property_key = property(ui, label);
    let id = property_key.with(ui.id());
    native(ui, id, |ui| {
        let label_response = ui.label(label);
        let escaping = ui.input(|input| input.key_pressed(Key::Escape));
        let mixed = ui.data(|data| {
            data.get_temp::<bool>(property_key.with("mixed"))
                .unwrap_or(false)
        });
        let before = *value;
        let attempted = RefCell::new(None);
        let parse = |text: &str| {
            text.chars()
                .filter(|character| !character.is_whitespace())
                .map(|character| if character == '−' { '-' } else { character })
                .collect::<String>()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
        };
        let mut drag = egui::DragValue::new(value)
            .range(min..=max)
            .speed(step)
            .max_decimals(decimals)
            .suffix(unit)
            .custom_formatter(number_text)
            .custom_parser(|text| {
                attempted.replace(Some(text.to_owned()));
                parse(text)
            });
        if mixed {
            drag = drag.custom_formatter(|_, _| "Mixed".into());
        }
        let response = ui.add(drag).labelled_by(label_response.id);
        let key = id.with("attempted-draft");
        let cancelled = (response.has_focus() || response.lost_focus()) && escaping;
        ui.data_mut(|data| {
            if let Some(text) = attempted.into_inner() {
                if parse(&text).is_none() && !(mixed && text == "Mixed" && !response.changed()) {
                    data.insert_temp(key, text);
                } else {
                    data.remove::<String>(key);
                }
            } else if response.changed() {
                data.remove::<String>(key);
            }
            if cancelled {
                data.remove::<String>(key);
            }
            data.insert_temp(property_key.with("native-response"), response.id);
        });
        let invalid = ui.data(|data| data.get_temp::<String>(key).is_some()) || !value.is_finite();
        if invalid || cancelled || !response.enabled() {
            *value = before;
        }
        let name = if unit.is_empty() {
            label.to_owned()
        } else {
            format!("{label} ({})", unit.trim())
        };
        if let Some(next) = accessibility::numeric(
            ui,
            &response,
            &name,
            *value as f32,
            min as f32,
            max as f32,
            step as f32,
            unit,
        ) {
            if !invalid && !cancelled && !response.changed() {
                *value = f64::from(next);
            }
        }
        let formatted = number_text(*value, 0..=decimals);
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_numeric_value(*value);
            node.set_min_numeric_value(min);
            node.set_max_numeric_value(max);
            node.set_numeric_value_step(step);
            if node.role() != egui::accesskit::Role::TextInput {
                node.set_value(format!("{formatted}{unit}"));
            }
        });
        if response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                )
            });
        }
        if invalid {
            error(ui, &response, "Enter a finite number");
        }
        let changed = response.enabled()
            && !invalid
            && !cancelled
            && (*value != before || (mixed && response.changed()));
        if mixed && !changed && !invalid {
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_state_description("Mixed");
                node.clear_numeric_value();
                node.set_value("Mixed");
            });
        }
        (response, changed)
    })
}

/// Mark a native numeric property as a mixed selection before drawing it.
/// Args: UI, property label and mixed decision. Returns no value and keeps the underlying selected values intact.
pub(super) fn mixed(ui: &Ui, label: &str, mixed: bool) {
    let id = property(ui, label);
    ui.data_mut(|data| data.insert_temp(id.with("mixed"), mixed));
}

/// Draw an existing editor command as a named stable native button.
/// Args: UI, command label and admission. Returns the actual native response.
pub(super) fn action(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let response = native(ui, property(ui, ("action", label)), |ui| {
        ui.add_enabled(enabled, egui::Button::new(label))
    });
    accessibility::button(ui, &response, label, None);
    response
}

/// Associate a domain failure with an already rendered property.
/// Args: UI, stable property label and failure. Returns no value and keeps a single visible message.
pub(super) fn property_error(ui: &mut Ui, label: &str, text: &str) {
    let id = property(ui, label);
    let native = ui.data(|data| data.get_temp::<egui::Id>(id.with("native-response")));
    if let Some(response) = native.and_then(|id| ui.ctx().read_response(id)) {
        error(ui, &response, text);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Editor {
    Session,
    Clips,
    Notes,
    Song,
    Timing,
    Audio,
    Routing,
    Delivery,
    Panels,
}

impl Editor {
    pub(super) const ALL: [Self; 9] = [
        Self::Session,
        Self::Clips,
        Self::Notes,
        Self::Song,
        Self::Timing,
        Self::Audio,
        Self::Routing,
        Self::Delivery,
        Self::Panels,
    ];

    /// Name an existing editor route for command search.
    /// Args: route. Returns the native command label, without starting playback or optional work.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Session => "Edit tracks and scenes",
            Self::Clips => "Create and edit Session clips",
            Self::Notes => "Create and edit MIDI notes and controller lanes",
            Self::Song => "Create and edit arrangement instances",
            Self::Timing => "Edit tempo and meter automation",
            Self::Audio => "Edit selected audio clip",
            Self::Routing => "Open audio routing",
            Self::Delivery => "Open audio export and recording",
            Self::Panels => "Arrange workspace panels",
        }
    }
}

impl App {
    /// Open command search from an editor without taking keys from native text or modal input.
    /// Args: native context. Returns true when the palette chord was consumed.
    pub(super) fn keyboard_palette_key(&mut self, ctx: &egui::Context) -> bool {
        if self.command_palette.open
            || keyboard::text_is_focused(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
            || egui::Popup::is_any_open(ctx)
            || !self.project.dialog_is_closed()
            || self.project.committing()
            || self.settings.open
            || !ctx.input(|input| input.focused)
        {
            return false;
        }
        let Some(key) = command_palette::chord(self.settings.profile()) else {
            return false;
        };
        let pressed = ctx.input_mut(|input| {
            let modifiers = input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key: pressed,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if *pressed == key
                    && modifiers.ctrl
                    && modifiers.shift
                    && !modifiers.alt
                    && !modifiers.mac_cmd =>
                {
                    Some(*modifiers)
                }
                _ => None,
            });
            modifiers.is_some_and(|modifiers| input.consume_key(modifiers, key))
        });
        if pressed {
            self.command_palette.open(ctx.viewport_id());
            keyboard::block_for_dialog(ctx);
        }
        pressed
    }

    /// Open an existing editor from the ordinary command palette.
    /// Args: requested route. Returns no value; each editor keeps its existing capture, admission and history.
    pub(super) fn open_keyboard_editor(&mut self, editor: Editor) {
        if self.project.committing()
            || !self.project.dialog_is_closed()
            || self.settings.busy()
            || self.engine.safe_mode()
        {
            self.status = "Finish the current project operation before opening an editor.".into();
            return;
        }
        match editor {
            Editor::Session => {
                self.reveal_studio_panel("session");
                self.session_editor.open = true;
            }
            Editor::Clips => {
                self.open_clip_manager(self.snap.selected_track, self.snap.selected_scene)
            }
            Editor::Notes => self.open_piano_roll(),
            Editor::Song => self.open_arrangement(),
            Editor::Timing => self.open_timing(),
            Editor::Audio => self.open_audio_clip(
                self.snap.selected_track as u8,
                self.snap.selected_scene as u16,
            ),
            Editor::Routing => self.audio_routing.open = true,
            Editor::Delivery => self.audio_delivery.open = true,
            Editor::Panels => self.open_panel_keyboard(),
        }
    }
}

#[cfg(test)]
mod tests;
