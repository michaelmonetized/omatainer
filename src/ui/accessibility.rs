//! Semantics and alternate input for the custom-painted performance surface.
//! eframe publishes these AccessKit nodes through Linux AT-SPI. Actions use
//! the same command handlers as pointer input, never a second engine model.
use super::*;
use egui::accesskit::{Action, ActionData, CustomAction, Role, Toggled};

const PREFIX: &str = "omatainer-accessible-scope";
const NUMBER: &str = "omatainer-accessible-number";
const RESULT: &str = "omatainer-accessible-number-result";
const RESTORE: &str = "omatainer-accessible-return-focus";
const ACTION_TARGET: &str = "omatainer-accessible-action-target";
const ACTION_RESULT: &str = "omatainer-accessible-action-result";

#[derive(Clone)]
struct ActionTarget {
    owner: egui::Id,
    label: String,
    actions: Vec<String>,
    seen: u64,
}

/// Linux's current AccessKit adapter exports Click but not custom actions.
/// This ordinary menu button is the native AT-SPI route to all alternatives.
pub(super) fn action_button(ui: &mut Ui) {
    let target = ui.data(|data| data.get_temp::<ActionTarget>(egui::Id::new(ACTION_TARGET)));
    let Some(target) = target else {
        ui.add_enabled(false, egui::Button::new("Control actions: focus a control")).help(ui, HelpControl::Actions);
        return;
    };
    let mut visible = target.label.chars().take(48).collect::<String>();
    if visible.len() < target.label.len() {
        visible.push('…');
    }
    let menu = ui.menu_button(format!("Actions for {visible}"), |ui| {
        keyboard::block_for_dialog(ui.ctx());
        for (index, label) in target.actions.iter().enumerate() {
            if ui.button(label).help(ui, HelpControl::Actions).clicked() {
                ui.data_mut(|data| {
                    data.insert_temp(egui::Id::new(ACTION_RESULT), (target.owner, index))
                });
                ui.close();
            }
        }
    });
    help::annotate(ui, &menu.response, HelpControl::Actions);
    menu.response.on_hover_text(&target.label).widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Actions for {}", target.label),
        )
    });
}

pub(super) fn begin_frame(ctx: &egui::Context) {
    // Native sliders process AccessKit before our shared value handler. Filter
    // invalid numbers first so their internal edit cannot reach a command.
    ctx.input_mut(|input| input.events.retain(|event| !matches!(event,
        egui::Event::AccessKitActionRequest(request)
            if request.action == Action::SetValue
                && matches!(request.data, Some(ActionData::NumericValue(value)) if !value.is_finite())
    )));
    // egui keeps the previous modal layer for one pass after closing. Restore
    // only once it has retired, before handling this frame's control keys.
    if ctx.memory(|memory| memory.top_modal_layer().is_none()) {
        let restore = ctx.data_mut(|data| {
            let key = egui::Id::new(RESTORE);
            let owner = data.get_temp::<egui::Id>(key);
            data.remove::<egui::Id>(key);
            owner
        });
        if let Some(owner) = restore {
            ctx.memory_mut(|memory| memory.request_focus(owner));
        }
    }
}

pub(super) fn scrollbars<R>(ui: &Ui, label: &str, area: &egui::scroll_area::ScrollAreaOutput<R>) {
    let mut state = area.state;
    if let Some(response) = ui.ctx().read_response(area.id.with("area")) {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, response.enabled(), label)
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(Role::ScrollView);
            node.set_value(format!("Horizontal {:.0}, vertical {:.0} pixels", state.offset.x, state.offset.y));
            node.set_description("Arrow keys scroll; Page Up/Down scroll a page; Home/End reach the vertical limits. Named scrollbars also support direct values.");
        });
        // The viewport overlaps its rows; keep their rich tooltips authoritative.
        help::describe(ui, &response, HelpControl::Scroll);
        focus(ui, &response);
        if response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let previous = state.offset;
            ui.input_mut(|input| {
                if input.modifiers != egui::Modifiers::NONE {
                    return;
                }
                for (key, axis, delta) in [
                    (Key::ArrowLeft, 0, -32.0),
                    (Key::ArrowRight, 0, 32.0),
                    (Key::ArrowUp, 1, -32.0),
                    (Key::ArrowDown, 1, 32.0),
                    (Key::PageUp, 1, -area.inner_rect.height()),
                    (Key::PageDown, 1, area.inner_rect.height()),
                ] {
                    let count = input.num_presses(key);
                    if count > 0 {
                        state.offset[axis] += delta * count as f32;
                        input.consume_key(egui::Modifiers::NONE, key);
                    }
                }
                if input.consume_key(egui::Modifiers::NONE, Key::Home) {
                    state.offset.y = 0.0;
                }
                if input.consume_key(egui::Modifiers::NONE, Key::End) {
                    state.offset.y = f32::MAX;
                }
            });
            state.offset = state
                .offset
                .max(Vec2::ZERO)
                .min((area.content_size - area.inner_rect.size()).max(Vec2::ZERO));
            if state.offset != previous {
                state.store(ui.ctx(), area.id);
                ui.ctx().request_repaint();
            }
        }
    }
    for axis in 0..2usize {
        let id = area.id.with(axis);
        let Some(response) = ui.ctx().read_response(id) else {
            continue;
        };
        let maximum = (area.content_size[axis] - area.inner_rect.size()[axis]).max(0.0);
        let label = format!(
            "{label}: {} scroll",
            if axis == 0 { "Horizontal" } else { "Vertical" }
        );
        if let Some(value) = numeric(
            ui,
            &response,
            &label,
            state.offset[axis],
            0.0,
            maximum,
            32.0,
            " px",
        ) {
            state.offset[axis] = value;
            state.store(ui.ctx(), area.id);
            ui.ctx().request_repaint();
        }
        ui.ctx()
            .accesskit_node_builder(id, |node| node.set_role(Role::ScrollBar));
        help::annotate(ui, &response, HelpControl::Scroll);
    }
}

pub(super) fn finish_frame(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let key = egui::Id::new(ACTION_TARGET);
        if data
            .get_temp::<ActionTarget>(key)
            .is_some_and(|target| target.seen != pass)
        {
            data.remove::<ActionTarget>(key);
            data.remove::<(egui::Id, usize)>(egui::Id::new(ACTION_RESULT));
        }
    });
}

pub(super) fn scope<R>(ui: &mut Ui, name: &str, draw: impl FnOnce(&mut Ui) -> R) -> R {
    let key = egui::Id::new(PREFIX);
    let previous = ui.data_mut(|data| data.get_temp::<String>(key));
    ui.data_mut(|data| data.insert_temp(key, name.to_owned()));
    let result = group(ui, name, draw);
    ui.data_mut(|data| match previous {
        Some(value) => data.insert_temp(key, value),
        None => {
            data.remove::<String>(key);
        }
    });
    result
}

/// Preserve context for native slider value fields and combo-box entries too.
pub(super) fn group<R>(ui: &mut Ui, label: &str, draw: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.id().with(("accessible-group", label));
    let ctx = ui.ctx().clone();
    ctx.accesskit_node_builder(id, |node| {
        node.set_role(Role::Group);
        node.set_label(label);
    });
    ctx.with_accessibility_parent(id, || draw(ui))
}

fn name(ui: &Ui, label: &str) -> String {
    ui.data(|data| data.get_temp::<String>(egui::Id::new(PREFIX)))
        .map(|prefix| format!("{prefix}: {label}"))
        .unwrap_or_else(|| label.to_owned())
}

pub(super) fn focus(ui: &Ui, response: &egui::Response) {
    if response.gained_focus() {
        response.scroll_to_me(Some(Align::Center));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            3.0,
            Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Outside,
        );
        // Space activates the focused control; it cannot also toggle transport.
        if ui.input(|input| input.key_pressed(Key::Space) || input.key_pressed(Key::Enter)) {
            keyboard::block_for_dialog(ui.ctx());
        }
    }
}

pub(super) fn button(ui: &Ui, response: &egui::Response, label: &str, state: Option<bool>) {
    let label = name(ui, label);
    ui.data_mut(|data| data.insert_temp(response.id.with("accessible-name"), label.clone()));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &label)
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Button);
        if let Some(on) = state {
            node.set_toggled(if on { Toggled::True } else { Toggled::False });
        } else {
            node.clear_toggled();
        }
    });
    focus(ui, response);
}

pub(super) fn status(ui: &Ui, response: &egui::Response, status: &str) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        let description = node
            .description()
            .map(|help| format!("{status}. {help}"))
            .unwrap_or_else(|| status.to_owned());
        node.set_description(description);
    });
}

/// Named alternatives cover pointer modifiers/right-click operations. They
/// are AccessKit custom actions and a keyboard menu (Shift+F10). Linux
/// also exposes them through the ordinary contextual action button.
pub(super) fn actions(ui: &Ui, response: &egui::Response, labels: &[&str]) -> Option<usize> {
    if labels.is_empty() {
        return None;
    }
    let key = egui::Id::new(ACTION_TARGET);
    let selected = ui
        .data(|data| data.get_temp::<ActionTarget>(key))
        .is_some_and(|target| target.owner == response.id);
    if response.enabled() && (response.has_focus() || selected) {
        let label = ui
            .data(|data| data.get_temp::<String>(response.id.with("accessible-name")))
            .unwrap_or_else(|| "focused control".into());
        let seen = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|data| {
            data.insert_temp(
                key,
                ActionTarget {
                    owner: response.id,
                    label,
                    actions: labels.iter().map(|label| (*label).to_owned()).collect(),
                    seen,
                },
            )
        });
    }
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.add_action(Action::CustomAction);
        node.add_action(Action::ShowContextMenu);
        node.set_custom_actions(
            labels
                .iter()
                .enumerate()
                .map(|(id, label)| CustomAction {
                    id: id as i32,
                    description: (*label).into(),
                })
                .collect::<Vec<_>>(),
        );
        let description = node
            .description()
            .map(|text| format!("{text} Shift+F10 opens all actions."))
            .unwrap_or_else(|| "Shift+F10 opens all actions.".into());
        node.set_description(description);
    });
    if !response.enabled() {
        return None;
    }
    let menu_action = ui.data_mut(|data| {
        let key = egui::Id::new(ACTION_RESULT);
        data.get_temp::<(egui::Id, usize)>(key)
            .and_then(|(owner, index)| {
                if owner == response.id {
                    data.remove::<(egui::Id, usize)>(key);
                    (index < labels.len()).then_some(index)
                } else {
                    None
                }
            })
    });
    if menu_action.is_some() {
        return menu_action;
    }
    let requested = ui.input(|input| {
        input
            .accesskit_action_requests(response.id, Action::CustomAction)
            .find_map(|request| match request.data {
                Some(ActionData::CustomAction(id)) if id >= 0 && (id as usize) < labels.len() => {
                    Some(id as usize)
                }
                _ => None,
            })
    });
    if requested.is_some() {
        return requested;
    }
    let open = (response.has_focus()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, Key::F10)))
        || ui.input(|input| {
            input.num_accesskit_action_requests(response.id, Action::ShowContextMenu) > 0
        });
    let mut picked = None;
    egui::Popup::from_response(response)
        .id(response.id.with("actions"))
        .kind(egui::PopupKind::Menu)
        .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
        .show(|ui| {
            keyboard::block_for_dialog(ui.ctx());
            for (index, label) in labels.iter().enumerate() {
                if ui.button(*label).help(ui, HelpControl::Actions).clicked() {
                    picked = Some(index);
                    ui.close();
                }
            }
        });
    picked
}

#[derive(Clone)]
struct NumericEdit {
    owner: egui::Id,
    label: String,
    text: String,
    min: f32,
    max: f32,
    error: bool,
    focus: bool,
}

/// Values are in the displayed units supplied by the caller. Invalid AT
/// values are ignored and finite values clamp exactly like pointer edits.
pub(super) fn numeric(
    ui: &Ui,
    response: &egui::Response,
    label: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &str,
) -> Option<f32> {
    let label = name(ui, label);
    ui.data_mut(|data| data.insert_temp(response.id.with("accessible-name"), label.clone()));
    // Standard Slider responses span the bar, value field and label. Preserve
    // the existing node bounds and text-editor role instead of using that union.
    ui.ctx().accesskit_node_builder(response.id, |node| {
        if !matches!(node.role(), Role::TextInput | Role::SpinButton) {
            node.set_role(Role::Slider);
        }
        node.set_label(label.as_str());
        node.set_numeric_value(value as f64);
        node.set_min_numeric_value(min as f64);
        node.set_max_numeric_value(max as f64);
        node.set_numeric_value_step(step as f64);
        if node.role() != Role::TextInput {
            node.set_value(format!("{value:.2}{unit}"));
        }
        node.set_description(
            "Arrow keys adjust; Shift for fine adjustment; Home/End for limits; F2 enters a value.",
        );
        if response.enabled() {
            node.add_action(Action::SetValue);
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
    });
    focus(ui, response);
    if !response.enabled() {
        return None;
    }
    let mut result = ui.data_mut(|data| {
        let key = egui::Id::new(RESULT);
        data.get_temp::<(egui::Id, f32)>(key)
            .and_then(|(owner, value)| {
                if owner == response.id {
                    data.remove::<(egui::Id, f32)>(key);
                    Some(value)
                } else {
                    None
                }
            })
    });
    if response.has_focus() && !keyboard::text_is_focused(ui.ctx()) {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        ui.input_mut(|input| {
            let mods = input.modifiers;
            if mods.ctrl || mods.alt || mods.command {
                return;
            }
            let delta = (input.num_presses(Key::ArrowRight) + input.num_presses(Key::ArrowUp))
                as f32
                - (input.num_presses(Key::ArrowLeft) + input.num_presses(Key::ArrowDown)) as f32;
            for key in [
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowLeft,
                Key::ArrowDown,
            ] {
                input.consume_key(mods, key);
            }
            if delta != 0.0 {
                result = Some(value + delta * step * if mods.shift { 0.1 } else { 1.0 });
            }
            if input.consume_key(mods, Key::Home) {
                result = Some(min);
            }
            if input.consume_key(mods, Key::End) {
                result = Some(max);
            }
        });
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::F2)) {
            ui.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(NUMBER),
                    NumericEdit {
                        owner: response.id,
                        label: format!("{label} ({unit})"),
                        text: value.to_string(),
                        min,
                        max,
                        error: false,
                        focus: true,
                    },
                )
            });
        }
    }
    ui.input(|input| {
        let increment = input.num_accesskit_action_requests(response.id, Action::Increment) as f32;
        let decrement = input.num_accesskit_action_requests(response.id, Action::Decrement) as f32;
        if increment != decrement {
            result = Some(value + (increment - decrement) * step);
        }
        for request in input.accesskit_action_requests(response.id, Action::SetValue) {
            if let Some(ActionData::NumericValue(value)) = request.data {
                if value.is_finite() {
                    result = Some(value.clamp(min as f64, max as f64) as f32);
                }
            }
        }
    });
    result
        .filter(|next| next.is_finite())
        .map(|next| next.clamp(min, max))
        .filter(|next| *next != value)
}

pub(super) fn cancel_editor(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<NumericEdit>(egui::Id::new(NUMBER));
        data.remove::<(egui::Id, f32)>(egui::Id::new(RESULT));
        data.remove::<egui::Id>(egui::Id::new(RESTORE));
        data.remove::<ActionTarget>(egui::Id::new(ACTION_TARGET));
        data.remove::<(egui::Id, usize)>(egui::Id::new(ACTION_RESULT));
    });
}

pub(super) fn numeric_editor(ctx: &egui::Context) {
    let key = egui::Id::new(NUMBER);
    let Some(mut edit) = ctx.data(|data| data.get_temp::<NumericEdit>(key)) else {
        return;
    };
    keyboard::block_for_dialog(ctx);
    let mut done = false;
    egui::Modal::new(key).show(ctx, |ui| {
        ui.heading(&edit.label);
        let field = ui.add(egui::TextEdit::singleline(&mut edit.text).id_salt("numeric-value"));
        field.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &edit.label)
        });
        help::annotate(ui, &field, HelpControl::NumericEditor);
        if edit.focus {
            field.request_focus();
            edit.focus = false;
        }
        ui.label(format!("Range {} to {}", edit.min, edit.max));
        if edit.error {
            ui.label("Enter a finite number in the displayed range.");
        }
        ui.horizontal(|ui| {
            let cancel =
                ui.button("Cancel").help(ui, HelpControl::NumericEditor).clicked() || ui.input(|input| input.key_pressed(Key::Escape));
            let apply =
                ui.button("Apply").help(ui, HelpControl::NumericEditor).clicked() || ui.input(|input| input.key_pressed(Key::Enter));
            if cancel {
                done = true;
            } else if apply {
                if let Ok(value) = edit.text.parse::<f32>() {
                    if value.is_finite() && (edit.min..=edit.max).contains(&value) {
                        ctx.data_mut(|data| {
                            data.insert_temp(egui::Id::new(RESULT), (edit.owner, value))
                        });
                        done = true;
                    } else {
                        edit.error = true;
                    }
                } else {
                    edit.error = true;
                }
            }
        });
    });
    ctx.data_mut(|data| {
        if done {
            data.remove::<NumericEdit>(key);
        } else {
            data.insert_temp(key, edit.clone());
        }
    });
    if done {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(RESTORE), edit.owner));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests;
