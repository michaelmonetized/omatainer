use super::*;
use crate::engine::{
    audio::routing::{model::*, plugins::Instance, prepared::Prepared},
    test_alloc, Command, Engine,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Prepare an explicit native fixture map.
/// Takes stable endpoints and a width; returns producer-owned identity maps without implicit bus conversion.
fn route(source: Group, tap: Tap, destination: Group, width: usize) -> Connection {
    Connection {
        source: Source { group: source, tap },
        destination,
        map: (0..width)
            .map(|c| ChannelMap {
                source: c as u8,
                destination: c as u8,
                gain: 1.,
            })
            .collect(),
    }
}

#[test]
#[ignore = "requires pinned CLAP headers and the current native worker; no device or editor"]
fn native_speaker_primary_beds_sidechain_auxiliary_latency_and_tail_match_independent_pcm() {
    let headers = PathBuf::from(
        std::env::var_os("OMATAINER_CLAP_HEADERS").expect("Pinned CLAP headers required"),
    );
    let retained = std::env::var_os("OMATAINER_ROUTING_RECEIPT_DIR").map(PathBuf::from);
    let root = retained
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "native-speaker-bed-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
    assert!(root.starts_with("/home"));
    std::fs::create_dir_all(&root).unwrap();
    for layout in [Layout::StereoLfe, Layout::Surround51Side] {
        let width = layout.channels();
        let path = root.join(format!("Bed{width}.clap"));
        assert!(std::process::Command::new("cc")
            .args(["-std=c11", "-shared", "-fPIC", "-O1"])
            .arg(format!("-DBED_CHANNELS={width}"))
            .arg("-I")
            .arg(&headers)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clap-speaker-bed.c"))
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        let (class, saved) =
            crate::engine::audio::routing::plugin_tests::probe(path.clone(), false);
        assert_eq!(
            class
                .layout
                .inputs
                .iter()
                .map(|p| p.channel_count)
                .collect::<Vec<_>>(),
            [width, 1]
        );
        assert_eq!(
            class
                .layout
                .outputs
                .iter()
                .map(|p| p.channel_count)
                .collect::<Vec<_>>(),
            [width, 1]
        );
        assert_eq!(class.latency, 19);
        for rate in [44100, 48000, 96000] {
            let (_engine, mut rt) = Engine::headless_for_test(rate, 256);
            rt.apply(Command::Stop);
            let physical = [3, 0, 5, 1, 8, 7];
            let mut model = Model::default();
            model.version = 3;
            model.next_id = 7;
            model.ports[0].channels = physical[..width].to_vec();
            model.ports.extend([
                Port {
                    id: 2,
                    alias: "Independent native bed input".into(),
                    direction: Direction::Input,
                    channels: (0..width as u16).collect(),
                },
                Port {
                    id: 4,
                    alias: "Independent sidechain".into(),
                    direction: Direction::Input,
                    channels: vec![width as u16],
                },
                Port {
                    id: 6,
                    alias: "Independent auxiliary output".into(),
                    direction: Direction::Output,
                    channels: vec![16],
                },
            ]);
            model.buses.push(Bus {
                id: 3,
                alias: "Declared native return".into(),
                channels: width as u8,
                gain: 1.,
                mute: false,
                mixer: Default::default(),
            });
            for group in [Group::Input(2), Group::Bus(3), Group::Output(1)] {
                model.set_speaker_layout(group, layout);
            }
            model.plugins.push(Instance {
                id: 5,
                name: "Original speaker bed equation".into(),
                saved: saved.clone(),
                inputs: vec![width as u8, 1],
                outputs: vec![width as u8, 1],
                midi_track: None,
                scene_track: None,
                return_bus: Some(3),
                instrument: false,
                bypass: false,
                latency: 787,
                parameters: vec![],
                automation: vec![],
                unavailable: None,
            });
            model.connections = vec![
                route(Group::Input(2), Tap::PostMixer, Group::Bus(3), width),
                route(Group::Bus(3), Tap::PreFx, Group::Plugin(5), width),
                Connection {
                    source: Source {
                        group: Group::Input(4),
                        tap: Tap::PostMixer,
                    },
                    destination: Group::Plugin(5),
                    map: vec![ChannelMap {
                        source: 0,
                        destination: width as u8,
                        gain: 1.,
                    }],
                },
                route(Group::Plugin(5), Tap::PostMixer, Group::Output(1), width),
                Connection {
                    source: Source {
                        group: Group::Plugin(5),
                        tap: Tap::PostFx,
                    },
                    destination: Group::Output(6),
                    map: vec![ChannelMap {
                        source: width as u8,
                        destination: 0,
                        gain: 1.,
                    }],
                },
            ];
            let mut graph = Prepared::at_rate(Arc::new(model), &rt.session, rate).unwrap();
            assert!(
                graph.plugins[0].error.is_none(),
                "{:?}",
                graph.plugins[0].error
            );
            assert_eq!(graph.output_delay(1), 787);
            assert_eq!(graph.output_delay(6), 787);
            graph.offline = true;
            for frame in 0..2048 {
                rt.routing_input_frame.fill(0.);
                for c in 0..width {
                    if frame == 8 + 13 * c {
                        rt.routing_input_frame[c] = (c + 1) as f32 / 32.;
                    }
                }
                if frame == 99 {
                    rt.routing_input_frame[width] = 0.125;
                }
                let actual = graph.render(&mut rt, false, 0., 17);
                for c in 0..width {
                    let mut expected = 0_f32;
                    if frame == 787 + 8 + 13 * c {
                        expected += 0.5 * (c + 1) as f32 / 32.;
                    }
                    if frame == 787 + 9 + 13 * c {
                        expected += 0.25 * (c + 1) as f32 / 32.;
                    }
                    if frame == 787 + 99 {
                        expected += 0.125 * 0.125;
                    }
                    assert!(
                        (actual[usize::from(physical[c])] - expected.tanh()).abs() < 1e-7,
                        "{layout:?}/{rate}/{frame}/{c}: {} != {expected}",
                        actual[usize::from(physical[c])]
                    );
                }
                assert!(
                    (actual[16]
                        - if frame == 787 + 99 {
                            (-0.125_f32).tanh()
                        } else {
                            0.
                        })
                    .abs()
                        < 1e-7,
                    "Auxiliary bus mixed into primary at{layout:?}/{rate}/{frame}"
                );
            }
            assert!(
                graph.plugins[0].error.is_none(),
                "{:?}",
                graph.plugins[0].error
            );
            graph.offline = false;
            assert_eq!(
                test_alloc::measure(|| {
                    for _ in 0..256 {
                        graph.render(&mut rt, false, 0., 17);
                    }
                }),
                Default::default()
            );
            if let Some(directory) = &retained {
                use sha2::{Digest, Sha256};
                let bytes = std::fs::read(&path).unwrap();
                std::fs::write(directory.join(format!("native-speaker-bed-{width}-{rate}.json")),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software","test":"engine::audio::routing::speakers::native_tests::native_speaker_primary_beds_sidechain_auxiliary_latency_and_tail_match_independent_pcm","rate":rate,"channels":width,"layout":layout,"primary_and_mono_buses_independent":true,"delay_frames":787,"module":{"path":path,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))},"equation":"native primary=.5*x+.25*x(previous frame)+.125*side, latency19+bridge768; independent aux=-side","physical_proof":false})).unwrap()).unwrap();
            }
        }
    }
    if retained.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
}
