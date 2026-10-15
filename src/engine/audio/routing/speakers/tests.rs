use super::*;
use crate::engine::{
    audio::routing::{model::*, prepared::Prepared},
    test_alloc, ClipKind, Command, Engine, Sample,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Capture one immutable revision while servicing only the headless renderer.
/// Takes a fixture engine and renderer; returns coherent saved identity and media without opening a device.
pub(crate) fn capture(
    engine: &Engine,
    rt: &mut crate::engine::RtEngine,
) -> crate::engine::project::Captured {
    let project = engine.project.clone();
    let task = std::thread::spawn(move || project.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !task.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    task.join().unwrap()
}

#[test]
fn stereo_lfe_declared_contract_preserves_order_and_rejects_ambiguous_widths() {
    let legacy: super::super::model::Model = serde_json::from_str(
        &serde_json::to_string(&super::super::model::Model::default()).unwrap(),
    )
    .unwrap();
    assert!(legacy.speaker_layouts.is_empty());
    assert!(!serde_json::to_string(&legacy)
        .unwrap()
        .contains("speaker_layouts"));
    let mut model = legacy;
    model.ports[0].channels = vec![3, 0, 5];
    assert!(model.channel_contract(Group::Output(1), 3).is_err());
    model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
    let contract = model.channel_contract(Group::Output(1), 3).unwrap();
    assert_eq!(contract.channels, [3, 0, 5]);
    assert_eq!(contract.channel_count, 3);
    assert_eq!(contract.wave_mask, Some(0xB));
    assert_eq!(
        serde_json::to_value(&contract.speakers).unwrap(),
        serde_json::json!(["front_left", "front_right", "low_frequency"])
    );
    assert_eq!(
        model
            .channel_contract(Group::Output(1), 2)
            .unwrap()
            .speakers,
        [Speaker::FrontLeft, Speaker::FrontRight]
    );
    assert!(model.channel_contract(Group::Output(1), 6).is_err());
    model.speaker_layouts.push(model.speaker_layouts[0]);
    assert!(model.channel_contract(Group::Output(1), 3).is_err());
    assert_eq!(Layout::StereoLfe.downmix(&[0.2, -0.3, 0.9]), [0.2, -0.3]);
}

/// Prepare the original three-channel graph fixture without a device.
/// Takes the native rate; returns its headless engine and renderer with unequal channel impulses at frames8/21/34 and physical order3/0/5.
pub(crate) fn fixture(rate: u32) -> (Engine, crate::engine::RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(rate, 256);
    rt.apply(Command::Stop);
    rt.quantize = false;
    rt.bpm = 120.;
    rt.master = 0.25;
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            *clip = crate::engine::Clip::empty();
        }
        track.fx.slots.clear();
    }
    for rack in &mut rt.scene_fx {
        rack.slots.clear();
    }
    let mut data = vec![0.; 4096 * 3];
    for channel in 0..3 {
        data[(8 + 13 * channel) * 3 + channel] = (channel + 1) as f32 / 32.;
    }
    let sample = Arc::new(Sample {
        name: "Original ordered speaker impulses".into(),
        sr: rate,
        ch: 3,
        data,
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.,
        path: String::new(),
    });
    let region = crate::engine::audio_clip::Region::full(&sample, 120.)
        .unwrap()
        .prepare(&sample)
        .unwrap();
    let track = &mut rt.tracks[0];
    track.gain = 1.;
    track.pan = 0.;
    track.kind = 4;
    track.eq.low_g = 1.;
    track.eq.mid_g = 1.;
    track.eq.high_g = 1.;
    track.clips[0].kind = ClipKind::Audio;
    track.clips[0].audio = Some(sample);
    track.clips[0].audio_region = Some(region);
    track.clips[0].bars = region.duration_beats as f32 / 4.;
    let mut model = super::super::model::Model::default();
    model.ports[0].channels = vec![3, 0, 5];
    model.connections[0].map = (0..3)
        .map(|i| ChannelMap {
            source: i,
            destination: i,
            gain: 1.,
        })
        .collect();
    model.set_speaker_layout(Group::Track(rt.session.tracks[0].id), Layout::StereoLfe);
    for scene in &rt.session.scenes {
        model.set_speaker_layout(Group::Scene(scene.id), Layout::StereoLfe);
    }
    model.set_speaker_layout(Group::Main, Layout::StereoLfe);
    model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
    let saved =
        serde_json::from_slice::<super::super::model::Model>(&serde_json::to_vec(&model).unwrap())
            .unwrap();
    rt.routing = Some(Box::new(
        Prepared::at_rate(Arc::new(saved), &rt.session, rate).unwrap(),
    ));

    (engine, rt)
}

#[test]
fn stereo_lfe_clip_track_scene_main_physical_channels_follow_independent_pcm_without_heap() {
    for rate in [44100, 48000, 96000] {
        for block in [1, 64, 127, 512] {
            let (_engine, mut rt) = fixture(rate);
            rt.apply(Command::LaunchScene { scene: 0 });
            let mut output = vec![0.; 4096 * 6];
            assert_eq!(
                test_alloc::measure(|| {
                    for chunk in output.chunks_mut(block * 6) {
                        rt.process_interleaved(chunk, 6);
                    }
                }),
                Default::default(),
                "{rate}/{block}"
            );
            for (frame, values) in output.chunks_exact(6).enumerate() {
                for physical in 0..6 {
                    let expected =
                        [3, 0, 5]
                            .iter()
                            .position(|p| *p == physical)
                            .map_or(0., |channel| {
                                if frame == 8 + 13 * channel {
                                    (((channel + 1) as f32 / 32.) * 0.25).tanh()
                                } else {
                                    0.
                                }
                            });
                    assert!(
                        (values[physical] - expected).abs() < 2e-6,
                        "{rate}/{block} frame{frame} physical{physical}: {} != {expected}",
                        values[physical]
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "child entry for the native wide archive fixture; outer check must skip this helper"]
fn stereo_lfe_fresh_process_renderer() {
    let path = std::path::PathBuf::from(
        std::env::var_os("OMATAINER_SPEAKER_ARCHIVE").expect("Archive child path required"),
    );
    let rate = std::env::var("OMATAINER_SPEAKER_RATE")
        .unwrap()
        .parse()
        .unwrap();
    let bundle = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let captured = crate::engine::project::Captured {
        state: bundle.state,
        media: bundle.media,
        revision: 0,
        checkpoint: Default::default(),
        playback_receipts: std::array::from_fn(|_| None),
    };
    let request = crate::audio_delivery::Export {
        end: 4096. / f64::from(rate),
        tail: 0.,
        output_alias: Some(1),
        options: crate::audio_delivery::Options {
            rate,
            channels: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let contract = crate::audio_delivery::output_contract(&captured.state, &request).unwrap();
    std::fs::write(
        path.with_extension("contract.json"),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    crate::audio_delivery::render_selected(
        captured,
        &request,
        None,
        &path.with_extension("f32"),
        (0, 4096, 0, 4096),
        &Arc::new(AtomicBool::new(false)),
        &crate::background::Reporter::default(),
    )
    .unwrap();
}

#[test]
fn stereo_lfe_native_archive_fresh_process_and_raw_selected_render_preserve_order_and_equations() {
    use crate::engine::audio::routing::selection::{Policy, Selection, Target};
    let directory = std::env::temp_dir().join(format!(
        "omatainer-speakers-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    assert!(directory.starts_with("/home"));
    std::fs::create_dir(&directory).unwrap();
    for rate in [44100, 48000, 96000] {
        let (engine, mut live) = fixture(rate);
        let captured = capture(&engine, &mut live);
        let path = directory.join(format!("ordered-{rate}.omat"));
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
        assert_eq!(contract["channel_count"], 3);
        assert_eq!(contract["channels"], serde_json::json!([3, 0, 5]));
        assert_eq!(contract["wave_mask"], 11);
        assert_eq!(
            contract["speakers"],
            serde_json::json!(["front_left", "front_right", "low_frequency"])
        );
        let bytes = std::fs::read(path.with_extension("f32")).unwrap();
        assert_eq!(bytes.len(), 4096 * 3 * 4);
        let fresh = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        live.apply(Command::LaunchScene { scene: 0 });
        let mut physical = vec![0.; 4096 * 6];
        for block in physical.chunks_mut(127 * 6) {
            live.process_interleaved(block, 6);
        }
        for frame in 0..4096 {
            for channel in 0..3 {
                let expected = if frame == 8 + 13 * channel {
                    (((channel + 1) as f32 / 32.) * 0.25).tanh()
                } else {
                    0.
                };
                let actual = fresh[frame * 3 + channel];
                assert!(
                    (actual - expected).abs() < 2e-6,
                    "fresh{rate}/{frame}/{channel}"
                );
                assert_eq!(
                    actual,
                    physical[frame * 6 + [3, 0, 5][channel]],
                    "live/fresh null{rate}/{frame}/{channel}"
                );
            }
        }
        let request = crate::audio_delivery::Export {
            end: 4096. / f64::from(rate),
            tail: 0.,
            output_alias: Some(1),
            options: crate::audio_delivery::Options {
                rate,
                channels: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let selection = Selection {
            namespace: captured.state.session.as_ref().unwrap().namespace,
            targets: vec![Target::Track(
                captured.state.session.as_ref().unwrap().tracks[0].id,
            )],
            policy: Policy::PostDevice,
        };
        let raw = directory.join(format!("contribution-{rate}.f32"));
        crate::audio_delivery::render_selected(
            captured,
            &request,
            Some(&selection),
            &raw,
            (0, 4096, 0, 4096),
            &Arc::new(AtomicBool::new(false)),
            &crate::background::Reporter::default(),
        )
        .unwrap();
        let bytes = std::fs::read(raw).unwrap();
        assert_eq!(bytes.len(), 4096 * 3 * 4);
        for (index, bytes) in bytes.chunks_exact(4).enumerate() {
            let frame = index / 3;
            let channel = index % 3;
            let actual = f32::from_le_bytes(bytes.try_into().unwrap());
            let expected = if frame == 8 + 13 * channel {
                (channel + 1) as f32 / 32.
            } else {
                0.
            };
            assert!(
                (actual - expected).abs() < 2e-6,
                "linear{rate}/{frame}/{channel}: {actual}/{expected}"
            );
        }
        if let Some(output) = std::env::var_os("OMATAINER_ROUTING_RECEIPT_DIR") {
            let output = std::path::PathBuf::from(output);
            assert!(output.starts_with("/home"));
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join(format!("stereo-lfe-{rate}.json")),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software","rate":rate,"layout":"stereo_lfe","wave_mask":11,"ordered_speakers":["FL","FR","LFE"],"physical_channels":[3,0,5],"independent_pcm_equation":"channel j impulse (j+1)/32 at frame8+13j; master=tanh(0.25*x)","native_archive":true,"fresh_process":true,"live_offline_null":true,"raw_selected_linear":true,"encoded_file":false,"physical_proof":false})).unwrap()).unwrap();
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stereo_lfe_positional_send_automation_and_controls_use_equal_power_without_lfe_or_heap() {
    use crate::engine::audio::routing::mixer::{Lane, Parameter, Send, Target};
    let (_engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    let mut model = super::super::model::Model::default();
    model.next_id = 4;
    model.ports[0].channels = vec![0, 1, 2];
    model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
    model.ports.push(Port {
        id: 2,
        alias: "Positional mono".into(),
        direction: Direction::Input,
        channels: vec![0],
    });
    model.connections.clear();
    model.sends.push(Send {
        id: 3,
        name: "Position".into(),
        route: Connection {
            source: Source {
                group: Group::Input(2),
                tap: Tap::PostMixer,
            },
            destination: Group::Output(1),
            map: (0..3)
                .map(|channel| ChannelMap {
                    source: 0,
                    destination: channel,
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
    let mut graph = Prepared::at_rate(Arc::new(model.clone()), &rt.session, 48000).unwrap();
    for (pan, left, right) in [
        (-1., 1., 0.),
        (
            0.,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ),
        (1., 0., 1.),
    ] {
        graph.mix_control(Target::Send(3), Parameter::Pan, pan);
        rt.routing_input_frame[0] = 0.125;
        let mut result = [0.; MAX_PHYSICAL_CHANNELS];
        assert_eq!(
            test_alloc::measure(|| result = graph.render(&mut rt, false, 0., 3)),
            Default::default()
        );
        assert!((result[0] - (0.125 * left).tanh()).abs() < 1e-7);
        assert!((result[1] - (0.125 * right).tanh()).abs() < 1e-7);
        assert_eq!(result[2], 0.);
    }
    model.sends[0].automation = vec![Lane {
        parameter: Parameter::Pan,
        points: vec![[0., -1.], [4., 1.]],
    }];
    let mut graph = Prepared::at_rate(Arc::new(model.clone()), &rt.session, 48000).unwrap();
    assert_eq!(
        test_alloc::measure(|| graph.mixer.evaluate(&model, 2., true)),
        Default::default()
    );
    for channel in 0..2 {
        assert!(
            (graph.mixer.channels[0].spatial_gains[channel] - std::f32::consts::FRAC_1_SQRT_2)
                .abs()
                < 1e-7
        );
    }
    assert_eq!(graph.mixer.channels[0].spatial_gains[2], 0.);
    let mut invalid = model.clone();
    invalid.sends[0].route.source.group = Group::Main;
    assert!(Prepared::at_rate(Arc::new(invalid), &rt.session, 48000).is_err());
    model.sends[0].spatial = false;
    model.sends[0].automation.clear();
    model.sends[0].route.map = vec![ChannelMap {
        source: 0,
        destination: 2,
        gain: 1.,
    }];
    let mut graph = Prepared::at_rate(Arc::new(model), &rt.session, 48000).unwrap();
    let output = graph.render(&mut rt, false, 0., 3);
    assert_eq!(
        output[2],
        0.125_f32.tanh(),
        "Explicit dedicated LFE route keeps unity digital gain"
    );
}

#[test]
fn stereo_lfe_bindings_follow_track_duplication_reorder_removal_undo_and_reused_slots() {
    use crate::engine::session::{Action, Axis, Request, Structure};
    let (engine, mut rt) = fixture(48000);
    let id = rt.session.tracks[0].id;
    let (request, ack) = Request::structural(
        capture(&engine, &mut rt),
        48000,
        Structure::Duplicate {
            axis: Axis::Track,
            id,
            name: "Ordered speaker copy".into(),
            position: 1,
        },
    )
    .unwrap();
    let command = Command::session_edit(request);
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        Default::default()
    );
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    let copied = rt
        .session
        .tracks
        .iter()
        .find(|t| t.name == "Ordered speaker copy")
        .unwrap()
        .id;
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .model
            .speaker_layout(Group::Track(copied)),
        Some(Layout::StereoLfe)
    );
    let (request, _) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Move {
            axis: Axis::Track,
            id: copied,
            position: 0,
        },
    )
    .unwrap();
    rt.apply(Command::session_edit(request));
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .model
            .speaker_layout(Group::Track(copied)),
        Some(Layout::StereoLfe)
    );
    let (request, _) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Delete {
            axis: Axis::Track,
            id,
        },
    )
    .unwrap();
    rt.apply(Command::session_edit(request));
    rt.apply(Command::Undo);
    assert!(rt.session.tracks.iter().any(|t| t.id == id && t.active));
    rt.apply(Command::Redo);
    let (request, _) = Request::structural(
        capture(&engine, &mut rt),
        48000,
        Structure::Track {
            name: "Fresh stereo identity".into(),
            audio: true,
            position: 0,
        },
    )
    .unwrap();
    let command = Command::session_edit(request);
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        Default::default()
    );
    assert_ne!(rt.session.tracks[0].id, id);
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .model
            .speaker_layout(Group::Track(rt.session.tracks[0].id)),
        Some(Layout::Stereo)
    );
    assert_eq!(
        rt.routing
            .as_ref()
            .unwrap()
            .model
            .speaker_layout(Group::Track(copied)),
        Some(Layout::StereoLfe)
    );
}
