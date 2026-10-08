use super::*;
use crate::engine::{audio::routing::latency, test_alloc};

fn prepare(mut model: Model) -> (Positions, Layout) {
    let layout = Layout::fresh(["Retained track".into()], 1);
    model.latency = Some(latency::Configuration {
        reports: vec![latency::Report {
            group: Group::Deck(0),
            external_micros: 0,
            processing_micros: 10_000,
        }],
        ..Default::default()
    });
    let order = model.order(&layout).unwrap();
    let plan = Plan::new(&model, &layout, &order, 8000).unwrap().unwrap();
    (Positions::new(&model, &layout, &order, Some(&plan)), layout)
}

#[test]
fn actual_compensation_paths_retain_loop_reverse_speed_and_media_changes_without_heap_work() {
    let (mut positions, layout) = prepare(Model::default());
    let mut decks = [DeckRt::new(8000.0), DeckRt::new(8000.0)];
    for deck in &mut decks {
        deck.keylock = false;
    }
    decks[0].history_key = 11;
    decks[1].history_key = 12;
    let mut retained = Vec::with_capacity(2000);
    let counts = test_alloc::measure(|| {
        for frame in 0..2000 {
            decks[0].pos = (frame % 17) as f64;
            decks[1].pos = if frame < 700 {
                2000.0 - frame as f64 * 0.5
            } else {
                (frame % 41) as f64 * 1.25
            };
            if frame == 1000 {
                decks[1].history_key = 13;
            }
            retained.push(std::array::from_fn::<_, DECKS, _>(|deck| {
                SourcePosition::from_deck(&decks[deck])
            }));
            let captured = positions.capture(&decks, &layout, 4, Some([2, 3]), false);
            assert_eq!(captured[0], retained[frame][0]);
            assert_eq!(
                captured[1],
                if frame < 80 {
                    SourcePosition::default()
                } else {
                    retained[frame - 80][1]
                }
            );
        }
    });
    assert_eq!(counts, Default::default());
}

#[test]
fn differently_delayed_duplicate_routes_are_ambiguous_but_zero_gain_routes_are_absent() {
    let mut model = Model::default();
    model.connections.push(Connection {
        source: Source {
            group: Group::Deck(0),
            tap: Tap::PreFx,
        },
        destination: Group::Output(1),
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    });
    let (mut positions, layout) = prepare(model.clone());
    let mut decks = [DeckRt::new(8000.0), DeckRt::new(8000.0)];
    decks[0].history_key = 11;
    decks[1].history_key = 12;
    for _ in 0..200 {
        positions.capture(&decks, &layout, 2, None, false);
    }
    let captured = positions.capture(&decks, &layout, 2, None, false);
    assert_eq!(captured[0].media_key, 0);
    assert_eq!(captured[1].media_key, 12);
    model.connections.last_mut().unwrap().map[0].gain = 0.0;
    let (mut positions, layout) = prepare(model);
    for _ in 0..200 {
        positions.capture(&decks, &layout, 2, None, false);
    }
    assert_eq!(
        positions.capture(&decks, &layout, 2, None, false)[0].media_key,
        11
    );
}

#[test]
fn absent_open_channels_and_headphone_overrides_never_claim_program_source_positions() {
    let mut model = Model::default();
    model.ports[0].channels = vec![62, 63];
    let (mut positions, layout) = prepare(model);
    let mut decks = [DeckRt::new(8000.0), DeckRt::new(8000.0)];
    decks[0].history_key = 11;
    decks[1].history_key = 12;
    for _ in 0..200 {
        positions.capture(&decks, &layout, 4, None, false);
    }
    assert_eq!(
        positions.capture(&decks, &layout, 4, None, false),
        [SourcePosition::default(); DECKS]
    );
    assert_eq!(
        positions.capture(&decks, &layout, 64, None, false)[1].media_key,
        12
    );
    assert_eq!(
        positions.capture(&decks, &layout, 64, Some([62, 63]), false),
        [SourcePosition::default(); DECKS]
    );
}

#[test]
fn history_transfer_and_reset_refuse_unfilled_mixed_and_stale_layout_positions() {
    let (mut positions, mut layout) = prepare(Model::default());
    let mut decks = [DeckRt::new(8000.0), DeckRt::new(8000.0)];
    decks[0].history_key = 11;
    decks[1].history_key = 12;
    for frame in 0..200 {
        decks[1].pos = frame as f64;
        positions.capture(&decks, &layout, 2, None, false);
    }
    let model = Model {
        latency: Some(latency::Configuration {
            reports: vec![latency::Report {
                group: Group::Deck(0),
                external_micros: 0,
                processing_micros: 12_000,
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let order = model.order(&layout).unwrap();
    let plan = Plan::new(&model, &layout, &order, 8000).unwrap().unwrap();
    let mut next = Positions::new(&model, &layout, &order, Some(&plan));
    assert_eq!(
        test_alloc::measure(|| next.inherit(&mut positions)),
        Default::default()
    );
    decks[1].pos = 200.0;
    assert_eq!(
        next.capture(&decks, &layout, 2, None, true),
        [SourcePosition::default(); DECKS]
    );
    decks[1].pos = 201.0;
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false)[1].source_frame,
        105.0
    );
    assert_eq!(test_alloc::measure(|| next.reset()), Default::default());
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false)[1].media_key,
        0
    );
    layout.generation += 1;
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false),
        [SourcePosition::default(); DECKS]
    );
    assert_eq!(
        test_alloc::measure(|| next.bind(&layout)),
        Default::default()
    );
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false)[0].media_key,
        11
    );
    layout.tracks[0].active = false;
    layout.generation += 1;
    next.bind(&layout);
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false),
        [SourcePosition::default(); DECKS]
    );
    layout.tracks[0].active = true;
    layout.generation += 1;
    next.bind(&layout);
    assert_eq!(
        next.capture(&decks, &layout, 2, None, false)[1].media_key,
        0
    );
}

#[test]
fn actual_graph_pcm_and_queued_positions_share_the_same_delayed_click_frame() {
    use crate::engine::audio::routing::prepared::Prepared;
    use std::sync::Arc;
    let (mut positions, layout) = prepare(Model::default());
    let mut model = Model::default();
    model.latency = Some(latency::Configuration {
        reports: vec![latency::Report {
            group: Group::Deck(0),
            external_micros: 0,
            processing_micros: 10_000,
        }],
        ..Default::default()
    });
    let mut graph = Prepared::at_rate(Arc::new(model), &layout, 8000).unwrap();
    let mut decks = [DeckRt::new(8000.0), DeckRt::new(8000.0)];
    decks[1].keylock = false;
    decks[1].history_key = 12;
    let mut expected = Vec::with_capacity(2200);
    let mut writer = crate::engine::audible::Writer::new();
    let reader = writer.handle();
    let counts = test_alloc::measure(|| {
        writer.begin(8000, Some(1_000_000_000));
        for frame in 0..2200 {
            decks[1].pos = if frame < 800 {
                (frame % 29) as f64
            } else {
                7000.0 - frame as f64 * 1.5
            };
            if frame == 1200 {
                decks[1].history_key = 13;
            }
            let click = if frame % 97 < 2 { 0.25 } else { 0.0 };
            expected.push((click, SourcePosition::from_deck(&decks[1])));
            graph.begin();
            for index in 0..graph.nodes.len() {
                graph.gather(index);
                let group = graph.nodes[index].group;
                if matches!(group, Group::Output(_) | Group::Record(_)) {
                    continue;
                }
                let value = if group == Group::Deck(1) {
                    click
                } else if group == Group::Main {
                    graph.nodes[index].input[0]
                } else {
                    0.0
                };
                graph.publish_stereo(index, [[value; 2]; 3]);
                if matches!(group, Group::Scene(_) | Group::Deck(_)) {
                    graph.source_send(index, graph.main, [value; 2], true);
                }
            }
            let captured = positions.capture(&decks, &layout, 2, None, false);
            assert_eq!(
                graph.outputs(2)[0],
                if frame < 80 {
                    0.0
                } else {
                    expected[frame - 80].0
                }
            );
            assert_eq!(
                captured[1],
                if frame < 80 {
                    SourcePosition::default()
                } else {
                    expected[frame - 80].1
                }
            );
            writer.push_sources(captured);
        }
        writer.finish();
    });
    assert_eq!(counts, Default::default());
    for frame in 80..2200 {
        let actual = reader
            .positions_at(1_000_000_000 + frame as u64 * 125000)
            .unwrap()[1];
        assert_eq!(actual.source_frame, expected[frame - 80].1.source_frame);
        assert_eq!(actual.media_key, expected[frame - 80].1.media_key);
    }
}

#[test]
fn admitted_routing_edits_and_metadata_undo_bind_the_final_generation() {
    use crate::engine::{
        midi_edit::Outcome,
        session::{Action, Axis, Request},
        Command, Engine,
    };
    use std::{
        sync::{atomic::AtomicBool, Arc},
        time::{Duration, Instant},
    };
    let (engine, mut rt) = Engine::headless_for_test(8000, 256);
    let owner = engine.project.clone();
    let worker = std::thread::spawn(move || owner.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !worker.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let (request, ack) = Request::routing(
        worker.join().unwrap(),
        8000,
        Some(Arc::new(Model::default())),
    )
    .unwrap();
    let command = Command::session_edit(request);
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    rt.decks[0].history_key = 11;
    rt.decks[1].history_key = 12;
    let check = |rt: &mut crate::engine::RtEngine| {
        let mut graph = rt.routing.take().unwrap();
        let result = graph.source_positions(rt, 2);
        rt.routing = Some(graph);
        assert_eq!(result[0].media_key, 11);
        assert_eq!(result[1].media_key, 12);
    };
    check(&mut rt);
    let (request, ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Rename {
            axis: Axis::Track,
            id: rt.session.tracks[0].id,
            name: "Renamed".into(),
        },
    )
    .unwrap();
    let command = Command::session_edit(request);
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    check(&mut rt);
    rt.apply(Command::Undo);
    check(&mut rt);
    rt.apply(Command::Redo);
    check(&mut rt);
}

#[test]
fn source_position_storage_is_charged_inside_the_existing_history_ceiling() {
    let layout = Layout::fresh((0..128).map(|index| format!("Track {index}")), 1);
    let mut model = Model {
        latency: Some(latency::Configuration::default()),
        ..Default::default()
    };
    let order = model.order(&layout).unwrap();
    let mut low = 1000;
    let mut high = 2_000_000;
    while low + 1 < high {
        let middle = (low + high) / 2;
        model.latency.as_mut().unwrap().reserve_micros = middle;
        if Plan::new(&model, &layout, &order, 48000).is_ok() {
            low = middle;
        } else {
            high = middle;
        }
    }
    model.latency.as_mut().unwrap().reserve_micros = low;
    let accepted = Plan::new(&model, &layout, &order, 48000).unwrap().unwrap();
    assert!(accepted.storage_bytes <= latency::MAX_STORAGE);
    model.latency.as_mut().unwrap().reserve_micros = high;
    let error = Plan::new(&model, &layout, &order, 48000).unwrap_err();
    assert!(error.contains("64 MiB"));
    let rejected_frames = (u64::from(high) * 48000 + 500000) / 1000000;
    let pcm_bytes_per_frame = (accepted.storage_bytes
        - Positions::storage_bytes(accepted.reserve, &layout))
        / (accepted.reserve as usize + 1);
    let pcm_bytes = pcm_bytes_per_frame * (rejected_frames as usize + 1);
    assert!(
        pcm_bytes <= latency::MAX_STORAGE,
        "The original PCM histories alone would still fit"
    );
    assert!(
        pcm_bytes + Positions::storage_bytes(rejected_frames as u32, &layout)
            > latency::MAX_STORAGE
    );
}
