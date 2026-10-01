use super::*;

impl App {
    pub(super) fn fx_slot_controls(
        &self,
        ui: &mut Ui,
        t: &Theme,
        slot: usize,
        name: &str,
        on: bool,
        mix: f32,
        parameters: [f32; 4],
    ) {
        let Some(id) = FxId::from_name(name) else {
            ui.label("Unknown effect");
            return;
        };
        let scope = if self.snap.fx_view >= 100 { format!("Scene {} effect {} {name}", self.snap.fx_view - 99, slot + 1) } else { format!("Track {} effect {} {name}", self.snap.fx_view.max(0) + 1, slot + 1) };
        ui.push_id(("fx-slot", self.snap.fx_view, slot, name), |ui| accessibility::group(ui, &scope, |ui| {
            ui.horizontal_wrapped(|ui| {
                let toggle = pill(ui, t, name, on, t.green);
                accessibility::button(ui, &toggle, &format!("{scope}: Enabled"), Some(on));
                help::annotate(ui, &toggle, HelpControl::FxToggle);
                if toggle.clicked() {
                    self.send(Command::FxToggle(slot));
                }
                for &control in id.controls() {
                    let control: crate::engine::fx::Control = control;
                    let normalized = control
                        .parameter
                        .map(|index| parameters[index as usize])
                        .unwrap_or(mix);
                    let original = control.display(normalized);
                    let mut value = original;
                    let label = control.label();
                    let response = ui
                        .push_id(control.parameter, |ui| {
                            ui.add(
                                egui::Slider::new(&mut value, control.min..=control.max)
                                    .clamping(egui::SliderClamping::Edits)
                                    .text(&label)
                                    .show_value(true)
                                    .max_decimals(2),
                            )
                        })
                        .inner;
                    let alternate = accessibility::numeric(ui, &response, &format!("{scope}: {label}"), original, control.min, control.max, (control.max - control.min) / 100.0, "");
                    response.clone().help_detail(ui, HelpControl::FxParameter, control.help);
                    if let Some(next) = alternate { value = next; }
                    if response.changed() || alternate.is_some() {
                        let value = control.normalized(value);
                        // A physical-unit edit can round back to the same
                        // stored normalized value (e.g. 15%).
                        if value == normalized {
                            continue;
                        }
                        if let Some(p) = control.parameter {
                            self.send(Command::FxParam { slot, p, value });
                        } else {
                            self.send(Command::FxMix { slot, value });
                        }
                    }
                }
            });
            if let Some(note) = id.fixed_settings() {
                ui.label(RichText::new(note).small().color(t.fg_dim));
            }
        }));
    }
}

#[cfg(test)]
mod tests;
