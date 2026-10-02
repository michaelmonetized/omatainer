use super::*;

#[derive(Clone, Copy)]
pub(super) struct ClipGainEdit {
    pub track: u8,
    pub scene: u16,
    pub value: f32,
    pub target: Option<(crate::engine::session::Reference,crate::engine::session::Reference)>,
}

impl App {
    pub(super) fn clip_gain_editor(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.clip_gain_edit else {
            return;
        };
        if let Some((track,scene))=edit.target {
            if self.snap.session.as_ref().is_none_or(|layout| !layout.resolves(crate::engine::session::Axis::Track,edit.track as usize,track) || !layout.resolves(crate::engine::session::Axis::Scene,edit.scene as usize,scene)) {
                self.clip_gain_edit=None;self.status="Clip gain target was deleted or replaced; reopen its gain control.".into();return;
            }
        }
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
                let original_gain = edit.value;
                let original = original_gain * 100.0;
                let mut percent = original;
                let response = ui.add(
                    egui::Slider::new(&mut percent, 0.0..=150.0).text("clip gain").suffix("%").max_decimals(1));
                let alternate = accessibility::numeric(ui, &response, &format!("Clip track {} scene {}: Gain", edit.track + 1, edit.scene + 1), original, 0.0, 150.0, 1.0, "%");
                help::annotate(ui, &response, HelpControl::ClipGain);
                if let Some(value) = alternate { percent = value; }
                edit.value = percent / 100.0;
                // Display rounding can change 52.999996% to 53% on an idle
                // frame while converting back to the identical stored gain.
                // Only an actual normalized edit belongs in renderer history.
                changed |= (response.changed() || alternate.is_some()) && edit.value != original_gain;
                ui.horizontal(|ui| {
                    let zero = ui.button("zero");
                    accessibility::button(ui, &zero, "Set clip gain to zero", None);
                    help::annotate(ui, &zero, HelpControl::ClipGain);
                    if zero.clicked() {
                        edit.value = 0.0;
                        changed = true;
                    }
                    let unity = ui.button("unity");
                    accessibility::button(ui, &unity, "Set clip gain to unity", None);
                    help::annotate(ui, &unity, HelpControl::ClipGain);
                    if unity.clicked() {
                        edit.value = 1.0;
                        changed = true;
                    }
                });
            });
        if changed {
            let command=Command::ClipGain {track:edit.track,scene:edit.scene,value:edit.value};
            let command=match edit.target { Some((track,scene))=>Command::SessionControl(crate::engine::session::Scoped {track:Some((edit.track as usize,track)),scene:Some((edit.scene as usize,scene)),command:Box::new(command)}),None=>command };
            if !self.submit(command) {return;}
        }
        self.clip_gain_edit = open.then_some(edit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{label_center, Fixture};

    #[test]
    fn open_gain_editor_keeps_reordered_identity_and_closes_on_deletion() {
        use crate::engine::session::{Action, Axis, Request};
        let mut gui = crate::ui::piano_roll::tests::Gui::new();
        let track = gui.rt.session.reference(Axis::Track, 2).unwrap();
        let scene = gui.rt.session.reference(Axis::Scene, 7).unwrap();
        gui.app.clip_gain_edit = Some(ClipGainEdit {track:2,scene:7,value:1.0,target:Some((track,scene))});
        for action in [Action::Move {axis:Axis::Track,id:track.id,position:0}, Action::Delete {axis:Axis::Track,id:track.id}] {
            let deleting = matches!(action, Action::Delete {..});
            let (request, ack) = Request::metadata(&gui.rt.session, gui.app.engine.undo.checkpoint().epoch, action).unwrap();
            gui.app.engine.send(Command::SessionEdit(request)).unwrap();
            for _ in 0..4 {gui.frame(vec![]);}
            assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
            if deleting {
                assert!(gui.app.clip_gain_edit.is_none());
                assert!(gui.app.status.contains("deleted or replaced"));
            } else {
                assert_eq!(gui.app.clip_gain_edit.unwrap().target, Some((track,scene)));
                assert_eq!(gui.rt.session.track_order[0], 2);
            }
        }
    }

    #[test]
    fn percent_display_rounding_does_not_add_idle_history_and_one_undo_restores_gain() {
        use egui::accesskit::{Action, ActionData, ActionRequest};
        let mut fixture = Fixture::new(64);
        fixture.app.clip_gain_edit = Some(ClipGainEdit {track:0,scene:0,value:1.0,target:None});
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut time = 0.0;
        let mut run = |fixture: &mut Fixture, events| {
            fixture.rt.process(&mut [0.0; 128]);
            fixture.rt.publish_for_test();
            time += 0.02;
            ctx.run(egui::RawInput {time:Some(time), screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(1600.0,1200.0))), events, ..Default::default()}, |ctx| fixture.app.update_frame(ctx))
        };
        run(&mut fixture, vec![]);
        let output = run(&mut fixture, vec![]);
        let id = output.platform_output.accesskit_update.unwrap().nodes.iter()
            .find(|(_, node)|node.label()==Some("Clip track 1 scene 1: Gain")).unwrap().0;
        let before = fixture.app.engine.undo.view().cursor;
        run(&mut fixture, vec![egui::Event::AccessKitActionRequest(ActionRequest {action:Action::SetValue,target:id,data:Some(ActionData::NumericValue(53.0))})]);
        for _ in 0..20 {run(&mut fixture, vec![]);}
        assert_eq!(fixture.rt.tracks[0].clips[0].gain, 0.53);
        assert_eq!(fixture.app.engine.undo.view().cursor, before + 1, "one edit must not grow history on idle frames");
        fixture.app.history_action(false);
        for _ in 0..4 {run(&mut fixture, vec![]);}
        assert_eq!(fixture.rt.tracks[0].clips[0].gain, 1.0);
        assert_eq!(fixture.app.engine.undo.view().cursor, before);
        fixture.app.history_action(true);
        for _ in 0..20 {run(&mut fixture, vec![]);}
        assert_eq!(fixture.rt.tracks[0].clips[0].gain, 0.53);
        assert_eq!(fixture.app.engine.undo.view().cursor, before + 1);
    }

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
