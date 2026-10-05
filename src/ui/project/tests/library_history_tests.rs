use super::*;
use crate::engine::{load_receipt::Receipt, media_source::BuiltinStem, preparation::Preparation};

fn settle_library(gui: &mut Gui) {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        gui.frame(vec![]);
        if !gui.app.library_metadata.active() && gui.app.library_initialized {
            assert!(
                gui.app.library_metadata.durable,
                "{}",
                gui.app.library_metadata.label()
            );
            return;
        }
        assert!(
            Instant::now() < until,
            "library persistence did not settle: {}",
            gui.app.library_metadata.label()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn stored(path: &PathBuf, stem: BuiltinStem) -> Preparation {
    crate::library::read(path)
        .unwrap()
        .version(&LibSource::Builtin(stem), None)
        .unwrap()
        .preparation
}
fn deck_preparation(gui: &Gui) -> Preparation {
    let deck = &gui.rt.decks[0];
    let sr = f64::from(deck.audio.as_ref().unwrap().sr);
    Preparation {
        grid: None,        cue: deck.cue_pos / sr,
        source_gain: deck.source_gain.policy(),
        hotcue_styles: [crate::engine::cue_metadata::Style::default(); 8],
        hotcues: std::array::from_fn(|i| deck.hotcues[i].set.then_some(deck.hotcues[i].pos / sr)),
        loop_region: (deck.loop_len > 0.0).then_some(crate::engine::preparation::Loop {
            start: deck.loop_start / sr,
            length: deck.loop_len / sr,
            enabled: deck.loop_on,
        }),
    }
}

#[test]
fn undo_redo_and_media_identity_restoration_publish_durable_preparation() {
    let files = Files::new();
    let store = files.path("library.json");
    let mut gui = Gui::new();
    gui.app.start_library_store(store.clone());
    settle_library(&mut gui);
    gui.app.send(Command::DeckSeek {
        deck: 0,
        frac: 0.25,
    });
    gui.app.send(Command::DeckHotCue {
        deck: 0,
        pad: 1,
        del: false,
    });
    gui.rt.process(&mut []);
    let original = deck_preparation(&gui);
    settle_library(&mut gui);
    assert_eq!(stored(&store, BuiltinStem::Drums), original);

    gui.app.load_source(
        0,
        Some(&crate::engine::media_source::Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "Harmony".into(), fingerprint: None, }),
    );
    gui.rt.process(&mut []);
    settle_library(&mut gui);
    let harmony = deck_preparation(&gui);
    gui.app.send(Command::DeckSeek { deck: 0, frac: 0.6 });
    gui.rt.process(&mut []);
    settle_library(&mut gui);
    assert_ne!(stored(&store, BuiltinStem::Harmony), harmony);
    history_key(&mut gui, false);
    settle_library(&mut gui);
    assert_eq!(stored(&store, BuiltinStem::Harmony), harmony);
    history_key(&mut gui, false);
    settle_library(&mut gui);
    assert_eq!(deck_preparation(&gui), original);
    assert_eq!(stored(&store, BuiltinStem::Drums), original);
    // An undo-restored watch remains live and updates the original identity.
    gui.app.send(Command::DeckSeek {
        deck: 0,
        frac: 0.42,
    });
    gui.rt.process(&mut []);
    let changed = deck_preparation(&gui);
    settle_library(&mut gui);
    assert_eq!(stored(&store, BuiltinStem::Drums), changed);
    history_key(&mut gui, false);
    settle_library(&mut gui);
    assert_eq!(stored(&store, BuiltinStem::Drums), original);
    history_key(&mut gui, true);
    settle_library(&mut gui);
    assert_eq!(stored(&store, BuiltinStem::Drums), changed);
    assert_eq!(stored(&store, BuiltinStem::Harmony), harmony);
}

#[test]
fn native_project_reopen_publishes_saved_preparation_with_verified_identity() {
    let files = Files::new();
    let store = files.path("library.json");
    let project = files.path("prepared.omat");
    let mut gui = Gui::new();
    gui.app.start_library_store(store.clone());
    settle_library(&mut gui);
    gui.app.send(Command::DeckSeek { deck: 0, frac: 0.2 });
    gui.app.send(Command::DeckHotCue {
        deck: 0,
        pad: 3,
        del: false,
    });
    gui.app.send(Command::DeckLoop {
        deck: 0,
        beats: 4.0,
    });
    gui.rt.process(&mut []);
    let saved = deck_preparation(&gui);
    gui.save_as_ui(&project);
    gui.app.send(Command::DeckSeek { deck: 0, frac: 0.7 });
    gui.app.send(Command::DeckHotCue {
        deck: 0,
        pad: 3,
        del: true,
    });
    gui.rt.process(&mut []);
    settle_library(&mut gui);
    assert_ne!(stored(&store, BuiltinStem::Drums), saved);
    gui.menu("New project");
    gui.click_label("Discard changes");
    gui.settle();
    gui.menu("Open project…");
    gui.enter_path(&project);
    gui.click_label("Open");
    gui.settle();
    settle_library(&mut gui);
    assert_eq!(deck_preparation(&gui), saved);
    assert_eq!(stored(&store, BuiltinStem::Drums), saved);
    assert!(
        !gui.app.project_dirty(),
        "publishing saved preparation does not edit the reopened project"
    );
}

#[test]
fn successful_startup_preparation_invalidates_earlier_clean_checkpoint_and_retires_receipt() {
    let mut gui = Gui::new();
    let before = gui.app.engine.undo.checkpoint();
    let preparation = Preparation {
        cue: 0.125,
        ..Default::default()
    };
    gui.app.send(Command::DeckRestorePreparation {
        deck: 0,
        receipt: gui.app.engine.initial_playback[0].clone().unwrap(),
        preparation,
    });
    gui.rt.process(&mut []);
    assert_ne!(gui.app.engine.undo.checkpoint(), before);
    assert!(gui.app.project_dirty());
    assert_eq!(deck_preparation(&gui), preparation);
    let revision = gui.app.engine.project.revision();
    let ignored = Command::DeckRestorePreparation {
        deck: 0,
        receipt: Receipt::new(),
        preparation: Preparation::default(),
    };
    let counts = crate::engine::test_alloc::measure(|| gui.rt.apply(ignored));
    assert_eq!(
        (counts.allocations, counts.frees),
        (0, 0),
        "stale last-owned receipt retires off audio"
    );
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert_eq!(deck_preparation(&gui), preparation);
}
