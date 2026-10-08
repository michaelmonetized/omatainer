use super::*;
#[test]
fn owned_profiles_are_bounded_and_select_compiled_capabilities_without_current_physical_claims() {
    let profiles = bundled().unwrap();
    assert_eq!(profiles.len(), 4);
    for profile in profiles {
        assert!(!profile.provenance.current_physical_qualification);
        assert_eq!(profile.preset_version, super::super::presets::VERSION);
        let bytes = serde_json::to_vec_pretty(&profile).unwrap();
        assert!(bytes.len() <= MAX_PROFILE);
        let decoded = Profile::decode(&bytes).unwrap();
        assert_eq!(decoded.map().bindings, profile.map().bindings);
        assert_eq!(decoded.map().name, profile.driver.name());
        assert!(!decoded.provenance.physical_receipts.is_empty());
    }
}
#[test]
fn untrusted_profile_cannot_choose_arbitrary_driver_model_code_or_partial_bindings() {
    let original = bundled().unwrap().remove(0);
    let mut cases = Vec::new();
    let mut p = original.clone();
    p.usb[0].product = 0x9999;
    cases.push(p);
    let mut p = original.clone();
    p.schema = 999;
    cases.push(p);
    let mut p = original.clone();
    p.preset_version += 1;
    cases.push(p);
    let mut p = original.clone();
    p.minimum_app = [99, 0, 0];
    cases.push(p);
    let mut p = original.clone();
    p.required_capabilities.push("execute-vendor-script".into());
    cases.push(p);
    let mut p = original.clone();
    p.provenance.current_physical_qualification = true;
    cases.push(p);
    let mut p = original.clone();
    p.initialization = Initialization::Apc40Mk2Host41;
    cases.push(p);
    let mut p = original.clone();
    p.bindings.resize(257, p.bindings[0]);
    cases.push(p);
    let mut p = original.clone();
    p.fixtures[0].bytes[1] = 127;
    cases.push(p);
    let mut p = original.clone();
    p.ports.output = 1;
    cases.push(p);
    for p in cases {
        assert!(p.validate().is_err());
    }
    let mut raw = serde_json::to_value(&original).unwrap();
    raw["script"] = serde_json::json!("vendor.py");
    assert!(Profile::decode(&serde_json::to_vec(&raw).unwrap()).is_err());
    assert!(Profile::decode(&vec![0; MAX_PROFILE + 1]).is_err());
    assert!(Catalog::decode(&vec![0; MAX_CATALOG + 1], 0).is_err());
}
#[test]
fn exact_usb_port_and_release_constraints_do_not_match_names_variants_or_other_roles() {
    let profile = bundled().unwrap().remove(1);
    let mut device = identity::Device {
        vendor: 0x09e8,
        product: 0x0029,
        release: 0x0100,
        serial: None,
        topology: "1-3.2".into(),
        port: 0,
        connection: "fixture-connection-1".into(),
    };
    assert!(profile.matches(&device));
    device.product = 0x0036;
    assert!(!profile.matches(&device));
    device.product = 0x0029;
    device.port = 1;
    assert!(!profile.matches(&device));
    device.port = 0;
    let key = device.key();
    device.topology = "1-3.3".into();
    assert_ne!(key, device.key());
    device.serial = Some("owned-unit-a".into());
    let key = device.key();
    device.topology = "1-3.4".into();
    assert_eq!(key, device.key());
    let mut profile = profile;
    profile.usb[0].minimum_release = Some(0x0200);
    assert!(!profile.matches(&device));
    assert!(identity::Device::discover("not-an-alsa-port").is_none());
}
#[test]
fn signed_real_catalog_profiles_refuse_tampering_replay_and_incompatible_payloads() {
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_PROFILE_ASSETS").unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("profiles/1.0.0")
                .into_os_string()
        }),
    );
    let signed = std::fs::read(root.join("catalog.json")).unwrap();
    let catalog = Catalog::decode(&signed, 1).unwrap();
    assert_eq!(catalog.profiles.len(), 4);
    assert!(Catalog::decode(&signed, catalog.generation + 1).is_err());
    let acquired = storage::Acquired::cached(&root, 1).unwrap();
    assert_eq!(acquired.profiles.len(), 4);
    assert_eq!(acquired.signed, signed);
    let mut envelope: Envelope = serde_json::from_slice(&signed).unwrap();
    envelope.payload.push(' ');
    assert!(Catalog::decode(&serde_json::to_vec(&envelope).unwrap(), 0).is_err());
    let mut envelope: Envelope = serde_json::from_slice(&signed).unwrap();
    envelope.signer = "unknown".into();
    assert!(Catalog::decode(&serde_json::to_vec(&envelope).unwrap(), 0).is_err());
    for entry in &catalog.profiles {
        let bytes = std::fs::read(root.join(&entry.file)).unwrap();
        let profile = catalog.profile(entry, &bytes).unwrap();
        assert_eq!(profile.id, entry.id);
        let mut changed = bytes;
        changed[0] ^= 1;
        assert!(catalog.profile(entry, &changed).is_err());
    }
    let before = std::fs::read(root.join("catalog.json")).unwrap();
    let cancel = AtomicBool::new(true);
    assert!(
        storage::Acquired::download("https://example.invalid/catalog.json", &root, 1, &cancel)
            .is_err()
    );
    assert_eq!(before, std::fs::read(root.join("catalog.json")).unwrap());
}
#[test]
#[ignore = "exports actual compiled owned profile data for immutable release signing"]
fn export_original_owned_profile_release() {
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_PROFILE_EXPORT").expect("new immutable export destination"),
    );
    export(&root).unwrap();
    for profile in bundled().unwrap() {
        let path = root.join(format!("{}-{}.json", profile.id, profile.version));
        assert_eq!(
            Profile::decode(&std::fs::read(path).unwrap())
                .unwrap()
                .map()
                .bindings,
            profile.map().bindings
        );
    }
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "controller-contract-{}-{}",
            std::process::id(),
            super::super::next_source_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn make_registry(root: &Path) -> std::sync::Arc<runtime::Registry> {
    let registry = runtime::Registry::start(root.into()).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !registry.loaded() {
        assert!(
            std::time::Instant::now() < end,
            "controller cache recovery deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    registry
}
fn apc(topology: &str) -> identity::Device {
    identity::Device {
        vendor: 0x09e8,
        product: 0x0029,
        release: 0x0100,
        serial: None,
        topology: topology.into(),
        port: 0,
        connection: "fixture-connection-1".into(),
    }
}
#[test]
fn exact_pairing_duplicate_instances_reconnect_and_offline_pins_preserve_source_endpoint() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (engine, _) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let a = apc("1-3.1");
    let b = apc("1-3.2");
    registry.refresh(
        vec![
            ("28:0".into(), "APC first".into(), Some(a.clone())),
            ("29:0".into(), "APC second".into(), Some(b.clone())),
        ],
        vec![
            ("28:0".into(), Some(a.clone())),
            ("29:0".into(), Some(b.clone())),
        ],
        &snapshot,
    );
    let view = registry.view();
    assert_eq!(view.devices.len(), 2);
    assert_eq!(view.devices[0].output.as_deref(), Some("28:0"));
    assert_eq!(view.devices[1].output.as_deref(), Some("29:0"));
    assert!(registry.output("28:0").is_none());
    registry.input_open("28:0", true);
    assert!(registry.output("28:0").is_some());
    registry.input_open("28:0", false);
    assert!(registry.output("28:0").is_none());
    registry.refresh(vec![], vec![], &snapshot);
    assert!(registry.view().devices.is_empty());
    registry.refresh(
        vec![("72:0".into(), "APC renumbered".into(), Some(a.clone()))],
        vec![("72:0".into(), Some(a.clone()))],
        &snapshot,
    );
    let policy = super::super::policy::InputPolicy::Selected(vec!["APC first".into()]);
    assert_eq!(
        registry.endpoint("72:0", "APC renumbered", &policy),
        ("APC first".into(), "28:0".into(), "APC first".into())
    );
    assert_eq!(registry.mapping("72:0", None).name, Driver::Apc40Mk2.name());
    drop(registry);
    let registry = make_registry(&files.0);
    registry.refresh(
        vec![("73:0".into(), "APC new client".into(), Some(a.clone()))],
        vec![("73:0".into(), Some(a.clone()))],
        &snapshot,
    );
    assert_eq!(
        registry.endpoint("73:0", "APC new client", &policy).1,
        "28:0"
    );
    let mut a = a;
    let mut b = b;
    a.serial = Some("duplicate".into());
    b.serial = a.serial.clone();
    registry.refresh(
        vec![
            ("28:0".into(), "Akai APC40 mkII".into(), Some(a.clone())),
            ("29:0".into(), "Akai APC40 mkII".into(), Some(b.clone())),
        ],
        vec![("28:0".into(), Some(a)), ("29:0".into(), Some(b))],
        &snapshot,
    );
    assert!(registry
        .view()
        .devices
        .iter()
        .all(|d| d.profile.is_none() && d.output.is_none()));
}
#[test]
fn guide_records_received_data_without_promoting_sends_or_triggering_live_actions() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (engine, _) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let device = apc("1-3.1");
    registry.refresh(
        vec![("28:0".into(), "APC".into(), Some(device.clone()))],
        vec![("28:0".into(), Some(device))],
        &snapshot,
    );
    assert!(registry.begin_check("28:0", "PAN", &snapshot).is_err());
    registry.input_open("28:0", true);
    registry.output_result("28:0", true, true, false);
    registry.begin_check("28:0", "PAN", &snapshot).unwrap();
    assert!(!registry.view().guide.as_ref().unwrap().input_received);
    assert!(registry.observe("28:0", &[0x90, 0x57, 127]));
    let view = registry.view();
    let check = view.guide.as_ref().unwrap();
    assert!(check.input_received);
    assert_eq!(check.packets, ["90 57 7f"]);
    assert!(check.application_observation.is_none() && check.physical_observation.is_none());
    assert!(registry.observations(Some("PAN selected"), None).is_err());
    registry.finish_check();
    assert!(!registry.observe("28:0", &[0x80, 0x57, 0]));
    registry
        .observations(Some("PAN selected in app"), Some("PAN light on"))
        .unwrap();
    let mut playing = snapshot.clone();
    playing.playing = true;
    assert!(registry.begin_check("28:0", "PLAY", &playing).is_err());
    assert!(registry.request_apply(false, &playing).is_err());
}
#[test]
fn guide_consumes_real_handoff_before_decoder_and_resumes_normal_control_dispatch() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let device = apc("1-3.1");
    registry.refresh(
        vec![("28:0".into(), "APC".into(), Some(device.clone()))],
        vec![("28:0".into(), Some(device))],
        &snapshot,
    );
    registry.input_open("28:0", true);
    let counters = std::sync::Arc::new(super::super::handoff::InputCounters::default());
    let map = registry.mapping("28:0", None);
    let (mut callback, worker) = super::super::handoff::start_profile(
        90001,
        map,
        engine.cmd.clone(),
        engine.midi.log.clone(),
        "APC".into(),
        "28:0".into(),
        Some((registry.clone(), "28:0".into())),
        counters.clone(),
        true,
        || {},
    )
    .unwrap();
    registry
        .begin_check("28:0", "Master fader", &snapshot)
        .unwrap();
    let counts = crate::engine::test_alloc::measure(|| callback.push(&[0xb0, 14, 25]));
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while counters.snapshot().dispatched == 0 {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    rt.process(&mut [0.0; 128]);
    assert_eq!(rt.master, snapshot.master);
    assert_eq!(
        registry.view().guide.as_ref().unwrap().packets,
        ["b0 0e 19"]
    );
    registry.finish_check();
    callback.push(&[0xb0, 14, 50]);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while counters.snapshot().dispatched < 2 {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    rt.process(&mut [0.0; 128]);
    assert!((rt.master - 50.0 / 127.0).abs() < 1e-6);
    drop(callback);
    drop(worker);
}
#[test]
fn unknown_models_other_ports_and_ambiguous_outputs_never_gain_feedback_from_a_label() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (engine, _) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let mut device = apc("1-3.1");
    device.product = 0x9999;
    registry.refresh(
        vec![(
            "28:0".into(),
            "Akai APC40 mkII".into(),
            Some(device.clone()),
        )],
        vec![("28:0".into(), Some(device))],
        &snapshot,
    );
    assert_eq!(registry.mapping("28:0", None).name, Driver::Generic.name());
    registry.input_open("28:0", true);
    assert!(registry.output("28:0").is_none());
    let device = apc("1-3.1");
    registry.refresh(
        vec![("28:0".into(), "APC".into(), Some(device.clone()))],
        vec![
            ("28:0".into(), Some(device.clone())),
            ("29:0".into(), Some(device)),
        ],
        &snapshot,
    );
    assert!(registry.view().devices[0].output.is_none());
}
#[test]
fn connection_manager_retains_selected_policy_and_learned_override_after_port_renumbering() {
    use super::super::{connections::test_support as backend, learn, policy::InputPolicy};
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (mut engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let device = apc("1-3.1");
    registry.refresh(
        vec![("28:0".into(), "APC original".into(), Some(device.clone()))],
        vec![("28:0".into(), Some(device.clone()))],
        &snapshot,
    );
    let config = learn::Config {
        mappings: vec![learn::Mapping {
            endpoint: learn::Endpoint {
                name: "APC original".into(),
                id: "28:0".into(),
            },
            binding: super::super::cbind(0, 7, Action::Master, 0, 0),
        }],
    };
    engine.cmd.midi_learn().configure(config.clone()).unwrap();
    let control = backend::install_profiled(
        &mut engine,
        InputPolicy::Selected(vec!["APC original".into()]),
        Some(registry.clone()),
    );
    control.discover(&[("28:0", "APC original")]);
    let old = control.connect("28:0", Ok(()));
    backend::until(|| !engine.midi.connections_busy());
    old.push(&[0xb0, 7, 20]);
    backend::until(|| engine.midi.input_stats().dispatched >= 1);
    rt.process(&mut [0.0; 128]);
    assert!((rt.master - 20.0 / 127.0).abs() < 1e-6);
    registry.refresh(
        vec![(
            "72:0".into(),
            "APC renamed client".into(),
            Some(device.clone()),
        )],
        vec![("72:0".into(), Some(device))],
        &engine.snapshot(),
    );
    assert_eq!(engine.midi.retry_connections(), super::super::Retry::Queued);
    control.discover(&[("72:0", "APC renamed client")]);
    let new = control.connect("72:0", Ok(()));
    backend::until(|| !engine.midi.connections_busy());
    assert!(old.is_closed());
    assert!(engine
        .midi
        .policy_status()
        .unwrap()
        .missing_names
        .is_empty());
    new.push(&[0xb0, 7, 60]);
    backend::until(|| engine.midi.input_stats().dispatched >= 2);
    rt.process(&mut [0.0; 128]);
    assert!((rt.master - 60.0 / 127.0).abs() < 1e-6);
    assert_eq!(engine.cmd.midi_learn().view().config, config);
    let mut replugged = apc("1-3.1");
    replugged.connection = "fixture-connection-2".into();
    registry
        .begin_check("72:0", "Before replug", &engine.snapshot())
        .unwrap();
    registry.refresh(
        vec![(
            "72:0".into(),
            "APC renamed client".into(),
            Some(replugged.clone()),
        )],
        vec![("72:0".into(), Some(replugged))],
        &engine.snapshot(),
    );
    assert!(registry.view().guide.as_ref().unwrap().connection_ended);
    assert_eq!(
        registry
            .capture_state("72:0")
            .load(std::sync::atomic::Ordering::Acquire)
            & 1,
        0
    );
    assert_eq!(engine.midi.retry_connections(), super::super::Retry::Queued);
    control.discover(&[("72:0", "APC renamed client")]);
    let replugged = control.connect("72:0", Ok(()));
    backend::until(|| !engine.midi.connections_busy());
    assert!(new.is_closed());
    replugged.push(&[0xb0, 7, 80]);
    backend::until(|| engine.midi.input_stats().dispatched >= 3);
    rt.process(&mut [0.0; 128]);
    assert!((rt.master - 80.0 / 127.0).abs() < 1e-6);
    assert_eq!(engine.cmd.midi_learn().view().config, config);
}
#[test]
fn acquired_owned_fixtures_admit_expected_commands_through_compiled_decoders() {
    use crate::engine::Command;
    for profile in bundled().unwrap() {
        let map = profile.map();
        let mut decoder = super::super::surface::Decoder::default();
        let (cmd, received) = crate::engine::CommandPort::channel(80);
        let log = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let shift = std::sync::Arc::new(parking_lot::Mutex::new([false; 4]));
        for fixture in &profile.fixtures {
            if !decoder.input_at(&map, &fixture.bytes, &cmd, 91001, std::time::Instant::now()) {
                super::super::handle_channel(
                    &fixture.bytes,
                    91001,
                    &map,
                    &cmd,
                    &log,
                    &shift,
                    profile.driver.name(),
                    false,
                );
            }
        }
        let commands: Vec<_> = received.try_iter().collect();
        assert!(commands.iter().any(|c|match profile.driver{Driver::Ns7=>matches!(c,Command::DeckPlay{deck:0}),Driver::Sp1=>matches!(c,Command::DeckSync{deck:0}),Driver::Apc40Mk2=>matches!(c,Command::ClipPress(press) if matches!(press.target,crate::engine::clip_launch::Target::Apc(32))),Driver::Mpd232=>matches!(c,Command::TrackGain{track:0,value} if (*value-77.0/127.0).abs()<1e-6),_=>false}),"{} fixture had no expected admitted command",profile.id);
    }
}
#[test]
fn authenticated_cache_publication_keeps_immutable_versions_and_stopped_rollback_pins() {
    let files = Files::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles/1.0.0");
    let signed = std::fs::read(root.join("catalog.json")).unwrap();
    let catalog = Catalog::decode(&signed, 1).unwrap();
    let data: Vec<_> = catalog
        .profiles
        .iter()
        .map(|e| (e.file.clone(), std::fs::read(root.join(&e.file)).unwrap()))
        .collect();
    let cancel = AtomicBool::new(false);
    storage::Acquired::publish(&files.0, 0, signed.clone(), data.clone(), &cancel).unwrap();
    let latest = std::fs::read(files.0.join("latest.json")).unwrap();
    assert_eq!(latest, signed);
    let mut incomplete = data.clone();
    incomplete.pop();
    assert!(storage::Acquired::publish(&files.0, 0, signed.clone(), incomplete, &cancel).is_err());
    assert_eq!(std::fs::read(files.0.join("latest.json")).unwrap(), latest);
    let second =
        include_bytes!("../../../../tests/fixtures/controller-catalogs/generation-2.json").to_vec();
    storage::Acquired::publish(&files.0, 1, second.clone(), data.clone(), &cancel).unwrap();
    assert_eq!(
        std::fs::read(files.0.join("last-good.json")).unwrap(),
        signed
    );
    let third =
        include_bytes!("../../../../tests/fixtures/controller-catalogs/generation-3.json").to_vec();
    let mut changed = data;
    let entry = changed
        .iter_mut()
        .find(|(name, _)| name == "numark-ns7-original-1.0.0.json")
        .unwrap();
    entry.1 =
        include_bytes!("../../../../tests/fixtures/controller-catalogs/changed-ns7.json").to_vec();
    assert!(
        storage::Acquired::publish(&files.0, 2, third, changed, &cancel)
            .err()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(std::fs::read(files.0.join("latest.json")).unwrap(), second);
    assert!(!files.0.join("generation-3").exists());
    assert!(std::fs::read_dir(&files.0).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".candidate")));
    let registry = make_registry(&files.0);
    let (engine, _) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let device = apc("1-3.1");
    let input = vec![("28:0".into(), "APC".into(), Some(device.clone()))];
    let output = vec![("28:0".into(), Some(device))];
    registry.refresh(input.clone(), output.clone(), &snapshot);
    assert_eq!(registry.view().devices[0].generation, 2);
    registry.request_apply(true, &snapshot).unwrap();
    registry.refresh(input, output, &snapshot);
    assert_eq!(registry.view().devices[0].generation, 1);
    assert_eq!(registry.view().cached_generation, Some(2));
    drop(registry);
    let registry = make_registry(&files.0);
    assert_eq!(registry.view().cached_generation, Some(2));
    assert!(Catalog::decode(&signed, 2).is_err());
}
#[test]
fn guide_reset_and_capture_are_scoped_to_the_selected_port_and_end_when_hidden() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let (engine, _) = crate::engine::Engine::headless_for_test(48000, 80);
    let snapshot = engine.snapshot();
    let a = apc("1-3.1");
    let b = apc("1-3.2");
    registry.refresh(
        vec![
            ("28:0".into(), "APC A".into(), Some(a.clone())),
            ("29:0".into(), "APC B".into(), Some(b.clone())),
        ],
        vec![("28:0".into(), Some(a)), ("29:0".into(), Some(b))],
        &snapshot,
    );
    registry.input_open("28:0", true);
    registry.input_open("29:0", true);
    registry
        .begin_check("28:0", "Control A", &snapshot)
        .unwrap();
    assert!(registry
        .begin_check("29:0", "Concurrent check", &snapshot)
        .is_err());
    assert!(registry.guide_revision("28:0") > 0);
    assert_eq!(registry.guide_revision("29:0"), 0);
    assert!(!registry.observe("29:0", &[0x90, 1, 127]));
    assert!(!registry.view().guide.as_ref().unwrap().input_received);
    registry.finish_check();
    let a_revision = registry.guide_revision("28:0");
    registry
        .begin_check("29:0", "Control B", &snapshot)
        .unwrap();
    assert_eq!(registry.guide_revision("28:0"), a_revision);
    registry.finish_when_unsafe(&snapshot, false);
    assert!(!registry.view().guide.as_ref().unwrap().capturing);
    assert!(!registry.observe("29:0", &[0x90, 2, 127]));
    assert!(!registry.view().guide.as_ref().unwrap().input_received);
    registry
        .begin_check("29:0", "Removed control", &snapshot)
        .unwrap();
    registry.refresh(vec![], vec![], &snapshot);
    assert!(registry.view().guide.as_ref().unwrap().connection_ended);
    assert!(registry
        .observations(Some("app changed"), Some("light on"))
        .is_err());
}
#[test]
fn queued_capture_packets_cannot_launch_after_finish_and_overflow_cannot_stop_other_sources() {
    let files = Files::new();
    let registry = make_registry(&files.0);
    let snapshot = crate::engine::Snapshot::default();
    let device = apc("1-3.1");
    registry.refresh(
        vec![("28:0".into(), "APC".into(), Some(device.clone()))],
        vec![("28:0".into(), Some(device))],
        &snapshot,
    );
    registry.input_open("28:0", true);
    let (cmd, received) = crate::engine::CommandPort::channel(80);
    let counters = std::sync::Arc::new(super::super::handoff::InputCounters::default());
    let (mut sink, mut step) = super::super::handoff::paused_profile_for_test(
        95001,
        registry.mapping("28:0", None),
        cmd,
        registry.clone(),
        "28:0".into(),
        counters.clone(),
    )
    .unwrap();
    registry.begin_check("28:0", "Clip pad", &snapshot).unwrap();
    sink.push(&[0x90, 32, 127]);
    registry.finish_check();
    assert!(step());
    assert!(received.try_iter().all(|c| !matches!(
        c,
        crate::engine::Command::ClipPress(_) | crate::engine::Command::Stop
    )));
    assert_eq!(counters.snapshot().dropped, 1);
    registry.begin_check("28:0", "Stop all", &snapshot).unwrap();
    for _ in 0..300 {
        sink.push(&[0x90, 0x5c, 127]);
    }
    for _ in 0..300 {
        step();
    }
    assert!(received
        .try_iter()
        .all(|c| !matches!(c, crate::engine::Command::Stop)));
    registry.finish_check();
    sink.push(&[0x90, 32, 127]);
    step();
    assert!(received
        .try_iter()
        .any(|c| matches!(c, crate::engine::Command::ClipPress(_))));
}
