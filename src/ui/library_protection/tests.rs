use super::*;
use crate::ui::library_annotations::tests::{Files, Gui};

#[test]
fn native_lock_review_persists_and_unlocks_without_replacing_playing_audio_or_source_bytes() {
    let files = Files::new();
    let bytes = std::fs::read(files.0.join("One.flac")).unwrap();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    let loaded = gui.rt.decks[0].audio.clone().unwrap();
    gui.click("locks…");
    gui.click("Capture filtered preparation");
    assert!(gui.app.library_protection.targets.len() >= 3);
    for label in [
        "Change grid lock",
        "Lock grid",
        "Change BPM lock",
        "Lock BPM",
        "Change metadata lock",
        "Lock metadata",
    ] {
        gui.click(label);
    }
    gui.click("Review preparation locks");
    gui.click("Save reviewed preparation locks");
    gui.finish();
    assert!(gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .all(|track| track.locks
            == Locks {
                grid: true,
                bpm: true,
                metadata: true
            }));
    assert!(Arc::ptr_eq(
        &loaded,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert_eq!(std::fs::read(files.0.join("One.flac")).unwrap(), bytes);
    let catalog = crate::library::read(&files.0.join("catalog.json")).unwrap();
    assert!(catalog.tracks.iter().all(|track| !track.locks.is_empty()));
    gui.click("Capture filtered preparation");
    gui.click("Change grid lock");
    gui.click("Lock grid");
    gui.click("Review preparation locks");
    gui.click("Save reviewed preparation locks");
    gui.finish();
    assert!(gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .all(|track| !track.locks.grid && track.locks.bpm && track.locks.metadata));
}

#[test]
fn changing_a_native_lock_draft_invalidates_its_review_and_requires_new_consent() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    gui.click("locks…");
    gui.click("Capture selected preparation");
    gui.click("Change BPM lock");
    gui.click("Lock BPM");
    gui.click("Review preparation locks");
    assert!(gui.app.library_protection.reviewed.is_some());
    gui.click("Lock BPM");
    assert!(gui.app.library_protection.reviewed.is_none());
    assert!(gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .all(|track| track.locks.is_empty()));
}

#[test]
fn actual_read_only_file_load_recalls_locked_bpm_and_rejects_a_new_manual_grid() {
    use crate::engine::{
        beatgrid::{Grid, GridEditAck, GridEditState},
        Command,
    };
    use std::os::unix::fs::PermissionsExt;
    let files = Files::new();
    let path = files.0.join("One.flac");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    gui.app.lib_filter = "One".into();
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), 1);
    let item = gui.app.library[gui.app.library_view.indices[0]].clone();
    let fingerprint = item.fingerprint.unwrap();
    gui.app.library_metadata.update(library_metadata::Patch {
        tags: None,
        source: item.source.clone(),
        fingerprint,
        bpm: Bpm::new(123.0, crate::ui::bpm::Origin::Heuristic),
        duration: None,
    });
    gui.wait(|g| {
        g.app
            .library_metadata
            .catalog
            .version(&item.source, Some(fingerprint))
            .is_some_and(|v| v.metadata.bpm.value() == Some(123.0))
            && !g.app.library_metadata.active()
    });
    gui.click("locks…");
    gui.click("Capture selected preparation");
    for label in [
        "Change BPM lock",
        "Lock BPM",
        "Change grid lock",
        "Lock grid",
    ] {
        gui.click(label);
    }
    gui.click("Review preparation locks");
    gui.click("Save reviewed preparation locks");
    gui.finish();
    gui.click("Load selected crate item to deck A");
    gui.wait(|g| {
        g.rt.decks[0]
            .audio
            .as_ref()
            .is_some_and(|audio| audio.path == path.to_string_lossy())
    });
    assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().bpm, 123.0);
    let receipt = gui.app.loads[0].as_ref().unwrap().receipt.clone().unwrap();
    assert!(receipt.grid_is_locked());
    let ack = GridEditAck::new();
    gui.rt.apply(Command::DeckGrid {
        deck: 0,
        grid: Some(Grid::new(2.0, 150.0).unwrap()),
        receipt,
        ack: ack.clone(),
    });
    assert_eq!(ack.state(), GridEditState::Rejected);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
