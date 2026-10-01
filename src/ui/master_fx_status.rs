use super::*;

impl App {
    pub(super) fn master_fx_status(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("master-fx-status").resizable(false).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Master FX");
                for slot in 0..3 {
                    ui.push_id(("legacy-master-fx", slot), |ui| {
                        let kind = self.snap.fx_kind[slot];
                        if ui.button(format!("{}: {}", slot + 1, kind.name()))
                            .on_hover_text("Select Echo → Reverb → Filter. Each slot has independent stereo history; Filter is a 1 kHz low-pass.").clicked() {
                            self.send(Command::FxSelect { slot: slot as u8 });
                        }
                        let mut wet = self.snap.fx_wet[slot];
                        if ui.add(egui::Slider::new(&mut wet, 0.0..=1.0).text("Wet")).changed() {
                            self.send(Command::FxWet { slot: slot as u8, value: wet });
                        }
                    });
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
