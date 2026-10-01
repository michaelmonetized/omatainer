use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("omat-crates-ui-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        for name in ["One.flac", "Two.flac", "Three.flac"] { std::fs::write(path.join(name), include_bytes!("../../../tests/fixtures/audio/tone.flac")).unwrap(); }
        Self(path)
    }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.library_metadata = library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata.set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default(); ctx.enable_accesskit();
        let mut gui = Self { app, rt, ctx, nodes: vec![], time: 0.0 };
        gui.wait(|gui| !gui.app.library_metadata.active());
        gui.app.library_crates.open = true;
        gui.frame(vec![]); gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))), focused: true, time: Some(self.time), events, ..Default::default() }, |ctx| self.app.update_frame(ctx));
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.rt.process(&mut [0.0; 256]); self.rt.publish_for_test();
    }
    fn wait(&mut self, mut predicate: impl FnMut(&Self) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(vec![]);
            if predicate(self) { break; }
            assert!(Instant::now() < until, "crate wait: {}; metadata {:?}", self.app.library_crates.message, self.app.library_metadata.storage_error);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes.iter().find(|(_, n)| n.label() == Some(label)).map(|(id, _)| *id).unwrap_or_else(|| panic!("missing {label}: {:?}", self.nodes.iter().filter_map(|(_, n)| n.label()).collect::<Vec<_>>()))
    }
    fn action(&mut self, target: NodeId, action: Action, data: Option<ActionData>) {
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action, data })]); self.frame(vec![]);
    }
    fn click(&mut self, label: &str) { self.action(self.node(label), Action::Click, None); }
    fn value(&mut self, label: &str, value: f64) { self.action(self.node(label), Action::SetValue, Some(ActionData::NumericValue(value))); }
    fn name(&mut self, name: &str) {
        let id = self.nodes.iter().find(|(_, node)| node.role() == egui::accesskit::Role::TextInput && node.label() == Some("Crate name")).map(|(id, _)| *id).unwrap();
        self.action(id, Action::Focus, None);
        self.frame(vec![egui::Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() } }, egui::Event::Text(name.into())]);
        self.frame(vec![]);
    }
    fn finish(&mut self) { self.wait(|gui| gui.app.library_crates.pending.is_none() && !gui.app.library_metadata.active()); }
    fn create(&mut self, name: &str, child: bool) -> CrateId {
        self.name(name); self.click(if child { "New child crate" } else { "New root crate" }); self.finish();
        self.app.library_crates.selected.clone().unwrap()
    }
    fn select(&mut self, id: Option<&CrateId>) {
        let row = id.and_then(|id| self.app.library_crates.tree.iter().position(|(index, _)| &self.app.library_metadata.catalog.crates.nodes()[*index].id == id)).map(|i| i + 1).unwrap_or(0);
        self.value("Crate tree row (0 = All tracks)", row as f64);
    }
    fn scan(&mut self, files: &Files) {
        assert!(self.app.library_scan.start(vec![files.0.clone()], self.app.library.clone()));
        self.wait(|gui| !gui.app.library_scan.active() && !gui.app.library_metadata.active());
    }
    fn members(&self, id: &CrateId) -> Vec<TrackId> { self.app.library_metadata.catalog.crates.node(id).unwrap().members.clone() }
}

#[test]
fn actual_manager_builds_nested_overlapping_ordered_crates_and_reopens_without_touching_audio() {
    let files = Files::new();
    let before: Vec<_> = ["One.flac", "Two.flac", "Three.flac"].map(|name| {
        let path = files.0.join(name);
        (path.clone(), std::fs::read(&path).unwrap(), FileFingerprint::read(&path).unwrap())
    }).into();
    let mut gui = Gui::new(&files); gui.scan(&files);
    let root = gui.create("Studio", false);
    let child = gui.create("Opening", true);
    assert_eq!(gui.app.library_crates.parents[&child], Some(root.clone()));
    gui.click("Use as destination"); gui.select(None);
    gui.app.lib_filter = ".flac".into(); // persisted titles omit suffix; choose all three by their known source IDs below
    gui.app.lib_filter.clear();
    gui.click("Add filtered tracks"); gui.finish();
    let all = gui.members(&child); assert!(all.len() >= 3);
    let second = gui.create("Peak", false);
    gui.click("Use as destination"); gui.select(Some(&child));
    gui.value("Member row", 1.0); gui.click("Toggle member at row");
    gui.click("Copy selected to destination"); gui.finish();
    assert_eq!(gui.members(&second), vec![all[0].clone()]);
    assert_eq!(gui.members(&child), all);
    gui.value("Insert before row (last + 1 appends)", (all.len() + 1) as f64);
    gui.click("Reorder selected members"); gui.finish();
    let reordered = gui.members(&child); assert_eq!(reordered.last(), Some(&all[0]));
    assert_eq!(&reordered[..reordered.len() - 1], &all[1..]);
    gui.name("Warmup"); gui.click("Rename crate"); gui.finish();
    assert_eq!(gui.app.library_metadata.catalog.crates.node(&child).unwrap().name, "Warmup");
    gui.click("Delete crate…"); gui.click("Keep crate"); assert!(gui.app.library_metadata.catalog.crates.node(&child).is_some());
    let view = gui.app.project_view(); assert_eq!(view.selected_crate, Some(child.clone()));
    drop(gui);
    let until = Instant::now() + Duration::from_secs(10);
    loop { match crate::library::Store::open(files.0.join("catalog.json")) { Ok(store) => { drop(store); break; }, Err(_) => { assert!(Instant::now() < until); std::thread::sleep(Duration::from_millis(2)); } } }
    let mut gui = Gui::new(&files); gui.select(Some(&child));
    assert_eq!(gui.members(&child), reordered);
    assert_eq!(gui.members(&second), vec![all[0].clone()]);
    let visible: Vec<_> = gui.app.library_view.indices.iter().map(|&index| gui.app.library_metadata.catalog.track(&gui.app.library[index].source).unwrap().id.clone()).collect();
    assert_eq!(visible, reordered);
    // A controller burst captures the admitted manual-order source even if the
    // GUI switches back to All tracks before dispatching its queued load.
    let target = gui.app.library_view.indices.iter().enumerate()
        .find(|(index, row)| *index > 0 && matches!(gui.app.library[**row].source, LibSource::File(_)))
        .map(|(index, row)| (index, gui.app.library[*row].source.clone())).unwrap();
    gui.app.lib_sel = target.0 - 1;
    gui.app.publish_library_selection();
    assert!(gui.app.engine.send(Command::Browse(1.0)).is_ok());
    assert!(gui.app.engine.send(Command::DeckLoadSelected { deck: 0 }).is_ok());
    gui.app.choose_named_crate(None);
    let LibSource::File(path) = target.1 else { unreachable!() };
    gui.wait(|gui| gui.rt.decks[0].audio.as_ref().is_some_and(|sample| sample.path == path.to_string_lossy()));
    for (path, bytes, fingerprint) in before { assert_eq!(std::fs::read(&path).unwrap(), bytes); assert_eq!(FileFingerprint::read(&path), Some(fingerprint)); }
}

#[test]
fn retained_native_membership_and_delete_actions_do_not_retarget_new_selections() {
    let files = Files::new();
    let mut gui = Gui::new(&files); gui.scan(&files);
    let id = gui.create("Review", false);
    gui.click("Use as destination"); gui.select(None);
    gui.click("Add filtered tracks"); gui.finish(); gui.select(Some(&id));
    let original = gui.members(&id);
    gui.click("Toggle member at row");
    let old_remove = gui.node("Remove selected memberships");
    gui.value("Member row", 2.0); gui.click("Toggle member at row");
    gui.action(old_remove, Action::Click, None); gui.finish();
    assert_eq!(gui.members(&id), original);
    gui.click("Remove selected memberships"); gui.finish();
    assert_eq!(gui.members(&id), original[2..]);
    gui.click("Delete crate…");
    let old_delete = gui.node("Confirm delete crate subtree");
    gui.select(None);
    let other = gui.create("Keep", false);
    gui.click("Delete crate…");
    gui.action(old_delete, Action::Click, None); gui.finish();
    assert!(gui.app.library_metadata.catalog.crates.node(&id).is_some());
    assert!(gui.app.library_metadata.catalog.crates.node(&other).is_some());
    gui.click("Keep crate");
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.name("Protected rename"); gui.click("Rename crate"); gui.finish();
    assert_eq!(gui.app.library_metadata.catalog.crates.node(&other).unwrap().name, "Keep");
    assert!(gui.app.library_crates.message.to_lowercase().contains("performance"));
}

#[test]
fn large_named_crate_and_tree_reuse_view_indices_and_render_only_visible_rows() {
    let files = Files::new();
    let mut store = crate::library::Store::open(files.0.join("catalog.json")).unwrap();
    for index in 0..10_000 {
        store.catalog.upsert(LibSource::File(files.0.join(format!("unavailable-{index}.wav"))), None,
            crate::library::Metadata { title: format!("Track {index:05}"), artist: "Fixture".into(),
                bpm: Bpm::UNKNOWN, key: "—".into(), duration: None, last_play: None }).unwrap();
    }
    let members: Vec<_> = store.catalog.tracks.iter().rev().map(|track| track.id.clone()).collect();
    let ids: Vec<_> = (1..=4096).map(|index| CrateId(format!("{index:032x}"))).collect();
    let nodes: Vec<_> = ids.iter().enumerate().map(|(index, id)| serde_json::json!({
        "id":id,"name":format!("Crate {index:04}"),"children":[],
        "members":if index == 0 { members.clone() } else { vec![] }})).collect();
    store.catalog.crates = serde_json::from_value(serde_json::json!({"revision":1,"roots":ids,"nodes":nodes})).unwrap();
    store.save().unwrap(); drop(store);
    let mut gui = Gui::new(&files); gui.select(Some(&ids[0]));
    assert_eq!(gui.app.library_view.indices.len(), 10_000);
    let stats = gui.app.library_view.stats;
    let trees = gui.app.library_crates.tree_rebuilds;
    let indices = gui.app.library_view.indices.clone();
    for _ in 0..20 {
        gui.frame(vec![]);
        assert!(Arc::ptr_eq(&indices, &gui.app.library_view.indices));
        assert_eq!(gui.app.library_view.stats.rebuilds, stats.rebuilds);
        assert_eq!(gui.app.library_view.stats.examined, stats.examined);
        assert_eq!(gui.app.library_crates.tree_rebuilds, trees);
        let visible = gui.nodes.iter().filter(|(_, node)| node.label().is_some_and(|label| label.starts_with("Member ") && label.contains(':'))).count();
        assert!(visible > 0 && visible < 32, "rendered {visible} member rows");
    }
}
