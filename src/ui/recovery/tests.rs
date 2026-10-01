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
        // Native Linux backends set both ctrl and the platform command bit.
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        self.raw(egui::RawInput {
            modifiers,
            events: vec![
                egui::Event::Key {
                    key: Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Text(value.into()),
            ],
            ..Default::default()
        });
        self.frame(vec![egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }]);
    }
    fn menu(&mut self, label: &str) {
        self.click("Project");
        self.click(label);
    }
    #[track_caller]
    fn wait(&mut self, condition: impl Fn(&Self) -> bool) {
        let caller = std::panic::Location::caller();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition(self) {
            self.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "Recovery timeout at {caller}: {:?}; project {:?}; worker {:?}; pending {:?}; controls {:?}",
                self.app.recovery.message,
                self.app.project_message_for_recovery_test(),
                self.app.recovery.worker.as_ref().map(|w| w.status().message.clone()),
                self.app.recovery.pending,
                self.nodes.iter().filter_map(|(_, n)| n.label()).collect::<Vec<_>>()
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
        assert_eq!(
            self.nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Project file path"))
                .unwrap()
                .1
                .value(),
            Some(path.to_str().unwrap()),
            "path input did not change"
        );
        self.click("Save");
        self.settled();
        assert_eq!(
            self.app.recovery_project_path().as_ref(),
            Some(path),
            "{:?}",
            self.app.project_message_for_recovery_test()
        );
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
        self.frame(vec![]);
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
        .any(|(_, n)| n.label() == Some("Save changes")));
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
        (after.sequence, after.captured_unix_ms, after.revision),
        (before.sequence, before.captured_unix_ms, before.revision)
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
    rt.apply(Command::LaunchScene { scene: 0 });
    let mut app = App::with_loader(engine, Theme::default(), None);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let (_, mut reference) = Engine::headless_for_test(48_000, 256);
    reference.apply(Command::LaunchScene { scene: 0 });
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

#[test]
fn protected_startup_keeps_durability_and_metadata_reports_but_refuses_pcm_preview_and_restore() {
    let files = Files::new();
    let (_, _, candidate) = existing(&files);
    let mut gui = Gui::new();
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.warnings.is_empty());
    assert!(gui.app.recovery.candidates.is_empty());
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.click("Refresh recovery list");
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.click("Refresh recovery list");
    assert!(gui
        .app
        .recovery
        .message
        .as_ref()
        .is_some_and(|m| m.contains("deferred")));
    assert!(gui.app.recovery.pending.is_none());
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    gui.app.request_recovery_restore(candidate);
    gui.frame(vec![]);
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert!(!gui.app.recovery_project_busy());
    let revision = gui.edit(0.52);
    gui.app.recovery.worker.as_ref().unwrap().force();
    gui.durable(revision);
    assert!(gui.app.engine.cmd.performance().status().protected);
}

#[test]
fn protect_unprotect_cancels_a_queued_pcm_preview_without_reviving_its_result() {
    let files = Files::new();
    let (_, _, _) = existing(&files);
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
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    resume.send(()).unwrap();
    gui.wait(|g| g.app.recovery.pending.is_none());
    assert!(gui.app.recovery.preview.is_none());
    assert!(gui.app.recovery.message.is_some());
}

#[test]
fn cancellation_after_explicit_delete_commit_keeps_the_real_outcome_visible() {
    let files = Files::new();
    let (saved, bytes, _) = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::AfterRemove);
    gui.click("Delete this recovery session…");
    gui.click("Delete recovery session");
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.click("Cancel recovery operation");
    resume.send(()).unwrap();
    gui.wait(|g| {
        g.app
            .recovery
            .message
            .as_ref()
            .is_some_and(|m| m.contains("committed before cancellation"))
    });
    assert_eq!(std::fs::read(saved).unwrap(), bytes);
    assert!(gui.app.recovery.candidates.is_empty());
}

#[test]
fn missing_provenance_is_visible_and_embedded_pcm_restores_but_missing_sidecar_keeps_current_document(
) {
    let files = Files::new();
    let root = files.path("recovery");
    let origin = files.path("disconnected-source.wav");
    let sample = Arc::new(crate::engine::dsp::Sample {
        name: "Verified embedded source".into(),
        sr: 44100,
        ch: 1,
        bpm: 100.0,
        path: origin.display().to_string(),
        data: vec![0.25; 128],
        peaks: Arc::new(vec![[0.25; 3]; 4]),
    });
    let mut first = Gui::new();
    assert!(first.app.submit(Command::DeckAudio {
        deck: 0,
        audio: sample.clone()
    }));
    first.rt.process(&mut []);
    let revision = first.app.engine.project.revision();
    first.start(&root);
    first.durable(revision);
    first.crash_worker();
    drop(first);
    let mut gui = Gui::new();
    gui.start(&root);
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    let candidate = gui.app.recovery.candidates[0].clone();
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    assert!(gui
        .app
        .recovery
        .preview
        .as_ref()
        .unwrap()
        .report
        .iter()
        .any(|line| line.contains("Original external media is unavailable")));
    let painted = gui.frame(vec![]);
    assert!(painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().contains("Original external media is unavailable"))));
    gui.click("Restore as untitled copy");
    gui.settled();
    assert_eq!(
        gui.rt.decks[0].audio.as_ref().unwrap().data.len(),
        sample.data.len()
    );
    assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().data, sample.data);
    assert!(!origin.exists());
    gui.open_panel();
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    let asset_dir = root.join(&candidate.session).join("assets");
    let asset = std::fs::read_dir(asset_dir)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "omat")
        })
        .unwrap()
        .path();
    std::fs::remove_file(asset).unwrap();
    gui.edit(0.73);
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    gui.click("Restore as untitled copy");
    gui.click("Discard changes");
    gui.settled();
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert_eq!(gui.rt.master, 0.73);
    assert_eq!(
        gui.rt.decks[0].audio.as_ref().unwrap().data.len(),
        sample.data.len()
    );
    assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().data, sample.data);
    assert!(gui
        .app
        .project_message_for_recovery_test()
        .is_some_and(|message| message.contains("Recovery refused; current session preserved")));
}

#[test]
fn save_as_updates_recovery_provenance_even_when_engine_and_view_are_unchanged() {
    let files = Files::new();
    let root = files.path("recovery");
    let mut gui = Gui::new();
    let revision = gui.app.engine.project.revision();
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
        .sequence;
    let path = files.path("named.omat");
    gui.save_as(&path);
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert!(!gui.app.project_dirty());
    gui.wait(|g| {
        g.app
            .recovery
            .worker
            .as_ref()
            .unwrap()
            .status()
            .durable
            .as_ref()
            .is_some_and(|durable| {
                durable.sequence > before && durable.view_revision == g.app.recovery.view_revision
            })
    });
    gui.crash_worker();
    let inventory = crate::recovery::discover(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        inventory.candidates[0].metadata.saved_path.as_ref(),
        Some(&path)
    );
    assert!(path.is_file());
}

#[test]
fn protection_invalidates_an_already_published_preview_and_requires_fresh_verification() {
    let files = Files::new();
    let _ = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.frame(vec![]);
    assert!(gui.app.recovery.preview.is_none());
    assert!(gui.app.recovery.candidates.is_empty());
    assert!(!gui.app.recovery.discovery_complete);
    assert!(gui
        .app
        .recovery
        .message
        .as_deref()
        .unwrap()
        .contains("expired"));
    gui.click("Refresh recovery list");
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    gui.click("Preview recovery");
    gui.wait(|g| g.app.recovery.preview.is_some());
    assert!(gui.app.recovery.discovery_complete);
}

#[test]
fn protection_after_deletion_commit_preserves_its_outcome_when_refresh_is_refused() {
    let files = Files::new();
    let (saved, bytes, _) = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::AfterRemove);
    gui.click("Delete this recovery session…");
    gui.click("Delete recovery session");
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    resume.send(()).unwrap();
    gui.wait(|g| g.app.recovery.pending.is_none());
    let message = gui.app.recovery.message.as_deref().unwrap();
    assert!(message.contains("Recovery session deleted"), "{message}");
    assert!(message.contains("discovery deferred"), "{message}");
    assert_eq!(std::fs::read(saved).unwrap(), bytes);
    assert!(gui.app.recovery.candidates.is_empty());
    assert!(!gui.app.recovery.discovery_complete);
}

#[test]
fn delayed_write_reports_captured_state_age_separately_from_later_commit_and_edits() {
    let files = Files::new();
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    let initial = gui.app.engine.project.revision();
    gui.durable(initial);
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::BeforeAppend);
    let captured_revision = gui.edit(0.31);
    gui.app.recovery.worker.as_ref().unwrap().force();
    let deadline = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
    }
    let before_write = worker::unix_ms();
    let newer_revision = gui.edit(0.64);
    assert!(newer_revision > captured_revision);
    std::thread::sleep(Duration::from_millis(25));
    let release_time = worker::unix_ms();
    resume.send(()).unwrap();
    gui.wait(|g| {
        g.app
            .recovery
            .worker
            .as_ref()
            .unwrap()
            .status()
            .durable
            .as_ref()
            .is_some_and(|d| d.revision == captured_revision)
    });
    let status = gui.app.recovery.worker.as_ref().unwrap().status();
    let durable = status.durable.as_ref().unwrap();
    assert!(durable.captured_unix_ms <= before_write);
    assert!(durable.committed_unix_ms >= release_time);
    assert!(durable.committed_unix_ms > durable.captured_unix_ms);
    assert_ne!(durable.revision, gui.app.engine.project.revision());
    gui.crash_worker();
    let inventory =
        crate::recovery::discover(&files.path("recovery"), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        inventory.candidates[0].metadata.captured_unix_ms,
        durable.captured_unix_ms
    );
    assert_eq!(inventory.candidates[0].metadata.revision, captured_revision);
}

#[test]
fn background_status_changes_do_not_retarget_native_recovery_actions() {
    let files = Files::new();
    let _ = existing(&files);
    let mut gui = Gui::new();
    gui.start(&files.path("recovery"));
    gui.wait(|g| !g.app.recovery.candidates.is_empty());
    gui.durable(gui.app.engine.project.revision());
    let preview = gui.node("Preview recovery");
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::BeforeAppend);
    gui.app.recovery.worker.as_ref().unwrap().force();
    let deadline = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        // Advance the actual capture boundary without redrawing the GUI. The
        // user's previously exposed native action must survive status changes.
        gui.rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(gui.app.recovery.worker.as_ref().unwrap().status().busy);
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target: preview,
        action: Action::Click,
        data: None,
    })]);
    assert!(
        gui.app.recovery.pending.is_some(),
        "Preview action was lost when the background status added a spinner"
    );
    resume.send(()).unwrap();
    gui.wait(|g| g.app.recovery.preview.is_some());

    let restore = gui.node("Restore as untitled copy");
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    let (entered, resume) = gui
        .app
        .recovery
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::BeforeAppend);
    gui.app.recovery.worker.as_ref().unwrap().force();
    let deadline = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target: restore,
        action: Action::Click,
        data: None,
    })]);
    assert!(
        gui.app.recovery_project_busy() || gui.app.engine.undo.checkpoint().epoch != epoch,
        "Restore action was lost when the background status changed"
    );
    resume.send(()).unwrap();
    gui.settled();
    assert_ne!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert_eq!(gui.rt.master, 0.37);
}
