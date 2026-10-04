use super::*;
use crate::ui::library_annotations::tests::{Files, Gui};
use media_health::Condition;
use std::os::unix::fs::PermissionsExt;

fn observation<'a>(gui: &'a Gui, path: &std::path::Path) -> &'a Observation {
    &gui.app.library_health.observations[&LibSource::File(path.into())].observation
}

#[test]
fn native_captured_crate_reports_partial_failures_without_replacing_live_decks() {
    let files = Files::new();
    let read_only = files.0.join("One.flac");
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o444)).unwrap();
    let damaged = files.0.join("Damaged.flac");
    let bytes = include_bytes!("../../../tests/fixtures/audio/tone.flac");
    std::fs::write(&damaged, &bytes[..bytes.len() - 64]).unwrap();
    let unsupported = files.0.join("Unknown.wav");
    std::fs::write(&unsupported, b"This is an unsupported audio container").unwrap();
    std::fs::write(
        files.0.join("Import-error.txt"),
        b"unsupported import extension",
    )
    .unwrap();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    assert!(
        gui.app
            .library_scan
            .summary
            .as_ref()
            .unwrap()
            .skipped_count()
            > 0
    );
    let missing = files.0.join("Two.flac");
    let unreadable = files.0.join("Three.flac");
    std::fs::remove_file(&missing).unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    gui.app.load_file(0, read_only.clone(), "One");
    gui.wait(|gui| {
        gui.rt.decks[0]
            .audio
            .as_ref()
            .is_some_and(|audio| audio.path == read_only.to_string_lossy())
    });
    let loaded = gui.rt.decks[0].audio.as_ref().unwrap().clone();
    gui.rt.apply(Command::Play);
    gui.rt.apply(Command::DeckPlay { deck: 0 });
    gui.wait(|gui| {
        !gui.app.library_metadata.active()
            && gui
                .app
                .library_metadata
                .catalog
                .version(
                    &LibSource::File(read_only.clone()),
                    FileFingerprint::read(&read_only),
                )
                .is_some_and(|version| {
                    version.metadata.duration.is_some() && version.metadata.last_play.is_some()
                })
    });
    let before = serde_json::to_vec(&*gui.app.library_metadata.catalog).unwrap();
    gui.click("health…");
    let rows = gui.app.library.clone();
    let indices = gui.app.library_view.indices.clone();
    gui.click("Validate filtered crate");
    let queue = gui.app.library_health.queue.as_ref().unwrap();
    assert!(Arc::ptr_eq(&queue.rows, &rows) && Arc::ptr_eq(&queue.indices, &indices));
    gui.app.lib_filter = "does-not-match".into();
    gui.app.refresh_library_view();
    gui.wait(|gui| !gui.app.library_health.busy());
    assert!(
        gui.rt.playing
            && gui.rt.decks[0].playing
            && Arc::ptr_eq(&loaded, gui.rt.decks[0].audio.as_ref().unwrap())
    );
    assert_eq!(observation(&gui, &read_only).condition, Condition::Ready);
    assert!(observation(&gui, &read_only).read_only);
    assert_eq!(observation(&gui, &missing).condition, Condition::Missing);
    assert_eq!(
        observation(&gui, &unreadable).condition,
        Condition::Unreadable
    );
    assert_eq!(observation(&gui, &damaged).condition, Condition::Corrupt);
    assert_eq!(
        observation(&gui, &unsupported).condition,
        Condition::Unsupported
    );
    assert!(
        gui.app.library_health.message.contains("4 need attention"),
        "{}",
        gui.app.library_health.message
    );
    assert_eq!(
        serde_json::to_vec(&*gui.app.library_metadata.catalog).unwrap(),
        before
    );
    gui.app.lib_filter = "title:One".into();
    gui.app.refresh_library_view();
    gui.frame(vec![]);
    assert!(gui
        .visible_text()
        .any(|text| text.contains("ready · read-only")));
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn native_cancel_protection_and_changed_rows_do_not_reuse_old_readiness() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    let (entered, seen) = std::sync::mpsc::sync_channel(1);
    let (resume, held) = std::sync::mpsc::sync_channel(1);
    let mut once = true;
    gui.app.loader = Some(Loader::with_health_hook(gui.app.engine.cmd.performance().clone(), move |_| {
        if once {
            once = false;
            entered.send(()).unwrap();
            held.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        }
    }).unwrap());
    gui.click("health…");
    gui.click("Validate filtered crate");
    seen.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    gui.click("Cancel media validation");
    assert!(gui.app.library_health.busy());
    resume.send(()).unwrap();
    gui.wait(|gui| !gui.app.library_health.busy());
    assert!(gui.app.library_health.message.contains("cancelled"));
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.click("Validate selected row");
    assert!(gui.app.library_health.queue.is_none());
    assert!(
        gui.app.library_health.message.contains("Performance"),
        "{}",
        gui.app.library_health.message
    );
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.app.lib_filter = "title:One".into();
    gui.app.refresh_library_view();
    gui.frame(vec![]);
    gui.click("Validate selected row");
    gui.wait(|gui| !gui.app.library_health.busy());
    let index = gui.app.library_view.indices[0];
    let mut changed = gui.app.library[index].clone();
    assert!(gui.app.library_health.observation(&changed).is_some());
    std::fs::write(files.0.join("One.flac"), b"replacement").unwrap();
    changed.fingerprint = FileFingerprint::read(&files.0.join("One.flac"));
    assert!(gui.app.library_health.observation(&changed).is_none());
}
