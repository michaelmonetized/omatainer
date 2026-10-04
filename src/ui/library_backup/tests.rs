use super::*;
use crate::engine::{
    audio::OutputCallback, beatgrid::Grid, media_source::FileFingerprint, preparation::Preparation,
};
use egui::accesskit::{Action as NativeAction, ActionRequest, Node, NodeId};
use std::{path::Path, time::Duration};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-backup-ui-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn prepared(&self) -> (PathBuf, crate::library::TrackId, Preparation) {
        let path = self.0.join("prepared 120 Am.wav");
        let frames = 48_000u32;
        let size = frames * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(96_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for frame in 0..frames {
            bytes.extend((((frame as f32 * 0.06).sin() * 8000.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let mut catalog = crate::library::Catalog::default();
        let source = LibSource::File(path.clone());
        let preparation = Preparation {
            cue: 0.1,
            grid: Some(Grid::new(0.01, 120.0).unwrap()),
            hotcues: [Some(0.2); 8],
            ..Default::default()
        };
        let version = catalog
            .upsert(
                source.clone(),
                FileFingerprint::read(&path),
                crate::library::Metadata {
                    title: "Prepared set".into(),
                    artist: "Owned fixture".into(),
                    bpm: Bpm::new(120.0, bpm::Origin::User),
                    key: "Am".into(),
                    duration: Some(1.0),
                    last_play: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(123)),
                },
            )
            .unwrap();
        version.preparation = preparation;
        let identity = catalog.track(&source).unwrap().id.clone();
        let id = crate::library::crates::CrateId("f".repeat(32));
        for edit in [
            crate::library::crates::Edit::Create {
                id: id.clone(),
                name: "Prepared set".into(),
                parent: None,
                before: None,
            },
            crate::library::crates::Edit::AddMembers {
                id: id.clone(),
                members: vec![identity.clone()],
                before: None,
            },
            crate::library::crates::Edit::SetFavorite { id, favorite: true },
        ] {
            catalog
                .crates
                .apply(catalog.crates.revision(), &edit, |track| track == &identity)
                .unwrap();
        }
        let store = self.0.join("first-profile/library.json");
        let mut owner = crate::library::Store::open(store.clone()).unwrap();
        owner.catalog = catalog;
        owner.save().unwrap();
        (store, identity, preparation)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    callback: OutputCallback,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    nonzero: bool,
}
impl Gui {
    fn new(store: PathBuf) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.start_library_store(store);
        app.library_backup.open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            callback: OutputCallback::new(rt, 2),
            ctx,
            nodes: vec![],
            time: 0.0,
            nonzero: false,
        };
        gui.wait(|gui| {
            gui.app.library_metadata.ready()
                && gui.app.library_metadata.durable
                && !gui.app.library_metadata.active()
        });
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                focused: true,
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let mut samples = [0f32; 512];
        self.callback.render(&mut samples);
        self.nonzero |= samples.iter().any(|sample| sample.abs() > 0.001);
    }
    fn wait(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.frame(vec![]);
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} / {}",
                self.app.library_backup.message,
                self.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn node(&self, name: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {name}"))
    }
    fn action(&mut self, name: &str, action: NativeAction) {
        let target = self.node(name);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn path(&mut self, name: &str, path: &Path) {
        self.action(name, NativeAction::Focus);
        self.frame(vec![egui::Event::Text(path.display().to_string())]);
        self.frame(vec![]);
    }
}

#[test]
fn native_export_restore_import_and_real_loader_play_prepared_music_on_a_clean_profile() {
    let files = Files::new();
    let (store, identity, preparation) = files.prepared();
    let source = files.0.join("prepared 120 Am.wav");
    let original = std::fs::read(&source).unwrap();
    let original_fp = FileFingerprint::read(&source).unwrap();
    let backup = files.0.join("backup");
    let mut first = Gui::new(store);
    let stale = first.node("Export library snapshot");
    first.path("New backup directory", &backup);
    first.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: NativeAction::Click,
        target: stale,
        data: None,
    })]);
    assert!(first.app.library_backup.active.is_none() && !backup.exists());
    assert!(!first.app.library_backup.collect);
    first.action("Authorize local music collection", NativeAction::Click);
    first.action("Export library snapshot", NativeAction::Click);
    first.wait(|gui| gui.app.library_backup.active.is_none());
    assert!(
        backup.join("manifest.json").exists(),
        "{}",
        first.app.library_backup.message
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(FileFingerprint::read(&source), Some(original_fp));
    std::fs::remove_file(&source).unwrap();
    let mut clean = Gui::new(files.0.join("clean-profile/library.json"));
    assert!(!clean
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .any(|track| track.id == identity));
    clean.path("Existing backup directory", &backup);
    clean.action("Verify backup", NativeAction::Click);
    clean.wait(|gui| gui.app.library_backup.active.is_none());
    assert!(clean
        .app
        .library_backup
        .message
        .starts_with("Verified backup"));
    let destination = files.0.join("different-volume-path");
    clean.path("New restore directory", &destination);
    clean.action("Restore to new directory", NativeAction::Click);
    clean.wait(|gui| gui.app.library_backup.active.is_none());
    assert!(
        clean.app.library_backup.restored.is_some(),
        "{}",
        clean.app.library_backup.message
    );
    assert!(!clean
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .any(|track| track.id == identity));
    clean.action("Import restored catalog", NativeAction::Click);
    clean.wait(|gui| {
        gui.app
            .library_metadata
            .catalog
            .tracks
            .iter()
            .any(|track| track.id == identity)
            && gui.app.library_metadata.durable
            && !gui.app.library_metadata.active()
    });
    let track = clean
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .find(|track| track.id == identity)
        .unwrap();
    let LibSource::File(path) = &track.source else {
        panic!()
    };
    assert!(path.starts_with(&destination));
    assert_eq!(track.versions[track.current].preparation, preparation);
    assert_eq!(
        track.versions[track.current].metadata.last_play,
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(123))
    );
    assert!(clean
        .app
        .library_metadata
        .catalog
        .crates
        .nodes()
        .iter()
        .any(|node| node.favorite && node.members == [identity.clone()]));
    let selection = Selection {
        source: track.source.clone(),
        title: "Restored prepared set".into(),
        fingerprint: track.versions[track.current].fingerprint,
    };
    clean.app.library_backup.open = false;
    clean.app.load_source(1, Some(&selection));
    clean.wait(|gui| {
        matches!(
            gui.app.loads[1].as_ref().map(|load| &load.phase),
            Some(Phase::Loaded)
        )
    });
    let receipt = clean.app.loads[1]
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap();
    assert_eq!(receipt.preparation().unwrap().1, preparation);
    clean.wait(|gui| gui.app.snap.decks[1].grid == preparation.grid);
    assert!(!clean.app.snap.decks[1].playing);
    assert_eq!(clean.app.snap.decks[1].grid, preparation.grid);
    clean.nonzero = false;
    clean
        .app
        .engine
        .send(Command::DeckPlay { deck: 1 })
        .unwrap();
    clean.wait(|gui| gui.nonzero && gui.app.snap.decks[1].playing);
    assert!(!source.exists(), "Playback must use the restored file");
    first.path("Existing backup directory", &backup);
    first.path(
        "New restore directory",
        &files.0.join("conflicting-restored"),
    );
    first.action("Restore to new directory", NativeAction::Click);
    first.wait(|gui| gui.app.library_backup.active.is_none());
    let before = first.app.library_metadata.catalog.tracks.clone();
    first.action("Import restored catalog", NativeAction::Click);
    first.wait(|gui| !gui.app.library_metadata.active());
    assert_eq!(first.app.library_metadata.catalog.tracks, before);
    assert!(first
        .app
        .library_metadata
        .label()
        .contains("different location"));
}

#[test]
fn backup_dialog_blocks_transport_immediately_and_closed_window_keeps_a_job_cancellable() {
    let files = Files::new();
    let mut gui = Gui::new(files.0.join("profile/library.json"));
    gui.frame(vec![egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!gui.app.snap.playing);
    assert!(keyboard::dialogs_block_input(&gui.ctx));
    let performance = gui.app.engine.cmd.performance().clone();
    gui.app
        .library_backup
        .submit(
            Action::Inspect {
                path: files.0.clone(),
            },
            &performance,
        )
        .unwrap();
    gui.app.library_backup.open = false;
    gui.app.library_backup.cancel();
    gui.wait(|gui| gui.app.library_backup.active.is_none());
    assert!(
        gui.app.library_backup.message.contains("cancelled")
            || gui.app.library_backup.message.contains("No such file")
    );
}
