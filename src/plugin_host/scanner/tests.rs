use super::*;
#[test]
fn malformed_cached_records_and_old_versions_are_refused() {
    let mut catalog = Catalog::default();
    catalog.schema = 0;
    assert!(catalog.validate().is_err());
    catalog.schema = 1;
    catalog.roots.push("../plugin".into());
    assert!(catalog.validate().is_err());
}
#[test]
fn frame_rejects_nonfinite_audio_and_out_of_block_events() {
    let mut f = Frame {
        inputs: vec![vec![vec![0.; 256]]],
        frames: 256,
        bpm: 120.,
        beat: 0.,
        sample_position: 0,
        playing: true,
        signature: [4, 4],
        parameters: vec![],
        midi: vec![],
    };
    assert!(f.validate().is_ok());
    f.midi.push((256, [0x90, 60, 127]));
    assert!(f.validate().is_err());
    f.midi.clear();
    f.inputs[0][0][1] = f32::NAN;
    assert!(f.validate().is_err());
}

fn directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "omatainer-{label}-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn catalog_conflicts_failed_writes_and_reopen_keep_the_previous_generation() {
    let root = directory("plugin-catalog");
    let path = root.join("plugins.json");
    let mut c = Catalog::default();
    c.roots = vec![root.clone()];
    let initial = save(&path, &c, None).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert_eq!(read(&path).unwrap().1, Some(initial));
    c.blacklist.insert(root.join("hidden.vst3"));
    assert!(save(&path, &c, None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let next = save(&path, &c, Some(initial)).unwrap();
    assert_ne!(initial, next);
    assert!(read(&path)
        .unwrap()
        .0
        .blacklist
        .contains(&root.join("hidden.vst3")));
    let blocked = root.join("blocked");
    std::fs::write(&blocked, b"regular file").unwrap();
    assert!(save(&blocked.join("plugins.json"), &c, None).is_err());
    assert_eq!(read(&path).unwrap().1, Some(next));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn incompatible_bundles_and_symlinks_are_not_loaded() {
    let root = directory("plugin-architecture");
    let bundle = root.join("Wrong.vst3");
    std::fs::create_dir_all(bundle.join("Contents/x86_64-win")).unwrap();
    assert!(identify(&bundle).unwrap_err().contains("incompatible"));
    std::os::unix::fs::symlink(&bundle, root.join("Linked.vst3")).unwrap();
    assert!(identify(&root.join("Linked.vst3"))
        .unwrap_err()
        .contains("symlink"));
    std::fs::remove_dir_all(root).unwrap();
}
fn native_exe() -> PathBuf {
    std::env::var_os("OMATAINER_TEST_BIN")
        .map(PathBuf::from)
        .expect("OMATAINER_TEST_BIN must name the freshly compiled native app")
}
fn native_fixtures() -> PathBuf {
    std::env::var_os("OMATAINER_VST3_FIXTURES")
        .map(PathBuf::from)
        .expect("OMATAINER_VST3_FIXTURES must name the private qualification directory")
}
fn exchange(p: &mut process::Process, r: &Request) -> Response {
    p.exchange(r, &AtomicBool::new(false), Duration::from_secs(10))
        .unwrap()
}
fn processor(path: &Path) -> (process::Process, Class, Saved) {
    let mut p = process::Process::start(&native_exe()).unwrap();
    let binary = match exchange(&mut p, &Request::Inspect { path: path.into() }) {
        Response::Identity { binary } => binary,
        _ => panic!("wrong identity response"),
    };
    let classes = match exchange(
        &mut p,
        &Request::Probe {
            binary: binary.clone(),
        },
    ) {
        Response::Classes { classes } => classes,
        _ => panic!("wrong probe response"),
    };
    assert!(!classes.is_empty());
    let class = classes[0].clone();
    let saved = Saved {
        schema: 1,
        binary,
        class_id: class.info.uid.clone(),
        plugin_version: class.info.version.clone(),
        state_codec: "vst3-host-0.9-state".into(),
        state: vec![],
    };
    let loaded = match exchange(
        &mut p,
        &Request::Load {
            saved: saved.clone(),
            rate: 48000,
        },
    ) {
        Response::Loaded { class } => class,
        _ => panic!("wrong load response"),
    };
    (p, loaded, saved)
}
fn frame(class: &Class) -> Frame {
    Frame {
        inputs: class
            .layout
            .inputs
            .iter()
            .map(|b| vec![vec![0.; BLOCK]; b.channel_count])
            .collect(),
        frames: BLOCK,
        bpm: 120.,
        beat: 16.,
        sample_position: 384000,
        playing: true,
        signature: [4, 4],
        parameters: vec![],
        midi: vec![],
    }
}
fn outputs(p: &mut process::Process, f: Frame) -> Vec<Vec<Vec<f32>>> {
    match exchange(p, &Request::Process { frame: f }) {
        Response::Audio { outputs, .. } => outputs,
        _ => panic!("wrong audio response"),
    }
}
#[test]
#[ignore = "requires freshly built native worker and explicitly selected private native VST3 fixtures; never scans user plugins"]
fn native_plugin_audio_automation_state_and_instruments() {
    let fixtures = native_fixtures();
    let gain = fixtures.join("vst3sdk-build/VST3/Release/again-sample-accurate.vst3");
    let (mut p, class, mut saved) = processor(&gain);
    let mut f = frame(&class);
    for c in f.inputs.iter_mut().flatten() {
        c.fill(0.25);
    }
    let parameter = class.parameters.iter().find(|p| p.name == "Gain").unwrap();
    f.parameters = vec![(parameter.id, 0., 0), (parameter.id, 1., 128)];
    let out = outputs(&mut p, f.clone());
    for channel in out.iter().flatten() {
        for (i, value) in channel[..128].iter().enumerate() {
            assert!((*value - 0.25 * (i + 1) as f32 / 128.0).abs() < 1e-6);
        }
        assert!(channel[128..].iter().all(|v| (*v - 0.25).abs() < 1e-6));
    }
    saved = match exchange(&mut p, &Request::State) {
        Response::State { saved, .. } => saved,
        _ => panic!("wrong state response"),
    };
    assert!(!saved.state.is_empty());
    assert!(saved.state.len() < MAX_STATE);
    let persisted = serde_json::to_vec(&saved).unwrap();
    let saved: Saved = serde_json::from_slice(&persisted).unwrap();
    let mut reopened = process::Process::start(&native_exe()).unwrap();
    exchange(
        &mut reopened,
        &Request::Load {
            saved: saved.clone(),
            rate: 48000,
        },
    );
    f.parameters.clear();
    for channel in outputs(&mut reopened, f).iter().flatten() {
        assert!(channel.iter().all(|v| (*v - 0.25).abs() < 1e-6));
    }
    let mut wrong = saved.clone();
    wrong.plugin_version = "different".into();
    let mut incompatible = process::Process::start(&native_exe()).unwrap();
    assert!(incompatible
        .exchange(
            &Request::Load {
                saved: wrong,
                rate: 48000
            },
            &AtomicBool::new(false),
            Duration::from_secs(2)
        )
        .unwrap_err()
        .contains("version changed"));
    exchange(&mut p, &Request::Quit);
    exchange(&mut reopened, &Request::Quit);
    let (mut synth, class, _) = processor(&fixtures.join("dpf-plugins/bin/Nekobi.vst3"));
    let mut f = frame(&class);
    assert!(class.info.has_midi_input);
    f.midi.push((0, [0x90, 60, 127]));
    let mut peak = 0f32;
    for _ in 0..16 {
        for x in outputs(&mut synth, f.clone()).iter().flatten().flatten() {
            peak = peak.max(x.abs());
        }
        f.midi.clear();
        f.sample_position += BLOCK as i64;
        f.beat += BLOCK as f64 * 120. / (48000. * 60.);
    }
    assert!(
        peak > 0.001,
        "native Nekobi note must generate audio, peak={peak}"
    );
    exchange(&mut synth, &Request::Quit);
    let (mut effect, class, _) = processor(&fixtures.join("dpf-plugins/bin/MVerb.vst3"));
    let mut f = frame(&class);
    for c in f.inputs.iter_mut().flatten() {
        c[0] = 1.;
    }
    let first = outputs(&mut effect, f.clone());
    assert!(first.iter().flatten().flatten().any(|v| v.abs() > 0.01));
    for c in f.inputs.iter_mut().flatten() {
        c.fill(0.);
    }
    let mut tail = 0f32;
    for _ in 0..200 {
        for v in outputs(&mut effect, f.clone()).iter().flatten().flatten() {
            tail = tail.max(v.abs());
        }
    }
    assert!(tail > 0.0001, "native MVerb must produce an effect tail");
    exchange(&mut effect, &Request::Quit);
}
fn fault_bundle(root: &Path, label: &str, body: &str) -> PathBuf {
    let bundle = root.join(format!("{label}.vst3"));
    let native = bundle.join(format!("Contents/{}-linux", std::env::consts::ARCH));
    std::fs::create_dir_all(&native).unwrap();
    let source = root.join(format!("{label}.c"));
    std::fs::write(&source,format!("#include <stdlib.h>\n#include <unistd.h>\n#include <stdbool.h>\nbool ModuleEntry(void *h) {{ (void)h; return true; }}\nbool ModuleExit(void) {{ return true; }}\nvoid *GetPluginFactory(void) {{ {body} }}\n")).unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&source)
        .arg("-o")
        .arg(native.join(format!("{label}.so")))
        .status()
        .unwrap()
        .success());
    bundle
}
#[test]
#[ignore = "requires freshly built native worker; privately compiles deliberately aborting/hanging native plugin fixtures"]
fn native_plugin_failures_are_bounded_and_cancelled() {
    let root = directory("plugin-faults");
    for (name, body) in [
        ("crash", "abort();"),
        ("hang", "for (;;) pause();"),
        ("invalid", "return 0;"),
    ] {
        let bundle = fault_bundle(&root, name, body);
        let binary = identify(&bundle).unwrap();
        let mut p = process::Process::start(&native_exe()).unwrap();
        let start = std::time::Instant::now();
        let error = p
            .exchange(
                &Request::Probe { binary },
                &AtomicBool::new(false),
                Duration::from_millis(300),
            )
            .err()
            .unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{name} was not bounded: {error}"
        );
    }
    let bundle = fault_bundle(&root, "cancel", "for (;;) pause();");
    let binary = identify(&bundle).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        signal.store(true, Ordering::Release);
    });
    let mut p = process::Process::start(&native_exe()).unwrap();
    let started = std::time::Instant::now();
    assert!(p
        .exchange(&Request::Probe { binary }, &cancel, Duration::from_secs(10))
        .unwrap_err()
        .contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(2));
    thread.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

fn finish(browser: &mut Browser) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while browser.busy() {
        assert!(std::time::Instant::now() < deadline, "{}", browser.message);
        browser.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "requires freshly built native worker and explicitly selected private SDK fixture"]
fn native_scanner_cache_quarantine_blacklist_cancel_and_reopen() {
    let root = directory("plugin-browser");
    let plugin_root = root.join("plugins");
    std::fs::create_dir(&plugin_root).unwrap();
    let source = native_fixtures().join("vst3sdk-build/VST3/Release/again-sample-accurate.vst3");
    let good = plugin_root.join("Gain.vst3");
    assert!(std::process::Command::new("cp")
        .arg("-r")
        .arg(&source)
        .arg(&good)
        .status()
        .unwrap()
        .success());
    let bad = fault_bundle(&plugin_root, "crash", "abort();");
    let path = root.join("catalog.json");
    let mut browser = Browser::default();
    browser.initialize(path.clone());
    finish(&mut browser);
    browser.roots = plugin_root.display().to_string();
    browser.scan(false);
    finish(&mut browser);
    assert_eq!(browser.catalog.records.len(), 2);
    let qualified = browser
        .catalog
        .records
        .iter()
        .find(|r| r.path == good)
        .unwrap();
    assert_eq!(qualified.classes.len(), 2);
    let failed = browser
        .catalog
        .records
        .iter()
        .find(|r| r.path == bad)
        .unwrap();
    assert!(failed.failure.is_some());
    assert!(failed.binary.is_some());
    let bytes = std::fs::read(&path).unwrap();
    browser.scan(false);
    finish(&mut browser);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "unchanged cached probe and quarantine must retain exact records"
    );
    browser.blacklist(bad.clone(), true);
    finish(&mut browser);
    assert!(read(&path).unwrap().0.blacklist.contains(&bad));
    browser.blacklist(bad, false);
    finish(&mut browser);
    let _ = fault_bundle(&plugin_root, "hang", "for (;;) pause();");
    browser.scan(true);
    std::thread::sleep(Duration::from_millis(100));
    browser.cancel();
    let previous = std::fs::read(&path).unwrap();
    finish(&mut browser);
    assert!(browser.message.contains("cancelled"));
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert_eq!(browser.catalog.records.len(), 2);
    let mut reopened = Browser::default();
    reopened.initialize(path);
    finish(&mut reopened);
    assert_eq!(reopened.catalog.records.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}
