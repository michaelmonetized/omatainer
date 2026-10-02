use super::*;
use crate::ui::test_support::Fixture;

fn render(
    app: &mut App,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let theme = app.theme.clone();
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1800.0, 700.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.fx_row(ui, &theme));
        },
    )
}
fn refresh(f: &mut Fixture) {
    f.rt.publish_for_test();
    f.app.snap = f.rt.snap.lock().clone();
}
fn show(f: &mut Fixture, id: FxId) {
    f.rt.fx_view = 2;
    f.rt.tracks[2].fx.slots = vec![crate::engine::fx::FxSlot::new(id, f.rt.sr)];
    refresh(f);
}

#[test]
fn every_visible_fx_slider_exposes_its_name_physical_range_and_value_and_routes_real_edits() {
    let mut f = Fixture::new(256);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut time = 0.0;
    for &id in FxId::all() {
        show(&mut f, id);
        for &control in id.controls() {
            time += 0.1;
            let output = render(&mut f.app, &ctx, time, vec![]);
            assert!(
                f.rt.cmd_rx.try_recv().is_err(),
                "idle display edited {id:?}"
            );
            let label = format!("Track 3 effect 1 {}: {}", id.name(), control.label());
            let tree = output.platform_output.accesskit_update.as_ref().unwrap();
            let node = tree
                .nodes
                .iter()
                .map(|(_, node)| node)
                .find(|node| {
                    node.label() == Some(label.as_str()) && node.min_numeric_value().is_some()
                })
                .unwrap_or_else(|| panic!("missing accessible slider {id:?} {label}"));
            let normalized = control
                .parameter
                .map(|p| f.rt.tracks[2].fx.slots[0].p[p as usize])
                .unwrap_or(f.rt.tracks[2].fx.slots[0].mix);
            assert_eq!(node.min_numeric_value(), Some(control.min as f64));
            assert_eq!(node.max_numeric_value(), Some(control.max as f64));
            assert!(
                (node.numeric_value().unwrap() - control.display(normalized) as f64).abs() < 1e-5
            );
            let bounds = node.bounds().unwrap();
            let pos = Pos2::new(
                (bounds.x0 + (bounds.x1 - bounds.x0) * 0.84) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
            for pressed in [true, false] {
                time += 0.01;
                render(
                    &mut f.app,
                    &ctx,
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
            let command =
                f.rt.cmd_rx
                    .try_recv()
                    .unwrap_or_else(|_| panic!("slider did not submit: {id:?} {label}"));
            let actual = match command {
                Command::FxParam { slot: 0, p, value } if Some(p) == control.parameter => value,
                Command::FxMix { slot: 0, value } if control.parameter.is_none() => value,
                _ => panic!("wrong slider command {id:?} {label}: {command:?}"),
            };
            assert!((actual - normalized).abs() > 0.01);
            f.rt.apply(command);
            while let Ok(command) = f.rt.cmd_rx.try_recv() {
                f.rt.apply(command);
            }
            refresh(&mut f);
        }
        let output = render(&mut f.app, &ctx, time + 0.03, vec![]);
        let sliders = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::Slider)
            .count();
        assert_eq!(
            sliders,
            id.controls().len(),
            "generic/inert slider remains for {id:?}"
        );
    }
}

#[test]
fn fixed_effect_notes_and_scene_arp_availability_are_visible() {
    let mut f = Fixture::new(256);
    let ctx = egui::Context::default();
    for &id in &[FxId::Delay, FxId::Reverb, FxId::Chorus, FxId::Arp] {
        show(&mut f, id);
        let output = render(&mut f.app, &ctx, 0.0, vec![]);
        crate::ui::test_support::label_center(&output, id.fixed_settings().unwrap());
    }
    f.rt.fx_view = crate::engine::session::SCENE_FX_BASE;
    refresh(&mut f);
    let output = render(&mut f.app, &ctx, 1.0, vec![]);
    assert!(!output.shapes.iter().any(
        |shape| matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.text()=="arp")
    ));
    f.rt.apply(Command::FxAdd(7));
    assert!(f.rt.scene_fx[0].slots.is_empty());
}
