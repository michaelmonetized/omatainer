use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

#[test]
fn native_source_transport_loss_deadline_and_confirmed_status_use_actual_accessible_handlers() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let outputs = crate::engine::midi::clock::Shared::default();
    let devices = vec![Device {
        source: 71,
        endpoint: crate::engine::midi::learn::Endpoint {
            name: "Fixture clock".into(),
            id: "clock:1".into(),
        },
        pad_modes: 0,
    }];
    let mut config = Config::default();
    let status = Status::default();
    let mut nodes = Vec::<(NodeId, Node)>::new();
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>, config: &mut Config| {
        time += 0.02;
        let output = ctx.run(
            egui::RawInput {
                events,
                time: Some(time),
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| edit_input(ui, config, status, &devices, &outputs));
            },
        );
        output.platform_output.accesskit_update.unwrap().nodes
    };
    for _ in 0..2 {
        nodes = frame(vec![], &mut config);
    }
    for (label, action, data) in [
        ("External clock source", Action::Click, None),
        ("Clock source Fixture clock [clock:1]", Action::Click, None),
        ("Follow MIDI Start, Continue and Stop", Action::Click, None),
        ("Stop song and release clip notes", Action::Click, None),
        (
            "External clock loss deadline",
            Action::SetValue,
            Some(ActionData::NumericValue(1000.0)),
        ),
    ] {
        let target = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        frame(
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target,
                action,
                data,
            })],
            &mut config,
        );
        nodes = frame(vec![], &mut config);
    }
    assert_eq!(config.source, Some(71));
    assert_eq!(config.loss, LossPolicy::Stop);
    assert_eq!(config.timeout_ms, 1000);
    assert!(!config.follow_transport);
    assert!(nodes.iter().any(|(_, n)| n
        .description()
        .is_some_and(|d| d.contains("Confirmed external clock input status"))
        && n.value().is_some_and(|d| d.contains("Internal"))));
}

#[test]
fn native_clock_echo_source_is_disabled_by_the_actual_output_port_guard() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let outputs = crate::engine::midi::clock::Shared::default();
    outputs.guard_ports_for_test(vec![crate::engine::midi::routing::Endpoint {
        name: "Fixture output".into(),
        id: Some("21:0".into()),
    }]);
    let devices = vec![Device {
        source: 71,
        endpoint: crate::engine::midi::learn::Endpoint {
            name: "Fixture input".into(),
            id: "21:7".into(),
        },
        pad_modes: 0,
    }];
    let mut config = Config::default();
    let mut time = 0.0;
    let mut frame = |events| {
        time += 0.02;
        ctx.run(
            egui::RawInput {
                events,
                time: Some(time),
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    edit_input(ui, &mut config, Status::default(), &devices, &outputs)
                });
            },
        )
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
    };
    frame(vec![]);
    let nodes = frame(vec![]);
    let target = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("External clock source"))
        .unwrap()
        .0;
    let nodes = frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target,
        action: Action::Click,
        data: None,
    })]);
    let node = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Clock source Fixture input [21:7]"))
        .unwrap();
    assert!(node.1.is_disabled());
    let target = node.0;
    frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target,
        action: Action::Click,
        data: None,
    })]);
    assert_eq!(config.source, None);
}
