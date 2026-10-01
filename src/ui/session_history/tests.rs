use super::*;
use crate::engine::audio::OutputCallback;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::time::Duration;
struct Files(PathBuf);
impl Files { fn new() -> Self { Self(std::env::temp_dir().join(format!("omat-history-ui-{}", crate::performance_history::storage::new_id().unwrap()))) } }
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
struct Gui { app: App, callback: OutputCallback, ctx: egui::Context, nodes: Vec<(NodeId, Node)>, time: f64 }
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256); rt.publish_for_test();
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.start_session_history(files.0.join("history"));
        let ctx = egui::Context::default(); ctx.enable_accesskit();
        let mut gui = Self { app, callback: OutputCallback::new(rt, 2), ctx, nodes: vec![], time: 0.0 };
        gui.wait(|g| g.worker().view().ready); gui.click("History"); gui.frame(vec![]); gui
    }
    fn worker(&self) -> &Worker { self.app.session_history.worker.as_ref().unwrap() }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))), focused: true, time: Some(self.time), events, ..Default::default() }, |ctx| self.app.update_frame(ctx));
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.callback.render(&mut [0.0_f32; 256]);
    }
    fn wait(&mut self, mut predicate: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop { self.frame(vec![]); if predicate(self) { break; }
            assert!(Instant::now() < deadline, "history wait: {:?}; {}", self.worker().view(), self.app.session_history.notice);
            std::thread::sleep(Duration::from_millis(2)); }
    }
    fn node(&self, label: &str) -> NodeId { self.nodes.iter().find(|(_, n)| n.label() == Some(label)).map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("missing {label}: {:?}", self.nodes.iter().filter_map(|(_, n)| n.label()).collect::<Vec<_>>())) }
    fn action(&mut self, target: NodeId, action: Action) { self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action, data: None })]); self.frame(vec![]); }
    fn click(&mut self, label: &str) { self.action(self.node(label), Action::Click); }
    fn text(&mut self, label: &str, value: &str) {
        self.action(self.node(label), Action::Focus);
        self.frame(vec![egui::Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() } }, egui::Event::Text(value.into())]); self.frame(vec![]);
    }
    fn finish(&mut self) { self.wait(|g| !g.worker().pending()); }
}
#[test]
fn actual_history_controls_measure_mark_type_export_and_cancel_exit() {
    let files = Files::new(); let mut gui = Gui::new(&files);
    gui.click("Start session"); gui.finish();
    assert!(gui.worker().view().active.is_some());
    gui.wait(|g| g.worker().view().selected.as_ref().is_some_and(|s| s.entries.len() == 2 && s.entries.iter().all(|e| matches!(e.source, Source::Catalog { .. }))));
    let session = gui.worker().view().selected.as_ref().unwrap();
    assert!(session.entries.iter().all(|e| e.measured_seconds() == 0.0 && !e.played()));
    gui.app.engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    gui.wait(|g| g.worker().view().selected.as_ref().unwrap().entries.iter().any(|e| e.measured_seconds() > 0.0));
    gui.click("End session"); gui.finish();
    let id = gui.worker().view().selected.as_ref().unwrap().id.clone();
    let before = gui.worker().view().selected.as_ref().unwrap().entries[0].measured_seconds();
    assert!(before > 0.0 && gui.worker().view().durable);
    // The node is scoped to the captured revision; replaying it after an edit
    // cannot change another entry or undo the first action.
    let stale = gui.node("Mark unplayed · Drums (session) · deck A · entry 1"); gui.action(stale, Action::Click); gui.finish();
    let revision = gui.worker().view().selected.as_ref().unwrap().edit_revision;
    gui.action(stale, Action::Click); gui.frame(vec![]);
    assert_eq!(gui.worker().view().selected.as_ref().unwrap().edit_revision, revision);
    assert!(!gui.worker().view().selected.as_ref().unwrap().entries[0].played(), "view {:?}; notice {}", gui.worker().view(), gui.app.session_history.notice);
    assert_eq!(gui.worker().view().selected.as_ref().unwrap().entries[0].measured_seconds(), before);
    gui.text("External track title", "Vinyl from guest"); gui.text("External track artist", "Guest artist");
    gui.click("Add external track"); gui.finish();
    assert!(gui.worker().view().selected.as_ref().unwrap().entries.iter().any(|e| matches!(&e.source, Source::External { title, artist } if title == "Vinyl from guest" && artist == "Guest artist")));
    let export = files.0.join("set.json"); gui.text("History export destination", export.to_str().unwrap());
    gui.click("Export session"); gui.finish();
    let bytes = std::fs::read(&export).unwrap(); let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["session_id"], id); assert!(!String::from_utf8_lossy(&bytes).contains("load_key"));
    gui.click("Export session"); gui.finish(); assert!(!gui.worker().view().receipt.as_ref().unwrap().applied);
    assert_eq!(std::fs::read(&export).unwrap(), bytes);
    let stale_external = gui.node("Add external track");
    let stale_export = gui.node("Export session");
    // Starting a new session retires every captured edit/export action from the
    // prior selection. Old native controls cannot apply to this new session.
    gui.click("Start session"); gui.finish();
    let next_id = gui.worker().view().active.clone().unwrap();
    assert_ne!(next_id, id);
    let receipt_id = gui.worker().view().receipt.as_ref().unwrap().id;
    for target in [stale_external, stale_export] { gui.action(target, Action::Click); }
    assert!(!gui.worker().pending());
    assert_eq!(gui.worker().view().receipt.as_ref().unwrap().id, receipt_id);
    assert_eq!(gui.worker().view().selected.as_ref().unwrap().edit_revision, 0);
    // Cancel while the boundary request is still pending, before its receipt.
    gui.app.begin_session_history_close(&gui.ctx);
    gui.frame(vec![]); gui.click("Keep working");
    gui.wait(|g| !g.app.session_history.keep_pending && !g.worker().pending());
    assert!(!gui.app.session_history.closing);
    assert!(gui.worker().view().active.is_none());
    gui.click("Start session"); gui.finish(); assert!(gui.worker().view().active.is_some());
    gui.click("End session"); gui.finish();
}

#[test]
fn keyboard_scroll_reaches_last_entry_in_a_full_virtualized_history() {
    let files = Files::new();
    let mut store = crate::performance_history::storage::Store::open(files.0.join("history")).unwrap();
    let mut session = crate::performance_history::Session::new(crate::performance_history::storage::new_id().unwrap(), 1, 1_000_000_000, 0).unwrap();
    for entry in 1..=crate::performance_history::MAX_ENTRIES {
        let title = if entry == 1 { "L".repeat(1024) } else { format!("External {entry}") };
        session.external(session.edit_revision, title, String::new()).unwrap();
    }
    session.end(1_000_000_000, 0, false, 0).unwrap(); store.save(&session).unwrap(); drop(store);
    let mut gui = Gui::new(&files);
    gui.wait(|g| g.nodes.iter().any(|(_,n)|n.label()==Some("History entries")));
    assert!(!gui.nodes.iter().any(|(_,n)|n.label().is_some_and(|s|s.contains("entry 4096"))), "offscreen entries must stay virtualized");
    gui.action(gui.node("History entries"), Action::Focus);
    for pressed in [true, false] {
        gui.frame(vec![egui::Event::Key { key: Key::End, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }]);
    }
    gui.wait(|g|g.nodes.iter().any(|(_,n)|n.label()==Some("Mark unplayed · External 4096 · no measured deck · entry 4096")));
    assert!(gui.nodes.iter().filter(|(_,n)|n.label().is_some_and(|s|s.starts_with("Mark played ·"))).count() <= 5);
    gui.click("Mark unplayed · External 4096 · no measured deck · entry 4096"); gui.finish();
    assert_eq!(gui.worker().view().selected.as_ref().unwrap().entries[4095].played_override, Some(false));
    gui.frame(vec![]);
    assert!(gui.nodes.iter().any(|(_,n)|n.label()==Some("Mark unplayed · External 4096 · no measured deck · entry 4096")), "marking the last entry must preserve its scroll position");
}

#[test]
fn simultaneous_end_and_edit_keep_the_first_boundary_action() {
    let files = Files::new(); let mut gui = Gui::new(&files);
    gui.click("Start session"); gui.finish();
    gui.wait(|g|g.nodes.iter().any(|(_,n)|n.label()==Some("Mark unplayed · Drums (session) · deck A · entry 1")));
    let end = gui.node("End session");
    let mark = gui.node("Mark unplayed · Drums (session) · deck A · entry 1");
    gui.frame(vec![end, mark].into_iter().map(|target|egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Click, data: None })).collect());
    gui.finish();
    assert!(gui.worker().view().active.is_none(), "a later edit in the same frame must not replace End");
    assert!(gui.worker().view().selected.as_ref().unwrap().entries.iter().all(|e|e.played_override.is_none()));
}
