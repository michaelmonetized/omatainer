use super::*;
use crate::engine::{test_alloc, RtEngine};
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "omatainer-recovery-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    close: usize,
    render: bool,
}
impl Drop for Gui {
    fn drop(&mut self) {
        if let Some(worker) = self.app.recovery.worker.take() {
            worker.stop_for_test();
        }
    }
}
impl Gui {
    fn new() -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, 256);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut result = Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx,
            nodes: vec![],
            time: 0.0,
            close: 0,
            render: true,
        };
        result.frame(vec![]);
        result.frame(vec![]);
        result
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.raw(egui::RawInput {
            events,
            ..Default::default()
        })
    }
    fn raw(&mut self, mut raw: egui::RawInput) -> egui::FullOutput {
        self.time += 0.02;
        raw.time = Some(self.time);
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1300.0)));
        let result = self.ctx.run(raw, |ctx| self.app.update_frame(ctx));
        self.nodes = result
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.close += result.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|c| matches!(c, egui::ViewportCommand::Close))
            .count();
        if self.render {
            self.rt.process(&mut [0.0; 256]);
            self.rt.publish_for_test();
        }
        result
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, label: &str, action: Action) {
        let target = self.node(label);
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
                modifiers: egui::Modifiers::CTRL,
            },
            egui::Event::Text(value.into()),
        ]);
        self.frame(vec![]);
    }
    fn menu(&mut self, label: &str) {
        self.click("Project");
        self.click(label);
    }
    fn wait(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition(self) {
            self.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "Recovery timeout: {:?}; project {:?}",
                self.app.recovery.message,
                self.app.recovery_project_path()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
    fn settled(&mut self) {
        self.wait(|g| !g.app.recovery_project_busy());
    }
    fn start(&mut self, root: &PathBuf) {
        self.app.start_recovery(root.clone());
        self.frame(vec![]);
    }
    fn durable(&mut self, revision: u64) {
        self.wait(|g| {
            g.app
                .recovery
                .worker
                .as_ref()
                .unwrap()
                .status()
                .durable
                .as_ref()
                .is_some_and(|d| d.revision == revision && Some(d.epoch) == g.app.recovery.epoch)
        });
    }
    fn edit(&mut self, value: f32) -> u64 {
        assert!(self.app.submit(Command::Master(value)));
        self.rt.process(&mut []);
        self.frame(vec![]);
        self.app.engine.project.revision()
    }
    fn save_as(&mut self, path: &PathBuf) {
        self.menu("Save project as…");
        self.text("Project file path", &path.display().to_string());
        self.click("Save");
        self.settled();
        assert_eq!(self.app.recovery_project_path().as_ref(), Some(path));
    }
    fn crash_worker(&mut self) {
        self.app.recovery.worker.take().unwrap().stop_for_test();
    }
    fn open_panel(&mut self) {
        self.app.recovery.open = true;
        self.frame(vec![]);
        self.frame(vec![]);
    }
    fn native_close(&mut self) {
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        self.raw(input);
    }
}
fn existing(files: &Files) -> (PathBuf, Vec<u8>, crate::recovery::Candidate) {
    let path = files.path("explicit.omat");
    let root = files.path("recovery");
    let mut gui = Gui::new();
    gui.save_as(&path);
    let saved = std::fs::read(&path).unwrap();
    gui.app.lib_filter = "Harmony".into();
    let revision = gui.edit(0.37);
    gui.start(&root);
    gui.durable(revision);
    gui.crash_worker();
    let inventory = crate::recovery::discover(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(inventory.candidates.len(), 1);
    (path, saved, inventory.candidates[0].clone())
}

#[test]
fn restart_preview_cancel_and_restore_preserve_explicit_file_and_current_unsaved_work() {
    let files = Files::new();
    let (path, saved, _) = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    assert!(gui.app.recovery.open);
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    let before = gui.rt.master;
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    assert_eq!(gui.rt.master, before);
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert!(!gui.app.project_dirty());
    gui.edit(0.81);
    gui.click("Restore as untitled copy");
    assert!(gui
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Unsaved project changes")));
    gui.click("Cancel");
    assert_eq!(gui.rt.master, 0.81);
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    gui.open_panel();
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    gui.click("Restore as untitled copy");
    gui.click("Discard changes");
    gui.settled();
    assert_eq!(gui.rt.master, 0.37);
    assert_eq!(gui.app.lib_filter, "Harmony");
    assert_ne!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert!(!gui.rt.playing && gui.rt.decks.iter().all(|deck| !deck.playing));
    assert!(gui.app.project_dirty());
    assert!(gui.app.recovery_project_path().is_none());
    assert_eq!(std::fs::read(path).unwrap(), saved);
}

#[test]
fn preview_cancellation_and_changed_current_document_never_install_stale_state() {
    let files = Files::new();
    let (_, _, candidate) = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::BeforeInspect);
    gui.click("Preview recovery");
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.click("Cancel recovery operation");
    resume.send(()).unwrap();
    gui.wait(|g| g.app.recovery.pending.is_none());
    for _ in 0..20 {
        gui.frame(vec![]);
    }
    assert!(gui.app.recovery.preview.is_none());
    // This is a real project Prepare/Ready handshake, with an edit inserted
    // while the prepared replacement remains on its worker.
    let (entered, resume) = gui.app.pause_project_prepare_for_recovery_test();
    gui.app.request_recovery_restore(candidate);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    gui.edit(0.69);
    resume.send(()).unwrap();
    gui.settled();
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert_eq!(gui.rt.master, 0.69);
}

#[test]
fn full_disk_keeps_prior_durable_age_and_ui_reports_retry_then_advances_after_recovery() {
    let files = Files::new();
    let root = files.path("recovery");
    let mut gui = Gui::new();
    let revision = gui.edit(0.25);
    gui.start(&root);
    gui.durable(revision);
    let before = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .status()
        .durable
        .as_ref()
        .unwrap()
        .clone();
    let fault = crate::recovery::testing::enospc(&root);
    let next = gui.edit(0.61);
    gui.open_panel();
    gui.click("Journal current edits now");
    gui.wait(|g| g.app.recovery.worker.as_ref().unwrap().status().warning);
    let status = gui.app.recovery.worker.as_ref().unwrap().status();
    let after = status.durable.as_ref().unwrap();
    assert_eq!(
        (after.sequence, after.unix_ms, after.revision),
        (before.sequence, before.unix_ms, before.revision)
    );
    assert!(gui.app.recovery_toolbar_text().contains("warning"));
    assert!(status.message.contains("not saved"));
    assert_eq!(gui.rt.master, 0.61);
    drop(fault);
    gui.click("Journal current edits now");
    gui.durable(next);
    assert!(!gui.app.recovery.worker.as_ref().unwrap().status().warning);
}

#[test]
fn cancelled_close_after_retirement_opens_a_fresh_recoverable_session() {
    let files = Files::new();
    let root = files.path("recovery");
    let mut gui = Gui::new();
    let revision = gui.edit(0.41);
    gui.start(&root);
    gui.durable(revision);
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::AfterRetire);
    gui.native_close();
    gui.click("Discard changes");
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.click("Keep working");
    resume.send(()).unwrap();
    gui.wait(|g| !g.app.recovery.closing);
    assert_eq!(gui.close, 0);
    assert!(!gui.app.project_admission_sealed());
    let revision = gui.edit(0.71);
    gui.durable(revision);
    gui.crash_worker();
    let list = crate::recovery::discover(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(list.candidates.len(), 1);
    let recovered =
        crate::recovery::recover::<project::Document>(&list.candidates[0], &AtomicBool::new(false))
            .unwrap();
    assert_eq!(recovered.bundle.state.engine.master, 0.71);
}

#[test]
fn cancelled_automatic_capture_reply_does_not_block_new_open_or_restore() {
    let files = Files::new();
    let (path, _, candidate) = existing(&files);
    for action in 0..4 {
        let mut gui = Gui::new();
        gui.render = false;
        gui.start(&files.path(&format!("pending-{action}")));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !gui
            .app
            .recovery
            .worker
            .as_ref()
            .unwrap()
            .capture_fence()
            .load(Ordering::SeqCst)
        {
            gui.frame(vec![]);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        match action {
            0 | 3 => gui.menu("New project"),
            1 => {
                gui.menu("Open project…");
                gui.text("Project file path", &path.display().to_string());
                gui.click("Open");
            }
            _ => gui.app.request_recovery_restore(candidate.clone()),
        }
        // Let the cancelled capture leave its reply pending while the project
        // worker waits. All progress resumes through the actual render boundary.
        std::thread::sleep(Duration::from_millis(15));
        if action == 3 {
            assert!(gui.app.submit(Command::Master(0.79)));
        }
        gui.render = true;
        gui.settled();
        match action {
            0 => assert!(gui
                .rt
                .tracks
                .iter()
                .all(|t| t.clips.iter().all(|c| c.notes.is_empty()))),
            1 => assert_eq!(gui.app.recovery_project_path().as_ref(), Some(&path)),
            2 => assert_eq!(gui.rt.master, 0.37),
            _ => {
                assert_eq!(gui.rt.master, 0.79);
                assert!(gui.app.project_dirty());
            }
        }
    }
}

#[test]
fn periodic_capture_and_disk_writes_keep_real_output_callback_heap_free() {
    let files = Files::new();
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.apply(Command::LaunchScene(0));
    let mut app = App::with_loader(engine, Theme::default(), None);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let (_, mut reference) = Engine::headless_for_test(48_000, 256);
    reference.apply(Command::LaunchScene(0));
    let mut reference = crate::engine::audio::OutputCallback::new(reference, 2);
    let mut block = [0.0f32; 256];
    let mut expected = block;
    for _ in 0..200 {
        callback.render(&mut block);
        reference.render(&mut expected);
    }
    app.start_recovery(files.path("recovery"));
    app.sync_recovery();
    let mut confirmed = None;
    let mut completed = 0;
    let mut peak = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut rendered = 0;
    while completed < 3 || rendered < 2048 {
        let started = Instant::now();
        let counts = test_alloc::measure(|| callback.render(&mut block));
        peak = peak.max(started.elapsed());
        assert_eq!(
            (counts.allocations, counts.frees),
            (0, 0),
            "capture/render/publication performed callback heap work"
        );
        assert!(block.iter().all(|sample| sample.is_finite()));
        reference.render(&mut expected);
        assert_eq!(block, expected, "recovery capture changed rendered output");
        app.poll_recovery();
        app.sync_recovery();
        let durable = app
            .recovery
            .worker
            .as_ref()
            .unwrap()
            .status()
            .durable
            .clone();
        if durable.as_ref().map(|d| d.sequence) != confirmed {
            confirmed = durable.as_ref().map(|d| d.sequence);
            if confirmed.is_some() {
                completed += 1;
                app.keys_open = !app.keys_open;
                app.sync_recovery();
            }
        }
        rendered += 1;
        assert!(
            Instant::now() < deadline,
            "no durable record: {}",
            app.recovery.worker.as_ref().unwrap().status().message
        );
        if rendered % 8 == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(completed, 3);
    eprintln!("recovery capture: {rendered} actual256-sample callbacks, observed max wall {peak:?}; not a hardware deadline or XRUN result");
    app.recovery.worker.take().unwrap().stop_for_test();
}

#[test]
fn durable_age_is_honest_when_wall_clock_moves_backward() {
    assert_eq!(durable_age(5500, 3000), "2 s ago");
    assert!(durable_age(2999, 3000).contains("unavailable"));
}
