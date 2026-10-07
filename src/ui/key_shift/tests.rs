use super::*;
use crate::library::tags::{Patch, Review};
use crate::ui::library_annotations::tests::{Files, Gui};

fn loaded(key: &str) -> (Files, Box<Gui>, LibSource) {
    let files = Files::new();
    let source = LibSource::File(files.0.join("One.flac"));
    let fingerprint = FileFingerprint::read(&files.0.join("One.flac")).unwrap();
    let mut store = crate::library::Store::open(files.0.join("catalog.json")).unwrap();
    store
        .catalog
        .upsert(
            source.clone(),
            Some(fingerprint),
            crate::library::Metadata {
                title: "One".into(),
                artist: String::new(),
                bpm: Bpm::hint(120.0),
                key: String::new(),
                duration: None,
                last_play: None,
            },
        )
        .unwrap();
    let id = store.catalog.track(&source).unwrap().id.clone();
    store
        .catalog
        .apply_tag_sidecar(
            &Review {
                id,
                source: source.clone(),
                fingerprint,
            },
            &Patch {
                key: Some(key.into()),
                ..Default::default()
            },
        )
        .unwrap();
    store.save().unwrap();
    drop(store);
    let mut gui = Box::new(Gui::new(&files));
    gui.app.library_annotations.open = false;
    gui.app.load_source(
        0,
        Some(&Selection {
            source: source.clone(),
            title: "One".into(),
            fingerprint: Some(fingerprint),
        }),
    );
    gui.wait(|gui| {
        gui.app.loads[0]
            .as_ref()
            .is_some_and(|load| matches!(load.phase, load_status::Phase::Loaded))
    });
    gui.frame(vec![]);
    gui.click("key shift…");
    gui.frame(vec![]);
    (files, gui, source)
}
#[test]
fn actual_native_offset_and_reset_are_independent_and_preserve_original_file_bytes() {
    let (files, mut gui, _) = loaded("C");
    let before = std::fs::read(files.0.join("One.flac")).unwrap();
    let original = gui.rt.decks[0].audio.clone().unwrap();
    let mixer = (gui.rt.master, gui.rt.xfader);
    gui.click("Deck A: Key shift up");
    assert_eq!(gui.app.snap.decks[0].key_shift, 1);
    assert_eq!(gui.app.snap.decks[1].key_shift, 0);
    assert!(!gui.rt.decks[0].keylock);
    gui.click("Deck B: Key shift up");
    assert_eq!(gui.app.snap.decks[1].key_shift, 1);
    gui.click("Deck A: Reset key shift");
    assert_eq!(gui.app.snap.decks[0].key_shift, 0);
    assert_eq!(gui.app.snap.decks[1].key_shift, 1);
    assert!(Arc::ptr_eq(
        gui.rt.decks[0].audio.as_ref().unwrap(),
        &original
    ));
    assert_eq!((gui.rt.master, gui.rt.xfader), mixer);
    assert!(!gui.rt.decks[0].playing);
    assert_eq!(std::fs::read(files.0.join("One.flac")).unwrap(), before);
    assert!(gui
        .visible_text()
        .any(|text| text == "Confirmed offset: +0 semitones"));
}
#[test]
fn actual_chosen_key_match_uses_loaded_provenance_after_browsing_and_undoes_lock_and_offset_together(
) {
    let (_files, mut gui, source) = loaded("C");
    assert_eq!(gui.app.loaded_musical_key(0).0, Key::parse("C"));
    gui.app.lib_filter = "Two".into();
    gui.frame(vec![]);
    let before = gui.app.engine.undo.view().cursor;
    let old_position = gui.rt.decks[0].pos;
    gui.click("Deck A: Target key");
    gui.click("D · 10B");
    gui.click("Deck A: Match chosen key");
    assert_eq!(gui.app.snap.decks[0].key_shift, 2);
    assert!(gui.app.snap.decks[0].keylock);
    assert_eq!(gui.rt.decks[0].pos, old_position);
    assert_eq!(gui.app.engine.undo.view().cursor, before + 1);
    assert_eq!(
        gui.app.loads[0]
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .source,
        source
    );
    assert!(gui
        .visible_text()
        .any(|text| text == "Requested result: D · 10B"));
    gui.rt.apply(Command::Undo);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    assert_eq!(gui.app.snap.decks[0].key_shift, 0);
    assert!(!gui.app.snap.decks[0].keylock);
    gui.rt.apply(Command::Redo);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    assert_eq!(gui.app.snap.decks[0].key_shift, 2);
    assert!(gui.app.snap.decks[0].keylock);
    assert_eq!(gui.app.snap.decks[1].key_shift, 0);
    println!(
        "KEY_SHIFT_NATIVE_RECEIPT {}",
        serde_json::json!({"source_key":"C","explicit_target":"D","confirmed_offset":2,"browser_retargeted":false,"single_undo_entry":true,"lock_and_offset_undo_redo":true,"transport_unchanged":true,"physical_devices_opened":false})
    );
}
#[test]
fn actual_unknown_and_opposite_mode_targets_refuse_matching_visibly_without_a_history_edit() {
    for source_key in ["C", ""] {
        let (_files, mut gui, _) = loaded(source_key);
        let before = gui.app.engine.undo.view().cursor;
        gui.click("Deck A: Target key");
        gui.click(if source_key.is_empty() {
            "D · 10B"
        } else {
            "Cm · 5A"
        });
        gui.click("Deck A: Match chosen key");
        assert_eq!(gui.app.engine.undo.view().cursor, before);
        assert_eq!(gui.app.snap.decks[0].key_shift, 0);
        assert!(!gui.app.snap.decks[0].keylock);
        assert!(
            gui.app.key_shift.messages[0].contains(if source_key.is_empty() {
                "Analyze"
            } else {
                "preserves major or minor"
            }),
            "{}",
            gui.app.key_shift.messages[0]
        );
    }
}
