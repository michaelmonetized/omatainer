use super::model::*;
use crate::engine::session::Layout;

fn layout() -> Layout {
    Layout::fresh(["First".into(), "Second".into()], 2)
}
fn connection(source: Group, destination: Group) -> Connection {
    Connection {
        source: Source {
            group: source,
            tap: Tap::PostMixer,
        },
        destination,
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    }
}

#[test]
fn default_graph_orders_sources_before_mixer_and_retains_explicit_alias_channels() {
    let layout = layout();
    let model = Model::default();
    let order = model.order(&layout).unwrap();
    let before = |a, b| {
        order.iter().position(|group| *group == a).unwrap()
            < order.iter().position(|group| *group == b).unwrap()
    };
    for track in &layout.tracks {
        for scene in &layout.scenes {
            assert!(before(Group::Track(track.id), Group::Scene(scene.id)));
        }
    }
    for scene in &layout.scenes {
        assert!(before(Group::Scene(scene.id), Group::Main));
    }
    assert!(before(Group::Main, Group::Output(1)));
    assert_eq!(model.port(1, Direction::Output).unwrap().channels, [0, 1]);
    assert_eq!(
        serde_json::from_slice::<Model>(&serde_json::to_vec(&model).unwrap()).unwrap(),
        model
    );
}

#[test]
fn feedback_and_invalid_channel_maps_are_rejected_before_any_state_changes() {
    let layout = layout();
    let mut model = Model::default();
    let baseline = model.clone();
    model
        .connections
        .push(connection(Group::Main, Group::Track(layout.tracks[0].id)));
    assert!(model.order(&layout).unwrap_err().contains("feedback"));
    model = baseline.clone();
    model.connections[0].map[0].source = 2;
    assert!(model.order(&layout).is_err());
    model = baseline.clone();
    model.connections[0].map[0].gain = f32::NAN;
    assert!(model.order(&layout).is_err());
    model = baseline.clone();
    model.connections[0].destination = Group::Input(1);
    assert!(model.order(&layout).is_err());
    assert_eq!(baseline, Model::default());
}

#[test]
fn removed_devices_and_reordered_or_inactive_tracks_do_not_reinterpret_aliases() {
    let mut layout = layout();
    let mut model = Model::default();
    model.next_id = 4;
    model.ports.push(Port {
        id: 2,
        alias: "Interface input 63/64".into(),
        direction: Direction::Input,
        channels: vec![62, 63],
    });
    model.ports.push(Port {
        id: 3,
        alias: "Recorded pair".into(),
        direction: Direction::Record,
        channels: vec![0, 1],
    });
    let track = layout.tracks[0].id;
    model
        .connections
        .push(connection(Group::Input(2), Group::Track(track)));
    model
        .connections
        .push(connection(Group::Track(track), Group::Record(3)));
    let baseline = serde_json::to_vec(&model).unwrap();
    model.order(&layout).unwrap();
    layout.track_order.reverse();
    model.order(&layout).unwrap();
    layout
        .delete(crate::engine::session::Axis::Track, track)
        .unwrap();
    model.order(&layout).unwrap();
    assert_eq!(serde_json::to_vec(&model).unwrap(), baseline);
    assert_eq!(model.port(2, Direction::Input).unwrap().channels, [62, 63]);
}

#[test]
fn explicit_bus_routes_order_independent_sources_and_reject_bus_feedback() {
    let layout = layout();
    let mut model = Model::default();
    model.next_id = 4;
    model.buses = vec![
        Bus {
            id: 2,
            alias: "Surround".into(),
            channels: 8,
            gain: 0.8,
            mute: false,
        },
        Bus {
            id: 3,
            alias: "Print".into(),
            channels: 8,
            gain: 1.0,
            mute: false,
        },
    ];
    model
        .connections
        .push(connection(Group::Deck(0), Group::Bus(2)));
    model
        .connections
        .push(connection(Group::Bus(2), Group::Bus(3)));
    model
        .connections
        .push(connection(Group::Bus(3), Group::Output(1)));
    model.order(&layout).unwrap();
    model
        .connections
        .push(connection(Group::Bus(3), Group::Bus(2)));
    assert!(model.order(&layout).unwrap_err().contains("feedback"));
}

fn engine() -> crate::engine::RtEngine {
    crate::engine::Engine::headless_for_test(48000, 256).1
}

#[test]
fn all_64_physical_outputs_render_distinct_mapped_samples_without_heap_work() {
    let mut rt = engine();
    let mut model = Model::default();
    model.ports.clear();
    model.connections.clear();
    model.next_id = 5;
    for half in 0..2 {
        let input = 1 + half * 2;
        let output = input + 1;
        let channels: Vec<_> = (half as u16 * 32..half as u16 * 32 + 32).collect();
        model.ports.push(Port {
            id: input,
            alias: format!("Input {half}"),
            direction: Direction::Input,
            channels: channels.clone(),
        });
        model.ports.push(Port {
            id: output,
            alias: format!("Output {half}"),
            direction: Direction::Output,
            channels,
        });
        model.connections.push(Connection {
            source: Source {
                group: Group::Input(input),
                tap: Tap::PostFx,
            },
            destination: Group::Output(output),
            map: (0..32)
                .map(|channel| ChannelMap {
                    source: channel as u8,
                    destination: channel as u8,
                    gain: 1.0,
                })
                .collect(),
        });
    }
    let mut graph =
        super::prepared::Prepared::new(std::sync::Arc::new(model), &rt.session).unwrap();
    rt.routing_input_frame = std::array::from_fn(|channel| (channel + 1) as f32 / 128.0);
    let frame = graph.render(&mut rt, false, 0.0, 64);
    for (channel, actual) in frame.into_iter().enumerate() {
        assert_eq!(
            actual,
            crate::engine::limiter(rt.routing_input_frame[channel])
        );
    }
    assert_eq!(
        crate::engine::test_alloc::measure(|| {
            let _ = graph.render(&mut rt, false, 0.0, 64);
        }),
        crate::engine::test_alloc::Counts::default()
    );
    let absent = graph.render(&mut rt, false, 0.0, 2);
    assert_eq!(&absent[2..], &[0.0; 62]);
    assert_eq!(graph.model.ports[3].channels, (32..64).collect::<Vec<_>>());
}

#[test]
fn independent_input_routes_and_emergency_silence_reach_last_physical_channel() {
    let mut rt = engine();
    let mut model = Model::default();
    model.next_id = 3;
    model.ports[0].channels = vec![63];
    model.ports.push(Port {
        id: 2,
        alias: "Mic".into(),
        direction: Direction::Input,
        channels: vec![0],
    });
    model.connections = vec![connection(Group::Input(2), Group::Output(1))];
    let mut graph =
        super::prepared::Prepared::new(std::sync::Arc::new(model), &rt.session).unwrap();
    rt.routing_input_frame[0] = 0.25;
    assert!(graph.render(&mut rt, false, 0.0, 64)[63] > 0.2);
    rt.performance
        .request_safety(crate::engine::performance::Safety::Silence);
    rt.process(&mut []);
    let mut last = 1.0;
    for _ in 0..(rt.sr * 0.002).ceil() as usize {
        let value = graph.render(&mut rt, false, 0.0, 64)[63];
        assert!(value <= last);
        last = value;
    }
    assert_eq!(last, 0.0);
    assert_eq!(graph.render(&mut rt, false, 0.0, 64), [0.0; 64]);
}

#[test]
fn deleted_track_routes_remain_silent_when_the_slot_is_reused() {
    let mut layout = layout();
    let old = layout.tracks[0].id;
    let mut model = Model::default();
    model
        .connections
        .push(connection(Group::Track(old), Group::Output(1)));
    layout
        .delete(crate::engine::session::Axis::Track, old)
        .unwrap();
    let (replacement, _) = layout
        .create(crate::engine::session::Axis::Track, "New".into(), None, 0)
        .unwrap();
    assert_ne!(replacement, old);
    let graph = super::prepared::Prepared::new(std::sync::Arc::new(model), &layout).unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.group == Group::Track(old) && node.slot.is_none()));
}

#[test]
fn independent_track_deck_and_bus_taps_preserve_pre_mixer_audio() {
    use crate::engine::{Command, Sample};
    use std::sync::Arc;
    let mut rt = engine();
    rt.apply(Command::Stop);
    rt.legacy_gain_math = true;
    rt.tracks[0].mute = true;
    rt.tracks[0].fx.slots.clear();
    rt.decks[0].audio = Some(Arc::new(Sample {
        name: "Independent source".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.2; 8192],
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    }));
    rt.decks[0].playing = true;
    rt.decks[0].sync = false;
    rt.decks[0].keylock = false;
    rt.decks[0].gain = 0.4;
    rt.decks[0].transition_remaining = 0;
    rt.xfader = 1.0;
    let track = rt.session.tracks[0].id;
    let mut model = Model::default();
    model.next_id = 4;
    model.ports[0].channels = (0..9).collect();
    model.ports.push(Port {
        id: 2,
        alias: "Mic".into(),
        direction: Direction::Input,
        channels: vec![0],
    });
    model.buses.push(Bus {
        id: 3,
        alias: "Print".into(),
        channels: 1,
        gain: 0.25,
        mute: false,
    });
    model.connections = vec![
        connection(Group::Input(2), Group::Track(track)),
        connection(Group::Input(2), Group::Bus(3)),
    ];
    model.tracks_without_default_send = rt.session.tracks.iter().map(|item| item.id).collect();
    model.decks_without_default_send = [true; 2];
    for (source, group) in [Group::Track(track), Group::Deck(0), Group::Bus(3)]
        .into_iter()
        .enumerate()
    {
        for (index, tap) in [Tap::PreFx, Tap::PostFx, Tap::PostMixer]
            .into_iter()
            .enumerate()
        {
            model.connections.push(Connection {
                source: Source { group, tap },
                destination: Group::Output(1),
                map: vec![ChannelMap {
                    source: 0,
                    destination: (source * 3 + index) as u8,
                    gain: 1.0,
                }],
            });
        }
    }
    let mut graph = super::prepared::Prepared::new(Arc::new(model), &rt.session).unwrap();
    rt.routing_input_frame[0] = 0.125;
    let frame = graph.render(&mut rt, false, 0.0, 9);
    assert_eq!(frame[0], crate::engine::limiter(0.125));
    assert!(frame[1] > 0.01);
    assert_eq!(frame[2], 0.0);
    assert_eq!(frame[3], crate::engine::limiter(0.2));
    assert!(frame[4] > 0.001 && frame[4] < frame[3]);
    assert!(frame[5].abs() < 0.000001);
    assert_eq!(frame[6], crate::engine::limiter(0.125));
    assert_eq!(frame[7], frame[6]);
    assert_eq!(frame[8], crate::engine::limiter(0.03125));
    assert_eq!(
        crate::engine::test_alloc::measure(|| {
            graph.render(&mut rt, false, 0.0, 9);
        }),
        crate::engine::test_alloc::Counts::default()
    );
}

#[test]
fn maximum_saved_graph_processes_full_native_width_without_callback_heap_work() {
    let mut rt = crate::engine::project::maximum_for_test().into_offline();
    rt.legacy_gain_math = false;
    let mut model = Model::default();
    model.ports.clear();
    model.buses.clear();
    model.connections.clear();
    for direction in [Direction::Input, Direction::Output, Direction::Record] {
        for index in 0..MAX_PORTS {
            let id = model.ports.len() as u64 + 1;
            model.ports.push(Port {
                id,
                alias: format!("Port {id}"),
                direction,
                channels: (0..if direction == Direction::Record {
                    MAX_RECORD_CHANNELS
                } else {
                    MAX_PORT_CHANNELS
                })
                    .map(|channel| {
                        ((channel
                            + if direction == Direction::Input {
                                index % 2 * 32
                            } else {
                                0
                            }) as u16)
                    })
                    .collect(),
            });
        }
    }
    for index in 0..MAX_BUSES {
        model.buses.push(Bus {
            id: (97 + index) as u64,
            alias: format!("Bus {index}"),
            channels: 32,
            gain: 0.5,
            mute: false,
        });
    }
    model.next_id = 129;
    for index in 0..MAX_CONNECTIONS {
        let slot = (index % 32) as u64;
        let (source, destination) = match index / 32 {
            0 => (Group::Bus(97 + slot), Group::Output(33 + slot)),
            1 => (Group::Bus(97 + slot), Group::Record(65 + slot)),
            _ => (Group::Input(1 + slot), Group::Bus(97 + slot)),
        };
        model.connections.push(Connection {
            source: Source {
                group: source,
                tap: Tap::PostMixer,
            },
            destination,
            map: (0..MAX_MAPS / MAX_CONNECTIONS)
                .map(|channel| ChannelMap {
                    source: channel as u8,
                    destination: channel as u8,
                    gain: 0.125,
                })
                .collect(),
        });
    }
    assert_eq!(
        model
            .connections
            .iter()
            .map(|route| route.map.len())
            .sum::<usize>(),
        MAX_MAPS
    );
    rt.routing = Some(Box::new(
        super::prepared::Prepared::new(std::sync::Arc::new(model), &rt.session).unwrap(),
    ));
    let mut block = [0.0; 32 * 128];
    rt.process_interleaved(&mut block, 32);
    let started = std::time::Instant::now();
    let heap = crate::engine::test_alloc::measure(|| rt.process_interleaved(&mut block, 32));
    assert_eq!(heap, crate::engine::test_alloc::Counts::default());
    assert!(block.iter().all(|value| value.is_finite()));
    println!("Maximum routing: 128 tracks, 512 scenes, 96 ports, 32 buses, 256 connections, 4096 maps; 128 frames × 32 native channels in {} ns; heap {:?}", started.elapsed().as_nanos(), heap);
    println!("Maximum routing render CPU: {:?} ns", rt.render_cpu_ns);
    let retained = rt.routing.take();
    let started = std::time::Instant::now();
    rt.process_interleaved(&mut block, 32);
    println!(
        "Same maximum session without custom routing: {} ns wall, {:?} ns render CPU",
        started.elapsed().as_nanos(),
        rt.render_cpu_ns
    );
    drop(retained);
}
