use super::tests::{capture, fixture_layout};
use super::*;
use crate::engine::{
    audio::routing::{latency, model::*, prepared::Prepared},
    test_alloc, Command,
};
use std::sync::{atomic::AtomicBool, Arc};

/// Add a deliberately declared silent feeder delay to exercise the complete causal bed.
/// Takes the fixture renderer and rate; prepares a10ms report on an empty track, returning the independent expected sample delay without any device.
pub(super) fn align(rt: &mut crate::engine::RtEngine, rate: u32) -> usize {
    let mut model = (*rt.routing.as_ref().unwrap().model).clone();
    model.latency = Some(latency::Configuration {
        reserve_micros: 20000,
        low_latency_monitor: false,
        reports: vec![latency::Report {
            group: Group::Track(rt.session.tracks[1].id),
            external_micros: 10000,
            processing_micros: 0,
        }],
    });
    rt.routing = Some(Box::new(
        Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap(),
    ));
    let expected = rate as usize / 100;
    assert_eq!(
        rt.routing.as_ref().unwrap().output_delay(1) as usize,
        expected
    );
    expected
}

#[test]
fn surround_51_canonical_order_fold_position_and_cross_layout_maps_have_independent_coefficients() {
    let layout = Layout::Surround51Side;
    assert_eq!(layout.wave_mask(), 0x60F);
    assert_eq!(
        serde_json::to_value(layout.speakers()).unwrap(),
        serde_json::json!([
            "front_left",
            "front_right",
            "front_center",
            "low_frequency",
            "side_left",
            "side_right"
        ])
    );
    let q = std::f32::consts::FRAC_1_SQRT_2;
    for channel in 0..6 {
        let mut input = [0.; 6];
        input[channel] = 1.;
        let expected = match channel {
            0 => [1., 0.],
            1 => [0., 1.],
            2 => [q, q],
            3 => [0., 0.],
            4 => [q, 0.],
            5 => [0., q],
            _ => unreachable!(),
        };
        assert_eq!(layout.downmix(&input), expected);
    }
    for (angle, channel) in [(-110., 4), (-30., 0), (0., 2), (30., 1), (110., 5)] {
        let gains = layout.pan_frame(angle / 180.);
        for (i, gain) in gains.into_iter().enumerate() {
            assert!(
                (gain - if i == channel { 1. } else { 0. }).abs() < 1e-6,
                "{angle}/{i}"
            );
        }
    }
    for (angle, left, right) in [
        (-180., 5, 4),
        (-70., 4, 0),
        (-15., 0, 2),
        (15., 2, 1),
        (70., 1, 5),
        (180., 5, 4),
    ] {
        let gains = layout.pan_frame(angle / 180.);
        for (i, gain) in gains.into_iter().enumerate() {
            assert!(
                (gain - if i == left || i == right { q } else { 0. }).abs() < 1e-6,
                "{angle}/{i}/{gain}"
            );
        }
    }
    for step in -180..=180 {
        let gains = layout.pan_frame(step as f32 / 180.);
        assert_eq!(gains[3], 0.);
        assert!((gains.iter().map(|g| g * g).sum::<f32>() - 1.).abs() < 1e-6);
    }
    let mut mapped = [0.; 6];
    Layout::StereoLfe.mix_into(layout, &[0.125, 0.25, 0.375], &mut mapped);
    assert_eq!(mapped, [0.125, 0.25, 0., 0.375, 0., 0.]);
    let maps = Layout::StereoLfe.maps_to(layout);
    assert!(maps
        .iter()
        .any(|m| m.source == 2 && m.destination == 3 && m.gain == 1.));
    assert!(!maps.iter().any(|m| m.source == 2 && m.destination == 2));
    let mut output = [0.; 3];
    layout.mix_into(
        Layout::StereoLfe,
        &[0., 0., 0.25, 0.125, 0., 0.],
        &mut output,
    );
    assert_eq!(output, [q * 0.25, q * 0.25, 0.125]);
}

#[test]
fn surround_51_actual_clip_bed_controls_and_causal_histories_follow_independent_pcm_without_heap() {
    let physical = [3, 0, 5, 1, 8, 7];
    for rate in [44100, 48000, 96000] {
        for delayed in [false, true] {
            for block in [1, 64, 127, 512] {
                let (_engine, mut rt) = fixture_layout(rate, Layout::Surround51Side);
                let delay = if delayed { align(&mut rt, rate) } else { 0 };
                rt.apply(Command::TrackGain {
                    track: 0,
                    value: 0.5,
                });
                rt.apply(Command::TrackPan {
                    track: 0,
                    value: 0.75,
                });
                assert_eq!(rt.tracks[0].pan, 0.5);
                rt.apply(Command::LaunchScene { scene: 0 });
                let mut output = vec![0.; (4096 + delay) * 9];
                assert_eq!(
                    test_alloc::measure(|| {
                        for chunk in output.chunks_mut(block * 9) {
                            rt.process_interleaved(chunk, 9);
                        }
                    }),
                    Default::default()
                );
                for (frame, values) in output.chunks_exact(9).enumerate() {
                    for channel in 0..9 {
                        let expected =
                            physical.iter().position(|p| *p == channel).map_or(0., |j| {
                                if frame == 8 + 13 * j + delay {
                                    let balance = if j == 0 {
                                        std::f32::consts::FRAC_1_SQRT_2
                                    } else {
                                        1.
                                    };
                                    ((j + 1) as f32 / 32. * 0.5 * balance * 0.25).tanh()
                                } else {
                                    0.
                                }
                            });
                        assert!(
                            (values[channel] - expected).abs() < 2e-6,
                            "{rate}/{delayed}/{block}/{frame}/{channel}: {} != {expected}",
                            values[channel]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn surround_51_native_archive_fresh_process_selected_render_and_delay_trim_agree() {
    use crate::engine::audio::routing::selection::{Policy, Selection, Target};
    let retained = std::env::var_os("OMATAINER_ROUTING_RECEIPT_DIR").map(std::path::PathBuf::from);
    let root = retained
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "surround51-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
    assert!(root.starts_with("/home"));
    std::fs::create_dir_all(&root).unwrap();
    for rate in [44100, 48000, 96000] {
        let (engine, mut rt) = fixture_layout(rate, Layout::Surround51Side);
        let delay = align(&mut rt, rate);
        let captured = capture(&engine, &mut rt);
        let path = root.join(format!("bed-{rate}.omat"));
        crate::project_file::save(
            &path,
            &crate::project_file::Bundle {
                state: captured.state.clone(),
                media: captured.media.clone(),
            },
            crate::project_file::Overwrite::Never,
            &Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "engine::audio::routing::speakers::tests::stereo_lfe_fresh_process_renderer",
                "--nocapture",
            ])
            .env("OMATAINER_SPEAKER_ARCHIVE", &path)
            .env("OMATAINER_SPEAKER_RATE", rate.to_string())
            .status()
            .unwrap();
        assert!(result.success());
        let contract: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.with_extension("contract.json")).unwrap())
                .unwrap();
        assert_eq!(contract["channel_count"], 6);
        assert_eq!(contract["wave_mask"], 1551);
        assert_eq!(contract["channels"], serde_json::json!([3, 0, 5, 1, 8, 7]));
        assert_eq!(
            contract["speakers"],
            serde_json::json!([
                "front_left",
                "front_right",
                "front_center",
                "low_frequency",
                "side_left",
                "side_right"
            ])
        );
        let bytes = std::fs::read(path.with_extension("f32")).unwrap();
        assert_eq!(bytes.len(), 4096 * 6 * 4);
        let fresh = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        rt.apply(Command::LaunchScene { scene: 0 });
        let mut live = vec![0.; (4096 + delay) * 9];
        for block in live.chunks_mut(127 * 9) {
            rt.process_interleaved(block, 9);
        }
        for frame in 0..4096 {
            for j in 0..6 {
                let expected = if frame == 8 + 13 * j {
                    ((j + 1) as f32 / 32. * 0.25).tanh()
                } else {
                    0.
                };
                assert!(
                    (fresh[frame * 6 + j] - expected).abs() < 2e-6,
                    "fresh {rate}/{frame}/{j}"
                );
                assert_eq!(
                    fresh[frame * 6 + j],
                    live[(frame + delay) * 9 + [3, 0, 5, 1, 8, 7][j]],
                    "trim/null {rate}/{frame}/{j}"
                );
            }
        }
        let mut artifacts = vec![
            path.clone(),
            path.with_extension("contract.json"),
            path.with_extension("f32"),
        ];
        for policy in [Policy::PostDevice, Policy::Dry] {
            let request = crate::audio_delivery::Export {
                end: 4096. / f64::from(rate),
                tail: 0.,
                output_alias: Some(1),
                options: crate::audio_delivery::Options {
                    rate,
                    channels: 6,
                    ..Default::default()
                },
                ..Default::default()
            };
            let selection = Selection {
                namespace: captured.state.session.as_ref().unwrap().namespace,
                targets: vec![Target::Track(
                    captured.state.session.as_ref().unwrap().tracks[0].id,
                )],
                policy,
            };
            let raw = root.join(format!("selected-{rate}-{policy:?}.f32"));
            crate::audio_delivery::render_selected(
                crate::engine::project::Captured {
                    state: captured.state.clone(),
                    media: captured.media.clone(),
                    revision: captured.revision,
                    checkpoint: captured.checkpoint,
                    playback_receipts: captured.playback_receipts.clone(),
                },
                &request,
                Some(&selection),
                &raw,
                (0, 4096, 0, 4096),
                &Arc::new(AtomicBool::new(false)),
                &crate::background::Reporter::default(),
            )
            .unwrap();
            artifacts.push(raw.clone());
            let bytes = std::fs::read(raw).unwrap();
            assert_eq!(bytes.len(), 4096 * 6 * 4);
            for (index, value) in bytes.chunks_exact(4).enumerate() {
                let frame = index / 6;
                let j = index % 6;
                let expected = if frame == 8 + 13 * j {
                    (j + 1) as f32 / 32.
                } else {
                    0.
                };
                assert!(
                    (f32::from_le_bytes(value.try_into().unwrap()) - expected).abs() < 2e-6,
                    "{policy:?}/{rate}/{frame}/{j}"
                );
            }
        }
        for width in [1_u16, 2] {
            let request = crate::audio_delivery::Export {
                end: 4096. / f64::from(rate),
                tail: 0.,
                output_alias: Some(1),
                options: crate::audio_delivery::Options {
                    rate,
                    channels: width,
                    ..Default::default()
                },
                ..Default::default()
            };
            let raw = root.join(format!("fold-{rate}-{width}.f32"));
            artifacts.push(raw.clone());
            crate::audio_delivery::render_selected(
                crate::engine::project::Captured {
                    state: captured.state.clone(),
                    media: captured.media.clone(),
                    revision: captured.revision,
                    checkpoint: captured.checkpoint,
                    playback_receipts: captured.playback_receipts.clone(),
                },
                &request,
                None,
                &raw,
                (0, 4096, 0, 4096),
                &Arc::new(AtomicBool::new(false)),
                &crate::background::Reporter::default(),
            )
            .unwrap();
            let bytes = std::fs::read(&raw).unwrap();
            assert_eq!(bytes.len(), 4096 * usize::from(width) * 4);
            let q = std::f32::consts::FRAC_1_SQRT_2;
            for (index, value) in bytes.chunks_exact(4).enumerate() {
                let frame = index / usize::from(width);
                let channel = index % usize::from(width);
                let mut expected = 0_f32;
                for (j, pair) in [[1., 0.], [0., 1.], [q, q], [0., 0.], [q, 0.], [0., q]]
                    .into_iter()
                    .enumerate()
                {
                    if frame == 8 + 13 * j {
                        let coefficient = if width == 1 {
                            0.5 * (pair[0] + pair[1])
                        } else {
                            pair[channel]
                        };
                        expected += ((j + 1) as f32 / 32. * 0.25).tanh() * coefficient;
                    }
                }
                assert!(
                    (f32::from_le_bytes(value.try_into().unwrap()) - expected).abs() < 2e-6,
                    "File fold post-safety equation{rate}/{width}/{frame}/{channel}"
                );
            }
        }
        if let Some(directory) = &retained {
            use sha2::{Digest, Sha256};
            let artifacts = artifacts.iter().map(|path| {
                let bytes = std::fs::read(path).unwrap();
                serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))})
            }).collect::<Vec<_>>();
            std::fs::write(directory.join(format!("surround51-{rate}.json")),serde_json::to_vec_pretty(&serde_json::json!({
                "schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software",
                "test":"engine::audio::routing::speakers::surround_tests::surround_51_native_archive_fresh_process_selected_render_and_delay_trim_agree",
                "rate":rate,"layout":"surround51_side","wave_mask":1551,"ordered_speakers":["FL","FR","FC","LFE","SL","SR"],
                "physical_channels":[3,0,5,1,8,7],"frames":4096,"deliberate_alignment_frames":delay,
                "equation":"channel j impulse (j+1)/32 at8+13j; master=tanh(.25*x); selected PostDevice/Dry=x",
                "live_fresh_null":true,"explicit_file_fold_after_native_channel_safety":true,"encoded_file":false,"physical_proof":false,"retained_artifacts":artifacts,
                "source_binding":"Exact binary/source belong to canonical helper receipt and frozen cohort plan; case JSON alone is not source proof"
            })).unwrap()).unwrap();
        }
    }
    if retained.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn surround_51_positional_graph_return_controls_and_pre_mixer_taps_keep_independent_bed_equations()
{
    use crate::engine::audio::routing::mixer::{Parameter, Send, Target};
    for rate in [44100, 48000, 96000] {
        let (_engine, mut rt) = fixture_layout(rate, Layout::Surround51Side);
        let mut model = Model::default();
        model.next_id = 5;
        model.ports[0].channels = vec![0, 1, 2, 3, 4, 5];
        model.ports.push(Port {
            id: 2,
            alias: "Mono positional input".into(),
            direction: Direction::Input,
            channels: vec![0],
        });
        model.buses.push(Bus {
            id: 3,
            alias: "Positioned full bed".into(),
            channels: 6,
            gain: 0.5,
            mute: false,
            mixer: Default::default(),
        });
        for group in [Group::Bus(3), Group::Output(1)] {
            model.set_speaker_layout(group, Layout::Surround51Side);
        }
        model.sends.push(Send {
            id: 4,
            name: "Mono position".into(),
            route: Connection {
                source: Source {
                    group: Group::Input(2),
                    tap: Tap::PostMixer,
                },
                destination: Group::Bus(3),
                map: [0, 1, 2, 4, 5]
                    .into_iter()
                    .map(|c| ChannelMap {
                        source: 0,
                        destination: c,
                        gain: 1.,
                    })
                    .collect(),
            },
            level: 1.,
            pan: 0.,
            spatial: true,
            mute: false,
            automation: vec![],
        });
        model.connections = vec![Connection {
            source: Source {
                group: Group::Bus(3),
                tap: Tap::PostMixer,
            },
            destination: Group::Output(1),
            map: (0..6)
                .map(|c| ChannelMap {
                    source: c,
                    destination: c,
                    gain: 1.,
                })
                .collect(),
        }];
        let mut graph = Prepared::at_rate(Arc::new(model.clone()), &rt.session, rate).unwrap();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        graph.mix_control(Target::Return(3), Parameter::Pan, 0.5);
        for (angle, gains) in [
            (-110., [0., 0., 0., 0., 1., 0.]),
            (-30., [1., 0., 0., 0., 0., 0.]),
            (0., [0., 0., 1., 0., 0., 0.]),
            (30., [0., 1., 0., 0., 0., 0.]),
            (110., [0., 0., 0., 0., 0., 1.]),
            (180., [0., 0., 0., 0., q, q]),
        ] {
            graph.mix_control(Target::Send(4), Parameter::Pan, angle / 180.);
            rt.routing_input_frame[0] = 0.125;
            let mut actual = [0.; MAX_PHYSICAL_CHANNELS];
            assert_eq!(
                test_alloc::measure(|| actual = graph.render(&mut rt, false, 0., 6)),
                Default::default()
            );
            for c in 0..6 {
                let balance = if c == 0 { q } else { 1. };
                assert!(
                    (actual[c] - (0.125_f32 * gains[c] * 0.5 * balance).tanh()).abs() < 1e-7,
                    "{rate}/{angle}/{c}"
                );
            }
        }
        graph.mix_control(Target::Return(3), Parameter::Mute, 1.);
        let actual = graph.render(&mut rt, false, 0., 6);
        assert_eq!(&actual[..6], &[0.; 6]);
        let node = graph
            .nodes
            .iter()
            .find(|n| n.group == Group::Bus(3))
            .unwrap();
        assert!(
            (node.taps[Tap::PreFx.index()][4] - 0.125 * q).abs() < 1e-7,
            "Muted return feeder must keep running"
        );
        assert_eq!(&node.taps[Tap::PostMixer.index()][..6], &[0.; 6]);
        model.sends[0].pan = 1.;
        model.buses[0].mute = true;
        model.connections[0].source.tap = Tap::PreFx;
        let mut graph = Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap();
        let output = graph.render(&mut rt, false, 0., 6);
        assert!((output[4] - (0.125 * q).tanh()).abs() < 1e-7);
        assert!((output[5] - (0.125 * q).tanh()).abs() < 1e-7);
        assert_eq!(output[3], 0.);
    }
}

#[test]
fn surround_51_monitor_bass_all_full_range_channels_follow_independent_average_without_heap() {
    for rate in [44100, 48000, 96000] {
        let mut model = Model::default();
        let physical: [usize; 6] = [3, 0, 5, 1, 8, 7];
        model.ports[0].channels = physical.iter().map(|c| *c as u16).collect();
        model.set_speaker_layout(Group::Output(1), Layout::Surround51Side);
        let mut monitor = super::monitor::Runtime::new(
            BassManagement {
                output: 1,
                cutoff: 80.,
                level: 0.75,
            },
            &model,
            rate,
        );
        let a = 1. - (-std::f32::consts::TAU * 80. / rate as f32).exp();
        let mut low = [0_f32; 6];
        for frame in 0..2048 {
            let source: [f32; 6] = std::array::from_fn(|c| {
                if frame == 8 + 13 * c {
                    (c + 1) as f32 / 32.
                } else {
                    0.
                }
            });
            let mut actual = [0.; MAX_PHYSICAL_CHANNELS];
            for c in 0..6 {
                actual[physical[c]] = source[c];
                if c != 3 {
                    low[c] = (1. - a) * low[c] + a * source[c];
                }
            }
            assert_eq!(
                test_alloc::measure(|| monitor.process(&mut actual)),
                Default::default()
            );
            let sub = source[3] + 0.75 * (low[0] + low[1] + low[2] + low[4] + low[5]) / 5.;
            for c in 0..6 {
                let expected = if c == 3 { sub } else { source[c] - low[c] };
                assert!(
                    (actual[physical[c]] - expected).abs() < 1e-7,
                    "{rate}/{frame}/{c}"
                );
            }
            for c in [2, 4, 6, 9, 63] {
                assert_eq!(actual[c], 0.);
            }
        }
    }
}

#[test]
fn surround_51_program_and_pfl_headphone_folds_keep_center_surround_and_original_alignment() {
    use crate::engine::monitor::{Control, Source as MonitorSource};
    let q = std::f32::consts::FRAC_1_SQRT_2;
    for rate in [44100, 48000, 96000] {
        for pfl in [false, true] {
            let (_engine, mut rt) = fixture_layout(rate, Layout::Surround51Side);
            let delay = align(&mut rt, rate);
            let mut model = (*rt.routing.as_ref().unwrap().model).clone();
            model.version = 2;
            model.next_id = 3;
            model.ports.push(Port {
                id: 2,
                alias: "Software headphone reference".into(),
                direction: Direction::Output,
                channels: vec![20, 21],
            });
            model.monitor_output = Some(2);
            rt.routing = Some(Box::new(
                Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap(),
            ));
            rt.monitor.apply(Control::Volume(1.));
            rt.monitor.apply(Control::Master(true));
            if pfl {
                rt.tracks[0].pfl = true;
                rt.monitor.apply(Control::Source(MonitorSource::Pfl));
            }
            let mut silent = vec![0.; 4096 * 22];
            rt.process_interleaved(&mut silent, 22);
            rt.apply(Command::LaunchScene { scene: 0 });
            let mut output = vec![0.; (4096 + delay) * 22];
            assert_eq!(
                test_alloc::measure(|| {
                    for block in output.chunks_mut(127 * 22) {
                        rt.process_interleaved(block, 22);
                    }
                }),
                Default::default()
            );
            for frame in 0..4096 + delay {
                for c in 0..2 {
                    let mut expected = 0_f32;
                    for (j, coefficients) in
                        [[1., 0.], [0., 1.], [q, q], [0., 0.], [q, 0.], [0., q]]
                            .into_iter()
                            .enumerate()
                    {
                        if frame == 8 + 13 * j + delay {
                            expected += (j + 1) as f32 / 32.
                                * coefficients[c]
                                * if pfl { 1. } else { 0.25 };
                        }
                    }
                    assert!(
                        (output[frame * 22 + 20 + c] - expected.tanh()).abs() < 2e-6,
                        "{rate}/pfl{pfl}/{frame}/{c}: {} != {expected}",
                        output[frame * 22 + 20 + c]
                    );
                }
            }
        }
    }
}
