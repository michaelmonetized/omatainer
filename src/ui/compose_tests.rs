use super::test_support::{label_center, Fixture};
use super::*;
use crate::engine::ComposeTarget;

fn frame(
    ctx: &egui::Context,
    f: &mut Fixture,
    time: f64,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> egui::FullOutput {
    f.rt.process(&mut [0.0; 2]);
    f.rt.publish_for_test();
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 1000.0))),
            time: Some(time),
            events,
            modifiers,
            ..Default::default()
        },
        |ctx| f.app.update_frame(ctx),
    )
}

fn click(ctx: &egui::Context, f: &mut Fixture, pos: Pos2, time: f64, modifiers: egui::Modifiers) {
    for (pressed, t) in [(true, time), (false, time + 0.01)] {
        frame(
            ctx,
            f,
            t,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ],
            modifiers,
        );
    }
}

fn notes(f: &Fixture) -> Vec<usize> {
    f.rt.tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.notes.len()))
        .collect()
}

#[test]
fn gui_shift_arm_stop_restart_pad_does_not_write_changed_selection() {
    let mut f = Fixture::new(64);
    f.rt.tracks[4].clips[3] = crate::engine::Clip::empty();
    f.rt.tracks[4].clips[3].kind = crate::engine::ClipKind::Midi;
    f.rt.tracks[4].clips[3].name = "compose-target".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut f, 0.0, vec![], Default::default());
    let out = frame(&ctx, &mut f, 0.1, vec![], Default::default());
    label_center(&out, "Compose disarmed");
    let target = label_center(&out, "[] compose-target");
    click(
        &ctx,
        &mut f,
        target,
        0.2,
        egui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    let out = frame(&ctx, &mut f, 0.3, vec![], Default::default());
    assert_eq!(
        f.rt.compose_target,
        Some(ComposeTarget { track: 4, scene: 3 })
    );
    let armed = format!("Compose armed: {} / scene 4", f.rt.tracks[4].name);
    label_center(&out, &armed);
    f.app.send(Command::Stop);
    frame(&ctx, &mut f, 0.35, vec![], Default::default());
    f.app.send(Command::TogglePlay);
    let out = frame(&ctx, &mut f, 0.4, vec![], Default::default());
    label_center(&out, "Compose disarmed");
    assert!(f.rt.playing);
    assert_ne!((f.rt.selected_track, f.rt.selected_scene), (4, 3));
    let before = notes(&f);
    // Actual sampler pointer-down/up interaction, not a synthetic note insertion.
    click(
        &ctx,
        &mut f,
        label_center(&out, "16"),
        0.5,
        Default::default(),
    );
    frame(&ctx, &mut f, 0.6, vec![], Default::default());
    assert_eq!(notes(&f), before);
}

#[test]
fn gui_arm_and_disarm_controls_report_renderer_state_and_preserve_target_during_playback() {
    let mut f = Fixture::new(64);
    f.app.send(Command::Select { track: 4, scene: 3 });
    let ctx = egui::Context::default();
    frame(&ctx, &mut f, 0.0, vec![], Default::default());
    let out = frame(&ctx, &mut f, 0.1, vec![], Default::default());
    click(
        &ctx,
        &mut f,
        label_center(&out, "Arm selected cell"),
        0.2,
        Default::default(),
    );
    f.app.send(Command::LaunchScene { scene: 0 });
    let out = frame(&ctx, &mut f, 0.3, vec![], Default::default());
    let armed = format!("Compose armed: {} / scene 4", f.rt.tracks[4].name);
    label_center(&out, &armed);
    let before = f.rt.tracks[4].clips[3].notes.len();
    click(
        &ctx,
        &mut f,
        label_center(&out, "16"),
        0.4,
        Default::default(),
    );
    let out = frame(&ctx, &mut f, 0.5, vec![], Default::default());
    assert_eq!(f.rt.tracks[4].clips[3].notes.len(), before + 1);
    click(
        &ctx,
        &mut f,
        label_center(&out, "Disarm compose"),
        0.6,
        Default::default(),
    );
    let out = frame(&ctx, &mut f, 0.7, vec![], Default::default());
    label_center(&out, "Compose disarmed");
    let before = notes(&f);
    click(
        &ctx,
        &mut f,
        label_center(&out, "16"),
        0.8,
        Default::default(),
    );
    frame(&ctx, &mut f, 0.9, vec![], Default::default());
    assert_eq!(notes(&f), before);
}
