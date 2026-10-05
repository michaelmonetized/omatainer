use super::{
    test_support::{label_center, Fixture},
    *,
};
use crate::engine::midi::{
    connection_test_support::{self as backend, Attempt, Reply},
    Retry,
};
use std::time::Duration;

fn frame(
    ctx: &egui::Context,
    fixture: &mut Fixture,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 1000.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| fixture.app.update_frame(ctx),
    );
    fixture.rt.process(&mut [0.0; 128]);
    output
}

#[test]
fn actual_midi_panel_reports_failure_retries_without_stalling_and_keeps_keyboard_working() {
    let mut fixture = Fixture::new(80);
    let control = backend::install(&mut fixture.app.engine);
    control.discover(&[("1", "MIDI keyboard")]);
    control.connect("1", Err("fixture port access denied"));
    backend::until(|| !fixture.app.engine.midi.connections_busy());
    fixture.app.midi_open = true;
    let ctx = egui::Context::default();
    for i in 0..3 {
        frame(&ctx, &mut fixture, i as f64 * 0.05, vec![]);
    }
    let output = frame(&ctx, &mut fixture, 0.2, vec![]);
    let failure = fixture.app.engine.snapshot().midi[1].clone();
    assert!(failure.contains("[failed]") && failure.contains("fixture port access denied"));
    label_center(&output, &failure); // Failure is visible without opening the log.
    label_center(&output, "[connected] keyboard + mouse · built-in");
    let button = label_center(&output, "Retry / rescan MIDI");
    for (pressed, time) in [(true, 0.25), (false, 0.3)] {
        frame(
            &ctx,
            &mut fixture,
            time,
            vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(matches!(control.next(), Attempt::Discover));
    assert_eq!(
        fixture.app.engine.midi.retry_connections(),
        Retry::AlreadyRunning
    );
    let started = Instant::now();
    for i in 0..24 {
        fixture
            .app
            .engine
            .cmd
            .send(Command::Master(i as f32 / 24.0))
            .unwrap();
        let output = frame(&ctx, &mut fixture, 0.4 + i as f64 * 0.02, vec![]);
        label_center(&output, "Checking MIDI connections…");
        assert_eq!(fixture.rt.master, i as f32 / 24.0);
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "UI waited for held discovery"
    );
    let playing = fixture.rt.playing;
    for (pressed, time) in [(true, 1.0), (false, 1.05)] {
        frame(
            &ctx,
            &mut fixture,
            time,
            vec![egui::Event::Key {
                key: Key::Space,
                physical_key: Some(Key::Space),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert_ne!(
        fixture.rt.playing, playing,
        "keyboard transport fallback was suppressed"
    );
    control
        .replies
        .send(Reply::Ports(Ok(vec![("1".into(), "MIDI keyboard".into())])))
        .unwrap();
    control.connect("1", Ok(()));
    backend::until(|| !fixture.app.engine.midi.connections_busy());
    let output = frame(&ctx, &mut fixture, 1.2, vec![]);
    let connected = fixture.app.engine.snapshot().midi[1].clone();
    assert!(connected.starts_with("[connected]"));
    label_center(&output, &connected);
    assert!(!format!("{:?}", output.shapes).contains("fixture port access denied"));
}
