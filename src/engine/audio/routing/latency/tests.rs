use super::*;
use crate::engine::audio::routing::{model::*, prepared::Prepared};
use std::sync::Arc;

#[test]
fn retained_zero_frames_keep_original_validity_signed_samples_and_wrapped_delays_without_heap_work() {
    for width in [1, 2, 16, MAX_PORT_CHANNELS] {
        let mut history = History::new(width, 16);
        let mut reference = vec![[0.0_f32; MAX_PORT_CHANNELS]; 17];
        let mut valid = [false; 17];
        let mut frame = [0.0; MAX_PORT_CHANNELS];
        let counts = crate::engine::test_alloc::measure(|| {
            for index in 0..1000 {
                frame.fill(0.0);
                if index % 101 < 3 {
                    for (channel, sample) in frame[..width].iter_mut().enumerate() {
                        *sample = match channel % 4 {
                            0 => index as f32 * 0.001 + 0.1,
                            1 => -0.0,
                            2 => f32::from_bits(1),
                            _ => -0.25,
                        };
                    }
                }
                let present = index % 13 != 0;
                history.push(&frame, present);
                reference[index % 17] = frame;
                valid[index % 17] = present;
                for delay in [0, 1, 8, 16, 17] {
                    let available = delay <= index && delay < 17;
                    let slot = (index + 17 - delay.min(17)) % 17;
                    for channel in 0..=width {
                        let expected = if available && channel < width { reference[slot][channel] } else { 0.0 };
                        let continuity = available && channel < width && valid[slot];
                        assert_eq!(history.sample(channel, delay as u32).to_bits(), expected.to_bits());
                        let (sample, complete) = history.read(channel, delay as u32);
                        assert_eq!(sample.to_bits(), expected.to_bits());
                        assert_eq!(complete, continuity);
                        for mix in [0.0, 0.3, 1.0] {
                            let old = if channel < width { reference[index % 17][channel] } else { 0.0 };
                            let expected = if delay == 0 || mix >= 1.0 { expected } else if mix <= 0.0 { old } else { old * (1.0 - mix) + expected * mix };
                            let (sample, complete) = history.blended(channel, 0, delay as u32, mix);
                            assert_eq!(sample.to_bits(), expected.to_bits());
                            let old_valid = channel < width && present;
                            assert_eq!(complete, if delay == 0 || mix >= 1.0 { continuity } else if mix <= 0.0 { old_valid } else { old_valid && continuity });
                        }
                    }
                    assert_eq!(history.valid(delay as u32), available && valid[slot]);
                }
            }
        });
        assert_eq!(counts, crate::engine::test_alloc::Counts::default());
        assert_eq!(history.bytes(), 17 * (width * 4 + 1));
    }
}

fn model() -> (Model, Layout) {
    let layout = Layout::fresh(["Instrument".into()], 1);
    let mut model = Model::default();
    model.next_id = 5;
    for (id, channel) in [(2, 0), (3, 1)] {
        model.ports.push(Port {
            id,
            alias: format!("Input {id}"),
            direction: Direction::Input,
            channels: vec![channel],
        });
    }
    model.buses.push(Bus {
        id: 4,
        alias: "Parallel null".into(),
        channels: 1,
        gain: 1.0,
        mute: false,
    });
    model.connections.clear();
    for (id, gain) in [(2, 1.0), (3, -1.0)] {
        model.connections.push(Connection {
            source: Source {
                group: Group::Input(id),
                tap: Tap::PostMixer,
            },
            destination: Group::Bus(4),
            map: vec![ChannelMap {
                source: 0,
                destination: 0,
                gain,
            }],
        });
    }
    model.connections.push(Connection {
        source: Source {
            group: Group::Bus(4),
            tap: Tap::PostMixer,
        },
        destination: Group::Output(1),
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    });
    model.latency = Some(Configuration {
        reports: vec![Report {
            group: Group::Input(3),
            external_micros: 7000,
            processing_micros: 0,
        }],
        ..Default::default()
    });
    (model, layout)
}
fn frame(graph: &mut Prepared, fast: f32, slow: f32) -> f32 {
    graph.begin();
    for index in 0..graph.nodes.len() {
        graph.gather(index);
        let group = graph.nodes[index].group;
        let value = match group {
            Group::Input(2) => fast,
            Group::Input(3) => slow,
            Group::Bus(4) => graph.nodes[index].input[0],
            _ => 0.0,
        };
        let mut source = [0.0; MAX_PORT_CHANNELS];
        source[0] = value;
        graph.publish(index, [source; 3]);
    }
    graph.outputs(2)[0]
}

#[test]
fn generated_numbered_transients_null_across_real_prepared_parallel_links_at_four_rates() {
    let mut compared = 0;
    for rate in [8000, 44100, 48000, 192000] {
        let (model, layout) = model();
        let mut graph = Prepared::at_rate(Arc::new(model), &layout, rate).unwrap();
        let offset = (u64::from(rate) * 7 + 500) / 1000;
        let source = |index: u64| {
            if index % 97 < 3 {
                (index / 97 % 17 + 1) as f32 / 32.0
            } else {
                0.0
            }
        };
        for index in 0..10000 {
            let slow = if index >= offset {
                source(index - offset)
            } else {
                0.0
            };
            assert_eq!(
                frame(&mut graph, source(index), slow),
                0.0,
                "{rate} Hz frame {index}"
            );
            compared += 1;
        }
        assert_eq!(graph.latency.as_ref().unwrap().plan.program, offset as u32);
    }
    println!("LATENCY_PARALLEL {{\"sample_rates\":4,\"compared_frames\":{compared},\"maximum_null_error\":0,\"physical_devices_opened\":false}}");
}

#[test]
fn original_continuity_and_repeated_reads_belong_to_the_retained_sample() {
    let mut history = History::new(2, 4);
    assert!(!history.valid(0));
    for index in 0..12 {
        history.push(&[index as f32, -(index as f32)], index != 8);
    }
    assert!(!history.valid(3));
    assert!(history.valid(4));
    for _ in 0..100 {
        assert_eq!(history.sample(0, 4), 7.0);
        assert_eq!(history.sample(1, 4), -7.0);
    }
    assert_eq!(history.sample(0, 5), 0.0);
    assert!(!history.valid(5));
}

#[test]
fn settled_and_transitioning_reads_preserve_numbered_wide_frames_through_ring_wraps() {
    let mut history = History::new(MAX_PORT_CHANNELS, 7);
    assert_eq!(history.blended(0, 0, 0, 1.0), (0.0, false));
    let counts = crate::engine::test_alloc::measure(|| {
        for index in 0..100 {
            let frame = std::array::from_fn::<_, MAX_PORT_CHANNELS, _>(|channel| (index * 100 + channel) as f32);
            history.push(&frame, index % 3 != 0);
            for channel in 0..MAX_PORT_CHANNELS {
                for delay in 0..8_u32 {
                    let expected = if delay as usize <= index {
                        let original = index - delay as usize;
                        ((original * 100 + channel) as f32, original % 3 != 0)
                    } else { (0.0, false) };
                    assert_eq!(history.read(channel, delay), expected);
                    assert_eq!(history.blended(channel, 7, delay, 1.0), expected);
                    assert_eq!(history.blended(channel, delay, 7, 0.0), expected);
                    assert_eq!(history.blended(channel, delay, delay, 0.25), expected);
                }
                let before = history.read(channel, 2);
                let after = history.read(channel, 5);
                assert_eq!(history.blended(channel, 2, 5, 0.25), (before.0 * 0.75 + after.0 * 0.25, before.1 && after.1));
            }
            assert_eq!(history.read(0, 8), (0.0, false));
        }
    });
    assert_eq!(counts, Default::default());
}

#[test]
fn implicit_sources_taps_and_terminal_offsets_have_one_causal_plan() {
    let (model, layout) = model();
    let mut model = model;
    model.connections.push(Connection {
        source: Source {
            group: Group::Bus(4),
            tap: Tap::PostMixer,
        },
        destination: Group::Track(layout.tracks[0].id),
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    });
    model.latency.as_mut().unwrap().reports.extend([
        Report {
            group: Group::Track(layout.tracks[0].id),
            external_micros: 0,
            processing_micros: 3000,
        },
        Report {
            group: Group::Main,
            external_micros: 0,
            processing_micros: 4000,
        },
        Report {
            group: Group::Output(1),
            external_micros: 2000,
            processing_micros: 0,
        },
    ]);
    model.connections.push(Connection {
        source: Source {
            group: Group::Main,
            tap: Tap::PostMixer,
        },
        destination: Group::Output(1),
        map: vec![ChannelMap {
            source: 0,
            destination: 1,
            gain: 1.0,
        }],
    });
    let order = model.order(&layout).unwrap();
    let plan = Plan::new(&model, &layout, &order, 48000).unwrap().unwrap();
    let node = |group| order.iter().position(|actual| *actual == group).unwrap();
    assert_eq!(plan.inputs[node(Group::Track(layout.tracks[0].id))], 336);
    assert_eq!(
        plan.taps[node(Group::Track(layout.tracks[0].id))],
        [336, 480, 480]
    );
    assert_eq!(plan.taps[node(Group::Main)], [480, 672, 672]);
    assert_eq!(plan.program, 768);
    assert_eq!(plan.inputs[node(Group::Output(1))], 672);
    assert!(plan.required[node(Group::Track(layout.tracks[0].id))][2]);
    assert!(plan.required[node(Group::Scene(layout.scenes[0].id))][2]);
}

#[test]
fn stale_duplicate_oversized_and_cumulative_reports_refuse_before_history_allocation() {
    let (model, layout) = model();
    let mut config = model.latency.clone().unwrap();
    config.reports.push(config.reports[0]);
    assert!(config.validate(&model, &layout).is_err());
    config.reports.pop();
    config.reports[0].group = Group::Input(900);
    assert!(config.validate(&model, &layout).is_err());
    config.reports[0].group = Group::Input(3);
    config.reports[0].processing_micros = 1;
    assert!(config.validate(&model, &layout).is_err());
    let mut model = model;
    model.latency.as_mut().unwrap().reserve_micros = 6000;
    assert!(Prepared::at_rate(Arc::new(model.clone()), &layout, 48000).is_err());
    model.latency.as_mut().unwrap().reserve_micros = 8000;
    model.latency.as_mut().unwrap().reports.push(Report {
        group: Group::Bus(4),
        external_micros: 0,
        processing_micros: 3000,
    });
    assert!(Prepared::at_rate(Arc::new(model), &layout, 48000)
        .unwrap_err()
        .contains("complete latency path"));
}

#[test]
fn dynamic_history_transfer_and_actual_gather_have_no_callback_heap_operations() {
    let (model, layout) = model();
    let mut current = Prepared::at_rate(Arc::new(model.clone()), &layout, 48000).unwrap();
    for index in 0..5000 {
        frame(&mut current, index as f32 / 5000.0, 0.0);
    }
    let mut changed = model;
    changed.latency.as_mut().unwrap().reports[0].external_micros = 9000;
    let mut next = Prepared::at_rate(Arc::new(changed), &layout, 48000).unwrap();
    assert_eq!(
        crate::engine::test_alloc::measure(|| next.inherit(&mut current)),
        Default::default()
    );
    assert_eq!(
        crate::engine::test_alloc::measure(|| {
            for _ in 0..1000 {
                assert!(frame(&mut next, 0.0, 0.0).is_finite());
            }
        }),
        Default::default()
    );
    assert_eq!(next.latency.as_ref().unwrap().remaining, 0);
}

#[test]
fn omitted_configuration_retains_legacy_saved_routes_and_direct_samples() {
    let model = Model::default();
    assert!(!serde_json::to_string(&model).unwrap().contains("latency"));
    let layout = Layout::fresh(["Original".into()], 1);
    let graph = Prepared::at_rate(Arc::new(model), &layout, 48000).unwrap();
    assert!(graph.latency.is_none());
}

#[test]
fn parallel_numbered_sources_null_through_the_actual_interleaved_renderer_without_callback_heap_work(
) {
    let mut compared = 0;
    for rate in [8000, 44100, 48000, 192000] {
        let (_, mut rt) = crate::engine::Engine::headless_for_test(rate, 256);
        let (model, _) = model();
        rt.routing = Some(Box::new(
            Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap(),
        ));
        rt.routing_pipe = crate::engine::audio::routing::input::Pipe::controlled_for_test(rate);
        let offset = (u64::from(rate) * 7 + 500) / 1000;
        let source = |index: u64| {
            if index % 97 < 3 {
                (index / 97 % 17 + 1) as f32 / 32.0
            } else {
                0.0
            }
        };
        let samples = |start| {
            (start..start + 64)
                .flat_map(|index| {
                    [
                        source(index),
                        if index >= offset {
                            source(index - offset)
                        } else {
                            0.0
                        },
                    ]
                })
                .collect::<Vec<_>>()
        };
        rt.routing_pipe.capture(&samples(0), 2, 0);
        for block in 0..64 {
            rt.routing_pipe.capture(&samples((block + 1) * 64), 2, 0);
            let mut output = [1.0; 128];
            assert_eq!(
                crate::engine::test_alloc::measure(|| rt.process(&mut output)),
                Default::default()
            );
            assert_eq!(output, [0.0; 128], "{rate} Hz block {block}");
            compared += 64;
        }
        assert_eq!(
            rt.routing_pipe
                .shared
                .underrun
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            rt.routing.as_ref().unwrap().latency_status().priming_frames,
            0
        );
    }
    println!("LATENCY_RENDERER {{\"sample_rates\":4,\"compared_frames\":{compared},\"maximum_null_error\":0,\"callback_allocations\":0,\"physical_devices_opened\":false}}");
}

#[test]
fn source_reset_removes_delayed_audio_using_only_bounded_counters() {
    let (model, layout) = model();
    let mut graph = Prepared::at_rate(Arc::new(model), &layout, 48000).unwrap();
    for _ in 0..4800 {
        frame(&mut graph, 0.5, 0.0);
    }
    assert!(frame(&mut graph, 0.5, 0.0) > 0.0);
    assert_eq!(
        crate::engine::test_alloc::measure(|| graph.reset_latency()),
        Default::default()
    );
    assert_eq!(graph.latency_status().priming_frames, 336);
    for _ in 0..4800 {
        assert_eq!(frame(&mut graph, 0.0, 0.0), 0.0);
    }
}

#[test]
fn report_only_edits_retain_running_transport_cancel_atomically_and_prepare_undo_at_changed_rates()
{
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 256);
    let mut model = Model::default();
    model.latency = Some(Configuration {
        reserve_micros: 10000,
        ..Default::default()
    });
    rt.routing = Some(Box::new(
        Prepared::at_rate(Arc::new(model.clone()), &rt.session, 48000).unwrap(),
    ));
    rt.apply(crate::engine::Command::Play);
    let capture = |rt: &mut crate::engine::RtEngine| {
        let handle = engine.project.clone();
        let worker = std::thread::spawn(move || {
            handle
                .capture(&std::sync::atomic::AtomicBool::new(false))
                .unwrap()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !worker.is_finished() {
            rt.process(&mut []);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        worker.join().unwrap()
    };
    let mut next = model.clone();
    next.latency.as_mut().unwrap().reports.push(Report {
        group: Group::Output(1),
        external_micros: 2000,
        processing_micros: 0,
    });
    let (request, ack) = crate::engine::session::Request::routing(
        capture(&mut rt),
        48000,
        Some(Arc::new(next.clone())),
    )
    .unwrap();
    assert!(!request.disruptive());
    let command = crate::engine::Command::session_edit(request);
    assert_eq!(
        crate::engine::test_alloc::measure(
            || rt.apply(command)
        ),
        Default::default()
    );
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    assert!(rt.playing);
    assert_eq!(*rt.routing.as_ref().unwrap().model, next);
    rt.process(&mut [0.0; 1024]);
    let (request, ack) = crate::engine::session::Request::routing(
        capture(&mut rt),
        48000,
        Some(Arc::new(model.clone())),
    )
    .unwrap();
    ack.cancel();
    rt.apply(crate::engine::Command::session_edit(request));
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Cancelled);
    assert_eq!(*rt.routing.as_ref().unwrap().model, next);
    rt.apply(crate::engine::Command::Stop);
    rt.set_sample_rate(44100).unwrap();
    assert_eq!(
        rt.routing.as_ref().unwrap().latency_status().program_frames,
        88
    );
    assert_eq!(
        crate::engine::test_alloc::measure(|| rt.apply(crate::engine::Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.routing.as_ref().unwrap().latency_status().rate, 44100);
    assert_eq!(*rt.routing.as_ref().unwrap().model, model);
    rt.apply(crate::engine::Command::Redo);
    assert_eq!(rt.routing.as_ref().unwrap().latency_status().rate, 44100);
    assert_eq!(*rt.routing.as_ref().unwrap().model, next);
}

#[test]
fn faster_program_and_slower_headphones_have_exact_independent_mic_mix_timing() {
    use crate::engine::{
        audio::routing::{input::Pipe, mic_aux},
        monitor::Control,
        Command, Engine,
    };
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.master = 0.5;
    rt.fx_wet.fill(0.0);
    let mut model = Model::default();
    model.version = 2;
    model.next_id = 4;
    model.ports.extend([
        Port {
            id: 2,
            alias: "Mic".into(),
            direction: Direction::Input,
            channels: vec![0],
        },
        Port {
            id: 3,
            alias: "Headphones".into(),
            direction: Direction::Output,
            channels: vec![2, 3],
        },
    ]);
    model.monitor_output = Some(3);
    model.latency = Some(Configuration {
        reports: vec![Report {
            group: Group::Output(3),
            external_micros: 7000,
            processing_micros: 0,
        }],
        ..Default::default()
    });
    let configuration = mic_aux::Configuration {
        channels: [
            mic_aux::Channel {
                input: Some(2),
                mute: false,
                master: Some(1),
                ..Default::default()
            },
            mic_aux::Channel::default(),
        ],
        ..Default::default()
    };
    configuration.validate(Some(&model)).unwrap();
    rt.routing = Some(Box::new(
        Prepared::at_rate(Arc::new(model), &rt.session, 48000).unwrap(),
    ));
    rt.mic_aux.set(Some(configuration));
    rt.apply(Command::Monitor(Control::Master(true)));
    rt.apply(Command::Monitor(Control::Volume(1.0)));
    rt.routing_pipe = Pipe::controlled_for_test(48000);
    rt.routing_pipe.capture(&[0.0_f32; 128], 1, 0);
    for _ in 0..32 {
        rt.routing_pipe.capture(&[0.0_f32; 64], 1, 0);
        rt.process_interleaved(&mut [0.0; 256], 4);
    }
    let mut rendered = vec![[0.0; 4]; 4096];
    for (block, destination) in rendered.chunks_exact_mut(64).enumerate() {
        let samples: Vec<f32> = (block * 64..(block + 1) * 64)
            .map(|index| {
                if index % 97 < 3 {
                    (index / 97 % 17 + 1) as f32 / 32.0
                } else {
                    0.0
                }
            })
            .collect();
        rt.routing_pipe.capture(&samples, 1, 0);
        let mut output = [0.0; 256];
        assert_eq!(
            crate::engine::test_alloc::measure(|| rt.process_interleaved(&mut output, 4)),
            Default::default()
        );
        for (destination, source) in destination.iter_mut().zip(output.chunks_exact(4)) {
            destination.copy_from_slice(source);
        }
    }
    let offset = rt.routing.as_ref().unwrap().output_delay(1) as usize;
    assert_eq!(offset, 336);
    assert!(rendered.iter().any(|frame| frame[2].abs() > 0.1));
    for index in 0..rendered.len() - offset {
        assert_eq!(
            rendered[index][2..],
            rendered[index + offset][..2],
            "frame {index}"
        );
    }
    println!("LATENCY_MONITOR {{\"compared_frames\":{},\"independent_headphone_delay_frames\":{offset},\"maximum_null_error\":0,\"callback_allocations\":0}}", rendered.len() - offset);
}

#[test]
fn immediate_cue_override_crossfades_only_the_original_pre_fader_deck_source() {
    let (model, layout) = model();
    let mut current = Prepared::at_rate(Arc::new(model.clone()), &layout, 48000).unwrap();
    let deck = current
        .nodes
        .iter()
        .position(|node| node.group == Group::Deck(0))
        .unwrap();
    let source = |index: u32| (index as f32 * 0.017).sin() * 0.2;
    for index in 0..5000 {
        current.begin();
        let frame = current
            .latency
            .as_mut()
            .unwrap()
            .monitor_deck(deck, [source(index), -source(index)]);
        let expected = if index < 336 {
            0.0
        } else {
            source(index - 336)
        };
        assert_eq!(frame, [expected, -expected]);
    }
    let mut next = model;
    next.latency.as_mut().unwrap().low_latency_monitor = true;
    let mut next = Prepared::at_rate(Arc::new(next), &layout, 48000).unwrap();
    next.inherit(&mut current);
    let mut maximum_jump = 0.0_f32;
    let mut prior = source(4999 - 336);
    for index in 5000..6000 {
        next.begin();
        let frame = next
            .latency
            .as_mut()
            .unwrap()
            .monitor_deck(deck, [source(index), -source(index)]);
        maximum_jump = maximum_jump.max((frame[0] - prior).abs());
        prior = frame[0];
        if index >= 5480 {
            assert_eq!(frame, [source(index), -source(index)]);
        }
    }
    assert!(maximum_jump < 0.02, "{maximum_jump}");
    assert_eq!(next.output_delay(1), 336);
    assert_eq!(next.latency_status().monitor_frames, 0);
    println!("LATENCY_CUE_TRANSITION {{\"maximum_sample_jump\":{maximum_jump},\"program_delay_frames\":336,\"transition_ms\":10}}");
}
