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
                "{} / {} / loads {:?}",
                self.app.library_scan.label(),
                self.app.library_metadata.label(),
                self.app.loads.iter().map(|l|l.as_ref().map(|l|match &l.phase {Phase::Failed(e)=>e.as_str(),Phase::Loading=>"loading",Phase::Queued=>"queued",Phase::Loaded=>"loaded",Phase::Superseded=>"superseded"})).collect::<Vec<_>>()
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

#[test]
fn actual_gui_enrollment_typed_reload_offline_reconnect_and_root_removal_preserve_audio_and_identity() {
    enrollment_gui(Files::new(),false);
}
#[test]
#[ignore = "local real block filesystem UUID and production Linux resolver qualification"]
fn local_block_volume_production_resolver_preserves_actual_gui_audio_and_identity() {
    let home=PathBuf::from(std::env::var_os("HOME").expect("local host home"));
    let root=home.join(format!(".cache/omat-import-block-ui-{}",crate::performance_history::storage::new_id().unwrap()));
    std::fs::create_dir(&root).unwrap();enrollment_gui(Files(root),true);
}
fn enrollment_gui(files:Files,real:bool) {
    use crate::media_location::Snapshot;
    use std::sync::Mutex;
    let path=files.wave();let fingerprint=FileFingerprint::read(&path).unwrap();let original=std::fs::read(&path).unwrap();
    let mut gui=Gui::new(&files);let file=LibSource::File(path.clone());
    gui.action(gui.node("Music file and folder paths"),Action::Focus);gui.frame(vec![egui::Event::Text(path.display().to_string())]);gui.frame(vec![]);
    gui.action(gui.node("Import music files/folders"),Action::Click);gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active() && g.app.library_metadata.catalog.track(&file).is_some());
    let identity=gui.app.library_metadata.catalog.track(&file).unwrap().id.clone();
    gui.app.load_source(1,Some(&Selection {title:"Imported source".into(),source:file.clone()}));gui.wait(|g|matches!(g.app.loads[1].as_ref().map(|l|&l.phase),Some(Phase::Loaded)));
    let receipt=gui.app.loads[1].as_ref().unwrap().receipt.clone().unwrap();
    gui.app.engine.send(Command::DeckSeek {deck:1,frac:0.25}).unwrap();gui.app.engine.send(Command::DeckCuePoint {deck:1,pad:0,del:false,receipt}).unwrap();
    gui.wait(|g|!g.app.library_metadata.active() && g.app.library_metadata.catalog.version(&file,Some(fingerprint)).is_some_and(|v|v.preparation.hotcues[0].is_some() && v.content_hash.is_some()));
    let preparation=gui.app.library_metadata.catalog.version(&file,Some(fingerprint)).unwrap().preparation;
    // Keep the real filesystem UUID/mount/namespace. Only removable
    // classification is injected; this is local software evidence, not USB QA.
    let mounted=if real {Snapshot::fixture_local_volume(&path).expect("local mounted block filesystem needed for this real-resolver case")}
        else {Snapshot::fixture_volume(&files.0,"SOFTWARE-107",1)};
    let typed=mounted.identify(&path).unwrap().source;assert!(matches!(typed,LibSource::Removable {..}));
    let mounts=Arc::new(Mutex::new(mounted.clone()));let inventory=mounts.clone();
    gui.app.library_scan=LibraryScan::with_inventory(move ||Ok(inventory.lock().unwrap().clone()));gui.app.library_scan.set_performance(gui.app.engine.cmd.performance().clone());
    gui.app.library_media_paths.clear();gui.app.library_media_revision+=1;gui.frame(vec![]);
    gui.action(gui.node("Music file and folder paths"),Action::Focus);gui.frame(vec![egui::Event::Text(path.display().to_string())]);gui.frame(vec![]);
    gui.action(gui.node("Import music files/folders"),Action::Click);gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active() && g.app.library_metadata.catalog.track(&typed).is_some());
    assert_eq!(gui.app.library_metadata.catalog.track(&typed).unwrap().id,identity);assert!(gui.app.library_metadata.catalog.track(&file).is_none());
    assert_eq!(gui.app.library.iter().filter(|i|matches!(i.source,LibSource::File(_) | LibSource::Removable {..})).count(),1);
    assert_eq!(gui.app.library_metadata.catalog.version(&file,Some(fingerprint)).unwrap().preparation,preparation);
    if !real {let inventory=mounts.clone();gui.app.loader=Some(crate::engine::media_load::Loader::with_inventory(move ||Ok(inventory.lock().unwrap().clone())).unwrap());}
    gui.app.load_source(1,Some(&Selection {title:"Typed volume".into(),source:typed.clone()}));gui.wait(|g|matches!(g.app.loads[1].as_ref().map(|l|&l.phase),Some(Phase::Loaded)) && !g.app.library_metadata.active());
    assert_eq!(gui.app.library_metadata.catalog.version(&typed,Some(fingerprint)).unwrap().preparation,preparation);
    assert_eq!(gui.app.loads[1].as_ref().unwrap().selection.as_ref().unwrap().source,typed);
    assert!(gui.app.library_scan.start_watched(vec![files.0.clone()],gui.app.library.clone(),"Live".into(),gui.app.library_metadata.catalog.clone()));
    gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active());
    assert!(gui.app.library_metadata.catalog.watched_roots.binding("Live",&files.0).is_some());
    *mounts.lock().unwrap()=Snapshot::fixture_offline();
    let inventory=mounts.clone();gui.app.loader=Some(crate::engine::media_load::Loader::with_inventory(move ||Ok(inventory.lock().unwrap().clone())).unwrap());
    gui.app.load_source(1,Some(&Selection {title:"Offline volume".into(),source:typed.clone()}));gui.wait(|g|matches!(g.app.loads[1].as_ref().map(|l|&l.phase),Some(Phase::Failed(e)) if e.contains("offline")));
    assert!(gui.app.library_scan.start_watched(vec![files.0.clone()],gui.app.library.clone(),"Live".into(),gui.app.library_metadata.catalog.clone()));gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active());
    assert!(gui.app.library_scan.summary.as_ref().unwrap().availability[&typed].contains("offline"));assert_eq!(gui.app.library_metadata.catalog.track(&typed).unwrap().id,identity);
    *mounts.lock().unwrap()=mounted;
    assert!(gui.app.library_scan.start_watched(vec![files.0.clone()],gui.app.library.clone(),"Live".into(),gui.app.library_metadata.catalog.clone()));gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active());
    assert_eq!(gui.app.library_metadata.catalog.track(&typed).unwrap().id,identity);assert_eq!(gui.app.library_metadata.catalog.track(&typed).unwrap().versions.len(),1);
    gui.app.load_source(1,Some(&Selection {title:"Reconnected volume".into(),source:typed.clone()}));gui.wait(|g|matches!(g.app.loads[1].as_ref().map(|l|&l.phase),Some(Phase::Loaded)) && !g.app.library_metadata.active());
    assert!(gui.app.library_scan.start_watched(Vec::new(),gui.app.library.clone(),"Live".into(),gui.app.library_metadata.catalog.clone()));gui.wait(|g|!g.app.library_scan.active() && !g.app.library_metadata.active());
    let saved=crate::library::read(&files.0.join("saved/library.json")).unwrap();assert!(saved.watched_roots.binding("Live",&files.0).is_none());assert_eq!(saved.track(&typed).unwrap().id,identity);assert_eq!(saved.version(&typed,Some(fingerprint)).unwrap().preparation,preparation);
    assert!(gui.nonzero);assert_eq!(FileFingerprint::read(&path),Some(fingerprint));assert_eq!(std::fs::read(path).unwrap(),original);
}
