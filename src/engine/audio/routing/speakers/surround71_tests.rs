use super::{
    surround_tests::align,
    tests::{capture, fixture_layout},
    *,
};
use crate::engine::{
    audio::routing::{
        model::*,
        selection::{Policy, Selection, Target},
    },
    test_alloc, Command,
};
use std::sync::{atomic::AtomicBool, Arc};

const PHYSICAL: [usize; 8] = [3, 0, 5, 1, 8, 7, 12, 10];
const Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
const FOLD: [[f32; 2]; 8] = [
    [1., 0.],
    [0., 1.],
    [Q, Q],
    [0., 0.],
    [Q, 0.],
    [0., Q],
    [Q, 0.],
    [0., Q],
];

/// Prepare the original mono-to-eight-speaker graph for command and clock equations.
/// Takes no live state; returns a bounded explicit send/return model with no native or device owner.
fn panner_model() -> Model {
    use crate::engine::audio::routing::mixer::Send;
    let mut model = Model::default();
    model.next_id = 5;
    model.ports[0].channels = PHYSICAL.iter().map(|c| *c as u16).collect();
    model.ports.push(Port {
        id: 2,
        alias: "Original mono position source".into(),
        direction: Direction::Input,
        channels: vec![0],
    });
    model.buses.push(Bus {
        id: 3,
        alias: "Original eight-channel return".into(),
        channels: 8,
        gain: 1.,
        mute: false,
        mixer: Default::default(),
    });
    for group in [Group::Bus(3), Group::Output(1)] {
        model.set_speaker_layout(group, Layout::Surround71);
    }
    model.sends.push(Send {
        id: 4,
        name: "Original positional send".into(),
        route: Connection {
            source: Source {
                group: Group::Input(2),
                tap: Tap::PostMixer,
            },
            destination: Group::Bus(3),
            map: [0, 1, 2, 4, 5, 6, 7]
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
        map: (0..8)
            .map(|c| ChannelMap {
                source: c,
                destination: c,
                gain: 1.,
            })
            .collect(),
    }];
    model
}

#[test]
fn surround_71_namespaced_normalized_controls_and_compensated_positional_automation_keep_independent_pcm(
) {
    use crate::engine::audio::routing::{
        latency,
        mixer::{Assignment, Lane, Parameter, Target as MixTarget},
        prepared::Prepared,
    };
    for rate in [44100, 48000, 96000] {
        let (_engine, mut rt) = fixture_layout(rate, Layout::Surround71);
        rt.routing = Some(Box::new(
            Prepared::at_rate(Arc::new(panner_model()), &rt.session, rate).unwrap(),
        ));
        let command = Assignment {
            namespace: rt.session.namespace,
            target: MixTarget::Send(4),
            parameter: Parameter::Pan,
        }
        .command(0.75);
        assert_eq!(
            test_alloc::measure(|| rt.apply(command)),
            Default::default()
        );
        assert_eq!(
            rt.routing
                .as_ref()
                .unwrap()
                .mix_values(MixTarget::Send(4))
                .unwrap()
                .pan,
            0.5
        );
        rt.routing_input_frame[0] = 0.125;
        let mut graph = rt.routing.take().unwrap();
        let out = graph.render(&mut rt, false, 0., 13);
        for c in 0..8 {
            assert!(
                (out[PHYSICAL[c]] - if c == 7 { 0.125_f32.tanh() } else { 0. }).abs() < 1e-7,
                "Normalized command{rate}/{c}"
            );
        }
        let mut model = panner_model();
        let step = 2. / f64::from(rate);
        model.sends[0].automation = vec![Lane {
            parameter: Parameter::Pan,
            points: vec![[0., -0.5], [240. * step, 0.5]],
        }];
        model.latency = Some(latency::Configuration {
            reports: vec![latency::Report {
                group: Group::Input(2),
                external_micros: 7000,
                processing_micros: 0,
            }],
            ..Default::default()
        });
        let mut graph = Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap();
        let offset = ((u64::from(rate) * 7 + 500) / 1000) as usize;
        assert_eq!(graph.output_delay(1), offset as u32);
        rt.playing = true;
        rt.bpm = 120.;
        let low = (std::f32::consts::PI / 8.).sin();
        let high = (std::f32::consts::PI / 8.).cos();
        for frame in 0..offset + 241 {
            rt.timeline_frames = frame as u64 + 1;
            rt.beat = (frame + 1) as f64 * step;
            rt.midi_beat = rt.beat;
            rt.midi_beat_reference = rt.beat;
            rt.last_midi_step = step;
            rt.routing_input_frame.fill(0.);
            let source = frame.checked_sub(offset);
            if source.is_some_and(|f| [0, 60, 120, 180, 240].contains(&f)) {
                rt.routing_input_frame[0] = 0.125;
            }
            let mut actual = [0.; MAX_PHYSICAL_CHANNELS];
            assert_eq!(
                test_alloc::measure(|| actual = graph.render(&mut rt, false, 0., 13)),
                Default::default()
            );
            let expected = match source {
                Some(0) => [0., 0., 0., 0., 0., 0., 1., 0.],
                Some(60) => [high, 0., 0., 0., 0., 0., low, 0.],
                Some(120) => [0., 0., 1., 0., 0., 0., 0., 0.],
                Some(180) => [0., high, 0., 0., 0., 0., 0., low],
                Some(240) => [0., 0., 0., 0., 0., 0., 0., 1.],
                _ => [0.; 8],
            };
            for c in 0..8 {
                assert!(
                    (actual[PHYSICAL[c]] - (0.125_f32 * expected[c]).tanh()).abs() < 2e-6,
                    "Compensated position{rate}/{frame}/{c}: {}",
                    actual[PHYSICAL[c]]
                );
            }
        }
    }
}

#[test]
fn surround_71_order_pan_fold_and_side_back_conversion_have_independent_coefficients() {
    let layout = Layout::Surround71;
    assert_eq!(layout.wave_mask(), 1599);
    assert_eq!(
        serde_json::to_value(layout.speakers()).unwrap(),
        serde_json::json!([
            "front_left",
            "front_right",
            "front_center",
            "low_frequency",
            "back_left",
            "back_right",
            "side_left",
            "side_right"
        ])
    );
    for c in 0..8 {
        let mut input = [0.; 8];
        input[c] = 1.;
        assert_eq!(layout.downmix(&input), FOLD[c]);
    }
    for (angle, c) in [
        (-150., 4),
        (-90., 6),
        (-30., 0),
        (0., 2),
        (30., 1),
        (90., 7),
        (150., 5),
    ] {
        let gains = layout.pan_frame(angle / 180.);
        for (j, value) in gains.into_iter().enumerate() {
            assert!(
                (value - if j == c { 1. } else { 0. }).abs() < 1e-6,
                "{angle}/{j}"
            );
        }
    }
    for (angle, a, b) in [
        (-180., 5, 4),
        (-120., 4, 6),
        (-60., 6, 0),
        (-15., 0, 2),
        (15., 2, 1),
        (60., 1, 7),
        (120., 7, 5),
        (180., 5, 4),
    ] {
        let gains = layout.pan_frame(angle / 180.);
        for (j, value) in gains.into_iter().enumerate() {
            assert!(
                (value - if j == a || j == b { Q } else { 0. }).abs() < 1e-6,
                "{angle}/{j}"
            );
        }
    }
    for angle in -180..=180 {
        let gains = layout.pan_frame(angle as f32 / 180.);
        assert_eq!(gains[3], 0.);
        assert!((gains.iter().map(|g| g * g).sum::<f32>() - 1.).abs() < 1e-6);
    }
    let maps = layout.maps_to(Layout::Surround51Side);
    for (source, dest, gain) in [
        (0, 0, 1.),
        (1, 1, 1.),
        (2, 2, 1.),
        (3, 3, 1.),
        (4, 4, Q),
        (5, 5, Q),
        (6, 4, Q),
        (7, 5, Q),
    ] {
        let mut input = [0.; 8];
        input[source] = 1.;
        let mut actual = [0.; 6];
        layout.mix_into(Layout::Surround51Side, &input, &mut actual);
        for c in 0..6 {
            assert_eq!(actual[c], if c == dest { gain } else { 0. });
        }
        assert_eq!(
            maps.iter()
                .filter(|m| usize::from(m.source) == source)
                .map(|m| (usize::from(m.destination), m.gain))
                .collect::<Vec<_>>(),
            vec![(dest, gain)]
        );
    }
    let mut up = [0.; 8];
    Layout::Surround51Side.mix_into(layout, &[0.125, 0.25, 0.375, 0.5, 0.625, 0.75], &mut up);
    assert_eq!(up, [0.125, 0.25, 0.375, 0.5, 0., 0., 0.625, 0.75]);
}

#[test]
fn surround_71_clip_bed_level_pan_and_causal_histories_follow_independent_pcm_without_heap() {
    for rate in [44100, 48000, 96000] {
        for delayed in [false, true] {
            for block in [1, 64, 127, 512] {
                let (_engine, mut rt) = fixture_layout(rate, Layout::Surround71);
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
                let mut out = vec![0.; (4096 + delay) * 13];
                assert_eq!(
                    test_alloc::measure(|| {
                        for values in out.chunks_mut(block * 13) {
                            rt.process_interleaved(values, 13);
                        }
                    }),
                    Default::default()
                );
                for (frame, values) in out.chunks_exact(13).enumerate() {
                    for c in 0..13 {
                        let expected = PHYSICAL.iter().position(|p| *p == c).map_or(0., |j| {
                            if frame == 8 + 13 * j + delay {
                                ((j + 1) as f32 / 32. * 0.5 * if j == 0 { Q } else { 1. } * 0.25)
                                    .tanh()
                            } else {
                                0.
                            }
                        });
                        assert!(
                            (values[c] - expected).abs() < 2e-6,
                            "{rate}/{delayed}/{block}/{frame}/{c}: {} != {expected}",
                            values[c]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn surround_71_monitor_split_averages_seven_full_range_channels_and_preserves_lfe_without_heap() {
    for rate in [44100, 48000, 96000] {
        let mut model = Model::default();
        model.ports[0].channels = PHYSICAL.iter().map(|c| *c as u16).collect();
        model.set_speaker_layout(Group::Output(1), Layout::Surround71);
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
        let mut low = [0_f32; 8];
        for frame in 0..2048 {
            let source: [f32; 8] = std::array::from_fn(|c| {
                if frame == 8 + 13 * c {
                    (c + 1) as f32 / 32.
                } else {
                    0.
                }
            });
            let mut actual = [0.; MAX_PHYSICAL_CHANNELS];
            for c in 0..8 {
                actual[PHYSICAL[c]] = source[c];
                if c != 3 {
                    low[c] = (1. - a) * low[c] + a * source[c];
                }
            }
            assert_eq!(
                test_alloc::measure(|| monitor.process(&mut actual)),
                Default::default()
            );
            let sub = source[3]
                + 0.75 * (low[0] + low[1] + low[2] + low[4] + low[5] + low[6] + low[7]) / 7.;
            for c in 0..8 {
                assert!(
                    (actual[PHYSICAL[c]] - if c == 3 { sub } else { source[c] - low[c] }).abs()
                        < 1e-7,
                    "{rate}/{frame}/{c}"
                );
            }
            for c in [2, 4, 6, 9, 11, 63] {
                assert_eq!(actual[c], 0.);
            }
        }
    }
}

#[test]
fn surround_71_program_pfl_headphone_fold_includes_back_and_side_at_the_original_causal_time() {
    use crate::engine::{
        audio::routing::prepared::Prepared,
        monitor::{Control, Source as MonitorSource},
    };
    for rate in [44100, 48000, 96000] {
        for pfl in [false, true] {
            let (_engine, mut rt) = fixture_layout(rate, Layout::Surround71);
            let delay = align(&mut rt, rate);
            let mut model = (*rt.routing.as_ref().unwrap().model).clone();
            model.version = 2;
            model.next_id = 3;
            model.ports.push(Port {
                id: 2,
                alias: "Independent software headphone reference".into(),
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
            rt.process_interleaved(&mut vec![0.; 4096 * 22], 22);
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
                    for j in 0..8 {
                        if frame == 8 + 13 * j + delay {
                            expected +=
                                (j + 1) as f32 / 32. * FOLD[j][c] * if pfl { 1. } else { 0.25 };
                        }
                    }
                    assert!(
                        (output[frame * 22 + 20 + c] - expected.tanh()).abs() < 2e-6,
                        "{rate}/pfl{pfl}/{frame}/{c}"
                    );
                }
            }
        }
    }
}

#[test]
fn surround_71_retained_native_archive_fresh_process_raw_contributions_and_folds_trim_once() {
    let retained = std::env::var_os("OMATAINER_ROUTING_RECEIPT_DIR").map(std::path::PathBuf::from);
    let root = retained
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "surround71-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
    assert!(root.starts_with("/home"));
    std::fs::create_dir_all(&root).unwrap();
    for rate in [44100, 48000, 96000] {
        let (engine, mut rt) = fixture_layout(rate, Layout::Surround71);
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
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "engine::audio::routing::speakers::tests::stereo_lfe_fresh_process_renderer",
                "--nocapture"
            ])
            .env("OMATAINER_SPEAKER_ARCHIVE", &path)
            .env("OMATAINER_SPEAKER_RATE", rate.to_string())
            .status()
            .unwrap()
            .success());
        let contract: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.with_extension("contract.json")).unwrap())
                .unwrap();
        assert_eq!(contract["channel_count"], 8);
        assert_eq!(contract["wave_mask"], 1599);
        assert_eq!(contract["channels"], serde_json::json!(PHYSICAL));
        assert_eq!(
            contract["speakers"],
            serde_json::json!([
                "front_left",
                "front_right",
                "front_center",
                "low_frequency",
                "back_left",
                "back_right",
                "side_left",
                "side_right"
            ])
        );
        let bytes = std::fs::read(path.with_extension("f32")).unwrap();
        assert_eq!(bytes.len(), 4096 * 8 * 4);
        rt.apply(Command::LaunchScene { scene: 0 });
        let mut live = vec![0.; (4096 + delay) * 13];
        for block in live.chunks_mut(127 * 13) {
            rt.process_interleaved(block, 13);
        }
        for (index, b) in bytes.chunks_exact(4).enumerate() {
            let f = index / 8;
            let c = index % 8;
            let expected = if f == 8 + 13 * c {
                ((c + 1) as f32 / 32. * 0.25).tanh()
            } else {
                0.
            };
            let actual = f32::from_le_bytes(b.try_into().unwrap());
            assert!((actual - expected).abs() < 2e-6);
            assert_eq!(
                actual,
                live[(f + delay) * 13 + PHYSICAL[c]],
                "{rate}/{f}/{c} native null"
            );
        }
        let mut artifacts = vec![
            path.clone(),
            path.with_extension("f32"),
            path.with_extension("contract.json"),
        ];
        for (width, policy) in [
            (8, Some(Policy::PostDevice)),
            (8, Some(Policy::Dry)),
            (1, None),
            (2, None),
        ] {
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
            let selection = policy.map(|policy| Selection {
                namespace: captured.state.session.as_ref().unwrap().namespace,
                targets: vec![Target::Track(
                    captured.state.session.as_ref().unwrap().tracks[0].id,
                )],
                policy,
            });
            let raw = root.join(format!("selected-{rate}-{width}-{policy:?}.f32"));
            crate::audio_delivery::render_selected(
                crate::engine::project::Captured {
                    state: captured.state.clone(),
                    media: captured.media.clone(),
                    revision: captured.revision,
                    checkpoint: captured.checkpoint,
                    playback_receipts: captured.playback_receipts.clone(),
                },
                &request,
                selection.as_ref(),
                &raw,
                (0, 4096, 0, 4096),
                &Arc::new(AtomicBool::new(false)),
                &crate::background::Reporter::default(),
            )
            .unwrap();
            let bytes = std::fs::read(&raw).unwrap();
            artifacts.push(raw);
            assert_eq!(bytes.len(), 4096 * usize::from(width) * 4);
            for (index, b) in bytes.chunks_exact(4).enumerate() {
                let frame = index / usize::from(width);
                let c = index % usize::from(width);
                let mut expected = 0_f32;
                for j in 0..8 {
                    if frame == 8 + 13 * j {
                        let x = (j + 1) as f32 / 32.;
                        expected += if width == 8 {
                            if c == j {
                                x
                            } else {
                                0.
                            }
                        } else {
                            (x * 0.25).tanh()
                                * if width == 1 {
                                    0.5 * (FOLD[j][0] + FOLD[j][1])
                                } else {
                                    FOLD[j][c]
                                }
                        };
                    }
                }
                assert!(
                    (f32::from_le_bytes(b.try_into().unwrap()) - expected).abs() < 2e-6,
                    "{rate}/{width}/{policy:?}/{frame}/{c}"
                );
            }
        }
        if let Some(directory) = &retained {
            use sha2::{Digest, Sha256};
            let artifacts=artifacts.iter().map(|p|{let b=std::fs::read(p).unwrap();serde_json::json!({"path":p,"bytes":b.len(),"sha256":format!("{:x}",Sha256::digest(b))})}).collect::<Vec<_>>();
            std::fs::write(directory.join(format!("surround71-{rate}.json")),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software","test":"engine::audio::routing::speakers::surround71_tests::surround_71_retained_native_archive_fresh_process_raw_contributions_and_folds_trim_once","rate":rate,"layout":"surround71","wave_mask":1599,"ordered_speakers":["FL","FR","FC","LFE","BL","BR","SL","SR"],"physical_channels":PHYSICAL,"deliberate_alignment_frames":delay,"equation":"channel j impulse (j+1)/32 at8+13j, master=tanh(.25*x); selected=x; LoRo q center/side/back and LFE0","live_fresh_null":true,"encoded_file":false,"physical_proof":false,"retained_artifacts":artifacts})).unwrap()).unwrap();
        }
    }
    if retained.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
}
