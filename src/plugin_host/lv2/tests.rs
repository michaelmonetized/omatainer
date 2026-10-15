use super::*;
use std::{sync::atomic::AtomicBool, time::Duration};
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
fn atom_midi_sequence_keeps_segment_offsets_and_alignment() {
    let mut bytes = vec![0u64; 64];
    fill_atom(
        &mut bytes,
        1,
        2,
        true,
        32,
        96,
        &[
            (8, [0x90, 60, 100]),
            (48, [0x92, 64, 100]),
            (100, [0x80, 60, 0]),
        ],
    )
    .unwrap();
    let bytes = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), 512) };
    assert_eq!(u32::from_ne_bytes(bytes[..4].try_into().unwrap()), 32);
    assert_eq!(i64::from_ne_bytes(bytes[16..24].try_into().unwrap()), 16);
    assert_eq!(&bytes[32..35], &[0x92, 64, 100]);
}
#[test]
fn features_remap_uris_and_reopen_embedded_state_paths() {
    let root = std::env::temp_dir().join(format!(
        "lv2-path-test-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let source = root.join("sample.dat");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&source, b"original source asset").unwrap();
    let a = features::Features::new(48000, root.join("first")).unwrap();
    let expected = a.id("urn:test:property").unwrap();
    assert_eq!(a.id("urn:test:property").unwrap(), expected);
    let path = CString::new(source.as_os_str().as_encoded_bytes()).unwrap();
    let abstract_path = unsafe { features::abstract_path(a._map.handle, path.as_ptr()) };
    assert!(!abstract_path.is_null());
    let id = unsafe { CStr::from_ptr(abstract_path).to_string_lossy().into_owned() };
    unsafe {
        features::free_path(ptr::null_mut(), abstract_path);
    }
    let assets = a.assets().unwrap();
    drop(a);
    let b = features::Features::new(48000, root.join("second")).unwrap();
    b.id("urn:test:another").unwrap();
    assert_ne!(b.id("urn:test:property").unwrap(), expected);
    b.restore_assets(&assets).unwrap();
    assert_eq!(
        std::fs::read(root.join("second").join(id)).unwrap(),
        b"original source asset"
    );
    drop(b);
    assert_eq!(std::fs::read(&source).unwrap(), b"original source asset");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "Requires native DPF release LV2 bundles and lilv; opens no device or editor"]
fn native_lv2_modules_scan_process_state_reopen_and_multibus() {
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_LV2_FIXTURES").expect("OMATAINER_LV2_FIXTURES required"),
    );
    let exe = std::env::var_os("OMATAINER_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap());
    for file in ["Nekobi.lv2", "MVerb.lv2", "3BandSplitter.lv2"] {
        let binary = identify(&root.join(file)).unwrap();
        let mut p = process::Process::start(&exe).unwrap();
        let Response::Classes { classes } = exchange(
            &mut p,
            Request::Probe {
                binary: binary.clone(),
            },
        ) else {
            panic!("Expected LV2 classes")
        };
        assert_eq!(classes.len(), 1);
        let class = &classes[0];
        assert!(class.info.uid.starts_with("lv2:"));
        let saved = Saved {
            schema: 1,
            binary,
            class_id: class.info.uid.clone(),
            plugin_version: class.info.version.clone(),
            state_codec: "lv2-state-1".into(),
            state: vec![],
        };
        let Response::Loaded { class } = exchange(&mut p, Request::Load { saved, rate: 48000 })
        else {
            panic!("Expected LV2 load")
        };
        let mut frame = block(&class);
        if class.info.has_midi_input {
            frame.midi.push((17, [0x92, 60, 110]));
        }
        if let Some(param) = class.writable_parameters().next() {
            frame.parameters.push((param.id, 0.42, 31));
        }
        let mut energy = 0.;
        for i in 0..12 {
            frame.sample_position = i * 256;
            let Response::Audio { outputs, .. } = exchange(
                &mut p,
                Request::Process {
                    frame: frame.clone(),
                },
            ) else {
                panic!("Expected LV2 audio")
            };
            assert_eq!(outputs.len(), class.layout.outputs.len());
            energy += outputs
                .iter()
                .flatten()
                .flatten()
                .map(|v| v.abs() as f64)
                .sum::<f64>();
            frame.midi.clear();
            frame.parameters.clear();
        }
        assert!(energy > 0., "{file} must process native samples");
        let Response::State { saved, parameters } = exchange(&mut p, Request::State) else {
            panic!("Expected LV2 state")
        };
        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        exchange(&mut p, Request::Quit);
        let mut reopened = process::Process::start(&exe).unwrap();
        exchange(&mut reopened, Request::Load { saved, rate: 48000 });
        let Response::State {
            parameters: restored,
            ..
        } = exchange(&mut reopened, Request::State)
        else {
            panic!("Expected restored LV2 state")
        };
        assert_eq!(parameters, restored);
        let mut resumed = block(&class);
        if class.info.has_midi_input {
            resumed.midi.push((17, [0x92, 60, 110]));
        }
        exchange(&mut reopened, Request::Process { frame: resumed });
        if file == "3BandSplitter.lv2" {
            assert!(
                class
                    .layout
                    .outputs
                    .iter()
                    .map(|b| b.channel_count)
                    .sum::<usize>()
                    > 2
            );
        }
        exchange(&mut reopened, Request::Quit);
    }
}

#[test]
#[ignore = "Compiles and runs the native LV2 state/Atom conformance module; no device or editor"]
fn native_lv2_atom_state_assets_latency_required_features_and_worker_recovery() {
    let root = std::env::temp_dir().join(format!(
        "native-lv2-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let bundle = root.join("Conformance.lv2");
    std::fs::create_dir_all(&bundle).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let status = std::process::Command::new("cc")
        .args(["-std=c11", "-shared", "-fPIC", "-O1"])
        .arg(fixtures.join("lv2-state.c"))
        .arg("-o")
        .arg(bundle.join("state.so"))
        .status()
        .unwrap();
    assert!(status.success());
    let ttl = std::fs::read_to_string(fixtures.join("lv2-state.ttl")).unwrap();
    std::fs::write(bundle.join("manifest.ttl"), &ttl).unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut p = process::Process::start(&exe).unwrap();
    let binary = identify(&bundle).unwrap();
    let Response::Classes { classes } = exchange(
        &mut p,
        Request::Probe {
            binary: binary.clone(),
        },
    ) else {
        panic!("Expected native LV2 metadata")
    };
    let class = &classes[0];
    let saved = Saved {
        schema: 1,
        binary,
        class_id: class.info.uid.clone(),
        plugin_version: class.info.version.clone(),
        state_codec: "lv2-state-1".into(),
        state: vec![],
    };
    exchange(&mut p, Request::Load { saved, rate: 48000 });
    let mut f = block(class);
    f.midi = vec![(17, [0x93, 60, 100]), (127, [0x83, 60, 0])];
    f.parameters = vec![(2, 0.25, 63)];
    let Response::Audio {
        outputs,
        latency,
        midi,
        ..
    } = exchange(&mut p, Request::Process { frame: f })
    else {
        panic!("Expected native LV2 output")
    };
    assert_eq!(latency, 11);
    assert_eq!(midi.len(), 2);
    let out = &outputs[0][0];
    assert!(out[..17].iter().all(|v| *v == 0.));
    assert!(out[17..63].iter().all(|v| *v == 0.75));
    assert!(out[63..127].iter().all(|v| *v == 0.5));
    assert!(out[127..].iter().all(|v| *v == 0.));
    let Response::State { saved, .. } = exchange(&mut p, Request::State) else {
        panic!("Expected portable LV2 state")
    };
    let encoded =
        serde_json::to_value(&serde_json::from_slice::<serde_json::Value>(&saved.state).unwrap())
            .unwrap();
    assert_eq!(encoded["assets"]["assets/0"], serde_json::json!([32]));
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
    let mut recovered = process::Process::start(&exe).unwrap();
    exchange(&mut recovered, Request::Load { saved, rate: 48000 });
    let mut f = block(class);
    f.midi.push((0, [0x93, 60, 100]));
    let Response::Audio { outputs, .. } = exchange(&mut recovered, Request::Process { frame: f })
    else {
        panic!("Expected restored native LV2 output")
    };
    assert!(outputs[0][0].iter().all(|v| *v == 0.5));
    exchange(&mut recovered, Request::Quit);
    drop(recovered);
    let unsupported = root.join("Unsupported.lv2");
    std::fs::create_dir(&unsupported).unwrap();
    std::fs::copy(bundle.join("state.so"), unsupported.join("state.so")).unwrap();
    std::fs::write(
        unsupported.join("manifest.ttl"),
        ttl.replace(
            "urid:map, state:mapPath",
            "<urn:omatainer:unsupported-required-feature>, urid:map, state:mapPath",
        ),
    )
    .unwrap();
    let binary = identify(&unsupported).unwrap();
    let mut check = process::Process::start(&exe).unwrap();
    let error = check
        .exchange(
            &Request::Probe { binary },
            &AtomicBool::new(false),
            Duration::from_secs(5),
        )
        .unwrap_err();
    assert!(error.contains("unsupported-required-feature"));
    drop(check);
    std::fs::remove_dir_all(root).unwrap();
}
