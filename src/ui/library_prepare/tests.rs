use super::*;
use crate::ui::library_annotations::tests::{Files, Gui};

fn setup(files: &Files) -> Gui {
    let mut gui = Gui::new(files);
    gui.app.library_annotations.open = false;
    gui.app.lib_filter.clear();
    gui.app.library_view.search_all = false;
    gui.app.refresh_library_view();
    gui.click("prepare…");
    gui
}
fn pick(gui: &mut Gui, title: &str) {
    gui.app.lib_filter = format!("title:{title}");
    gui.app.refresh_library_view();
    gui.app.lib_sel = 0;
    gui.frame(vec![]);
}

#[test]
fn ns7_controls_prepare_browser_and_load_use_the_queue_in_the_native_gui() {
    let files = Files::new(); let mut gui = setup(&files);
    pick(&mut gui, "One"); gui.app.prepare_browser_rows(false);
    pick(&mut gui, "Two"); gui.app.prepare_browser_rows(false); gui.app.publish_library_selection();
    gui.app.library_prepare.selected = Some(gui.app.library_prepare.entries[0].id.clone());
    gui.app.publish_prepare_controller_view();
    let map = crate::engine::midi::builtin_maps().unwrap().into_iter().find(|map| map.name == "Numark NS7 (original)").unwrap();
    let mut input = gui.app.engine.midi.open_for_test(&gui.app.engine.cmd, 88, map, "Synthetic NS7 controls", "synthetic:ns7-prepare");
    input.push(&[0x90,9,127]); input.push(&[0xb0,0x44,1]); input.push(&[0x90,0x0c,127]);
    gui.wait(|gui| matches!(gui.app.loads[0].as_ref().map(|load| &load.phase), Some(Phase::Loaded)));
    assert!(gui.app.library_prepare.open); assert_eq!(gui.rt.decks[0].title, "Two");
    assert_eq!(gui.app.library_prepare.selected.as_ref(), Some(&gui.app.library_prepare.entries[1].id));
    input.push(&[0x90,10,127]); gui.frame(vec![]); assert!(!gui.app.library_prepare.open);
    assert_eq!(gui.app.lib_filter, "title:Two");
}

#[test]
fn native_queue_reorder_preview_rejected_load_and_durable_restore_preserve_sources() {
    let files = Files::new();
    let mut gui = setup(&files);
    let bytes = std::fs::read(files.0.join("One.flac")).unwrap();
    gui.click("Queue filtered crate");
    let original: Vec<_> = gui
        .app
        .library_prepare
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(original.len(), 5); // two builtins plus the three imported fixture tracks
    let selected = gui
        .app
        .library_prepare
        .entries
        .iter()
        .find(|entry| entry.selection.title == "One")
        .unwrap()
        .id
        .clone();
    gui.app.library_prepare.selected = Some(selected.clone());
    gui.frame(vec![]);
    gui.click("Move prepared track up");
    assert_eq!(gui.app.library_prepare.entries[1].id, selected);
    gui.click("Load prepared track on deck A");
    gui.wait(|gui| {
        matches!(
            gui.app.loads[0].as_ref().map(|load| &load.phase),
            Some(Phase::Loaded)
        )
    });
    assert!(
        gui.app
            .library_prepare
            .entries
            .iter()
            .any(|entry| entry.id == selected)
    );
    let loaded = gui.rt.decks[0].audio.as_ref().unwrap().clone();
    let position = gui.rt.decks[0].pos;
    gui.click("Preview prepared track on deck A");
    gui.frame(vec![]);
    assert!(!gui.rt.decks[0].playing);
    assert!(gui.app.engine.snapshot().decks[0].previewing);
    assert!(
        gui.app
            .library_prepare
            .entries
            .iter()
            .any(|entry| entry.id == selected)
    );
    gui.click("Stop prepared preview");
    assert_eq!(gui.rt.decks[0].pos, position);
    gui.rt.apply(Command::DeckLoadLock {
        deck: 0,
        enabled: true,
    });
    gui.rt.apply(Command::DeckPlay { deck: 0 });
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    let other = gui
        .app
        .library_prepare
        .entries
        .iter()
        .find(|entry| entry.selection.title == "Two")
        .unwrap()
        .id
        .clone();
    gui.app.library_prepare.selected = Some(other.clone());
    gui.frame(vec![]);
    gui.click("Load prepared track on deck A");
    assert!(gui.app.deck_load_review.is_some());
    gui.click("Keep playing track");
    assert!(Arc::ptr_eq(
        &loaded,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert!(gui.rt.decks[0].playing);
    assert!(
        gui.app
            .library_prepare
            .entries
            .iter()
            .any(|entry| entry.id == other)
    );
    gui.text("Saved queue name", "Requests");
    gui.click("Save prepare queue as crate");
    gui.wait(|gui| {
        gui.app.library_crates.pending.is_none() && gui.app.library_prepare.saved.is_some()
    });
    let saved = gui.app.library_prepare.saved.clone().unwrap();
    let expected: Vec<_> = gui
        .app
        .library_prepare
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(
        gui.app
            .library_metadata
            .catalog
            .crates
            .node(&saved)
            .unwrap()
            .members,
        expected
    );
    assert!(
        gui.app.library_crates.selected.is_none(),
        "queue save must not navigate the browser"
    );
    gui.click("Remove prepared track");
    assert_eq!(gui.app.library_prepare.entries.len(), 4);
    gui.click("Restore last saved queue");
    assert_eq!(
        gui.app
            .library_prepare
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    let disk = crate::library::read(&files.0.join("catalog.json")).unwrap();
    assert_eq!(disk.crates.node(&saved).unwrap().members, expected);
    assert_eq!(std::fs::read(files.0.join("One.flac")).unwrap(), bytes);
}

#[test]
fn captured_hardware_order_and_changed_file_load_fail_without_consuming_queue() {
    let files = Files::new();
    let mut gui = setup(&files);
    pick(&mut gui, "One");
    gui.app.publish_library_selection();
    gui.app.engine.midi.receive_map_for_test(
        &gui.app.engine.cmd,
        123,
        crate::engine::midi::MidiMap {
            name: "Prepare fixture".into(),
            matchers: vec![],
            unmapped_notes: crate::engine::midi::UnmappedNotes::Ignore,
            bindings: vec![crate::engine::midi::Binding {
                kind: crate::engine::midi::MsgKind::Note,
                ch: 0,
                data: 60,
                action: crate::engine::midi::Action::Prepare,
                deck: 0,
                extra: 0,
                relative: None,
                controls: None,
                pair_order: None,
            }],
        },
        "Synthetic prepare controller",
        &[0x90, 60, 127],
    );
    pick(&mut gui, "Two");
    gui.app.publish_library_selection();
    gui.frame(vec![]);
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    assert_eq!(gui.app.library_prepare.entries[0].selection.title, "One");
    let version = gui.app.library_prepare.entries[0].selection.fingerprint;
    std::fs::write(
        files.0.join("One.flac"),
        include_bytes!("../../../tests/fixtures/audio/tone.flac"),
    )
    .unwrap();
    assert_ne!(FileFingerprint::read(&files.0.join("One.flac")), version);
    let prior = gui.rt.decks[1].audio.clone();
    gui.click("Load prepared track on deck B");
    gui.wait(|gui| {
        matches!(
            gui.app.loads[1].as_ref().map(|load| &load.phase),
            Some(Phase::Failed(_))
        )
    });
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    assert!(Arc::ptr_eq(
        prior.as_ref().unwrap(),
        gui.rt.decks[1].audio.as_ref().unwrap()
    ));
    assert!(
        matches!(&gui.app.loads[1].as_ref().unwrap().phase,Phase::Failed(error) if error.contains("changed"))
    );
}

fn converted_output(gui: &mut Gui) -> f64 {
    let (_, placeholder) = Engine::headless_for_test(48000, 256);
    let rt = std::mem::replace(&mut *gui.rt, placeholder);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let mut energy = 0.0;
    for _ in 0..16 {
        let mut output = [0.0_f32; 256];
        callback.render(&mut output);
        energy += output
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>();
    }
    callback.renderer_mut_for_test().publish_for_test();
    std::mem::swap(&mut *gui.rt, callback.renderer_mut_for_test());
    gui.frame(vec![]);
    energy
}

#[test]
fn native_retain_silent_load_paused_preview_and_actual_play_follow_distinct_policies() {
    let files = Files::new();
    let mut gui = setup(&files);
    pick(&mut gui, "One");
    gui.click("Queue selected track");
    let id = gui.app.library_prepare.entries[0].id.clone();
    gui.click("Retain after play");
    assert!(gui.app.library_prepare.retain);
    gui.click("Load prepared track on deck A");
    gui.wait(|gui| {
        matches!(
            gui.app.loads[0].as_ref().map(|load| &load.phase),
            Some(Phase::Loaded)
        )
    });
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    gui.rt.decks[0].loop_on = true;
    gui.rt.decks[0].loop_start = 0.0;
    gui.rt.decks[0].loop_len = gui.rt.decks[0].audio.as_ref().unwrap().frames() as f64;
    gui.rt.decks[0].playing = true;
    gui.rt.xfader = 0.0;
    gui.rt.master = 1.0;
    assert!(converted_output(&mut gui) > 0.1);
    assert_eq!(gui.app.library_prepare.entries[0].id, id);
    gui.rt.master = 0.0;
    converted_output(&mut gui);
    gui.click("Retain after play");
    assert!(!gui.app.library_prepare.retain);
    assert_eq!(converted_output(&mut gui), 0.0);
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    gui.rt.decks[0].playing = false;
    gui.rt.master = 1.0;
    gui.rt.publish_for_test();
    gui.wait(|gui| !gui.app.snap.decks[0].playing);
    gui.click("Preview prepared track on deck A");
    let energy = converted_output(&mut gui);
    assert!(
        energy > 0.1,
        "energy={energy}, playing={}, position={}, master={}, cue={}, preview={}, message={}, submission={:?}",
        gui.rt.decks[0].playing,
        gui.rt.decks[0].pos,
        gui.rt.master,
        gui.rt.decks[0].cue_pos,
        gui.app.engine.snapshot().decks[0].previewing,
        gui.app.library_prepare.message,
        gui.app.engine.cmd.stats().last_error
    );
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    gui.click("Stop prepared preview");
    gui.rt.decks[0].playing = true;
    assert!(converted_output(&mut gui) > 0.1);
    assert!(gui.app.library_prepare.entries.is_empty());
    assert!(
        gui.app
            .library_prepare
            .message
            .contains("confirmed playing output")
    );
}

#[test]
fn atomic_queue_admission_rejects_stale_batches_without_adding_healthy_prefixes() {
    let files = Files::new();
    let mut gui = setup(&files);
    pick(&mut gui, "One");
    let item = gui.app.selected_library_item().unwrap();
    let healthy = Arc::new(Selection {
        source: item.source.clone(),
        title: item.title.clone(),
        fingerprint: item.fingerprint,
    });
    let stale = Arc::new(Selection {
        source: LibSource::File(files.0.join("gone.flac")),
        title: "gone".into(),
        fingerprint: healthy.fingerprint,
    });
    gui.app.prepare_selections(vec![healthy.clone(), stale]);
    assert!(gui.app.library_prepare.entries.is_empty());
    gui.app.prepare_selections(vec![healthy.clone(), healthy]);
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    let id = gui.app.library_prepare.entries[0].id.clone();
    pick(&mut gui, "Two");
    let item = gui.app.selected_library_item().unwrap();
    let oversized = Arc::new(Selection {
        source: item.source.clone(),
        title: "oversized".repeat(300000),
        fingerprint: item.fingerprint,
    });
    gui.app.prepare_selections(vec![oversized]);
    assert_eq!(gui.app.library_prepare.entries.len(), 1);
    assert_eq!(gui.app.library_prepare.entries[0].id, id);
    assert!(gui.app.library_prepare.message.contains("2 MiB"));
}
