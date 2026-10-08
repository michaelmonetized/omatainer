use super::{
    tests::{block, control, engine},
    *,
};
use crate::engine::{
    audio::routing::{
        model::{Direction, Model, Port},
        prepared::Prepared,
    },
    Command,
};
use std::sync::Arc;

fn routed(rt: &mut super::super::RtEngine, pair: [u16; 2]) -> Model {
    let mut model = Model::default();
    model.version = 2;
    model.next_id = 3;
    model.ports.push(Port {
        id: 2,
        alias: "Headphones".into(),
        direction: Direction::Output,
        channels: pair.to_vec(),
    });
    model.monitor_output = Some(2);
    rt.routing = Some(Box::new(
        Prepared::new(Arc::new(model.clone()), &rt.session).unwrap(),
    ));
    model
}

#[test]
fn pfl_level_blend_split_and_selection_never_change_program_samples_in_either_renderer() {
    for graph in [false, true] {
        let mut actual = engine();
        let mut reference = engine();
        if graph {
            routed(&mut actual, [2, 3]);
            routed(&mut reference, [2, 3]);
        }
        for (blend, volume, split, a, b) in [
            (0.0, 1.0, false, true, false),
            (1.0, 0.25, false, false, true),
            (0.5, 0.0, true, true, true),
            (0.8, 1.0, true, false, false),
            (0.0, 0.5, false, true, true),
        ] {
            actual.apply(Command::CueMix(blend));
            for v in [
                Control::Volume(volume),
                Control::Split(split),
                Control::Pfl {
                    deck: 0,
                    enabled: a,
                },
                Control::Pfl {
                    deck: 1,
                    enabled: b,
                },
            ] {
                control(&mut actual, v);
            }
            let a = block(&mut actual);
            let b = block(&mut reference);
            for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
                assert_eq!(a[..2], b[..2], "graph {graph}");
            }
            assert_eq!(actual.monitor.status.source, Source::Pfl);
        }
    }
}

#[test]
fn selected_cues_are_prefader_and_multiple_selections_sum_before_split_mono_fold() {
    for graph in [false, true] {
        let mut rt = engine();
        if graph {
            routed(&mut rt, [2, 3]);
        }
        for deck in [0, 1] {
            control(&mut rt, Control::Fader { deck, value: 0.0 });
            control(
                &mut rt,
                Control::Pfl {
                    deck,
                    enabled: true,
                },
            );
        }
        control(&mut rt, Control::Source(Source::Pfl));
        control(&mut rt, Control::Volume(0.5));
        let output = block(&mut rt);
        let last = &output[output.len() - 4..];
        assert_eq!(last[..2], [0.0; 2]);
        for (value, expected) in last[2..].iter().zip([0.2_f32, 0.3]) {
            assert!((*value - expected.tanh()).abs() < 0.0003, "{last:?}");
        }
        control(&mut rt, Control::Split(true));
        let output = block(&mut rt);
        let last = &output[output.len() - 4..];
        assert!((last[2] - 0.25_f32.tanh()).abs() < 0.0003, "{last:?}");
        assert_eq!(last[3], 0.0);
        control(
            &mut rt,
            Control::Fader {
                deck: 1,
                value: 1.0,
            },
        );
        let output = block(&mut rt);
        let last = &output[output.len() - 4..];
        assert!((last[2] - 0.25_f32.tanh()).abs() < 0.0003, "{last:?}");
        assert!((last[3] - 0.0875_f32.tanh()).abs() < 0.0003, "{last:?}");
    }
}

#[test]
fn explicit_pair_retains_exact_channels_and_missing_pair_never_uses_native_fallback() {
    let mut rt = engine();
    routed(&mut rt, [4, 7]);
    control(&mut rt, Control::Source(Source::Pfl));
    control(
        &mut rt,
        Control::Pfl {
            deck: 0,
            enabled: true,
        },
    );
    control(&mut rt, Control::Volume(1.0));
    let mut output = vec![0.0; 4096 * 8];
    rt.process_interleaved(&mut output, 8);
    let last = &output[output.len() - 8..];
    assert_eq!(rt.monitor.status.channels, Some([4, 7]));
    assert!(rt.monitor.status.available);
    assert!(last[4] > 0.05 && last[7] > 0.1);
    for c in [2, 3, 5, 6] {
        assert_eq!(last[c], 0.0);
    }
    rt.process_interleaved(&mut output[..4096 * 6], 6);
    assert!(!rt.monitor.status.available);
    assert_eq!(rt.monitor.status.channels, Some([4, 7]));
    assert!(output[..4096 * 6]
        .chunks_exact(6)
        .all(|frame| frame[2..].iter().all(|v| *v == 0.0)));
    rt.process_interleaved(&mut [], 2);
    assert!(!rt.monitor.status.available);
    rt.process_interleaved(&mut output, 8);
    assert!(rt.monitor.status.available);
}

#[test]
fn monitor_alias_validation_uses_actual_maps_and_preserves_versioned_project_roundtrips() {
    let mut rt = engine();
    let mut model = routed(&mut rt, [4, 5]);
    model.ports[0].channels = vec![0, 1, 4, 5];
    assert!(model.order(&rt.session).is_ok());
    let good = model.clone();
    model.connections[0].map[0].destination = 2;
    assert!(model.order(&rt.session).unwrap_err().contains("overlap"));
    for pair in [vec![0, 1], vec![4], vec![4, 5, 6]] {
        let mut invalid = good.clone();
        invalid.ports[1].channels = pair;
        assert!(invalid.order(&rt.session).is_err());
    }
    let mut invalid = good.clone();
    invalid.version = 1;
    assert!(invalid.order(&rt.session).is_err());
    for version in [0, u32::MAX] {
        invalid = good.clone();
        invalid.version = version;
        assert!(invalid.order(&rt.session).is_err());
    }
    let old = serde_json::to_value(Model::default()).unwrap();
    assert!(old.get("monitor_output").is_none());
    assert_eq!(
        serde_json::from_value::<Model>(old).unwrap().monitor_output,
        None
    );
    for version in [2, 3] {
        let mut expected = good.clone();
        expected.version = version;
        assert!(expected.order(&rt.session).is_ok());
        let bytes = serde_json::to_vec(&expected).unwrap();
        let model = serde_json::from_slice(&bytes).unwrap();
        let mut prepared = crate::engine::project::Prepared::empty(96_000).unwrap();
        prepared.rt.routing = Some(Box::new(
            Prepared::new(Arc::new(model), &prepared.rt.session).unwrap(),
        ));
        assert_eq!(*prepared.rt.routing.as_ref().unwrap().model, expected);
        prepared.swap_into(&mut rt);
        rt.process_interleaved(&mut [], 2);
        assert_eq!(rt.monitor.status.channels, Some([4, 5]));
        assert!(!rt.monitor.status.available);
    }
}

#[test]
fn quiet_routing_checks_are_finite_separate_and_cancel_on_playback_routes_safety_or_output_changes()
{
    for rate in [44_100, 48_000, 96_000] {
        let mut rt = engine();
        rt.sr = rate as f32;
        rt.master = 0.0;
        for d in &mut rt.decks {
            d.playing = false;
        }
        routed(&mut rt, [4, 5]);
        rt.process_interleaved(&mut [], 6);
        for channel in [0, 1] {
            control(&mut rt, Control::Tone(channel));
            assert_eq!(rt.monitor.status.tone, Some(channel));
            let mut output = vec![0.0; (rate as usize + 256) * 6];
            let heap =
                crate::engine::test_alloc::measure(|| rt.process_interleaved(&mut output, 6));
            assert_eq!(heap, crate::engine::test_alloc::Counts::default());
            assert_eq!(rt.monitor.status.tone, None);
            let mut energy = 0.0;
            for (index, frame) in output.chunks_exact(6).enumerate() {
                for (c, value) in frame.iter().enumerate() {
                    assert!(value.is_finite());
                    assert!(value.abs() <= 0.01001);
                    if c != usize::from(channel) + 4 || index >= rate as usize {
                        assert_eq!(*value, 0.0);
                    } else {
                        energy += value * value;
                    }
                }
            }
            assert!(energy > 0.1);
        }
        for fence in 0..7 {
            let mut rt = engine();
            rt.sr = rate as f32;
            rt.master = 0.0;
            for deck in &mut rt.decks {
                deck.playing = false;
            }
            routed(&mut rt, [4, 5]);
            rt.process_interleaved(&mut [], 6);
            control(&mut rt, Control::Tone(0));
            assert_eq!(rt.monitor.status.tone, Some(0));
            match fence {
                0 => control(&mut rt, Control::CancelTone),
                1 => {
                    rt.routing_pipe.recorder.invalidate();
                    rt.process_interleaved(&mut [], 6);
                }
                2 => {
                    rt.apply(Command::SafetyStop(
                        crate::engine::performance::Safety::Stop,
                    ));
                }
                3 => {
                    rt.decks[0].playing = true;
                    rt.process_interleaved(&mut [], 6);
                    rt.decks[0].playing = false;
                }
                4 => {
                    let mut prepared = crate::engine::project::Prepared::empty(rate).unwrap();
                    prepared.swap_into(&mut rt);
                }
                5 => rt.monitor.output(&crate::engine::audio::config::Plan {
                    backend: "ALSA".into(),
                    device: "Software fixture".into(),
                    channels: 2,
                    rate,
                    format: cpal::SampleFormat::F32,
                    buffer: Some(64),
                    warning: None,
                    graph: Default::default(),
                }),
                _ => {
                    rt.playing = true;
                    assert_eq!(rt.render_monitor([0.0; 2], [0.0; 2]), [0.0; 2]);
                }
            }
            assert_eq!(rt.monitor.status.tone, None);
        }
    }
}

#[test]
fn monitor_controls_and_warmed_routed_callback_have_no_heap_work() {
    let mut rt = engine();
    routed(&mut rt, [2, 3]);
    control(&mut rt, Control::Source(Source::Pfl));
    control(&mut rt, Control::Volume(0.5));
    block(&mut rt);
    let mut output = [0.0; 256 * 4];
    let heap = crate::engine::test_alloc::measure(|| {
        for blend in [0.0, 0.5, 1.0] {
            for v in [
                Control::Blend(blend),
                Control::Split(blend > 0.0),
                Control::Pfl {
                    deck: 0,
                    enabled: true,
                },
                Control::Pfl {
                    deck: 1,
                    enabled: blend > 0.0,
                },
            ] {
                rt.apply(Command::Monitor(v));
            }
            rt.process_interleaved(&mut output, 4);
        }
    });
    assert_eq!(heap, crate::engine::test_alloc::Counts::default());
    for control in [
        Control::Volume(f32::NAN),
        Control::Blend(f32::INFINITY),
        Control::Tone(2),
        Control::Pfl {
            deck: 2,
            enabled: true,
        },
    ] {
        assert!(!control.valid());
    }
}
