use super::*;
use std::{sync::atomic::AtomicBool, time::Duration};

#[test]
fn saved_clap_identity_and_legacy_vst3_codec_are_distinct() {
    let binary = BinaryIdentity {
        bundle: "/home/test/Gain.clap".into(),
        binary: "/home/test/Gain.clap".into(),
        sha256: "a".repeat(64),
        arch: std::env::consts::ARCH.into(),
    };
    let saved = Saved {
        schema: 1,
        binary,
        class_id: "clap:org.example.Gain".into(),
        plugin_version: "1".into(),
        state_codec: "clap-state-1".into(),
        state: vec![],
    };
    saved.validate().unwrap();
    let mut wrong = saved.clone();
    wrong.state_codec = "vst3-host-0.9-state".into();
    assert!(wrong.validate().is_err());
    wrong = saved.clone();
    wrong.binary.bundle = "/home/test/Gain.vst3".into();
    assert!(wrong.validate().is_err());
    assert_eq!(
        Saved::codec("0123456789abcdef0123456789abcdef"),
        "vst3-host-0.9-state"
    );
    let reopened: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    assert_eq!(saved, reopened);
}
#[test]
fn input_events_preserve_offsets_channels_and_plain_parameter_ranges() {
    let binary = BinaryIdentity {
        bundle: "/home/test/Gain.clap".into(),
        binary: "/home/test/Gain.clap".into(),
        sha256: "a".repeat(64),
        arch: std::env::consts::ARCH.into(),
    };
    let mut class = empty_class(&binary);
    class.native = Some(Metadata {
        audio_inputs: vec![],
        audio_outputs: vec![],
        note_inputs: vec![Port {
            id: 0,
            name: "notes".into(),
            channels: 0,
            flags: 0,
            dialects: CLAP_NOTE_DIALECT_CLAP,
        }],
        note_outputs: vec![],
        parameters: vec![Parameter {
            id: 9,
            module: String::new(),
            flags: CLAP_PARAM_IS_AUTOMATABLE,
            min: -12.,
            max: 12.,
            default: 0.,
        }],
        extensions: vec![],
        unsupported_host_extensions: vec![],
    });
    let frame = Frame {
        inputs: vec![],
        frames: 256,
        bpm: 120.,
        beat: 0.,
        sample_position: 0,
        playing: true,
        signature: [4, 4],
        parameters: vec![(9, 0.75, 64)],
        midi: vec![(10, [0x93, 60, 100]), (128, [0x83, 60, 0])],
        native: vec![],
    };
    frame.validate().unwrap();
    let e = events::input(&frame, &class).unwrap();
    assert_eq!(
        e.iter().map(|e| e.header().time).collect::<Vec<_>>(),
        vec![10, 64, 128]
    );
    match &e[0] {
        events::Event::Note(n) => assert_eq!((n.channel, n.key), (3, 60)),
        _ => panic!("Expected CLAP note"),
    }
    match &e[1] {
        events::Event::Param(p) => assert_eq!(p.value, 6.),
        _ => panic!("Expected CLAP parameter"),
    }
}

fn exchange(p: &mut process::Process, r: Request) -> Response {
    p.exchange(&r, &AtomicBool::new(false), Duration::from_secs(10))
        .unwrap()
}
fn block(class: &Class) -> Frame {
    Frame {
        inputs: class
            .layout
            .inputs
            .iter()
            .map(|b| vec![vec![0.1; 256]; b.channel_count])
            .collect(),
        frames: 256,
        bpm: 120.,
        beat: 0.,
        sample_position: 0,
        playing: true,
        signature: [4, 4],
        parameters: vec![],
        midi: vec![],
        native: vec![],
    }
}

#[test]
#[ignore = "Requires real native CLAP release modules; no audio devices or editor windows are opened"]
fn native_clap_modules_scan_process_state_reopen_and_worker_recovery() {
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_CLAP_FIXTURES").expect("OMATAINER_CLAP_FIXTURES is required"),
    );
    let executable = std::env::var_os("OMATAINER_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap());
    for file in ["Nekobi.clap", "MVerb.clap", "3BandSplitter.clap"] {
        let path = root.join(file);
        let binary = identify(&path).unwrap();
        let mut process = process::Process::start(&executable).unwrap();
        let Response::Classes { classes } = exchange(
            &mut process,
            Request::Probe {
                binary: binary.clone(),
            },
        ) else {
            panic!("Expected native classes")
        };
        assert!(!classes.is_empty());
        let class = &classes[0];
        assert!(class.info.uid.starts_with("clap:"));
        class.validate().unwrap();
        let saved = Saved {
            schema: 1,
            binary,
            class_id: class.info.uid.clone(),
            plugin_version: class.info.version.clone(),
            state_codec: "clap-state-1".into(),
            state: vec![],
        };
        let Response::Loaded { class } =
            exchange(&mut process, Request::Load { saved, rate: 48000 })
        else {
            panic!("Expected native load")
        };
        let mut frame = block(&class);
        if class.info.has_midi_input {
            frame.midi.push((17, [0x92, 60, 110]));
        }
        if let Some(parameter) = class.writable_parameters().next() {
            frame.parameters.push((parameter.id, 0.42, 31));
        }
        let mut energy = 0.;
        for i in 0..12 {
            frame.sample_position = i * 256;
            let Response::Audio {
                outputs,
                parameters,
                ..
            } = exchange(
                &mut process,
                Request::Process {
                    frame: frame.clone(),
                },
            )
            else {
                panic!("Expected native audio")
            };
            assert_eq!(outputs.len(), class.layout.outputs.len());
            for (bus, layout) in outputs.iter().zip(&class.layout.outputs) {
                assert_eq!(bus.len(), layout.channel_count);
            }
            assert_eq!(parameters.len(), class.writable_parameters().count());
            energy += outputs
                .iter()
                .flatten()
                .flatten()
                .map(|v| v.abs() as f64)
                .sum::<f64>();
            frame.midi.clear();
            frame.parameters.clear();
        }
        assert!(energy > 0., "{file} must produce real native output");
        let Response::State { saved, parameters } = exchange(&mut process, Request::State) else {
            panic!("Expected native state")
        };
        assert!(!saved.state.is_empty());
        let json = serde_json::to_vec(&saved).unwrap();
        let saved: Saved = serde_json::from_slice(&json).unwrap();
        exchange(&mut process, Request::Quit);
        drop(process);
        let mut reopened = process::Process::start(&executable).unwrap();
        exchange(&mut reopened, Request::Load { saved, rate: 48000 });
        let Response::State {
            parameters: restored,
            ..
        } = exchange(&mut reopened, Request::State)
        else {
            panic!("Expected restored state")
        };
        let mut resumed = block(&class);
        if class.info.has_midi_input {
            resumed.midi.push((17, [0x92, 60, 110]));
        }
        exchange(&mut reopened, Request::Process { frame: resumed });
        assert_eq!(
            parameters, restored,
            "{file} must preserve parameter state across independent workers"
        );
        if file == "3BandSplitter.clap" {
            assert!(
                class.layout.outputs.len() > 1
                    || class
                        .layout
                        .outputs
                        .iter()
                        .map(|b| b.channel_count)
                        .sum::<usize>()
                        > 2
            );
        }
        let mut invalid = block(&class);
        invalid.frames = 0;
        assert!(reopened
            .exchange(
                &Request::Process { frame: invalid },
                &AtomicBool::new(false),
                Duration::from_secs(3)
            )
            .is_err());
        let mut recovered = process::Process::start(&executable).unwrap();
        exchange(&mut recovered, Request::Inspect { path });
        exchange(&mut recovered, Request::Quit);
    }
}

#[test]
#[ignore = "Private inherited-socket worker entry, invoked only by native host tests"]
fn worker_entry() {
    if std::env::var_os("OMATAINER_PLUGIN_TEST_WORKER").is_some() {
        crate::plugin_host::worker::run().unwrap();
    }
}

#[test]
fn mixed_plugin_folder_keeps_every_sibling_and_nested_bundle() {
    let root = std::env::temp_dir().join(format!(
        "clap-scan-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(root.join("a.vst3")).unwrap();
    std::fs::create_dir_all(root.join("nested/b.vst3")).unwrap();
    std::fs::write(root.join("first.clap"), b"fixture").unwrap();
    std::fs::write(root.join("second.clap"), b"fixture").unwrap();
    let paths =
        crate::plugin_host::scanner::candidates(&[root.clone()], &AtomicBool::new(false)).unwrap();
    assert_eq!(paths.len(), 4);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Compiles and runs the native C ABI conformance plugin using explicitly selected CLAP headers; no device or window"]
fn native_expression_offsets_identity_state_latency_and_process_crash_recovery() {
    use expression::{NativeEvent as E, Target};
    let headers = PathBuf::from(
        std::env::var_os("OMATAINER_CLAP_HEADERS").expect("OMATAINER_CLAP_HEADERS required"),
    );
    let root = std::env::temp_dir().join(format!(
        "native-expression-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("Conformance.clap");
    let status = std::process::Command::new("cc")
        .args(["-std=c11", "-shared", "-fPIC", "-O1"])
        .arg("-I")
        .arg(headers)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clap-expression.c"))
        .arg("-o")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let executable = std::env::current_exe().unwrap();
    let mut p = process::Process::start(&executable).unwrap();
    let binary = identify(&path).unwrap();
    let Response::Classes { classes } = exchange(
        &mut p,
        Request::Probe {
            binary: binary.clone(),
        },
    ) else {
        panic!("Expected real native class")
    };
    let class = &classes[0];
    assert_eq!(class.latency, 19);
    assert!(class
        .native
        .as_ref()
        .unwrap()
        .unsupported_host_extensions
        .iter()
        .any(|s| s == "clap.thread-pool"));
    let saved = Saved {
        schema: 1,
        binary,
        class_id: class.info.uid.clone(),
        plugin_version: "1".into(),
        state_codec: "clap-state-1".into(),
        state: vec![],
    };
    exchange(&mut p, Request::Load { saved, rate: 48000 });
    let target = Target {
        note_id: 42,
        port: 0,
        channel: 3,
        key: 60,
    };
    let mut f = block(class);
    f.native = vec![
        (
            7,
            E::Note {
                target,
                on: true,
                velocity: 1.,
            },
        ),
        (
            31,
            E::Expression {
                target,
                kind: 6,
                value: 0.5,
            },
        ),
        (
            47,
            E::Expression {
                target: Target {
                    note_id: 99,
                    ..target
                },
                kind: 6,
                value: 0.1,
            },
        ),
        (
            63,
            E::Modulation {
                target: Target::all(),
                parameter: 7,
                amount: 0.125,
            },
        ),
        (
            127,
            E::Note {
                target,
                on: false,
                velocity: 0.,
            },
        ),
    ];
    let Response::Audio {
        outputs, latency, ..
    } = exchange(&mut p, Request::Process { frame: f })
    else {
        panic!("Expected native output")
    };
    assert_eq!(latency, 19);
    let out = &outputs[0][0];
    assert!(out[..7].iter().all(|v| *v == 0.));
    assert!(out[7..31].iter().all(|v| *v == 0.25));
    assert!(out[31..63].iter().all(|v| *v == 0.75));
    assert!(out[63..127].iter().all(|v| *v == 0.875));
    assert!(out[127..].iter().all(|v| *v == 0.));
    let Response::State { saved, .. } = exchange(&mut p, Request::State) else {
        panic!("Expected state")
    };
    let mut seek = block(class);
    seek.sample_position = -256;
    exchange(&mut p, Request::Process { frame: seek });
    p.kill_for_test();
    assert!(p
        .exchange(
            &Request::State,
            &AtomicBool::new(false),
            Duration::from_secs(2)
        )
        .is_err());
    drop(p);
    let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut recovered = process::Process::start(&executable).unwrap();
    exchange(&mut recovered, Request::Load { saved, rate: 48000 });
    exchange(
        &mut recovered,
        Request::Process {
            frame: block(class),
        },
    );
    exchange(&mut recovered, Request::Quit);
    drop(recovered);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_platform_and_legacy_formats_have_specific_migration_diagnostics() {
    for (file, reason) in [
        ("Legacy.so", "Legacy VST2"),
        ("Windows.dll", "Windows plugin"),
        ("Mac.component", "Audio Units"),
        ("Mac.aupreset", "Audio Units"),
    ] {
        let path = Path::new("/home/fixtures").join(file);
        let message = super::super::identify(&path).unwrap_err();
        assert!(message.contains(reason));
        assert!(message.contains("rendered stem"));
    }
}
