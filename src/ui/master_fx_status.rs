use super::*;

impl App {
    pub(super) fn master_fx_status(&self, ctx: &egui::Context) { self.master_controls(ctx, None); }
    pub(super) fn master_controls(&self, ctx: &egui::Context, parent: Option<&mut Ui>) {
        toolbar(ctx, parent, "master-fx-status", "FX", |ui| {
            if self.engine.safe_mode() {ui.disable();}
            ui.horizontal_wrapped(|ui| {
                ui.label("Master FX");
                for slot in 0..3 {
                    ui.push_id(("legacy-master-fx", slot), |ui| accessibility::group(ui, &format!("Master effect {}", slot + 1), |ui| {
                        let kind = self.snap.fx_kind[slot];
                        let select = ui.button(format!("{}: {}", slot + 1, kind.name()));
                        accessibility::button(ui, &select, &format!("Master effect {}: {}", slot + 1, kind.name()), None);
                        help::annotate(ui, &select, HelpControl::MasterFxSelect);
                        if select.clicked() {
                            self.send(Command::FxSelect { slot: slot as u8 });
                        }
                        let original = self.snap.fx_wet[slot] * 100.0;
                        let mut wet = original;
                        let response = ui.add(egui::Slider::new(&mut wet, 0.0..=100.0).text("Wet").suffix("%"));
                        let alternate = accessibility::numeric(ui, &response, &format!("Master effect {}: Wet", slot + 1), original, 0.0, 100.0, 1.0, "%");
                        help::annotate(ui, &response, HelpControl::MasterFxWet);
                        if let Some(value) = alternate { wet = value; }
                        if response.changed() || alternate.is_some() {
                            self.send(Command::FxWet { slot: slot as u8, value: wet / 100.0 });
                        }
                    }));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FxKind;
    #[test]
    fn selected_types_are_painted_and_selector_click_queues_real_command() {
        let mut f = test_support::Fixture::new(32);
        f.app.snap.fx_kind = [FxKind::Filter, FxKind::Echo, FxKind::Reverb];
        let ctx = egui::Context::default();
        let render = |app: &App, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.master_fx_status(ctx),
            )
        };
        let output = render(&f.app, 1.0, vec![]);
        for label in ["1: Filter", "2: Echo", "3: Reverb"] {
            test_support::label_center(&output, label);
        }
        let pos = test_support::label_center(&output, "2: Echo");
        for (pressed, time) in [(true, 1.1), (false, 1.2)] {
            render(
                &f.app,
                time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(matches!(
            f.rt.cmd_rx.try_recv().unwrap(),
            Command::FxSelect { slot: 1 }
        ));
    }
}
