use super::*;

#[derive(Clone, Copy)]
pub(super) struct ClipGainEdit {
    pub track: u8,
    pub scene: u8,
    pub value: f32,
}

impl App {
    pub(super) fn clip_gain_editor(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.clip_gain_edit else {
            return;
        };
        let mut open = true;
        let mut changed = false;
        let name = self
            .snap
            .tracks
            .get(edit.track as usize)
            .map(|track| track.name.as_str())
            .unwrap_or("track");
        egui::Window::new(format!("Clip gain · {name} / scene {}", edit.scene + 1))
            .id(egui::Id::new("clip-gain-editor"))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Applies to new clip notes and hits.");
                ui.label("Held notes and release tails keep their original gain.");
                let original = edit.value * 100.0;
                let mut percent = original;
                let response = ui.add(
                    egui::Slider::new(&mut percent, 0.0..=150.0).text("clip gain").suffix("%").max_decimals(1));
                let alternate = accessibility::numeric(ui, &response, &format!("Clip track {} scene {}: Gain", edit.track + 1, edit.scene + 1), original, 0.0, 150.0, 1.0, "%");
                if let Some(value) = alternate { percent = value; }
                edit.value = percent / 100.0;
                changed |= response.changed() || alternate.is_some();
                ui.horizontal(|ui| {
                    let zero = ui.button("zero");
                    accessibility::button(ui, &zero, "Set clip gain to zero", None);
                    if zero.clicked() {
                        edit.value = 0.0;
                        changed = true;
                    }
                    let unity = ui.button("unity");
                    accessibility::button(ui, &unity, "Set clip gain to unity", None);
                    if unity.clicked() {
                        edit.value = 1.0;
                        changed = true;
                    }
                });
            });
        if changed
            && !self.submit(Command::ClipGain {
                track: edit.track,
                scene: edit.scene,
                value: edit.value,
            })
        {
            return;
        }
        self.clip_gain_edit = open.then_some(edit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{label_center, Fixture};

    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        time: f64,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        app.snap = app.engine.snapshot();
        let theme = app.theme.clone();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
                time: Some(time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.sequencer_row(ui, &theme));
                app.clip_gain_editor(ctx);
            },
        )
    }

    fn click(
        ctx: &egui::Context,
        app: &mut App,
        position: Pos2,
        time: f64,
        modifiers: egui::Modifiers,
    ) {
        for (pressed, time) in [(true, time), (false, time + 0.01)] {
            frame(
                ctx,
                app,
                time,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
                modifiers,
            );
        }
    }

    fn publish(fixture: &mut Fixture, expected: f32) {
        fixture.rt.process(&mut [0.0; 2]);
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while fixture.app.engine.snapshot().tracks[1].clips[0].gain != expected {
            fixture.rt.publish();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn alt_click_editor_changes_exact_clip_and_reports_failed_admission() {
        let mut fixture = Fixture::new(32);
        let ctx = egui::Context::default();
        let output = frame(&ctx, &mut fixture.app, 0.0, vec![], Default::default());
        click(
            &ctx,
            &mut fixture.app,
            label_center(&output, "Bassline"),
            0.1,
            egui::Modifiers::ALT,
        );
        assert_eq!(
            fixture.app.engine.cmd.len(),
            0,
            "gain editing must not launch the clip"
        );
        let edit = fixture.app.clip_gain_edit.unwrap();
        assert_eq!((edit.track, edit.scene, edit.value), (1, 0, 0.95));
        frame(&ctx, &mut fixture.app, 0.2, vec![], Default::default());
        let output = frame(&ctx, &mut fixture.app, 0.3, vec![], Default::default());
        label_center(&output, "Applies to new clip notes and hits.");
        click(
            &ctx,
            &mut fixture.app,
            label_center(&output, "zero"),
            0.4,
            Default::default(),
        );
        publish(&mut fixture, 0.0);
        assert_eq!(fixture.rt.tracks[0].clips[0].gain, 1.0);
        assert_eq!(fixture.rt.tracks[1].clips[1].gain, 1.0);
        let output = frame(&ctx, &mut fixture.app, 0.5, vec![], Default::default());
        click(
            &ctx,
            &mut fixture.app,
            label_center(&output, "unity"),
            0.6,
            Default::default(),
        );
        publish(&mut fixture, 1.0);
        let output = frame(&ctx, &mut fixture.app, 0.7, vec![], Default::default());
        while fixture.app.engine.send(Command::Master(0.5)).is_ok() {}
        click(
            &ctx,
            &mut fixture.app,
            label_center(&output, "zero"),
            0.8,
            Default::default(),
        );
        assert!(fixture.app.submission_error.get().is_some());
        assert_eq!(fixture.app.clip_gain_edit.unwrap().value, 1.0);
        fixture.rt.process(&mut [0.0; 2]);
        assert_eq!(fixture.rt.tracks[1].clips[0].gain, 1.0);
    }
}
