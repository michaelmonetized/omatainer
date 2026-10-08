use super::*;
use crate::engine::midi::{
    learn::{Config, Mapping},
    Action, Binding, MidiMap, MsgKind, UnmappedNotes,
};
use crate::ui::library_annotations::tests::{Files, Gui};

fn setup(files: &Files) -> Gui {
    let mut gui = Gui::new(files);
    gui.app.library_annotations.open = false;
    gui.app.midi_open = true;
    gui.app.settings =
        preferences::Settings::with_worker_for_test(files.0.join("preferences.json"));
    gui.frame(vec![]);
    gui.click("MIDI mapping presets");
    gui.frame(vec![]);
    gui
}
fn binding(action: Action, data: u8) -> Binding {
    Binding {
        kind: MsgKind::Note,
        ch: 0,
        data,
        action,
        deck: 0,
        extra: 0,
        relative: None,
        controls: None,
        pair_order: None,
    }
}
fn map() -> MidiMap {
    MidiMap {
        name: "Preset fixture".into(),
        matchers: vec![],
        bindings: vec![binding(Action::DeckPlay, 60), binding(Action::DeckPlay, 61)],
        unmapped_notes: UnmappedNotes::Ignore,
    }
}
fn port(id: &str) -> Endpoint {
    Endpoint {
        name: "Same controller".into(),
        id: id.into(),
    }
}
fn saved(gui: &mut Gui) {
    gui.wait(|gui| !gui.app.settings.busy());
    assert!(
        !gui.app.settings.message.contains("failed"),
        "{}",
        gui.app.settings.message
    );
}

#[test]
fn native_preset_save_duplicate_rename_export_import_and_defaults_persist_with_explicit_port_load()
{
    let files = Files::new();
    let mut gui = setup(&files);
    let first = port("machine-a:1");
    let mut input =
        gui.app
            .engine
            .midi
            .open_for_test(&gui.app.engine.cmd, 1601, map(), &first.name, &first.id);
    let config = Config {
        mappings: vec![Mapping {
            endpoint: first.clone(),
            binding: binding(Action::DeckCueHold, 60),
        }],
    };
    gui.app
        .engine
        .cmd
        .midi_learn()
        .configure(config.clone())
        .unwrap();
    gui.app.midi_learn.presets.target = Some(first.clone());
    gui.app.midi_learn.presets.name = "Stage".into();
    gui.click("Save port as preset");
    saved(&mut gui);
    assert_eq!(gui.app.settings.profile().midi_presets.len(), 1);
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, config);
    gui.app.midi_learn.presets.selected = Some("Stage".into());
    gui.app.midi_learn.presets.name = "Copy".into();
    gui.click("Duplicate preset");
    saved(&mut gui);
    gui.app.midi_learn.presets.selected = Some("Copy".into());
    gui.app.midi_learn.presets.name = "Renamed".into();
    gui.click("Rename preset");
    saved(&mut gui);
    assert_eq!(
        gui.app.midi_learn.presets.selected.as_deref(),
        Some("Renamed")
    );
    assert_eq!(
        gui.app
            .settings
            .profile()
            .midi_presets
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Stage", "Renamed"]
    );
    let export = files.0.join("stage.json");
    gui.app.midi_learn.presets.selected = Some("Stage".into());
    gui.app.midi_learn.presets.path = export.to_string_lossy().into();
    gui.click("Export MIDI preset");
    saved(&mut gui);
    assert!(!std::fs::read_to_string(&export)
        .unwrap()
        .contains(&first.id));
    gui.click("Import MIDI preset");
    gui.wait(|gui| !gui.app.settings.busy());
    assert!(gui.app.midi_learn.presets.imported.is_some());
    gui.click("Save imported preset to bank");
    assert!(!gui.app.settings.busy());
    assert!(gui.app.midi_learn.presets.message.contains("distinct"));
    gui.app.midi_learn.presets.name = "Imported".into();
    gui.click("Save imported preset to bank");
    saved(&mut gui);
    assert_eq!(gui.app.settings.profile().midi_presets.len(), 3);
    drop(input);
    let second = port("machine-b:9");
    input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        1602,
        map(),
        &second.name,
        &second.id,
    );
    gui.app.midi_learn.presets.target = Some(second.clone());
    gui.app.midi_learn.presets.selected = Some("Imported".into());
    gui.click("Review preset load");
    assert!(gui.app.midi_learn.presets.review.is_some());
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, config);
    gui.click("Apply reviewed MIDI preset");
    let current = gui.app.engine.cmd.midi_learn().view().config;
    assert_eq!(current.mappings.len(), 2);
    assert_eq!(current.mappings[0], config.mappings[0]);
    assert_eq!(current.mappings[1].endpoint, second);
    input.push(&[0x90, 60, 100]);
    gui.frame(vec![]);
    assert!(gui.app.engine.snapshot().decks[0].controls.cue_held);
    gui.click("Review factory defaults");
    gui.click("Apply reviewed MIDI preset");
    input.push(&[0x80, 60, 0]);
    gui.frame(vec![]);
    assert!(!gui.app.engine.snapshot().decks[0].controls.cue_held);
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, config);
    input.push(&[0x90, 61, 100]);
    gui.frame(vec![]);
    assert!(
        gui.rt.decks[0].playing,
        "unlisted factory control must remain active"
    );
    gui.click("Save MIDI assignments");
    saved(&mut gui);
    let disk = crate::preferences::storage::load(
        &files.0.join("preferences.json"),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        disk.preferences.current().unwrap().midi_presets,
        gui.app.settings.profile().midi_presets
    );
    assert_eq!(disk.preferences.current().unwrap().midi_learn, config);
}

#[test]
fn native_preset_stale_review_import_profile_close_and_save_failure_preserve_runtime_and_bank() {
    let files = Files::new();
    let mut gui = setup(&files);
    let endpoint = port("destination:1");
    let _input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        1611,
        map(),
        &endpoint.name,
        &endpoint.id,
    );
    let config = Config {
        mappings: vec![Mapping {
            endpoint: endpoint.clone(),
            binding: binding(Action::DeckCueHold, 60),
        }],
    };
    gui.app
        .engine
        .cmd
        .midi_learn()
        .configure(config.clone())
        .unwrap();
    let preset = Preset::capture("Stage".into(), String::new(), &endpoint, &config).unwrap();
    gui.app.save_midi_preset_bank(vec![preset.clone()]).unwrap();
    saved(&mut gui);
    gui.app.midi_learn.presets.target = Some(endpoint.clone());
    gui.app.review_midi_preset(None).unwrap();
    let changed = Config {
        mappings: vec![Mapping {
            endpoint: endpoint.clone(),
            binding: binding(Action::DeckCue, 60),
        }],
    };
    gui.app
        .engine
        .cmd
        .midi_learn()
        .configure(changed.clone())
        .unwrap();
    assert!(gui.app.confirm_midi_preset().is_err());
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, changed);
    let path = files.0.join("portable.json");
    std::fs::write(&path, serde_json::to_vec(&preset).unwrap()).unwrap();
    gui.app.midi_learn.presets.path = path.to_string_lossy().into();
    gui.app
        .settings
        .worker
        .as_ref()
        .unwrap()
        .delay
        .store(30, std::sync::atomic::Ordering::Release);
    gui.app.import_midi_preset().unwrap();
    gui.app.settings.applied.active = "Performance".into();
    gui.app.settings.draft = gui.app.settings.applied.clone();
    gui.wait(|gui| !gui.app.settings.busy());
    assert!(gui.app.midi_learn.presets.imported.is_none());
    assert!(gui.app.midi_learn.presets.message.contains("owner changed"));
    gui.app.import_midi_preset().unwrap();
    gui.app.midi_open = false;
    gui.frame(vec![]);
    gui.wait(|gui| !gui.app.settings.busy());
    assert!(gui.app.midi_learn.presets.imported.is_none());
    gui.app.midi_open = true;
    let before = gui.app.settings.applied.clone();
    std::fs::write(files.0.join("preferences.json"), b"changed externally").unwrap();
    gui.app.save_midi_preset_bank(vec![preset]).unwrap();
    gui.wait(|gui| !gui.app.settings.busy());
    assert!(gui.app.settings.message.contains("failed"));
    assert_eq!(gui.app.settings.applied, before);
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, changed);
    assert_eq!(
        std::fs::read(files.0.join("preferences.json")).unwrap(),
        b"changed externally"
    );
}
