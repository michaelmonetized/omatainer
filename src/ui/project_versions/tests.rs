use super::*;
use crate::engine::dsp::Sample;
use crate::engine::{project::Captured, RtEngine};
use crate::project_file::{Bundle, Limits};
use egui::accesskit::{Action, ActionRequest, Node, NodeId};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-project-import-{}-{:?}",
            std::process::id(),
            crate::engine::midi_edit::NoteId::new().words()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.join().unwrap()
}
struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    reference: Option<(Engine, Box<RtEngine>)>,
    nonzero_blocks: usize,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let fixture = test_support::Fixture::new(256);
        let mut app = fixture.app;
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: fixture.rt,
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            reference: None,
            nonzero_blocks: 0,
        };
        gui.wait(|g| !g.app.library_metadata.active());
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        self.rt.publish_for_test();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        let mut pcm = [0.0; 256];
        self.rt.process(&mut pcm);
        if let Some((_, reference)) = &mut self.reference {
            let mut expected = [0.0; 256];
            reference.process(&mut expected);
            assert_eq!(
                pcm, expected,
                "Storage work changed independent playing PCM"
            );
            self.nonzero_blocks += usize::from(pcm.iter().any(|x| x.abs() > 0.001));
        }
    }
    fn wait(&mut self, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.frame(vec![]);
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} {:?}",
                self.app.project_versions.message,
                self.app.project_versions.error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn action(&mut self, label: &str, action: Action) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click);
    }
    fn text(&mut self, label: &str, value: &str) {
        self.action(label, Action::Focus);
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text(value.into()),
        ]);
        self.frame(vec![]);
    }
}
impl Gui {
    fn playing(&mut self) {
        self.rt.decks[0].audio = Some(Arc::new(Sample {
            name: "Independent playback".into(),
            path: "native-playing-check".into(),
            sr: 48000,
            ch: 2,
            bpm: 120.0,
            peaks: Arc::new(vec![]),
            spectrum: None,
            data: vec![0.1; 48000 * 2 * 10],
        }));
        self.app.engine.send(Command::DeckPlay { deck: 0 }).unwrap();
        self.frame(vec![]);
        let (engine, mut reference) = Engine::headless_for_test(48000, 256);
        reference.decks = self.rt.decks.clone();
        self.reference = Some((engine, Box::new(reference)));
    }
    fn named(&mut self, root: &std::path::Path) {
        self.click("Project");
        self.click("Named versions…");
        self.text("Version storage folder", root.to_str().unwrap());
    }
    fn snapshot(&mut self, name: &str) {
        self.text("Version name", name);
        self.text("Revision notes", "Native snapshot note");
        self.click("Save named snapshot");
        self.wait(|g| !g.app.project_versions.busy());
        assert!(
            self.app.project_versions.error.is_none(),
            "{:?}",
            self.app.project_versions.error
        );
    }
    fn select(&mut self, name: &str) {
        let label = self
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label())
            .find(|s| s.starts_with(&format!("Version: {name} [")))
            .unwrap()
            .to_string();
        self.click(&label);
    }
    fn compare(&mut self) {
        self.click("Compare selected version");
        self.wait(|g| !g.app.project_versions.busy());
        assert!(
            self.app.project_versions.compared.is_some(),
            "{:?}",
            self.app.project_versions.error
        );
    }
}
fn storage_archive(files: &Files) -> PathBuf {
    let path = files.0.join("storage-source.omat");
    crate::project_file::save(
        &path,
        &crate::project_versions::storage::tests::fixture(),
        crate::project_file::Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    path
}
#[test]
fn native_storage_report_review_apply_reopen_and_recovery_preserve_independent_playback() {
    let files = Files::new();
    let source = storage_archive(&files);
    let original = std::fs::read(&source).unwrap();
    let mut gui = Gui::new(&files);
    gui.playing();
    gui.click("Project");
    gui.click("Named versions…");
    gui.text(
        "Native archive to inspect or compact",
        source.to_str().unwrap(),
    );
    gui.click("Inspect project storage");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.error.is_none(),
        "{:?}",
        gui.app.project_versions.error
    );
    assert!(gui
        .app
        .project_versions
        .message
        .contains("Referenced slices: 2"));
    assert!(gui.app.project_versions.root.is_empty());
    let destination = files.0.join("compacted.omat");
    gui.text("New compacted archive path", destination.to_str().unwrap());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.compacted.is_some(),
        "{:?}",
        gui.app.project_versions.error
    );
    assert!(gui
        .app
        .project_versions
        .message
        .contains("5 → 3 embedded sources"));
    assert!(!destination.exists());
    assert!(
        gui.app.engine.cmd.performance().jobs().snapshot().reserved
            >= 1536 * crate::background::MIB
    );
    gui.click("Save reviewed compacted copy");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.error.is_none(),
        "{:?}",
        gui.app.project_versions.error
    );
    let reopened = crate::project_file::load::<project::Document>(
        &destination,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.media.len(), 3);
    reopened.state.engine.validate(&reopened.media).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(
        gui.app.engine.cmd.performance().jobs().snapshot().reserved < 1536 * crate::background::MIB
    );
    let root = files.0.join("native-cleanup");
    gui.text("Version storage folder", root.to_str().unwrap());
    gui.snapshot("Keep");
    gui.snapshot("Restore me");
    let revision = gui
        .app
        .project_versions
        .entries
        .iter()
        .find(|e| e.name == "Restore me")
        .unwrap()
        .id
        .clone();
    let index = std::fs::read(root.join("versions.omat")).unwrap();
    gui.select("Restore me");
    gui.click("Preview pruning and unused assets");
    gui.wait(|g| !g.app.project_versions.busy());
    gui.click("Apply reviewed pruning");
    gui.wait(|g| !g.app.project_versions.busy());
    assert_eq!(gui.app.project_versions.entries.len(), 1);
    assert!(gui
        .app
        .project_versions
        .message
        .contains("Quarantine retains disk bytes"));
    let blocks = gui.nonzero_blocks;
    gui.app.project_versions = Panel::default();
    gui.app.project_versions.open = true;
    gui.frame(vec![]);
    gui.text("Version storage folder", root.to_str().unwrap());
    gui.click("Refresh named versions");
    gui.wait(|g| !g.app.project_versions.busy());
    gui.click("Inspect cleanup recovery");
    gui.wait(|g| !g.app.project_versions.busy());
    assert_eq!(gui.app.project_versions.recoveries.len(), 1);
    let id = gui.app.project_versions.recoveries[0].id.clone();
    let removed_path = root.join("revisions").join(format!("{revision}.omat"));
    std::fs::write(&removed_path, b"foreign revision owner").unwrap();
    gui.click(&format!("Restore cleanup {id}"));
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.error.is_some());
    assert_eq!(
        std::fs::read(&removed_path).unwrap(),
        b"foreign revision owner"
    );
    assert_eq!(gui.app.project_versions.entries.len(), 1);
    std::fs::rename(&removed_path, files.0.join("preserved-foreign-revision")).unwrap();
    let (entered, resume) = gui
        .app
        .project_versions
        .worker
        .as_ref()
        .unwrap()
        .pause_next();
    gui.click(&format!("Restore cleanup {id}"));
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    gui.click("Cancel version operation");
    resume.send(()).unwrap();
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(!removed_path.exists());
    assert_eq!(gui.app.project_versions.entries.len(), 1);
    gui.click(&format!("Restore cleanup {id}"));
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.error.is_none(),
        "{:?}",
        gui.app.project_versions.error
    );
    assert_eq!(gui.app.project_versions.entries.len(), 2);
    assert_eq!(std::fs::read(root.join("versions.omat")).unwrap(), index);
    gui.click("Inspect cleanup recovery");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.recoveries[0].restored);
    assert_eq!(gui.app.project_versions.recoveries[0].bytes, 0);
    assert!(gui.nonzero_blocks > blocks && blocks > 10);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    println!("NATIVE_PROJECT_STORAGE {{\"real_accesskit_report_review_apply_reopen_restore\":true,\"worker_owns_compacted_pcm_and_budget\":true,\"independent_playing_pcm_unchanged\":true,\"nonzero_blocks\":{},\"physical_devices_opened\":false}}", gui.nonzero_blocks);
}
#[test]
fn native_compaction_cancel_changed_review_collision_and_protection_preserve_playback() {
    let files = Files::new();
    let source = storage_archive(&files);
    let before = std::fs::read(&source).unwrap();
    let mut gui = Gui::new(&files);
    gui.playing();
    gui.click("Project");
    gui.click("Named versions…");
    gui.text(
        "Native archive to inspect or compact",
        source.to_str().unwrap(),
    );
    let destination = files.0.join("cancelled.omat");
    gui.text("New compacted archive path", destination.to_str().unwrap());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.compacted.is_some(),
        "{:?}",
        gui.app.project_versions.error
    );
    let (entered, resume) = gui
        .app
        .project_versions
        .worker
        .as_ref()
        .unwrap()
        .pause_next();
    gui.click("Save reviewed compacted copy");
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    gui.click("Cancel version operation");
    resume.send(()).unwrap();
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(!destination.exists());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    gui.click("Cancel compaction review");
    gui.wait(|g| {
        g.app.engine.cmd.performance().jobs().snapshot().reserved < 1536 * crate::background::MIB
    });
    assert!(gui.app.project_versions.compacted.is_none());
    assert!(!destination.exists());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    let changed = files.0.join("different-destination.omat");
    gui.text("New compacted archive path", changed.to_str().unwrap());
    assert!(gui.app.project_versions.compacted.is_none());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    std::fs::write(&changed, b"another writer's destination").unwrap();
    gui.click("Save reviewed compacted copy");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.error.is_some());
    assert_eq!(
        std::fs::read(&changed).unwrap(),
        b"another writer's destination"
    );
    let protected = files.0.join("protected.omat");
    gui.text("New compacted archive path", protected.to_str().unwrap());
    gui.click("Review compacted archive copy");
    gui.wait(|g| !g.app.project_versions.busy());
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.wait(|g| g.app.project_versions.compacted.is_none());
    assert!(!protected.exists());
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), before);
    assert!(gui.nonzero_blocks > 10);
    println!("NATIVE_PROJECT_STORAGE_CANCEL {{\"actual_cancel_control\":true,\"edited_destination_invalidates_review\":true,\"occupied_destination_preserved\":true,\"protected_review_released_on_worker\":true,\"independent_playing_pcm_unchanged\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn native_named_versions_compare_branch_restore_reopen_and_preview_prune() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let original = files.0.join("original.omat");
    gui.click("Project");
    gui.click("Save project as…");
    gui.text("Project file path", original.to_str().unwrap());
    gui.click("Save");
    gui.wait(|g| !g.app.project_pending_for_test());
    let original_bytes = std::fs::read(&original).unwrap();
    let original_bpm = gui.rt.bpm;
    let root = files.0.join("named");
    std::fs::create_dir(&root).unwrap();
    gui.named(&root);
    gui.snapshot("First mix");
    let shared_audio_count = std::fs::read_dir(root.join("audio")).unwrap().count();
    gui.app.engine.send(Command::SetBpm(153.0)).unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.snapshot("Faster mix");
    assert_eq!(
        std::fs::read_dir(root.join("audio")).unwrap().count(),
        shared_audio_count
    );
    assert_eq!(gui.app.project_versions.entries.len(), 2);
    gui.select("First mix");
    gui.compare();
    assert!(gui
        .app
        .project_versions
        .message
        .contains(&format!("{} → {} BPM", 153, original_bpm)));
    let branch = files.0.join("alternative.omat");
    gui.text("New branch project path", branch.to_str().unwrap());
    gui.click("Branch compared version into new project");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(
        gui.app.project_versions.error.is_none(),
        "{:?}",
        gui.app.project_versions.error
    );
    let loaded = crate::project_file::load::<project::Document>(
        &branch,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(loaded.state.engine.bpm, original_bpm);
    loaded.state.engine.validate(&loaded.media).unwrap();
    assert_eq!(gui.rt.bpm, 153.0);
    assert_eq!(gui.app.project_result_for_test().0, Some(original.clone()));
    gui.compare();
    gui.click("Restore compared version as unsaved copy");
    gui.click("Discard changes");
    gui.wait(|g| !g.app.project_pending_for_test());
    assert_eq!(gui.rt.bpm, original_bpm);
    assert!(gui.app.project_dirty());
    assert!(gui.app.project_result_for_test().0.is_none());
    gui.click("Refresh named versions");
    gui.wait(|g| !g.app.project_versions.busy());
    assert_eq!(gui.app.project_versions.entries.len(), 2);
    gui.select("Faster mix");
    gui.click("Preview pruning and unused assets");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.message.contains("unreferenced"));
    assert_eq!(
        std::fs::read_dir(root.join("revisions")).unwrap().count(),
        2
    );
    gui.click("Apply reviewed pruning");
    gui.wait(|g| !g.app.project_versions.busy());
    assert_eq!(gui.app.project_versions.entries.len(), 1);
    assert_eq!(
        std::fs::read_dir(root.join("revisions")).unwrap().count(),
        1
    );
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    gui.select("First mix");
    gui.click("Preview pruning and unused assets");
    gui.wait(|g| !g.app.project_versions.busy());
    gui.click("Apply reviewed pruning");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.entries.is_empty());
    assert_eq!(std::fs::read_dir(root.join("audio")).unwrap().count(), 0);
    gui.app.project_versions.open = false;
    gui.frame(vec![]);
    gui.click("Project");
    gui.click("Open project…");
    gui.click("Discard changes");
    gui.text("Project file path", branch.to_str().unwrap());
    gui.click("Open");
    gui.wait(|g| !g.app.project_pending_for_test());
    assert_eq!(gui.rt.bpm, original_bpm);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
}
#[test]
fn native_cancel_stale_comparison_and_failed_branch_preserve_existing_state() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let root = files.0.join("named");
    gui.named(&root);
    gui.snapshot("Original");
    let before = std::fs::read(root.join("versions.omat")).unwrap();
    let (entered, resume) = gui
        .app
        .project_versions
        .worker
        .as_ref()
        .unwrap()
        .pause_next();
    gui.text("Version name", "Cancelled");
    gui.click("Save named snapshot");
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    gui.click("Cancel version operation");
    resume.send(()).unwrap();
    gui.wait(|g| !g.app.project_versions.busy());
    assert_eq!(std::fs::read(root.join("versions.omat")).unwrap(), before);
    gui.select("Original");
    gui.compare();
    gui.app.engine.send(Command::SetBpm(149.0)).unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.app.project_versions.compared.is_none());
    gui.compare();
    let destination = files.0.join("existing.omat");
    std::fs::write(&destination, b"existing project").unwrap();
    gui.text("New branch project path", destination.to_str().unwrap());
    gui.click("Branch compared version into new project");
    gui.wait(|g| !g.app.project_versions.busy());
    assert!(gui.app.project_versions.error.is_some());
    assert_eq!(std::fs::read(&destination).unwrap(), b"existing project");
    assert_eq!(gui.rt.bpm, 149.0);
    gui.compare();
    let (entered, resume) = gui.app.pause_project_prepare_for_recovery_test();
    gui.click("Restore compared version as unsaved copy");
    gui.click("Discard changes");
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    gui.click("Cancel project operation");
    resume.send(()).unwrap();
    gui.wait(|g| !g.app.project_pending_for_test());
    assert_eq!(gui.rt.bpm, 149.0);
    assert_eq!(std::fs::read(root.join("versions.omat")).unwrap(), before);
}

#[test]
fn comparison_reports_stable_track_clip_routing_and_device_changes() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let captured = capture(&gui.app.engine, &mut gui.rt);
    let current = Bundle {
        state: project::Document {
            engine: captured.state,
            view: project::UiState::default(),
            mapping_schema: project::FACTORY_MAPPING_SCHEMA,
        },
        media: captured.media,
    };
    let mut state = current.state.engine.clone();
    let layout = state.session.as_mut().unwrap();
    layout.tracks[0].name = "Renamed track".into();
    state.tracks[0].name = "Renamed track".into();
    state.tracks[0].clips[0].name = "Edited clip".into();
    state.tracks[0].gain = 0.43;
    state.tracks[0].input_monitor = Some(crate::engine::input_monitor::Mode::In);
    state.tracks[0].synth.cutoff = 1200.0;
    let saved = Bundle {
        state: project::Document {
            engine: state,
            view: project::UiState::default(),
            mapping_schema: project::FACTORY_MAPPING_SCHEMA,
        },
        media: current.media.clone(),
    };
    saved.state.engine.validate(&saved.media).unwrap();
    let text = worker::compare(&current, &saved, &AtomicBool::new(false)).unwrap();
    for title in [
        "Tracks and scenes",
        "Clips and controller lanes",
        "Routing and mixer",
        "Instruments and effects",
    ] {
        assert!(
            text.contains(&format!("{title}: 0 added, 0 removed, 1 changed.")),
            "{text}"
        );
    }
    assert!(text.contains("Renamed track"));
    assert!(text.contains("gain:"));
    assert!(text.contains("input_monitor:"), "{text}");
    assert!(text.contains("cutoff:"));
    assert!(text.contains("Edited clip"));
    assert!(worker::compare(&current, &saved, &AtomicBool::new(true)).is_err());
}

#[test]
fn native_close_cancels_and_waits_for_snapshot_branch_and_prune_workers() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let root = files.0.join("close-versions");
    gui.named(&root);
    gui.snapshot("Original");
    gui.select("Original");
    gui.compare();
    let record = gui
        .app
        .project_versions
        .compared
        .as_ref()
        .unwrap()
        .0
        .clone();
    let store =
        crate::project_versions::Store::open(&root, false, &AtomicBool::new(false)).unwrap();
    let review = store
        .review_prune(&[record.entry.id.clone()], &AtomicBool::new(false))
        .unwrap();
    drop(store);
    let before = std::fs::read(root.join("versions.omat")).unwrap();
    let branch = files.0.join("cancelled-branch.omat");
    let tasks = [
        Task::Snapshot {
            name: "Cancelled".into(),
            notes: String::new(),
            view: gui.app.project_view(),
            identities: gui.app.project_watch_identities(),
        },
        Task::Branch {
            record,
            path: branch.clone(),
        },
        Task::Prune { review },
    ];
    for task in tasks {
        let (entered, resume) = gui
            .app
            .project_versions
            .worker
            .as_ref()
            .unwrap()
            .pause_next();
        gui.app.project_versions.start(&gui.app.engine, task);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancel = gui.app.project_versions.active.as_ref().unwrap().clone();
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let out = gui.ctx.run(raw, |ctx| gui.app.update_frame(ctx));
        assert!(out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
        assert!(cancel.load(Ordering::Acquire));
        assert!(gui.app.project_versions.busy());
        assert!(!out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close)));
        assert!(gui
            .app
            .project_result_for_test()
            .1
            .unwrap()
            .contains("named version work settles"));
        resume.send(()).unwrap();
        gui.wait(|g| !g.app.project_versions.busy());
        assert_eq!(std::fs::read(root.join("versions.omat")).unwrap(), before);
        assert!(!branch.exists());
    }
    assert!(std::fs::read_dir(root.join("audio")).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".tmp")));
}

#[test]
fn safe_mode_restart_uses_the_same_worker_close_gate_as_the_viewport() {
    let files = Files::new();
    let engine = Engine::start_safe().unwrap();
    let mut app = App::with_loader(engine, Theme::default(), None);
    app.library_metadata =
        library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
    app.project_versions.root = files.0.join("restart-versions").display().to_string();
    app.project_versions.worker = Some(Worker::start(app.engine.project.clone()).unwrap());
    let (entered, resume) = app.project_versions.worker.as_ref().unwrap().pause_next();
    let view = app.project_view();
    let identities = app.project_watch_identities();
    app.project_versions.start(
        &app.engine,
        Task::Snapshot {
            name: "Cancelled by restart".into(),
            notes: String::new(),
            view,
            identities,
        },
    );
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let cancel = app.project_versions.active.as_ref().unwrap().clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
            ..Default::default()
        },
        |ctx| app.update_frame(ctx),
    );
    let target = out
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Restart normally"))
        .unwrap()
        .0;
    let _out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
            events: vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target,
                action: Action::Click,
                data: None,
            })],
            ..Default::default()
        },
        |ctx| app.update_frame(ctx),
    );
    assert!(cancel.load(Ordering::Acquire));
    assert!(!app.project_pending_for_test());
    assert!(app
        .project_result_for_test()
        .1
        .unwrap()
        .contains("named version work settles"));
    resume.send(()).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while app.project_versions.busy() {
        app.project_versions.poll();
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!files.0.join("restart-versions/versions.omat").exists());
}
#[test]
fn comparison_reports_saved_mic_aux_controls_as_a_mixer_change() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let captured = capture(&engine, &mut rt);
    let current = Bundle {
        state: project::Document {
            engine: captured.state,
            view: project::UiState::default(),
            mapping_schema: project::FACTORY_MAPPING_SCHEMA,
        },
        media: captured.media,
    };
    let mut changed = project::Document {
        engine: current.state.engine.clone(),
        view: project::UiState::default(),
        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
    };
    changed.engine.mic_aux = Some(crate::engine::audio::routing::mic_aux::Configuration::default());
    let saved = Bundle {
        state: changed,
        media: current.media.clone(),
    };
    let text = worker::compare(&current, &saved, &AtomicBool::new(false)).unwrap();
    assert!(text.contains("Changed: Master and decks"), "{text}");
}
