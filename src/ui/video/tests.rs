use super::*;
use crate::engine::RtEngine;
use crate::video::decoder::tests::Files;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::sync::atomic::AtomicBool;
struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    render: bool,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            nodes: vec![],
            time: 0.0,
            render: true,
        };
        gui.wait(|gui| !gui.app.library_metadata.active());
        gui.click("Video");
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
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
        if self.render {
            self.rt.process(&mut [0.0; 256]);
            self.rt.publish_for_test();
        }
    }
    fn wait(&mut self, mut condition: impl FnMut(&Self) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(vec![]);
            if condition(self) {
                break;
            }
            assert!(
                Instant::now() < until,
                "{} {:?}",
                self.app.library_crates.message,
                self.app.library_metadata.storage_error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
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
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(label);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
    fn text(&mut self, label: &str, value: &str) {
        self.action(label, Action::Focus, None);
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
    fn finish(&mut self) {
        self.wait(|gui| {
            gui.app.library_crates.pending.is_none() && !gui.app.library_metadata.active()
        });
    }
}

#[test]
fn native_import_frame_edits_locators_render_and_reopen_keep_picture_and_live_audio_intact() {
    let files = Files::new();
    let path = files.video("input.mp4", "25", "libx264");
    let original = std::fs::read(&path).unwrap();
    let mut gui = Gui::new(&files);
    gui.text("Local video path", path.to_str().unwrap());
    gui.click("Import local video");
    gui.wait(|gui| gui.app.video.validated && gui.app.video.texture.is_some());
    gui.click("Pause picture decoding");
    gui.wait(|gui| gui.app.video.job.is_none());
    gui.action(
        "Picture trim in frame",
        Action::SetValue,
        Some(ActionData::NumericValue(3.0)),
    );
    gui.action(
        "Picture trim end frame (exclusive)",
        Action::SetValue,
        Some(ActionData::NumericValue(20.0)),
    );
    gui.action(
        "Picture placement project frame",
        Action::SetValue,
        Some(ActionData::NumericValue(5.0)),
    );
    gui.action(
        "Preview latency offset (ms; negative delays picture)",
        Action::SetValue,
        Some(ActionData::NumericValue(40.0)),
    );
    gui.click("Apply picture trim and placement");
    assert_eq!(
        (
            gui.app.video.clip.as_ref().unwrap().trim_in,
            gui.app.video.clip.as_ref().unwrap().placement
        ),
        (3, 5)
    );
    gui.click("Seek picture start");
    gui.frame(vec![]);
    assert!((gui.rt.timeline_seconds() - 0.2).abs() < 1e-9);
    assert_eq!(
        gui.app.video.clip.as_ref().unwrap().source_frame(0.2),
        Some(4)
    );
    gui.text("Picture locator name", "Cue 日本語");
    gui.click("Add picture locator");
    assert_eq!(gui.app.video.clip.as_ref().unwrap().locators[0].frame, 5);
    gui.click("Close window");
    let project_path = files.0.join("picture-project.omat");
    gui.click("Project");
    gui.click("Save project as…");
    gui.text("Project file path", project_path.to_str().unwrap());
    gui.click("Save");
    gui.wait(|gui| !gui.app.project_pending_for_test());
    let persisted = crate::project_file::load::<project::Document>(
        &project_path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(persisted.state.view.video, gui.app.video.clip);
    assert_eq!(
        persisted
            .state
            .view
            .video
            .as_ref()
            .unwrap()
            .preview_offset_ms,
        40
    );
    assert_eq!(persisted.state.engine.timeline_seconds, 0.2);
    gui.click("Project");
    gui.click("New project");
    gui.wait(|gui| !gui.app.project_pending_for_test());
    assert!(gui.app.video.clip.is_none());
    gui.click("Project");
    gui.click("Open project…");
    gui.text("Project file path", project_path.to_str().unwrap());
    gui.click("Open");
    gui.wait(|gui| !gui.app.project_pending_for_test());
    gui.click("Video");
    gui.wait(|gui| gui.app.video.validated);
    if !gui.app.video.preview_paused {
        gui.click("Pause picture decoding");
    }
    gui.wait(|gui| gui.app.video.job.is_none());
    let saved_view = gui.app.project_view();
    let json = serde_json::to_vec(&saved_view).unwrap();
    let restored: project::UiState = serde_json::from_slice(&json).unwrap();
    assert_eq!(restored.video, saved_view.video);
    let output = files.0.join("rendered");
    gui.text("New score output folder", output.to_str().unwrap());
    let before = (gui.rt.beat, gui.rt.timeline_seconds(), gui.rt.playing);
    gui.click("Render selected scene against picture");
    gui.wait(|gui| output.exists() && gui.app.video.job.is_none());
    assert_eq!(
        (gui.rt.beat, gui.rt.timeline_seconds(), gui.rt.playing),
        before
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let alignment: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("alignment.json")).unwrap()).unwrap();
    assert_eq!(alignment["audio_frames"], 42240);
    assert_eq!(alignment["picture_first_sample"], 9600);
    let movie = data::decoder::probe(output.join("picture.mkv"), &AtomicBool::new(false)).unwrap();
    assert_eq!(movie.info.frames, 22);
    let score = crate::engine::decode::decode_audio(&output.join("score.wav"))
        .unwrap()
        .sample;
    assert_eq!(score.frames(), 42240);
    assert_eq!(score.sr, 48000);
    assert_eq!(score.ch, 2);
    assert!(score.data.iter().any(|value| value.abs() > 0.001));
    let retained = std::fs::read(output.join("alignment.json")).unwrap();
    gui.click("Render selected scene against picture");
    gui.wait(|gui| gui.app.video.job.is_none());
    assert_eq!(
        std::fs::read(output.join("alignment.json")).unwrap(),
        retained
    );
    assert!(!gui.app.video.message.contains("saved to"));
    let cancelled = files.0.join("cancelled-output");
    gui.text("New score output folder", cancelled.to_str().unwrap());
    gui.render = false;
    gui.click("Render selected scene against picture");
    gui.click("Cancel video job");
    gui.render = true;
    gui.wait(|gui| gui.app.video.job.is_none());
    assert!(!cancelled.exists());
    assert_eq!(gui.app.video.message, "Video job cancelled before publication");

    gui.app.initialize_startup_session(&gui.ctx, restored, None);
    gui.app.video.preview_paused = false;
    gui.app.video.open = true;
    gui.wait(|gui| gui.app.video.validated && gui.app.video.texture.is_some());
    assert_eq!(
        gui.app.video.clip.as_ref().unwrap().locators[0].name,
        "Cue 日本語"
    );
    assert_eq!(gui.app.video.texture.as_ref().unwrap().0, 4);
    gui.click("Remove picture reference");
    gui.frame(vec![]);
    assert!(gui.app.video.clip.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
