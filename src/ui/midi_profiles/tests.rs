use super::*;
use crate::engine::midi::{
    catalog::{identity::Device, runtime::Registry},
    connection_test_support as backend, InputPolicy,
};
use std::time::{Duration, Instant};
fn frame(
    ctx: &egui::Context,
    fixture: &mut super::super::test_support::Fixture,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1400.0, 1900.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| fixture.app.controller_profiles_ui(ui, ctx));
        },
    )
}
fn click(
    ctx: &egui::Context,
    fixture: &mut super::super::test_support::Fixture,
    time: f64,
    pos: Pos2,
) {
    for (i, pressed) in [true, false].into_iter().enumerate() {
        frame(
            ctx,
            fixture,
            time + i as f64 * 0.02,
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
}
#[test]
fn native_widgets_arm_finish_and_record_a_bounded_check_without_starting_transport() {
    let directory = std::env::temp_dir().join(format!(
        "controller-ui-{}-{}",
        std::process::id(),
        crate::engine::midi::next_source_id()
    ));
    let registry = Registry::start(directory.clone()).unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while !registry.loaded() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut fixture = super::super::test_support::Fixture::new(80);
    let device = Device {
        vendor: 0x09e8,
        product: 0x0029,
        release: 0x0100,
        serial: None,
        topology: "test-ui-apc".into(),
        port: 0,
        connection: "fixture-connection-1".into(),
    };
    registry.refresh(
        vec![(
            "28:0".into(),
            "APC owned fixture".into(),
            Some(device.clone()),
        )],
        vec![("28:0".into(), Some(device))],
        &fixture.app.engine.snapshot(),
    );
    let control = backend::install_profiled(
        &mut fixture.app.engine,
        InputPolicy::All,
        Some(registry.clone()),
    );
    control.discover(&[("28:0", "APC owned fixture")]);
    let input = control.connect("28:0", Ok(()));
    backend::until(|| !fixture.app.engine.midi.connections_busy());
    fixture.app.midi_learn.profiles.control = "Master fader".into();
    let ctx = egui::Context::default();
    for i in 0..3 {
        frame(&ctx, &mut fixture, i as f64 * 0.02, vec![]);
    }
    let output = frame(&ctx, &mut fixture, 0.1, vec![]);
    let pos = super::super::test_support::label_center(
        &output,
        "Controller profiles and connection check",
    );
    click(&ctx, &mut fixture, 0.2, pos);
    let output = frame(&ctx, &mut fixture, 0.3, vec![]);
    let pos = super::super::test_support::label_center(
        &output,
        "APC owned fixture · USB 09e8:0029 · test-ui-apc",
    );
    click(&ctx, &mut fixture, 0.4, pos);
    let output = frame(&ctx, &mut fixture, 0.5, vec![]);
    let pos = super::super::test_support::label_center(&output, "Capture this control");
    click(&ctx, &mut fixture, 0.6, pos);
    assert!(registry.view().guide.as_ref().unwrap().capturing);
    input.push(&[0xb0, 14, 30]);
    backend::until(|| registry.view().guide.as_ref().unwrap().input_received);
    fixture.rt.process(&mut [0.0; 128]);
    assert!(!fixture.rt.playing);
    assert_eq!(fixture.rt.master, fixture.app.snap.master);
    let output = frame(&ctx, &mut fixture, 0.7, vec![]);
    let pos = super::super::test_support::label_center(&output, "Finish input capture");
    click(&ctx, &mut fixture, 0.8, pos);
    assert!(!registry.view().guide.as_ref().unwrap().capturing);
    fixture.app.midi_learn.profiles.application =
        "Master fader moved after normal dispatch resumed".into();
    fixture.app.midi_learn.profiles.physical = "No motor or LED sweep was initiated".into();
    let output = frame(&ctx, &mut fixture, 0.9, vec![]);
    let pos = super::super::test_support::label_center(&output, "Record observations");
    click(&ctx, &mut fixture, 1.0, pos);
    assert!(registry
        .view()
        .guide
        .as_ref()
        .unwrap()
        .physical_observation
        .is_some());
    fixture.app.snap.playing = true;
    let output = frame(&ctx, &mut fixture, 1.1, vec![]);
    let pos = super::super::test_support::label_center(&output, "Capture this control");
    click(&ctx, &mut fixture, 1.2, pos);
    assert!(!registry.view().guide.as_ref().unwrap().capturing);
    drop(input);
    drop(control);
    drop(fixture);
    drop(registry);
    let _ = std::fs::remove_dir_all(directory);
}
