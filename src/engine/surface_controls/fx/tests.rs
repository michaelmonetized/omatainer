use super::*;
use crate::engine::test_alloc;
use crate::engine::{audio::OutputCallback, Command, Engine, Sample};
use std::sync::Arc;

fn echo(state: &mut State, bank: usize, milliseconds: f32) {
    let settings = &mut state.status.fx[bank];
    settings.kinds = [crate::engine::FxKind::Echo; 3];
    settings.on = [true, false, false];
    settings.wet = [1.0; 3];
    settings.parameter = [0.0; 3];
    settings.timing = Timing::Manual;
    settings.manual_ms = milliseconds;
    for source in &mut state.deck_fx[bank] {
        for processor in source {
            processor.parameter(state.fx_rate, 0.0);
        }
    }
}

#[test]
fn manual_echo_arrivals_and_disengaged_stereo_tails_keep_exact_independent_clocks_without_heap_work(
) {
    let mut compared = 0;
    for rate in [8000, 32000, 44100, 48000, 96000, 192000] {
        let mut state = Box::new(State::new(rate as f32).unwrap());
        echo(&mut state, 0, 250.0);
        let spb = f64::from(rate) * 60.0 / 117.0;
        for _ in 0..rate / 100 {
            state.deck(0, [0.0; 2], spb);
        }
        assert_eq!(state.deck(0, [1.0, -0.5], spb), [0.0; 2]);
        state.status.fx[0].on[0] = false;
        let delay = rate / 4;
        assert_eq!(
            test_alloc::measure(|| {
                for frame in 1..=delay + rate / 100 {
                    let output = state.deck(0, [0.0; 2], spb * 2.0);
                    assert_eq!(
                        output,
                        if frame == delay {
                            [1.0, -0.5]
                        } else {
                            [0.0; 2]
                        },
                        "rate{rate} frame{frame}"
                    );
                    assert_eq!(state.deck(1, [0.0; 2], spb), [0.0; 2]);
                    compared += 1;
                }
            }),
            Default::default()
        );
        assert!(state.status.fx[0].tails[0]);
        for _ in 0..rate * 3 {
            state.deck(0, [0.0; 2], spb);
        }
        assert!(!state.status.fx[0].tails[0]);
        assert_eq!(state.deck(0, [0.37, -0.29], spb), [0.37, -0.29]);
    }
    println!("DJ_FX_TAILS {{\"sample_rates\":6,\"compared_frames\":{compared},\"maximum_sample_error\":0,\"callback_allocations\":0,\"physical_devices_opened\":false}}");
}

#[test]
fn beat_clock_manual_time_and_all_slot_parameters_are_explicit_and_bounded() {
    let mut state = EffectBank::default();
    for rate in [8000, 44100, 96000, 192000] {
        for bpm in [40.0, 120.0, 300.0] {
            let spb = f64::from(rate) * 60.0 / bpm;
            for beats in -4..=3 {
                state.beats = beats;
                let expected = (spb * 0.75 * 2_f64.powi(i32::from(beats)))
                    .clamp(1.0, f64::from(rate) * 2.0 - 2.0) as f32;
                assert_eq!(state.frames(spb, rate as f32), expected);
            }
            state.timing = Timing::Manual;
            state.manual_ms = 321.0;
            assert_eq!(
                state.frames(spb, rate as f32),
                (f64::from(rate) * 0.321) as f32
            );
            state.timing = Timing::Beat;
        }
    }
    for invalid in [
        Control::ManualMs(f32::NAN),
        Control::ManualMs(0.0),
        Control::ManualMs(1999.0),
        Control::Beats(4),
        Control::Deck {
            deck: 2,
            enabled: true,
        },
        Control::Enabled {
            slot: 3,
            enabled: true,
        },
    ] {
        assert!(!invalid.valid());
    }
}

#[test]
fn independent_sampler_routes_keep_original_bus_tails_and_reject_stale_or_replaced_targets() {
    let (_engine, mut rt) = Engine::headless_for_test(48000, 256);
    let first = rt.session.reference(Axis::Track, 0).unwrap();
    let second = rt.session.reference(Axis::Track, 1).unwrap();
    assert!(rt.dj_fx_control(
        0,
        Control::Sampler {
            target: Some(first)
        }
    ));
    assert!(rt.dj_fx_control(
        1,
        Control::Sampler {
            target: Some(second)
        }
    ));
    echo(&mut rt.surface, 0, 10.0);
    echo(&mut rt.surface, 1, 20.0);
    rt.surface.resolve_fx_samplers(&rt.session);
    for _ in 0..480 {
        let mut buses = [[0.0; 2]; crate::engine::session::MAX_TRACKS];
        rt.surface
            .sampler_fx_at(&mut buses, Placement::PreFader, 24000.0, 48000.0);
    }
    let mut buses = [[0.0; 2]; crate::engine::session::MAX_TRACKS];
    buses[0] = [1.0, 0.0];
    buses[1] = [0.0, -0.5];
    buses[2] = [0.31, -0.41];
    rt.surface
        .sampler_fx_at(&mut buses, Placement::PreFader, 24000.0, 48000.0);
    assert_eq!(buses[0], [0.0; 2]);
    assert_eq!(buses[1], [0.0; 2]);
    assert_eq!(buses[2], [0.31, -0.41]);
    assert!(rt.dj_fx_control(0, Control::Sampler { target: None }));
    assert!(rt.dj_fx_control(1, Control::Sampler { target: None }));
    assert!(!rt.dj_fx_control(
        0,
        Control::Sampler {
            target: Some(second)
        }
    ));
    assert_eq!(
        test_alloc::measure(|| {
            for frame in 1..=960 {
                let mut buses = [[0.0; 2]; crate::engine::session::MAX_TRACKS];
                buses[2] = [0.31, -0.41];
                rt.surface
                    .sampler_fx_at(&mut buses, Placement::PreFader, 24000.0, 48000.0);
                assert_eq!(buses[0], if frame == 480 { [1.0, 0.0] } else { [0.0; 2] });
                assert_eq!(buses[1], if frame == 960 { [0.0, -0.5] } else { [0.0; 2] });
                assert_eq!(buses[2], [0.31, -0.41]);
            }
        }),
        Default::default()
    );
    rt.session.namespace = [999, 998];
    rt.surface.resolve_fx_samplers(&rt.session);
    assert_eq!(rt.surface.sampler_slots, [None; 2]);
    assert!(!rt.surface.status.fx[0].tails[2]);
    assert!(!rt.dj_fx_control(
        0,
        Control::Sampler {
            target: Some(first)
        }
    ));
}

fn pulse(rate: u32) -> Arc<Sample> {
    let mut data = vec![0.0; rate as usize * 2];
    data[2200] = 0.2;
    data[2201] = -0.1;
    Arc::new(Sample {
        name: "Stereo FX impulse".into(),
        path: String::new(),
        sr: rate,
        ch: 2,
        data,
        peaks: Vec::new().into(),
        spectrum: None,
        bpm: 120.0,
    })
}
fn render_crossfader_case(placement: Placement, graph: bool) -> Vec<f32> {
    let (_engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: pulse(48000),
    });
    rt.apply(Command::DeckGain {
        deck: 0,
        value: 1.0,
    });
    rt.apply(Command::Master(0.25));
    rt.apply(Command::Xfader(0.0));
    rt.decks[0].sync = false;
    assert!(rt.dj_fx_control(0, Control::Placement(placement)));
    echo(&mut rt.surface, 0, 250.0);
    if graph {
        rt.routing = Some(Box::new(
            crate::engine::audio::routing::prepared::Prepared::at_rate(
                Arc::new(Default::default()),
                &rt.session,
                48000,
            )
            .unwrap(),
        ));
    }
    rt.apply(Command::DeckPlay { deck: 0 });
    let mut warm = vec![0.0; 4000];
    rt.process(&mut warm);
    rt.apply(Command::Xfader(1.0));
    rt.apply(Command::DeckPlay { deck: 0 });
    let mut output = vec![0.0; 26000];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut output)),
        Default::default()
    );
    output
}

#[test]
fn actual_renderer_post_crossfader_tail_survives_a_cut_and_pause_in_both_native_graphs() {
    let mut cases = 0;
    for graph in [false, true] {
        let before = render_crossfader_case(Placement::PreFader, graph);
        let after = render_crossfader_case(Placement::PostFader, graph);
        assert!(before.iter().all(|value| value.abs() < 1e-6));
        let peak = after.iter().map(|value| value.abs()).fold(0.0, f32::max);
        assert!(
            (0.04..0.06).contains(&peak),
            "Post-fader paused tail peak {peak}"
        );
        for frame in after.chunks_exact(2) {
            assert!((f64::from(frame[0]).atanh() + f64::from(frame[1]).atanh() * 2.0).abs() < 1e-6);
        }
        cases += 1;
    }
    println!("DJ_FX_RENDERER {{\"graph_cases\":{cases},\"cut_and_pause_tails\":true,\"compared_frames\":52000,\"callback_allocations\":0,\"physical_devices_opened\":false}}");
}

#[test]
fn actual_output_callback_drains_independent_master_sources_without_changing_other_deck() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: pulse(48000),
    });
    rt.apply(Command::DeckGain {
        deck: 0,
        value: 1.0,
    });
    rt.apply(Command::Master(0.25));
    rt.apply(Command::Xfader(0.0));
    rt.decks[0].sync = false;
    assert!(rt.dj_fx_control(
        0,
        Control::Deck {
            deck: 0,
            enabled: false
        }
    ));
    assert!(rt.dj_fx_control(0, Control::Master(true)));
    assert!(rt.dj_fx_control(0, Control::Placement(Placement::PostFader)));
    echo(&mut rt.surface, 0, 50.0);
    rt.apply(Command::DeckPlay { deck: 0 });
    let other = (rt.decks[1].history_key, rt.decks[1].pos, rt.decks[1].gain);
    let mut callback = OutputCallback::new(rt, 2);
    let mut observed: Vec<f32> = Vec::new();
    for _ in 0..40 {
        let mut block = [0.0; 256];
        assert_eq!(
            test_alloc::measure(|| callback.render(&mut block)),
            Default::default()
        );
        observed.extend(block);
    }
    engine
        .send(Command::Surface(Input::DjFx {
            bank: 0,
            control: Control::Enabled {
                slot: 0,
                enabled: false,
            },
        }))
        .unwrap();
    for _ in 0..40 {
        let mut block = [0.0; 256];
        assert_eq!(
            test_alloc::measure(|| callback.render(&mut block)),
            Default::default()
        );
        observed.extend(block);
    }
    let state = callback.renderer_mut_for_test();
    assert_eq!(
        (
            state.decks[1].history_key,
            state.decks[1].pos,
            state.decks[1].gain
        ),
        other
    );
    let peak = observed.iter().map(|value| value.abs()).fold(0.0, f32::max);
    assert!((0.04..0.06).contains(&peak));
    for frame in observed.chunks_exact(2) {
        assert!((f64::from(frame[0]).atanh() + f64::from(frame[1]).atanh() * 2.0).abs() < 1e-6);
    }
    println!("DJ_FX_CALLBACK {{\"compared_frames\":10240,\"callback_allocations\":0,\"callback_frees\":0,\"other_deck_unchanged\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn actual_rate_reconstruction_preserves_reviewed_controls_but_retires_old_histories() {
    let (_engine, mut rt) = Engine::headless_for_test(44100, 256);
    let target = rt.session.reference(Axis::Track, 0).unwrap();
    assert!(rt.dj_fx_control(
        0,
        Control::Sampler {
            target: Some(target)
        }
    ));
    assert!(rt.dj_fx_control(0, Control::Master(true)));
    echo(&mut rt.surface, 0, 10.0);
    let controls = rt.surface.status.fx[0];
    rt.surface.deck(0, [1.0, 0.0], 22050.0);
    rt.set_sample_rate(96000).unwrap();
    assert_eq!(rt.surface.fx_rate, 96000.0);
    assert_eq!(rt.surface.status.fx[0].sampler, controls.sampler);
    assert_eq!(rt.surface.status.fx[0].manual_ms, controls.manual_ms);
    assert_eq!(rt.surface.status.fx[0].parameter, controls.parameter);
    assert_eq!(rt.surface.status.fx[0].on, controls.on);
    assert_eq!(rt.surface.status.fx[0].tails, [false; 4]);
    for _ in 0..960 {
        assert_eq!(rt.surface.deck(0, [0.0; 2], 48000.0), [0.0; 2]);
    }
    assert_eq!(rt.surface.status.fx[0].frames(48000.0, 96000.0), 960.0);
}
