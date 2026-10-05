use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use lofty::{
    config::WriteOptions,
    file::TaggedFileExt,
    tag::{ItemKey, Tag, TagExt},
};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::atomic::AtomicBool,
    time::Duration,
};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "omat-tags-ui-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        Self(root)
    }
    fn store(&self) -> PathBuf {
        self.0.join("saved/library.json")
    }
    fn media(&self, fixture: &str, name: &str, title: Option<&str>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .join(fixture),
            &path,
        )
        .unwrap();
        if let Some(title) = title {
            let parsed = lofty::probe::Probe::open(&path)
                .unwrap()
                .guess_file_type()
                .unwrap()
                .read()
                .unwrap();
            let kind = parsed.primary_tag_type();
            let mut tag = Tag::new(kind);
            assert!(tag.insert_text(ItemKey::TrackTitle, title.into()));
            assert!(tag.insert_text(ItemKey::TrackArtist, "作曲家 / Björk".into()));
            let bpm = if ItemKey::Bpm.map_key(kind).is_some() {
                ItemKey::Bpm
            } else {
                ItemKey::IntegerBpm
            };
            assert!(tag.insert_text(bpm, "128".into()));
            assert!(tag.insert_text(ItemKey::InitialKey, "F#m".into()));
            tag.save_to_path(&path, WriteOptions::new().lossy_text_encoding(false))
                .unwrap();
        }
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    rendered: u64,
    reference: Option<(Engine, Box<RtEngine>)>,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.start_library_store(files.store());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            rendered: 0,
            reference: None,
        };
        gui.settle();
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<f32> {
        self.time += 0.02;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| {
                if let egui::Event::Key { modifiers, .. } = event {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1800.0))),
                time: Some(self.time),
                focused: true,
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        let mut samples = vec![0.0; 256];
        self.rt.process(&mut samples);
        if let Some((_, reference)) = &mut self.reference {
            let mut expected = vec![0.0; 256];
            reference.process(&mut expected);
            assert_eq!(
                samples, expected,
                "tag review or persistence changed playing PCM output"
            );
        }
        self.rt.publish_for_test();
        self.rendered += 128;
        samples
    }
    fn compare_playing_decks(&mut self) {
        let (engine, mut reference) = Engine::headless_for_test(48_000, 256);
        reference.decks = self.rt.decks.clone();
        self.reference = Some((engine, Box::new(reference)));
    }
    fn wait(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.frame(Vec::new());
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "tag wait: {} / {} / {}",
                self.app.library_tags.message,
                self.app.library_scan.label(),
                self.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn settle(&mut self) {
        let mut quiet = 0;
        self.wait(|gui| {
            if !gui.app.library_metadata.active()
                && !gui.app.library_scan.active()
                && !gui.app.library_tags.busy()
                && gui.app.library_tags.recovery_checked
            {
                quiet += 1;
            } else {
                quiet = 0;
            }
            quiet >= 3
        });
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, target: NodeId, action: Action) {
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data: None,
        })]);
        self.frame(Vec::new());
    }
    fn click(&mut self, label: &str) {
        self.action(self.node(label), Action::Click);
    }
    fn text(&mut self, label: &str, value: &str) {
        self.action(self.node(label), Action::Focus);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
        if value.is_empty() {
            for pressed in [true, false] {
                self.frame(vec![egui::Event::Key {
                    key: Key::Backspace,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::default(),
                }]);
            }
        } else {
            self.frame(vec![egui::Event::Text(value.into())]);
        }
        self.frame(Vec::new());
    }
    fn import(&mut self, paths: Vec<PathBuf>) {
        assert!(
            self.app.library_scan.import_with_catalog(
                paths,
                self.app.library.clone(),
                self.app.library_metadata.catalog.clone()
            ),
            "{} / protected {} / {}",
            self.app.library_scan.label(),
            self.app.engine.cmd.performance().protected(),
            self.app.library_tags.message
        );
        self.settle();
    }
    fn select(&mut self, path: &PathBuf) {
        self.app.lib_filter.clear();
        self.app.refresh_library_view();
        self.app.lib_sel = self
            .app
            .library_view
            .indices
            .iter()
            .position(|&index| self.app.library[index].source == LibSource::File(path.clone()))
            .unwrap();
        self.app.refresh_library_view();
    }
    fn inspect(&mut self, batch: bool) {
        self.app.library_tags.open = true;
        self.frame(Vec::new());
        self.frame(Vec::new());
        self.click(if batch {
            "Inspect filtered crate (batch)"
        } else {
            "Inspect selected track"
        });
        self.wait(|gui| {
            gui.app.library_tags.preview.is_some() && gui.app.library_tags.pending.is_none()
        });
        self.frame(Vec::new());
    }
    fn change(&mut self, field: &str, value: &str) {
        self.click(&format!("Change {field}"));
        self.text(&format!("New {field}"), value);
    }
    fn apply(&mut self) {
        self.click("Review these field changes");
        self.click("Apply reviewed edits to captured tracks");
        self.settle();
    }
    fn version(&self, path: &PathBuf) -> &crate::library::Version {
        let track = self
            .app
            .library_metadata
            .catalog
            .track(&LibSource::File(path.clone()))
            .unwrap();
        &track.versions[track.current]
    }
}
fn wait_store_closed(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(store) = crate::library::Store::open(path.to_path_buf()) {
            drop(store);
            return;
        }
        assert!(Instant::now() < deadline, "catalog owner did not retire");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn real_scan_tags_override_filename_hints_and_single_gui_edit_preserves_audio_and_identity() {
    let files = Files::new();
    let path = files.media(
        "tone-tags.wav",
        "Filename Artist - Guess 90 Am.wav",
        Some("Media 夜明け 🎹"),
    );
    let original = crate::engine::decode::decode_audio(&path).unwrap().sample;
    let mut gui = Gui::new(&files);
    gui.import(vec![path.clone()]);
    let source = LibSource::File(path.clone());
    let track = gui.app.library_metadata.catalog.track(&source).unwrap();
    let id = track.id.clone();
    let old_fp = track.versions[track.current].fingerprint;
    assert_eq!(gui.version(&path).metadata.title, "Media 夜明け 🎹");
    assert_eq!(gui.version(&path).metadata.artist, "作曲家 / Björk");
    assert_eq!(
        gui.version(&path).metadata.bpm.origin,
        bpm::Origin::EmbeddedTag
    );
    assert_eq!(gui.version(&path).metadata.bpm.value(), Some(128.0));
    assert_eq!(
        gui.version(&path).tags.as_ref().unwrap().fallback.title,
        "Guess 90 Am"
    );
    gui.select(&path);
    gui.app.load_sel(0);
    gui.wait(|gui| {
        gui.app.loads[0]
            .as_ref()
            .is_some_and(|load| matches!(load.phase, load_status::Phase::Loaded))
    });
    gui.settle();
    let resident = gui.rt.decks[0].audio.clone().unwrap();
    gui.app.engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    gui.frame(Vec::new());
    gui.compare_playing_decks();
    gui.inspect(false);
    gui.change("Title", "Edited Café 東京");
    gui.change("Artist", "Producer 夜");
    gui.change("BPM", "132.5");
    gui.change("Key", "D#m");
    let before_frames = gui.rendered;
    gui.apply();
    assert!(
        gui.app.library_tags.message.contains("1 saved"),
        "{}",
        gui.app.library_tags.message
    );
    assert!(gui.rendered > before_frames);
    assert!(Arc::ptr_eq(
        &resident,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert_eq!(
        gui.app.library_metadata.catalog.track(&source).unwrap().id,
        id
    );
    assert_ne!(gui.version(&path).fingerprint, old_fp);
    assert_eq!(gui.version(&path).metadata.title, "Edited Café 東京");
    assert_eq!(gui.version(&path).metadata.bpm.value(), Some(132.5));
    let observed = crate::media_tags::inspect(
        &crate::media_location::Location::resolve(&source).unwrap(),
        FileFingerprint::read(&path).unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(observed.fields.title.unwrap().value, "Edited Café 東京");
    assert_eq!(observed.fields.artist.unwrap().value, "Producer 夜");
    assert_eq!(observed.fields.key.unwrap().value, "D#m");
    let after = crate::engine::decode::decode_audio(&path).unwrap().sample;
    assert_eq!(original.data, after.data);
    // A late capture from the still-loaded old fingerprint updates the proven
    // equivalent current version instead of switching current back to old media.
    let mut preparation = crate::engine::preparation::Preparation::default();
    preparation.hotcues[0] = Some(0.1);
    gui.app.library_metadata.capture(library_store::Capture {
        source: source.clone(),
        fingerprint: old_fp,
        metadata: gui.app.capture_metadata(&source, old_fp),
        preparation: Some(preparation),
        played: Some(SystemTime::now()),
    });
    gui.settle();
    assert_eq!(gui.version(&path).preparation, preparation);
    assert_eq!(gui.version(&path).fingerprint, FileFingerprint::read(&path));
    assert!(gui.app.library_metadata.durable);
    assert!(crate::media_tags::write::recover(
        gui.app.library_metadata.tag_recovery_root.as_ref().unwrap()
    )
    .is_empty());
}

#[test]
fn native_unscanned_deck_loads_read_real_tags_and_persist_explicit_filename_fallbacks() {
    for (fixture, name) in [
        ("tone.mp3", "Filename - Guess 90 Am.mp3"),
        ("tone.flac", "Filename - Guess 90 Am.flac"),
        ("tone-tags.wav", "Filename - Guess 90 Am.wav"),
        ("tone-tags.aiff", "Filename - Guess 90 Am.aiff"),
    ] {
        let files = Files::new();
        let path = files.media(fixture, name, Some("Actual 夜明け 🎹"));
        let before = std::fs::read(&path).unwrap();
        let mut gui = Gui::new(&files);
        let initial_tracks = gui.app.library_metadata.catalog.tracks.len();
        assert!(gui
            .app
            .library_metadata
            .catalog
            .track(&LibSource::File(path.clone()))
            .is_none());
        gui.app.load_file(0, path.clone(), "Guessed selection");
        gui.wait(|gui| {
            gui.app.loads[0]
                .as_ref()
                .is_some_and(|load| matches!(load.phase, load_status::Phase::Loaded))
        });
        gui.settle();
        let version = gui.version(&path);
        assert_eq!(version.metadata.title, "Actual 夜明け 🎹");
        assert_eq!(version.metadata.artist, "作曲家 / Björk");
        assert_eq!(version.metadata.key, "F#m");
        assert_eq!(
            version.metadata.bpm,
            Bpm::new(128.0, bpm::Origin::EmbeddedTag)
        );
        let fallback = &version.tags.as_ref().unwrap().fallback;
        assert_eq!(fallback.artist, "Filename");
        assert_eq!(fallback.title, "Guess 90 Am");
        assert_eq!(fallback.bpm.origin, bpm::Origin::FilenameHint);
        assert_eq!(
            gui.rt.decks[0].audio.as_ref().unwrap().name,
            "Actual 夜明け 🎹"
        );
        assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().bpm, 128.0);
        assert_eq!(
            gui.app.loads[0].as_ref().unwrap().bpm.unwrap().origin,
            bpm::Origin::EmbeddedTag
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(gui.app.library_metadata.durable);
        let saved = crate::library::read(&files.store()).unwrap();
        assert_eq!(saved.tracks.len(), initial_tracks + 1);
        let saved_track = saved.track(&LibSource::File(path.clone())).unwrap();
        assert_eq!(
            saved_track.versions[saved_track.current].metadata.title,
            "Actual 夜明け 🎹"
        );
    }
}

#[test]
fn reviewed_multi_format_batch_keeps_captured_targets_after_browsing_changes() {
    let files = Files::new();
    let paths = [
        files.media("tone.mp3", "First.mp3", Some("Media One")),
        files.media("tone.flac", "Second.flac", Some("Media Two")),
        files.media("tone-tags.aiff", "Third.aiff", Some("Media Three")),
    ];
    let mut gui = Gui::new(&files);
    gui.import(paths.to_vec());
    gui.app.lib_filter = "Media".into();
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), 3);
    gui.inspect(true);
    gui.change("Artist", "Batch 作曲家 🎹");
    gui.click("Review these field changes");
    gui.app.lib_filter = "no captured row matches now".into();
    gui.app.refresh_library_view();
    gui.click("Apply reviewed edits to captured tracks");
    gui.settle();
    assert!(
        gui.app.library_tags.message.contains("3 saved"),
        "{}",
        gui.app.library_tags.message
    );
    for path in paths {
        assert_eq!(gui.version(&path).metadata.artist, "Batch 作曲家 🎹");
        let source = LibSource::File(path.clone());
        let observed = crate::media_tags::inspect(
            &crate::media_location::Location::resolve(&source).unwrap(),
            FileFingerprint::read(&path).unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(observed.fields.artist.unwrap().value, "Batch 作曲家 🎹");
    }
}

#[test]
fn read_only_user_sidecar_and_explicit_clears_survive_rescan_and_restart() {
    let files = Files::new();
    let path = files.media("tone.flac", "Guess 99 Am.flac", Some("Media original"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut gui = Gui::new(&files);
    gui.import(vec![path.clone()]);
    gui.select(&path);
    gui.inspect(false);
    gui.change("Title", "Sidecar 夜");
    gui.change("Artist", "");
    gui.change("BPM", "");
    gui.apply();
    assert!(
        gui.app.library_tags.message.contains("1 saved"),
        "{}",
        gui.app.library_tags.message
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(gui.version(&path).metadata.title, "Sidecar 夜");
    assert_eq!(gui.version(&path).metadata.artist, "");
    assert_eq!(gui.version(&path).metadata.bpm, Bpm::USER_CLEARED);
    gui.app.library_tags.open = false;
    gui.import(vec![path.clone()]);
    assert_eq!(gui.version(&path).metadata.bpm, Bpm::USER_CLEARED);
    drop(gui);
    wait_store_closed(&files.store());
    let reopened = Gui::new(&files);
    assert_eq!(reopened.version(&path).metadata.title, "Sidecar 夜");
    assert_eq!(reopened.version(&path).metadata.artist, "");
    assert_eq!(reopened.version(&path).metadata.bpm, Bpm::USER_CLEARED);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn changed_review_and_cancelled_or_protected_drafts_cannot_edit_other_bytes() {
    for action in ["stale", "cancel", "protect"] {
        let files = Files::new();
        let path = files.media("tone-tags.wav", "Reviewed.wav", Some("Media original"));
        let mut gui = Gui::new(&files);
        gui.import(vec![path.clone()]);
        gui.select(&path);
        gui.inspect(false);
        gui.change("Title", "Should not be installed");
        gui.click("Review these field changes");
        let old_action = gui.node("Apply reviewed edits to captured tracks");
        match action {
            "stale" => {
                std::fs::write(&path, b"external replacement kept intact").unwrap();
            }
            "cancel" => {
                gui.app.library_tags.cancel();
            }
            "protect" => {
                gui.app.engine.send(Command::PerformanceMode(true)).unwrap();
                gui.frame(Vec::new());
            }
            _ => unreachable!(),
        }
        let before = std::fs::read(&path).unwrap();
        gui.action(old_action, Action::Click);
        gui.settle();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(gui.version(&path).metadata.title, "Media original");
    }
    // Changing a reviewed value invalidates the old action identity.
    let files = Files::new();
    let path = files.media(
        "tone-tags.wav",
        "Review revision.wav",
        Some("Media original"),
    );
    let mut gui = Gui::new(&files);
    gui.import(vec![path.clone()]);
    gui.select(&path);
    gui.inspect(false);
    gui.change("Title", "First draft");
    gui.click("Review these field changes");
    let old_action = gui.node("Apply reviewed edits to captured tracks");
    gui.text("New Title", "Changed after review");
    assert!(gui.app.library_tags.reviewed.is_none());
    let before = std::fs::read(&path).unwrap();
    gui.action(old_action, Action::Click);
    gui.settle();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn blocked_tag_io_keeps_essential_cue_saves_serviceable_and_cancel_leaves_media_intact() {
    let files = Files::new();
    let path = files.media("tone-tags.wav", "Blocked tags.wav", Some("Media original"));
    let mut gui = Gui::new(&files);
    gui.import(vec![path.clone()]);
    gui.select(&path);
    gui.inspect(false);
    gui.change("Title", "Cancelled rewrite");
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let resume = std::sync::Mutex::new(resume_rx);
    gui.app.library_scan.set_tag_hook(move |task| {
        if matches!(task, Task::Apply { .. }) {
            started_tx.send(()).unwrap();
            resume.lock().unwrap().recv().unwrap();
        }
    });
    let bytes = std::fs::read(&path).unwrap();
    let source = LibSource::File(path.clone());
    let fingerprint = FileFingerprint::read(&path);
    gui.click("Review these field changes");
    gui.click("Apply reviewed edits to captured tracks");
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let mut preparation = crate::engine::preparation::Preparation::default();
    preparation.hotcues[1] = Some(0.15);
    gui.app.library_metadata.capture(library_store::Capture {
        source: source.clone(),
        fingerprint,
        metadata: gui.app.capture_metadata(&source, fingerprint),
        preparation: Some(preparation),
        played: Some(SystemTime::now()),
    });
    gui.wait(|gui| {
        gui.version(&path).preparation == preparation && gui.app.library_metadata.durable
    });
    assert!(matches!(
        gui.app.library_tags.pending,
        Some(Pending::Apply(_))
    ));
    gui.click("Cancel tag work");
    resume_tx.send(()).unwrap();
    gui.settle();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(gui.version(&path).preparation, preparation);
    assert!(gui.app.library_tags.message.contains("0 saved"));
    assert!(
        gui.app.library_metadata.durable,
        "cancelled optional tags hid a confirmed cue save"
    );
}

#[test]
fn partial_batch_cancel_protection_and_close_preserve_the_completed_first_track() {
    for stop in ["cancel", "protect", "close"] {
        let files = Files::new();
        let paths = [
            files.media("tone-tags.wav", "First.wav", Some("Media One")),
            files.media("tone-tags.wav", "Second.wav", Some("Media Two")),
        ];
        let mut gui = Gui::new(&files);
        gui.import(paths.to_vec());
        gui.app.lib_filter = "Media".into();
        gui.app.refresh_library_view();
        gui.inspect(true);
        let Reply::Preview(preview) = gui
            .app
            .library_tags
            .preview
            .as_ref()
            .unwrap()
            .reply
            .as_ref()
        else {
            unreachable!()
        };
        let captured: Vec<_> = preview
            .rows
            .iter()
            .map(|row| row.target.as_ref().unwrap().source.clone())
            .collect();
        let before: Vec<_> = captured
            .iter()
            .map(|source| {
                let LibSource::File(path) = source else {
                    unreachable!()
                };
                std::fs::read(path).unwrap()
            })
            .collect();
        gui.change("Title", "Completed before stop");
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
        let resume = std::sync::Mutex::new(resume_rx);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        gui.app.library_scan.set_tag_hook(move |task| {
            if matches!(task, Task::Apply { .. })
                && calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 1
            {
                started_tx.send(()).unwrap();
                resume.lock().unwrap().recv().unwrap();
            }
        });
        gui.click("Review these field changes");
        gui.click("Apply reviewed edits to captured tracks");
        gui.wait(|_| started_rx.try_recv().is_ok());
        assert_eq!(gui.app.library_tags.queue.as_ref().unwrap().saved, 1);
        match stop {
            "cancel" => gui.click("Cancel tag work"),
            "protect" => {
                gui.app.engine.send(Command::PerformanceMode(true)).unwrap();
                gui.frame(Vec::new());
            }
            "close" => {
                assert!(matches!(
                    gui.app.prepare_library_close(),
                    library_store::CloseState::Pending
                ));
            }
            _ => unreachable!(),
        }
        resume_tx.send(()).unwrap();
        gui.settle();
        assert!(
            gui.app.library_tags.message.contains("1 saved"),
            "{stop}: {}",
            gui.app.library_tags.message
        );
        for (index, source) in captured.iter().enumerate() {
            let LibSource::File(path) = source else {
                unreachable!()
            };
            if index == 0 {
                assert_eq!(gui.version(path).metadata.title, "Completed before stop");
                assert_ne!(std::fs::read(path).unwrap(), before[index]);
            } else {
                assert_eq!(std::fs::read(path).unwrap(), before[index]);
                assert_ne!(gui.version(path).metadata.title, "Completed before stop");
            }
        }
        assert!(gui.app.library_metadata.durable);
        if stop == "close" {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                match gui.app.prepare_library_close() {
                    library_store::CloseState::Ready => break,
                    library_store::CloseState::Failed(error) => panic!("close failed: {error}"),
                    library_store::CloseState::Pending => {}
                }
                assert!(
                    Instant::now() < deadline,
                    "close fence/save did not complete"
                );
                gui.frame(Vec::new());
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[test]
fn startup_recovers_installed_tag_media_before_retiring_the_original_backup() {
    let files = Files::new();
    let path = files.media("tone-tags.wav", "Interrupted.wav", Some("Media original"));
    let mut gui = Gui::new(&files);
    gui.import(vec![path.clone()]);
    let source = LibSource::File(path.clone());
    let track = gui.app.library_metadata.catalog.track(&source).unwrap();
    let target = crate::library::tags::Review {
        id: track.id.clone(),
        source: source.clone(),
        fingerprint: gui.version(&path).fingerprint.unwrap(),
    };
    let root = gui.app.library_metadata.tag_recovery_root.clone().unwrap();
    drop(gui);
    wait_store_closed(&files.store());
    let work = crate::engine::performance::Handle::default()
        .optional_work()
        .unwrap();
    let applied = crate::media_tags::write::apply(
        &crate::media_location::Location::resolve(&source).unwrap(),
        &target,
        &Patch {
            title: Some("Recovered 東京 🎹".into()),
            ..Patch::default()
        },
        &root,
        &work,
    )
    .unwrap();
    let journal = applied.record.journal_path();
    assert!(journal.exists());
    assert_eq!(
        crate::library::read(&files.store())
            .unwrap()
            .track(&source)
            .unwrap()
            .versions[0]
            .metadata
            .title,
        "Media original"
    );
    let reopened = Gui::new(&files);
    assert_eq!(reopened.version(&path).metadata.title, "Recovered 東京 🎹");
    assert_eq!(
        reopened
            .app
            .library_metadata
            .catalog
            .track(&source)
            .unwrap()
            .id,
        target.id
    );
    assert_eq!(
        reopened.version(&path).fingerprint,
        Some(applied.proof.new_fingerprint)
    );
    assert!(reopened.app.library_metadata.durable);
    assert!(!journal.exists());
    assert!(crate::media_tags::write::recover(&root).is_empty());
}

#[test]
fn native_tag_refresh_preview_displays_saved_locks_and_preserves_read_only_media() {
    let files=Files::new();let path=files.media("tone.flac","Prepared.flac",Some("Embedded title"));
    std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o444)).unwrap();let bytes=std::fs::read(&path).unwrap();
    let mut gui=Gui::new(&files);gui.import(vec![path.clone()]);gui.select(&path);
    gui.app.library_protection.open=true;gui.frame(vec![]);gui.click("Capture selected preparation");
    for label in ["Change BPM lock","Lock BPM","Change metadata lock","Lock metadata"] {gui.click(label);}
    gui.click("Review preparation locks");gui.click("Save reviewed preparation locks");gui.settle();
    gui.app.library_protection.open=false;gui.inspect(false);
    let labels:Vec<_>=gui.nodes.iter().filter_map(|(_,node)|node.label().or(node.value())).collect();
    assert!(labels.contains(&"Tag refresh replacement preview"),"{labels:?}");
    assert!(labels.iter().any(|label|label.contains("Preparation locked: BPM, metadata")),"{labels:?}");
    assert!(labels.iter().any(|label|label.contains("Title: Embedded title → Embedded title")),"{labels:?}");
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
}
