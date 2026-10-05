use super::*;

fn imported(
    gui: &mut Gui,
    files: &Files,
    path: &std::path::Path,
) -> (LibSource, crate::library::TrackId) {
    let source = LibSource::File(path.to_owned());
    gui.action(gui.node("Music file and folder paths"), Action::Focus);
    gui.frame(vec![egui::Event::Text(path.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Import music files/folders"), Action::Click);
    gui.wait(|g| {
        !g.app.library_scan.active()
            && !g.app.library_metadata.active()
            && g.app.library_metadata.catalog.track(&source).is_some()
    });
    gui.app.library_import_open = false;
    gui.app.load_source(
        1,
        Some(&Selection {
            title: "Prepared replacement".into(),
            source: source.clone(), fingerprint: None, }),
    );
    gui.wait(|g| {
        matches!(
            g.app.loads[1].as_ref().map(|l| &l.phase),
            Some(Phase::Loaded)
        )
    });
    let receipt = gui.app.loads[1].as_ref().unwrap().receipt.clone().unwrap();
    gui.app
        .engine
        .send(Command::DeckSeek {
            deck: 1,
            frac: 0.25,
        })
        .unwrap();
    gui.app
        .engine
        .send(Command::DeckCuePoint {
            deck: 1,
            pad: 0,
            del: false,
            receipt: receipt.clone(),
        })
        .unwrap();
    let grid = crate::engine::beatgrid::Grid::new(0.005, 127.0).unwrap();
    gui.app
        .engine
        .send(Command::DeckGrid {
            deck: 1,
            grid: Some(grid),
            receipt,
            ack: crate::engine::beatgrid::GridEditAck::new(),
        })
        .unwrap();
    gui.wait(|g| {
        !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&source)
                .is_some_and(|t| {
                    t.versions[t.current].content_hash.is_some()
                        && t.versions[t.current].preparation.hotcues[0].is_some()
                })
    });
    let id = gui
        .app
        .library_metadata
        .catalog
        .track(&source)
        .unwrap()
        .id
        .clone();
    assert!(files.0.join("saved/library.json").exists());
    (source, id)
}
fn open(gui: &mut Gui, source: &LibSource) {
    gui.app.refresh_library_view();
    gui.app.lib_sel = gui
        .app
        .library_view
        .indices
        .iter()
        .position(|i| &gui.app.library[*i].source == source)
        .unwrap();
    gui.app.open_cue_relocation();
    gui.frame(vec![]);
    gui.frame(vec![]);
}
#[test]
fn real_loader_gui_search_reviews_duplicates_rejects_swaps_and_preserves_preparation_with_playing_reference(
) {
    let files = Files::new();
    let original = files.wave();
    let bytes = std::fs::read(&original).unwrap();
    let mut gui = Gui::new(&files);
    let (source, identity) = imported(&mut gui, &files, &original);
    let prep = gui
        .app
        .library_metadata
        .catalog
        .track(&source)
        .unwrap()
        .versions[0]
        .preparation;
    let root = files.0.join("renamed-tree");
    std::fs::create_dir(&root).unwrap();
    let chosen = root.join("renamed.wav");
    let other = root.join("duplicate.wav");
    let decoy = root.join("Imported 120 Am.wav");
    std::fs::rename(&original, &chosen).unwrap();
    std::fs::write(&other, &bytes).unwrap();
    let mut wrong = bytes.clone();
    *wrong.last_mut().unwrap() ^= 1;
    std::fs::write(&decoy, wrong).unwrap();
    let before = std::fs::read(files.0.join("saved/library.json")).unwrap();
    open(&mut gui, &source);
    let stale_search = gui.node("Search replacement folders");
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text(root.display().to_string())]);
    gui.frame(vec![]);
    gui.action(stale_search, Action::Click);
    assert!(
        !gui.app.library_scan.active(),
        "obsolete text action must not search new roots"
    );
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| !g.app.library_scan.active() && !g.app.library_metadata.active());
    gui.frame(vec![]);
    assert!(gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some(chosen.to_str().unwrap())));
    assert!(gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some(other.to_str().unwrap())));
    assert!(!gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some(decoy.to_str().unwrap())));
    assert!(
        !gui.nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Verify and use selected replacement")),
        "no automatic first/unique choice"
    );
    assert_eq!(
        std::fs::read(files.0.join("saved/library.json")).unwrap(),
        before
    );
    gui.action(gui.node(chosen.to_str().unwrap()), Action::Click);
    let stale_choice = gui.node("Verify and use selected replacement");
    gui.action(gui.node(other.to_str().unwrap()), Action::Click);
    gui.action(stale_choice, Action::Click);
    assert!(
        !gui.app.library_metadata.active(),
        "obsolete candidate action must not select another copy"
    );
    gui.action(gui.node(chosen.to_str().unwrap()), Action::Click);
    let mut changed = bytes.clone();
    *changed.last_mut().unwrap() ^= 2;
    std::fs::write(&chosen, changed).unwrap();
    gui.action(
        gui.node("Verify and use selected replacement"),
        Action::Click,
    );
    gui.wait(|g| !g.app.library_metadata.active());
    assert!(gui.app.library_metadata.catalog.track(&source).is_some());
    assert!(gui
        .app
        .library_metadata
        .catalog
        .track(&LibSource::File(chosen.clone()))
        .is_none());
    assert_eq!(
        std::fs::read(files.0.join("saved/library.json")).unwrap(),
        before,
        "failed choice must preserve saved catalog"
    );
    // A fresh search/review can use the unchanged identical copy.
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| !g.app.library_scan.active() && !g.app.library_metadata.active());
    gui.frame(vec![]);
    gui.action(gui.node(other.to_str().unwrap()), Action::Click);
    gui.action(
        gui.node("Verify and use selected replacement"),
        Action::Click,
    );
    let destination = LibSource::File(other.clone());
    gui.wait(|g| {
        !g.app.library_metadata.active()
            && g.app.library_metadata.catalog.track(&destination).is_some()
    });
    let saved = crate::library::read(&files.0.join("saved/library.json")).unwrap();
    let track = saved.track(&destination).unwrap();
    assert_eq!(track.id, identity);
    assert_eq!(track.versions[track.current].preparation, prep);
    assert_eq!(
        saved
            .version(&source, FileFingerprint::read(&other))
            .map(|v| v.preparation),
        None,
        "new inode is not an old receipt"
    );
    assert_eq!(std::fs::read(other).unwrap(), bytes);
    assert!(gui.nonzero);
}

#[test]
fn blocked_replacement_filesystem_does_not_block_essential_cue_saves_and_cancel_is_inert() {
    let files = Files::new();
    let original = files.wave();
    let mut gui = Gui::new(&files);
    let (source, identity) = imported(&mut gui, &files, &original);
    let root = files.0.join("moved");
    std::fs::create_dir(&root).unwrap();
    std::fs::rename(&original, root.join("moved.wav")).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let mut first = true;
    gui.app.library_scan = LibraryScan::with_inventory(move || {
        if first {
            first = false;
            started_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        }
        crate::media_location::Snapshot::discover()
    });
    gui.app
        .library_scan
        .set_performance(gui.app.engine.cmd.performance().clone());
    open(&mut gui, &source);
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text(root.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Search replacement folders"), Action::Click);
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let receipt = gui.app.loads[1].as_ref().unwrap().receipt.clone().unwrap();
    let style = crate::engine::cue_metadata::Style {
        name: crate::engine::cue_metadata::Name::new("Edited during search").unwrap(),
        color: Some([1, 2, 3]),
    };
    gui.app
        .engine
        .send(Command::DeckCueStyle {
            deck: 1,
            pad: 0,
            style,
            receipt,
        })
        .unwrap();
    gui.wait(|g| {
        !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&source)
                .unwrap()
                .versions[0]
                .preparation
                .hotcue_styles[0]
                == style
    });
    assert!(gui.app.library_scan.active());
    gui.action(gui.node("Cancel replacement search"), Action::Click);
    resume_tx.send(()).unwrap();
    gui.wait(|g| !g.app.library_scan.active());
    assert!(!gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Verify and use selected replacement")));
    let saved = crate::library::read(&files.0.join("saved/library.json")).unwrap();
    let track = saved.track(&source).unwrap();
    assert_eq!(track.id, identity);
    assert_eq!(
        track.versions[track.current].preparation.hotcue_styles[0],
        style
    );
    assert!(gui.nonzero);
}

#[test]
fn reviewed_relocation_save_conflict_keeps_old_association_and_external_bytes() {
    let files = Files::new();
    let original = files.wave();
    let mut gui = Gui::new(&files);
    let (source, identity) = imported(&mut gui, &files, &original);
    let root = files.0.join("moved");
    std::fs::create_dir(&root).unwrap();
    let moved = root.join("replacement.wav");
    std::fs::rename(&original, &moved).unwrap();
    open(&mut gui, &source);
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text(root.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| !g.app.library_scan.active() && !g.app.library_metadata.active());
    gui.frame(vec![]);
    gui.action(gui.node(moved.to_str().unwrap()), Action::Click);
    std::fs::write(
        files.0.join("saved/library.json"),
        b"preserved external change",
    )
    .unwrap();
    gui.action(
        gui.node("Verify and use selected replacement"),
        Action::Click,
    );
    gui.wait(|g| !g.app.library_metadata.active());
    assert!(!gui.app.library_metadata.durable);
    assert!(gui.app.library_metadata.label().contains("outside"));
    assert_eq!(
        gui.app.library_metadata.catalog.track(&source).unwrap().id,
        identity
    );
    assert!(gui
        .app
        .library_metadata
        .catalog
        .track(&LibSource::File(moved))
        .is_none());
    assert_eq!(
        std::fs::read(files.0.join("saved/library.json")).unwrap(),
        b"preserved external change"
    );
}

#[test]
fn original_hash_is_durably_qualified_before_first_search_choice_can_commit() {
    let files = Files::new();
    let original = files.wave();
    let mut gui = Gui::new(&files);
    let source = LibSource::File(original.clone());
    gui.action(gui.node("Music file and folder paths"), Action::Focus);
    gui.frame(vec![egui::Event::Text(original.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Import music files/folders"), Action::Click);
    gui.wait(|g| {
        !g.app.library_scan.active()
            && !g.app.library_metadata.active()
            && g.app.library_metadata.catalog.track(&source).is_some()
    });
    assert!(gui
        .app
        .library_metadata
        .catalog
        .track(&source)
        .unwrap()
        .versions[0]
        .content_hash
        .is_none());
    gui.app.library_import_open = false;
    let root = files.0.join("copies");
    std::fs::create_dir(&root).unwrap();
    let copy = root.join("copy.wav");
    std::fs::copy(&original, &copy).unwrap();
    open(&mut gui, &source);
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text(root.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| {
        !g.app.library_scan.active()
            && !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&source)
                .unwrap()
                .versions[0]
                .content_hash
                .is_some()
    });
    let saved = crate::library::read(&files.0.join("saved/library.json")).unwrap();
    assert!(saved.track(&source).unwrap().versions[0]
        .content_hash
        .is_some());
    assert!(saved.track(&LibSource::File(copy.clone())).is_none());
    gui.action(gui.node(copy.to_str().unwrap()), Action::Click);
    gui.action(
        gui.node("Verify and use selected replacement"),
        Action::Click,
    );
    gui.wait(|g| {
        !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&LibSource::File(copy.clone()))
                .is_some()
    });
    assert_eq!(
        std::fs::read(original).unwrap(),
        std::fs::read(copy).unwrap()
    );
}

#[test]
fn completed_review_cannot_survive_performance_protection_or_edited_roots() {
    let files = Files::new();
    let original = files.wave();
    let mut gui = Gui::new(&files);
    let (source, _) = imported(&mut gui, &files, &original);
    let root = files.0.join("moved");
    std::fs::create_dir(&root).unwrap();
    let moved = root.join("moved.wav");
    std::fs::rename(&original, &moved).unwrap();
    open(&mut gui, &source);
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text(root.display().to_string())]);
    gui.frame(vec![]);
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| !g.app.library_scan.active() && !g.app.library_metadata.active());
    gui.frame(vec![]);
    gui.action(gui.node(moved.to_str().unwrap()), Action::Click);
    let stale = gui.node("Verify and use selected replacement");
    gui.app.engine.send(Command::PerformanceMode(true)).unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.app
        .engine
        .send(Command::PerformanceMode(false))
        .unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.action(stale, Action::Click);
    assert!(gui.app.library_metadata.catalog.track(&source).is_some());
    assert!(!gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Verify and use selected replacement")));
    gui.action(gui.node("Search replacement folders"), Action::Click);
    gui.wait(|g| !g.app.library_scan.active() && !g.app.library_metadata.active());
    gui.frame(vec![]);
    gui.action(gui.node(moved.to_str().unwrap()), Action::Click);
    let stale = gui.node("Verify and use selected replacement");
    gui.action(gui.node("Replacement search folders"), Action::Focus);
    gui.frame(vec![egui::Event::Text("/changed-root".into())]);
    gui.frame(vec![]);
    gui.action(stale, Action::Click);
    assert!(gui.app.library_metadata.catalog.track(&source).is_some());
    assert!(!gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Verify and use selected replacement")));
}
