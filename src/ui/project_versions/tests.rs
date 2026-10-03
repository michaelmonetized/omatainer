use super::*;
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
    rt: RtEngine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
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
        self.rt.process(&mut [0.0; 256]);
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
    assert!(text.contains("cutoff:"));
    assert!(text.contains("Edited clip"));
    assert!(worker::compare(&current, &saved, &AtomicBool::new(true)).is_err());
}
