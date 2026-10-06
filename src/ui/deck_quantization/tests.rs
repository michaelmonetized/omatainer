use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest};

#[test]
fn native_quantization_is_independent_and_shows_renderer_confirmed_pending_onsets() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut fixture = Fixture::new(128);
    let mut time = 0.0;
    let mut frame = |fixture: &mut Fixture, events| {
        time += 0.02;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| fixture.app.update_frame(ctx),
        );
        fixture.rt.process(&mut []);
        fixture.rt.publish_for_test();
        output
    };
    frame(&mut fixture, vec![]);
    frame(&mut fixture, vec![]);
    for label in [
        "Deck A: Quantize",
        "Deck B: Quantize division",
        "0.25 beats",
    ] {
        let nodes = frame(&mut fixture, vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        let target = nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            });
        frame(
            &mut fixture,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target,
                action: Action::Click,
                data: None,
            })],
        );
        frame(&mut fixture, vec![]);
    }
    let snapshot = fixture.app.engine.snapshot();
    assert!(snapshot.decks[0].controls.quantize);
    assert!(!snapshot.decks[1].controls.quantize);
    assert_eq!(snapshot.decks[1].controls.quantize_division, 1);
    assert!(fixture.rt.decks.iter().all(|deck| !deck.playing));
    fixture.rt.apply(Command::DeckSeek { deck: 0, frac: 0.2 });
    fixture.rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 0,
        del: false,
    });
    fixture.rt.apply(Command::DeckPlay { deck: 0 });
    fixture.rt.apply(Command::DeckSeek {
        deck: 0,
        frac: 0.031,
    });
    fixture.rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 0,
        del: false,
    });
    fixture.rt.publish_for_test();
    frame(&mut fixture, vec![]);
    let output = frame(&mut fixture, vec![]);
    assert!(fixture.app.engine.snapshot().decks[0]
        .controls
        .pending
        .is_some());
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.text().starts_with("Queued Cue 1 · beat"))));
}
