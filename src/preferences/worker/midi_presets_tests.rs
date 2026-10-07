use super::*;
use crate::engine::midi::{
    presets::{Layer, Preset},
    Action, Binding, MsgKind,
};
use std::time::{Duration, Instant};

#[test]
fn sync_mode_presets_and_preferences_preserve_explicit_targets_and_refuse_older_headers() {
    use crate::engine::midi::learn::{Config, Endpoint, Mapping};
    let files = Files::new();
    let endpoint = Endpoint { name: "Recorded sync controls".into(), id: "exact-sync-port".into() };
    let mut bindings = Vec::new();
    let binding = |data, action, deck, extra| Binding { kind: MsgKind::Note, ch: 2, data, action, deck, extra, relative: None, controls: None, pair_order: None };
    for mode in 0..4 { bindings.push(binding(60 + mode as u8, Action::DeckSyncMode, 1, mode)); }
    for leader in 0..3 { bindings.push(binding(70 + leader as u8, Action::DeckSyncLeader, 0, leader)); }
    let config = Config { mappings: bindings.iter().map(|binding| Mapping { endpoint: endpoint.clone(), binding: *binding }).collect() };
    let preset = Preset::capture("Explicit sync".into(), String::new(), &endpoint, &config).unwrap();
    let path = files.0.join("sync-controls.json");
    let mut worker = Worker::with_discovery(files.0.join("unused.json"), || Err("No discovery".into())).unwrap();
    worker.request(Job::ExportMidiPreset { path: path.clone(), preset: preset.clone() }).unwrap();
    assert!(matches!(wait(&mut worker), Event::MidiPresetExported(_)));
    worker.request(Job::ImportMidiPreset { path: path.clone(), token: 193 }).unwrap();
    assert!(matches!(wait(&mut worker), Event::MidiPresetImported { token: 193, preset: loaded } if loaded == preset));
    let portable = serde_json::to_value(&preset).unwrap();
    for version in 1..crate::engine::midi::presets::VERSION { let mut old = portable.clone(); old["version"] = version.into(); assert!(Preset::decode(&serde_json::to_vec(&old).unwrap()).is_err()); }
    let mut preferences = super::super::Preferences::defaults(&files.0);
    let profile = preferences.profiles.get_mut(&preferences.active).unwrap();
    profile.midi_learn = config;
    profile.midi_presets = vec![preset];
    let path = files.0.join("actual-sync-preferences.json");
    let cancelled = AtomicBool::new(false);
    super::storage::save(&path, &preferences, super::storage::Overwrite::Exact(None), &cancelled).unwrap();
    assert_eq!(super::storage::load(&path, &cancelled).unwrap().preferences, preferences);
    let mut old = serde_json::to_value(&preferences).unwrap();
    old["version"] = 23.into();
    assert!(super::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    let mut legacy = super::super::Preferences::defaults(&files.0);
    legacy.version = 23;
    let (migrated, changed) = super::storage::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(changed); assert_eq!(migrated.version, super::super::VERSION);
    for binding in bindings {
        for invalid in [Binding { kind: MsgKind::Cc, ..binding }, Binding { extra: 4, ..binding }, Binding { deck: 2, ..binding }] { assert!(crate::engine::midi::learn::validate_binding(&invalid).is_err()); }
        if binding.action == Action::DeckSyncLeader { assert!(crate::engine::midi::learn::validate_binding(&Binding { deck: 1, ..binding }).is_err()); }
    }
}

#[test]
fn deck_pad_presets_export_import_and_real_preferences_reopen_preserve_fixed_modes_and_strict_versions() {
    use crate::engine::midi::learn::{Config,Endpoint,Mapping};
    let files=Files::new();let endpoint=Endpoint{name:"Recorded pad controller".into(),id:"recorded-exact-port".into()};
    let actions=[Action::DeckPad,Action::DeckPadMode,Action::DeckPadParameterLeft,Action::DeckPadParameterRight,Action::DeckPadParameterShiftLeft,Action::DeckPadParameterShiftRight];
    let config=Config {mappings:actions.into_iter().enumerate().map(|(i,action)|Mapping {endpoint:endpoint.clone(),binding:Binding {kind:MsgKind::Note,ch:3,data:50+i as u8,action,deck:1,extra:if i<2 {7}else{0},relative:None,controls:None,pair_order:None}}).collect()};
    let preset=Preset::capture("Eight pad modes".into(),String::new(),&endpoint,&config).unwrap();
    let path=files.0.join("deck-pads.json");let mut worker=Worker::with_discovery(files.0.join("prefs.json"),||Err("No device discovery".into())).unwrap();
    worker.request(Job::ExportMidiPreset{path:path.clone(),preset:preset.clone()}).unwrap();assert!(matches!(wait(&mut worker),Event::MidiPresetExported(_)));
    worker.request(Job::ImportMidiPreset{path:path.clone(),token:192}).unwrap();assert!(matches!(wait(&mut worker),Event::MidiPresetImported{token:192,preset:p} if p==preset));
    let bytes=std::fs::read(&path).unwrap();for version in 1..5 {let mut old=serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();old["version"]=version.into();assert!(Preset::decode(&serde_json::to_vec(&old).unwrap()).is_err());}
    let mut preferences=super::super::Preferences::defaults(&files.0);let profile=preferences.profiles.get_mut(&preferences.active).unwrap();profile.midi_learn=config.clone();profile.midi_presets=vec![preset];
    let cancelled=AtomicBool::new(false);
    let prefs_path=files.0.join("actual-preferences.json");super::storage::save(&prefs_path,&preferences,super::storage::Overwrite::Exact(None),&cancelled).unwrap();
    let loaded=super::storage::load(&prefs_path,&cancelled).unwrap();assert_eq!(loaded.preferences,preferences);
    let current=serde_json::to_value(&preferences).unwrap();let mut old=current.clone();old["version"]=22.into();assert!(super::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    for mapping in &config.mappings {for bad in [Binding{deck:2,..mapping.binding},Binding{kind:MsgKind::Cc,..mapping.binding},Binding{extra:8,..mapping.binding}] {assert!(crate::engine::midi::learn::validate_binding(&bad).is_err());}}
}

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-midi-presets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn preset() -> Preset {
    Preset {
        version: 1,
        name: "Portable".into(),
        device_hint: "Old controller".into(),
        revision_hint: String::new(),
        layer: Layer::FactoryOverlay,
        bindings: vec![Binding {
            kind: MsgKind::Note,
            ch: 0,
            data: 60,
            action: Action::DeckCueHold,
            deck: 0,
            extra: 0,
            relative: None,
            controls: None,
            pair_order: None,
        }],
    }
}
fn wait(worker: &mut Worker) -> Event {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = worker.poll() {
            return event;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn midi_preset_worker_exports_private_new_files_and_refuses_overwrites_symlinks_and_invalid_imports(
) {
    let files = Files::new();
    let path = files.0.join("portable.json");
    let mut worker = Worker::with_discovery(files.0.join("preferences.json"), || {
        Err("No audio discovery needed".into())
    })
    .unwrap();
    worker
        .request(Job::ExportMidiPreset {
            path: path.clone(),
            preset: preset(),
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::MidiPresetExported(_)));
    let original = std::fs::read(&path).unwrap();
    assert_eq!(Preset::decode(&original).unwrap(), preset());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    worker
        .request(Job::ExportMidiPreset {
            path: path.clone(),
            preset: preset(),
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::Failed(_)));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    worker
        .request(Job::ImportMidiPreset {
            path: path.clone(),
            token: 42,
        })
        .unwrap();
    assert!(
        matches!(wait(&mut worker),Event::MidiPresetImported {token:42,preset:p} if p==preset())
    );
    let bad = files.0.join("bad.json");
    std::fs::write(&bad, b"{\"version\":1,\"bindings\":[\"invented\"]}").unwrap();
    worker
        .request(Job::ImportMidiPreset {
            path: bad,
            token: 43,
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::Failed(_)));
    #[cfg(unix)]
    {
        let link = files.0.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        worker
            .request(Job::ImportMidiPreset {
                path: link,
                token: 44,
            })
            .unwrap();
        assert!(matches!(wait(&mut worker), Event::Failed(_)));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
fn midi_preset_worker_cancellation_fences_before_read_and_after_reply_delivery() {
    let files = Files::new();
    let path = files.0.join("import.json");
    std::fs::write(&path, serde_json::to_vec(&preset()).unwrap()).unwrap();
    let mut worker =
        Worker::with_discovery(files.0.join("preferences.json"), || Err("unused".into())).unwrap();
    worker.delay.store(50, Ordering::Release);
    worker
        .request(Job::ImportMidiPreset {
            path: path.clone(),
            token: 1,
        })
        .unwrap();
    worker.cancel();
    assert!(matches!(wait(&mut worker), Event::Cancelled));
    worker.delay.store(0, Ordering::Release);
    worker
        .request(Job::ImportMidiPreset { path, token: 2 })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while worker.results.is_empty() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.cancel();
    assert!(matches!(wait(&mut worker), Event::Cancelled));
    assert!(!worker.busy());
}
