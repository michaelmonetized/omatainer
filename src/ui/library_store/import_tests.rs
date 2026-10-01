use super::*;
use crate::engine::audio::OutputCallback;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "omat-import-ui-{}",
            crate::performance_history::storage::new_id().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn wave(&self) -> PathBuf {
        let path = self.0.join("Imported 120 Am.wav");
        let frames = 4096u32;
        let size = frames * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for i in 0..frames {
            bytes.extend((((i as f32 * 0.04).sin() * 8192.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Gui {
    app: App,
    callback: OutputCallback,
    reference: OutputCallback,
    _reference_engine: Engine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    nonzero: bool,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let (reference_engine, reference_rt) = Engine::headless_for_test(48000, 256);
        engine.send(Command::DeckPlay { deck: 0 }).unwrap();
        reference_engine
            .send(Command::DeckPlay { deck: 0 })
            .unwrap();
        let mut app = App::with_loader(
            engine,
            Theme::default(),
            Some(crate::engine::media_load::Loader::start().unwrap()),
        );
        app.start_library_store(files.0.join("saved/library.json"));
        app.library_import_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            callback: OutputCallback::new(rt, 2),
            reference: OutputCallback::new(reference_rt, 2),
            _reference_engine: reference_engine,
            ctx,
            nodes: vec![],
            time: 0.0,
            nonzero: false,
        };
        gui.wait(|g| g.app.library_metadata.durable && !g.app.library_metadata.active());
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        let mut actual = [0.0f32; 512];
        let mut expected = [0.0f32; 512];
        self.callback.render(&mut actual);
        self.reference.render(&mut expected);
        assert_eq!(
            actual, expected,
            "import or text controls altered playing deck output"
        );
        self.nonzero |= actual.iter().any(|v| v.abs() > 0.001);
    }
    fn wait(&mut self, mut predicate: impl FnMut(&Self) -> bool) {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            self.frame(vec![]);
            if predicate(self) {
                return;
            }
            assert!(
                Instant::now() < until,
                "{} / {}",
                self.app.library_scan.label(),
                self.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("missing {label}"))
    }
    fn action(&mut self, target: NodeId, action: Action) {
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data: None,
        })]);
        self.frame(vec![]);
    }
}
#[test]
fn actual_typed_import_persists_once_without_changing_playing_audio() {
    let files = Files::new();
    let path = files.wave();
    let fingerprint = FileFingerprint::read(&path).unwrap();
    let mut gui = Gui::new(&files);
    let stale = gui.node("Import music files/folders");
    gui.action(gui.node("Music file and folder paths"), Action::Focus);
    gui.frame(vec![egui::Event::Text(format!(
        "{}\n{}",
        path.display(),
        files.0.display()
    ))]);
    gui.frame(vec![]);
    gui.action(stale, Action::Click);
    assert_eq!(
        gui.app.library_scan.state,
        super::super::library_scan::ScanState::Idle
    );
    gui.action(gui.node("Import music files/folders"), Action::Click);
    gui.wait(|g| {
        !g.app.library_scan.active()
            && !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&LibSource::File(path.clone()))
                .is_some()
    });
    assert!(gui.nonzero);
    assert_eq!(FileFingerprint::read(&path), Some(fingerprint));
    let catalog = crate::library::read(&files.0.join("saved/library.json")).unwrap();
    assert_eq!(
        catalog
            .tracks
            .iter()
            .filter(|t| t.source == LibSource::File(path.clone()))
            .count(),
        1
    );
    assert!(
        gui.app
            .library_scan
            .summary
            .as_ref()
            .unwrap()
            .skipped_count()
            > 0,
        "overlap and saved JSON should expose skip reasons"
    );
    gui.action(
        gui.node("Manage music folders in Preferences"),
        Action::Click,
    );
    assert!(gui.app.settings.open);
}
