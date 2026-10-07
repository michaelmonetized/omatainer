use super::*;
use crate::engine::midi::{MidiMap, UnmappedNotes};
use crate::ui::library_annotations::tests::{Files, Gui};

fn setup(files: &Files) -> Gui {
    let mut gui = Gui::new(files);
    gui.app.library_annotations.open = false;
    gui.app.midi_open = true;
    gui.frame(vec![]);
    gui
}
fn map(bindings: Vec<Binding>) -> MidiMap {
    MidiMap {
        name: "Learn fixture".into(),
        matchers: vec![],
        bindings,
        unmapped_notes: UnmappedNotes::Live,
    }
}
fn note(action: Action, data: u8) -> Binding {
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

#[test]
fn native_sync_assignment_choosers_use_fixed_mode_and_global_leader_ids() {
    use crate::engine::deck_sync::{Leader, Mode};
    let files = Files::new(); let mut gui = setup(&files);
    gui.app.midi_learn.binding = Binding { deck: 1, extra: 0, ..note(Action::DeckSyncMode, 61) };
    gui.frame(vec![]);
    gui.click("MIDI sync mode"); gui.click(&format!("MIDI sync mode {}", Mode::Bar.label()));
    assert_eq!(gui.app.midi_learn.binding.extra, 3);
    assert_eq!(gui.app.midi_learn.binding.deck, 1);
    gui.app.midi_learn.binding = Binding { deck: 1, extra: 0, ..note(Action::DeckSyncLeader, 62) };
    gui.frame(vec![]);
    gui.click("MIDI sync leader"); gui.click(&format!("MIDI sync leader {}", Leader::DeckB.label()));
    assert_eq!(gui.app.midi_learn.binding.extra, 2);
    assert_eq!(gui.app.midi_learn.binding.deck, 0);
    assert!(learn::validate_binding(&gui.app.midi_learn.binding).is_ok());
}

#[test]
fn native_capture_preview_conflict_replace_edit_remove_and_cancel_use_real_dispatch() {
    let files = Files::new();
    let mut gui = setup(&files);
    let mut input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        4096,
        map(vec![note(Action::DeckPlay, 60)]),
        "Fixture USB controller",
        "fixture:0",
    );
    gui.click("Capture MIDI control");
    input.push(&[0x90, 60, 100]);
    gui.frame(vec![]);
    assert!(gui.app.engine.cmd.midi_learn().view().capture.is_some());
    assert!(
        !gui.rt.decks[0].playing,
        "capture must not execute its built-in transport action"
    );
    gui.click("Add MIDI assignment");
    assert!(gui.app.midi_learn.message.contains("already assigned"));
    gui.click("Test captured MIDI action");
    gui.frame(vec![]);
    assert!(gui.rt.decks[0].playing);
    gui.click("Replace MIDI assignment");
    assert_eq!(
        gui.app.engine.cmd.midi_learn().view().config.mappings.len(),
        1
    );
    input.push(&[0x80, 60, 0]);
    gui.app.midi_learn.selected = Some((gui.app.engine.cmd.midi_learn().view().revision, 0));
    gui.app.midi_learn.binding.action = Action::DeckCue;
    gui.frame(vec![]);
    gui.click("Update selected MIDI action");
    assert_eq!(
        gui.app.engine.cmd.midi_learn().view().config.mappings[0]
            .binding
            .action,
        Action::DeckCue
    );
    gui.rt.decks[0].pos = 1200.0;
    input.push(&[0x90, 60, 100]);
    gui.frame(vec![]);
    assert!(!gui.rt.decks[0].playing);
    assert!(gui.rt.decks[0].pos < 1200.0);
    gui.click(&description(
        &gui.app.engine.cmd.midi_learn().view().config.mappings[0],
    ));
    gui.click("Remove selected MIDI assignment");
    assert!(
        gui.app
            .engine
            .cmd
            .midi_learn()
            .view()
            .config
            .mappings
            .is_empty()
    );
    gui.app.midi_learn.binding = note(Action::DeckPlay, 0);
    gui.click("Capture MIDI control");
    gui.click("Cancel MIDI learning");
    input.push(&[0x90, 60, 100]);
    gui.frame(vec![]);
    assert!(
        gui.rt.decks[0].playing,
        "cancel must restore ordinary performance input"
    );
    gui.click("Capture MIDI control");
    gui.app.midi_open = false;
    gui.frame(vec![]);
    assert!(!gui.app.engine.cmd.midi_learn().view().armed);
}

#[test]
fn native_absolute_cc_exact_port_disconnect_and_saved_configuration_remain_explicit() {
    let files = Files::new();
    let mut gui = setup(&files);
    let mut input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        4097,
        map(vec![]),
        "Fixture USB fader",
        "fixture:1",
    );
    gui.frame(vec![]);
    gui.click("MIDI action");
    gui.click("Master gain");
    assert_eq!(gui.app.midi_learn.binding.action, Action::Master);
    assert_eq!(gui.app.midi_learn.binding.kind, MsgKind::Cc);
    gui.click("Capture MIDI control");
    assert!(gui.app.engine.cmd.midi_learn().view().armed);
    input.push(&[0xb3, 7, 32]);
    gui.frame(vec![]);
    let captured = gui.app.engine.cmd.midi_learn().view().capture.unwrap();
    assert_eq!(captured.mapping.binding.ch, 3);
    assert_eq!(captured.mapping.endpoint.id, "fixture:1");
    gui.click("Add MIDI assignment");
    input.push(&[0xb3, 7, 64]);
    gui.frame(vec![]);
    assert_eq!(gui.rt.master, 64.0 / 127.0);
    let installed = gui.app.engine.cmd.midi_learn().view().config;
    gui.app.midi_learn.endpoint = Some(captured.mapping.endpoint);
    gui.click("Capture MIDI control");
    drop(input);
    gui.wait(|gui| gui.app.engine.cmd.midi_learn().view().devices.is_empty());
    assert!(!gui.app.engine.cmd.midi_learn().view().armed);
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, installed);
    assert!(
        gui.app
            .engine
            .cmd
            .midi_learn()
            .view()
            .message
            .contains("disconnected")
    );
    gui.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_learn = installed.clone();
    gui.app
        .engine
        .cmd
        .midi_learn()
        .configure(Config::default())
        .unwrap();
    gui.frame(vec![]);
    gui.click("Restore saved MIDI assignments");
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, installed);
    let bytes = serde_json::to_vec(&gui.app.settings.applied).unwrap();
    assert_eq!(
        crate::preferences::storage::decode(&bytes)
            .unwrap()
            .0
            .current()
            .unwrap()
            .midi_learn,
        installed
    );
}

#[test]
fn native_save_waits_for_durable_receipt_and_failure_preserves_live_assignments() {
    let files = Files::new();
    let mut gui = setup(&files);
    let path = files.0.join("preferences.json");
    gui.app.settings = preferences::Settings::with_worker_for_test(path.clone());
    let mut input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        4098,
        map(vec![]),
        "Fixture USB controller",
        "fixture:2",
    );
    gui.click("Capture MIDI control");
    input.push(&[0x90, 61, 100]);
    gui.frame(vec![]);
    gui.click("Add MIDI assignment");
    let installed = gui.app.engine.cmd.midi_learn().view().config;
    assert!(gui.app.settings.profile().midi_learn.mappings.is_empty());
    gui.click("Save MIDI assignments");
    gui.wait(|gui| !gui.app.settings.busy() && gui.app.settings.profile().midi_learn == installed);
    let disk = crate::preferences::storage::load(&path, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(disk.preferences.current().unwrap().midi_learn, installed);
    let bytes = std::fs::read(&path).unwrap();
    gui.app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .appearance
        .reduced_motion = true;
    gui.frame(vec![]);
    gui.click("Save MIDI assignments");
    assert!(!gui.app.settings.busy());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        gui.app
            .settings
            .draft
            .current()
            .unwrap()
            .appearance
            .reduced_motion
    );
    gui.app.settings.draft = gui.app.settings.applied.clone();
    gui.frame(vec![]);
    std::fs::write(&path, b"external modification").unwrap();
    gui.click("Save MIDI assignments");
    gui.wait(|gui| !gui.app.settings.busy());
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, installed);
    assert_eq!(std::fs::read(&path).unwrap(), b"external modification");
    assert_ne!(gui.app.settings.message, "Preferences operation pending…");
    std::fs::write(&path, bytes).unwrap();
}

#[test]
fn native_profile_revision_cannot_retarget_old_selection_and_escape_restores_keyboard_input() {
    let files = Files::new();
    let mut gui = setup(&files);
    let mut input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        4099,
        map(vec![]),
        "Fixture USB controller",
        "fixture:3",
    );
    gui.click("Capture MIDI control");
    input.push(&[0x90, 60, 100]);
    gui.frame(vec![]);
    gui.click("Add MIDI assignment");
    let old = gui.app.engine.cmd.midi_learn().view().config.mappings[0].clone();
    gui.click(&description(&old));
    let replacement = Config {
        mappings: vec![learn::Mapping {
            endpoint: old.endpoint,
            binding: note(Action::DeckCue, 61),
        }],
    };
    gui.app
        .engine
        .cmd
        .midi_learn()
        .configure(replacement.clone())
        .unwrap();
    gui.frame(vec![]);
    gui.click("Update selected MIDI action");
    gui.click("Remove selected MIDI assignment");
    assert_eq!(gui.app.engine.cmd.midi_learn().view().config, replacement);
    gui.click(&description(&replacement.mappings[0]));
    gui.click("Remove selected MIDI assignment");
    assert!(
        gui.app
            .engine
            .cmd
            .midi_learn()
            .view()
            .config
            .mappings
            .is_empty()
    );
    gui.click("Capture MIDI control");
    gui.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!gui.app.engine.cmd.midi_learn().view().armed);
    gui.rt.playing = false;
    gui.rt.publish_for_test();
    gui.frame(vec![egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(
        gui.rt.playing,
        "cancelled learning must leave later performance shortcuts available"
    );
}
