use super::*;
use crate::engine::{project::Captured, RtEngine};
use crate::project_file::{Bundle, Limits, Overwrite};
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
    fn frame(&mut self, events: Vec<egui::Event>) { self.frame_with_audio(events, true); }
    fn frame_with_audio(&mut self, events: Vec<egui::Event>, audio: bool) {
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
        if audio { self.rt.process(&mut [0.0; 256]); }
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
                self.app.project_import.message,
                self.app.project_import.error
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
    fn browse(&mut self, path: &std::path::Path) {
        self.click("Project");
        self.click("Import from another project…");
        self.text("Source native project path", path.to_str().unwrap());
        self.click("Browse source project");
        self.wait(|g| !g.app.project_import.busy());
        assert!(
            self.app.project_import.catalog.is_some(),
            "{:?}",
            self.app.project_import.error
        );
        self.click("Keep destination tempo and meter; use source beat positions");
    }
    fn review(&mut self) {
        self.click("Review selected project material");
        self.wait(|g| g.app.project_import.ready);
    }
    fn history(&mut self, prefix: &str) {
        self.click("Edit");
        let label = self
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label())
            .find(|label| label.starts_with(prefix))
            .unwrap()
            .to_string();
        self.click(&label);
        self.frame(vec![]);
    }
}
fn source(gui: &mut Gui, files: &Files) -> PathBuf {
    let captured = capture(&gui.app.engine, &mut gui.rt);
    let document = project::Document {
        engine: captured.state,
        view: gui.app.project_view(),
        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
    };
    let path = files.0.join("source.omat");
    crate::project_file::save(
        &path,
        &Bundle {
            state: document,
            media: captured.media,
        },
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    path
}

#[test]
fn native_browse_review_apply_undo_redo_save_new_open_preserve_source_and_destination_material() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let path = source(&mut gui, &files);
    let original = std::fs::read(&path).unwrap();
    let tracks = gui.rt.tracks.len();
    let scenes = gui.rt.scene_fx.len();
    let address = &*gui.rt.tracks[2] as *const _ as usize;
    gui.browse(&path);
    assert_eq!(gui.rt.tracks.len(), tracks);
    gui.review();
    assert_eq!(gui.rt.tracks.len(), tracks);
    gui.click("Apply reviewed project import");
    gui.wait(|g| !g.app.project_import.busy());
    assert_eq!(
        (gui.rt.tracks.len(), gui.rt.scene_fx.len()),
        (tracks * 2, scenes * 2)
    );
    assert_eq!(&*gui.rt.tracks[2] as *const _ as usize, address);
    gui.history("Undo Edit session layout");
    assert_eq!(gui.rt.tracks.len(), tracks);
    gui.history("Redo Edit session layout");
    assert_eq!(gui.rt.tracks.len(), tracks * 2);
    gui.app.project_import.open = false;
    gui.frame(vec![]);
    let saved = files.0.join("destination.omat");
    gui.click("Project");
    gui.click("Save project as…");
    gui.text("Project file path", saved.to_str().unwrap());
    gui.click("Save");
    gui.wait(|g| !g.app.project_pending_for_test());
    let reopened = crate::project_file::load::<project::Document>(
        &saved,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.state.engine.tracks.len(), tracks * 2);
    reopened.state.engine.validate(&reopened.media).unwrap();
    gui.click("Project");
    gui.click("New project");
    gui.wait(|g| !g.app.project_pending_for_test());
    gui.click("Project");
    gui.click("Open project…");
    gui.text("Project file path", saved.to_str().unwrap());
    gui.click("Open");
    gui.wait(|g| !g.app.project_pending_for_test());
    assert_eq!(gui.rt.tracks.len(), tracks * 2);
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn native_cancellation_and_source_or_destination_changes_refuse_reviewed_import() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    let path = source(&mut gui, &files);
    let tracks = gui.rt.tracks.len();
    gui.browse(&path);
    gui.review();
    gui.click("Cancel project import");
    gui.wait(|g| !g.app.project_import.busy());
    assert_eq!(gui.rt.tracks.len(), tracks);
    gui.review();
    gui.app.engine.send(Command::SetBpm(153.0)).unwrap();
    gui.frame(vec![]);
    gui.click("Apply reviewed project import");
    gui.wait(|g| !g.app.project_import.busy());
    assert_eq!(gui.rt.tracks.len(), tracks);
    assert!(gui.app.project_import.error.is_some());
    gui.review();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"changed")
        .unwrap();
    gui.click("Apply reviewed project import");
    gui.wait(|g| !g.app.project_import.busy());
    assert_eq!(gui.rt.tracks.len(), tracks);
    assert!(gui
        .app
        .project_import
        .error
        .as_ref()
        .unwrap()
        .contains("Source project changed"));
}

#[test]
fn native_project_view_change_invalidates_reviewed_saveability() {
    let files=Files::new();let mut gui=Gui::new(&files);let path=source(&mut gui,&files);
    let tracks=gui.rt.tracks.len();gui.browse(&path);gui.review();
    gui.app.lib_filter="View changed after review".into();gui.frame(vec![]);
    gui.wait(|g| !g.app.project_import.busy());assert_eq!(gui.rt.tracks.len(),tracks);
    assert!(!gui.app.project_import.ready);
}

fn queued_view_change(open: bool) {
    let files=Files::new();let mut gui=Gui::new(&files);let path=source(&mut gui,&files);
    let tracks=gui.rt.tracks.len();gui.browse(&path);gui.review();
    let target=gui.nodes.iter().find(|(_,n)| n.label()==Some("Apply reviewed project import")).unwrap().0;
    gui.frame_with_audio(vec![egui::Event::AccessKitActionRequest(ActionRequest { target,action:Action::Click,data:None })],false);
    let deadline=Instant::now()+Duration::from_secs(5);
    while gui.app.project_import.active.is_some() { gui.frame_with_audio(vec![],false); assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1)); }
    assert_eq!(gui.app.project_import.pending.as_ref().unwrap().state(),Outcome::Pending);
    gui.app.project_import.open = open;
    gui.app.lib_filter="View edited after Apply".into();gui.frame_with_audio(vec![],false);
    gui.rt.process(&mut [0.0;256]);gui.wait(|g| !g.app.project_import.busy());assert_eq!(gui.rt.tracks.len(),tracks);
}

#[test]
fn native_view_changes_after_apply_cancel_import_before_renderer_claim() { queued_view_change(true); }
#[test]
fn native_hidden_import_keeps_view_invalidation_until_renderer_claim() { queued_view_change(false); }

#[test]
fn native_close_cancels_review_and_unclaimed_import_before_exiting() {
    for applied in [false, true] {
        let files = Files::new(); let mut gui = Gui::new(&files); let path = source(&mut gui, &files);
        let tracks = gui.rt.tracks.len(); gui.browse(&path); gui.review();
        if applied {
            let target = gui.nodes.iter().find(|(_, n)| n.label() == Some("Apply reviewed project import")).unwrap().0;
            gui.frame_with_audio(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Click, data: None })], false);
            let deadline = Instant::now() + Duration::from_secs(5);
            while gui.app.project_import.active.is_some() {
                gui.frame_with_audio(vec![], false);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(gui.app.project_import.pending.as_ref().unwrap().state(), Outcome::Pending);
        }
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))), ..Default::default() };
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        let out = gui.ctx.run(raw, |ctx| gui.app.update_frame(ctx));
        assert!(out.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
        assert!(gui.app.project_result_for_test().1.unwrap().contains("project import settles"));
        gui.rt.process(&mut [0.0; 256]);
        gui.wait(|g| !g.app.project_import.busy());
        assert_eq!(gui.rt.tracks.len(), tracks);
    }
}
