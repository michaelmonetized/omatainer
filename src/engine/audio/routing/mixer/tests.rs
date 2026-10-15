use super::*;
use crate::engine::{test_alloc, Command, Engine};
use std::sync::Arc;

fn route(source: Group, destination: Group) -> Connection {
    Connection {
        source: Source {
            group: source,
            tap: Tap::PostMixer,
        },
        destination,
        map: vec![
            ChannelMap {
                source: 0,
                destination: 0,
                gain: 1.,
            },
            ChannelMap {
                source: 1,
                destination: 1,
                gain: 1.,
            },
        ],
    }
}
fn model() -> Model {
    let mut m = Model::default();
    m.next_id = 8;
    m.ports.push(Port {
        id: 2,
        alias: "Independent impulse".into(),
        direction: Direction::Input,
        channels: vec![0, 1],
    });
    for id in [3, 4] {
        m.buses.push(Bus {
            id,
            alias: format!("Return {id}"),
            channels: 2,
            gain: if id == 3 { 0.5 } else { 1. },
            mute: false,
            mixer: Default::default(),
        });
    }
    m.connections = vec![
        route(Group::Input(2), Group::Output(1)),
        route(Group::Bus(4), Group::Output(1)),
    ];
    m.connections[0]
        .map
        .iter_mut()
        .for_each(|map| map.gain = 0.25);
    m.sends = vec![
        Send {
            id: 5,
            name: "Left half".into(),
            route: route(Group::Input(2), Group::Bus(3)),
            level: 0.5,
            pan: -1.,
            mute: false,
            automation: vec![],
        },
        Send {
            id: 6,
            name: "Right quarter".into(),
            route: route(Group::Input(2), Group::Bus(3)),
            level: 0.25,
            pan: 1.,
            mute: false,
            automation: vec![],
        },
        Send {
            id: 7,
            name: "Two-hop return".into(),
            route: route(Group::Bus(3), Group::Bus(4)),
            level: 1.,
            pan: 0.,
            mute: false,
            automation: vec![],
        },
    ];
    m
}
#[test]
fn named_mixer_independent_impulses_taps_solo_mute_pan_and_no_callback_heap() {
    for rate in [44100, 48000, 96000] {
        let (_, mut rt) = Engine::headless_for_test(rate, 256);
        rt.apply(Command::Stop);
        let mut graph = Prepared::at_rate(Arc::new(model()), &rt.session, rate).unwrap();
        for (solo, mute, pre) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, true, true),
        ] {
            assert!(graph.mix_control(
                Target::Return(4),
                Parameter::Solo,
                if solo { 1. } else { 0. }
            ));
            assert!(graph.mix_control(
                Target::Return(3),
                Parameter::Mute,
                if mute { 1. } else { 0. }
            ));
            if pre {
                let mut m = (*graph.model).clone();
                m.sends[2].route.source.tap = Tap::PreFx;
                graph = Prepared::at_rate(Arc::new(m), &rt.session, rate).unwrap();
                graph.mix_control(Target::Return(4), Parameter::Solo, 1.);
                graph.mix_control(Target::Return(3), Parameter::Mute, 1.);
            }
            let heap = test_alloc::measure(|| {
                for n in 0..8193 {
                    let x = if [0, 2048, 8000].contains(&n) {
                        [0.125f32, -0.0625]
                    } else {
                        [0.; 2]
                    };
                    rt.routing_input_frame[..2].copy_from_slice(&x);
                    let actual = graph.render(&mut rt, false, 0., 2);
                    let dry = if solo { 0. } else { 0.25 };
                    let wet = if mute && !pre { 0. } else { 1. };
                    let expected = [
                        (x[0] * (dry + 0.25 * wet)).tanh(),
                        (x[1] * (dry + 0.125 * wet)).tanh(),
                    ];
                    assert!(
                        (actual[0] - expected[0]).abs() < 1e-7,
                        "left {rate}/{solo}/{mute}/{pre}/{n}: {} != {}",
                        actual[0],
                        expected[0]
                    );
                    assert!(
                        (actual[1] - expected[1]).abs() < 1e-7,
                        "right {rate}/{solo}/{mute}/{pre}/{n}"
                    );
                }
            });
            assert_eq!(heap, test_alloc::Counts::default());
        }
    }
}
#[test]
fn named_mixer_sample_automation_endpoints_seeks_and_atomic_cycle_limits() {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    let mut m = model();
    m.sends[0].automation = vec![
        Lane {
            parameter: Parameter::Level,
            points: vec![[0., 0.], [1., 1.], [2., 0.5]],
        },
        Lane {
            parameter: Parameter::Mute,
            points: vec![[0., 0.], [1.5, 1.]],
        },
    ];
    let mut graph = Prepared::at_rate(Arc::new(m.clone()), &rt.session, 48000).unwrap();
    for (beat, level, mute) in [
        (0., 0., false),
        (0.5, 0.5, false),
        (1., 1., false),
        (1.25, 0.875, false),
        (1.5, 0.75, true),
        (9., 0.5, true),
        (0.25, 0.25, false),
    ] {
        graph.mixer.evaluate(&graph.model, beat, true);
        assert!((graph.mixer.channels[0].effective.level - level).abs() < 1e-7);
        assert_eq!(graph.mixer.channels[0].effective.mute, mute);
    }
    let original = serde_json::to_vec(&m).unwrap();
    m.sends[2].route.destination = Group::Bus(3);
    assert!(m.order(&rt.session).unwrap_err().contains("feedback"));
    assert_eq!(serde_json::to_vec(&*graph.model).unwrap(), original);
    m = (*graph.model).clone();
    m.sends[0].automation[0].points.push([1., 0.]);
    assert!(m.order(&rt.session).is_err());
    assert!(!graph.mix_control(Target::Send(5), Parameter::Level, f32::NAN));
    assert!(!graph.mix_control(Target::Send(5), Parameter::Solo, 1.));
    assert!(!graph.mix_control(Target::Return(999), Parameter::Mute, 1.));
}
#[test]
fn named_mixer_typed_journal_stable_targets_capture_and_worker_checkpoint() {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.routing = Some(Box::new(
        Prepared::at_rate(Arc::new(model()), &rt.session, 48000).unwrap(),
    ));
    let namespace = rt.session.namespace;
    for (target, parameter, value) in [
        (Target::Send(5), Parameter::Level, 0.7),
        (Target::Return(4), Parameter::Pan, -0.5),
        (Target::Return(4), Parameter::SoloSafe, 1.),
    ] {
        assert_eq!(
            test_alloc::measure(|| rt.apply(Command::RoutingMix {
                namespace,
                target,
                parameter,
                value
            })),
            test_alloc::Counts::default()
        );
    }
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .mix_values(Target::Send(5))
            .unwrap()
            .level,
        0.7
    );
    rt.apply(Command::Undo);
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .mix_values(Target::Return(4))
            .unwrap()
            .pan,
        0.
    );
    rt.apply(Command::Redo);
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .mix_values(Target::Return(4))
            .unwrap()
            .pan,
        -0.5
    );
    let saved = rt
        .routing
        .as_ref()
        .unwrap()
        .checkpoint(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(saved.sends[0].level, 0.7);
    assert_eq!(saved.buses[1].mixer.pan, -0.5);
    assert!(saved.buses[1].mixer.solo_safe);
    let reopened: Model = serde_json::from_slice(&serde_json::to_vec(&*saved).unwrap()).unwrap();
    assert_eq!(*saved, reopened);
    let old = rt.routing.as_ref().unwrap().mix_values(Target::Send(5));
    rt.apply(Command::RoutingMix {
        namespace: [1, 2],
        target: Target::Send(5),
        parameter: Parameter::Level,
        value: 0.,
    });
    assert_eq!(
        rt.routing.as_ref().unwrap().mix_values(Target::Send(5)),
        old
    );
}

#[test]
fn named_mixer_total_automation_budget_refuses_before_replacing_the_graph() {
    let (_, rt) = Engine::headless_for_test(48000, 256);
    let original = model();
    let mut next = original.clone();
    let mut fourth = next.sends[0].clone();
    fourth.id = 8;
    fourth.name = "Fourth bounded automation".into();
    next.sends.push(fourth);
    next.next_id = 10;
    for send in &mut next.sends {
        send.automation = vec![Lane {
            parameter: Parameter::Level,
            points: (0..MAX_LANE_POINTS).map(|i| [i as f64, 0.5]).collect(),
        }];
    }
    next.order(&rt.session).unwrap();
    let mut fifth = next.sends[0].clone();
    fifth.id = 9;
    fifth.name = "One point too many".into();
    fifth.automation[0].points = vec![[0., 0.5]];
    next.sends.push(fifth);
    assert!(next.order(&rt.session).unwrap_err().contains("65536"));
    assert_eq!(original, model());
}

#[test]
fn named_return_solo_before_shared_sums_keeps_feeders_and_monitor_equivalent() {
    for rate in [44100, 48000, 96000] {
        let (_, mut rt) = Engine::headless_for_test(rate, 256);
        rt.apply(Command::Stop);
        let mut m = model();
        m.sends.clear();
        m.next_id = 9;
        m.buses.push(Bus {
            id: 8,
            alias: "Shared downstream sum".into(),
            channels: 2,
            gain: 1.,
            mute: false,
            mixer: Default::default(),
        });
        m.connections = vec![
            route(Group::Input(2), Group::Bus(3)),
            route(Group::Input(2), Group::Bus(4)),
            route(Group::Bus(3), Group::Bus(8)),
            route(Group::Bus(4), Group::Bus(8)),
            route(Group::Bus(8), Group::Output(1)),
        ];
        m.latency = Some(Default::default());
        let mut graph = Prepared::at_rate(Arc::new(m), &rt.session, rate).unwrap();
        graph.mix_control(Target::Return(3), Parameter::Solo, 1.);
        rt.routing_input_frame[..2].copy_from_slice(&[0.125, -0.0625]);
        for _ in 0..4 {
            graph.render(&mut rt, false, 0., 2);
        }
        let left = graph
            .nodes
            .iter()
            .position(|n| n.group == Group::Bus(3))
            .unwrap();
        let right = graph
            .nodes
            .iter()
            .position(|n| n.group == Group::Bus(4))
            .unwrap();
        let sum = graph
            .nodes
            .iter()
            .position(|n| n.group == Group::Bus(8))
            .unwrap();
        assert_eq!(&graph.nodes[left].taps[2][..2], &[0.0625, -0.03125]);
        assert_eq!(
            &graph.nodes[right].taps[2][..2],
            &[0.125, -0.0625],
            "Unselected return still processes its feeders"
        );
        assert_eq!(&graph.nodes[sum].taps[2][..2], &[0.0625, -0.03125]);
        let output = graph
            .nodes
            .iter()
            .position(|n| n.group == Group::Output(1))
            .unwrap();
        let monitor = graph.monitor_mix(output).unwrap().0;
        assert_eq!(&monitor[..2], &[0.0625, -0.03125]);
        graph.mix_control(Target::Return(4), Parameter::SoloSafe, 1.);
        graph.render(&mut rt, false, 0., 2);
        assert_eq!(&graph.nodes[sum].taps[2][..2], &[0.1875, -0.09375]);
        assert_eq!(
            test_alloc::measure(|| {
                graph.render(&mut rt, false, 0., 2);
            }),
            test_alloc::Counts::default()
        );
    }
}
