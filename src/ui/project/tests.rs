use super::*;
use crate::engine::{MidiNote, RtEngine};
use crate::project_file::Limits;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

fn history_key(gui: &mut Gui, redo: bool) {
    // Closing a text/path dialog owns its closing frame's keys. Start the
    // shortcut only after the normal next-frame focus guard has settled.
    gui.frame(vec![]);
    gui.frame(vec![]);
    let modifiers = if redo {
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::CTRL
    };
    for pressed in [true, false] {
        gui.frame(vec![egui::Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed,
            repeat: false,
            modifiers,
        }]);
    }
}

#[test]
fn undo_returns_to_saved_content_but_navigation_and_redo_remain_dirty() {
    let files = Files::new();
    let mut gui = Gui::new();
    gui.edited();
    gui.save_as_ui(&files.path("history.omat"));
    let saved_master = gui.rt.master;
    let saved_revision = gui.app.engine.project.revision();
    gui.app.send(Command::Master(0.43));
    gui.rt.process(&mut []);
    assert!(gui.app.project_dirty());
    history_key(&mut gui, false);
    assert_eq!(gui.rt.master, saved_master);
    assert!(gui.app.engine.project.revision() > saved_revision);
    assert!(
        !gui.app.project_dirty(),
        "undo to saved content uses the captured checkpoint, not monotonic admission revision"
    );
    history_key(&mut gui, true);
    assert_eq!(gui.rt.master, 0.43);
    assert!(gui.app.project_dirty());
    gui.app.send(Command::SelectDeck(1));
    gui.rt.process(&mut []);
    history_key(&mut gui, false);
    assert_eq!(gui.rt.master, saved_master);
    assert!(
        gui.app.project_dirty(),
        "persistent nonhistory selection was not reverted by undo"
    );
}

#[test]
fn save_during_one_gesture_cannot_mark_later_grouped_values_clean() {
    let files = Files::new();
    let path = files.path("mid-gesture.omat");
    let mut gui = Gui::new();
    let id = gui.app.engine.undo.gesture();
    gui.app
        .engine
        .send(Command::Gesture {
            id,
            command: Box::new(Command::Master(0.23)),
        })
        .unwrap();
    gui.rt.process(&mut []);
    let captured_checkpoint = gui.app.engine.undo.checkpoint();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Captured);
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app
        .engine
        .send(Command::Gesture {
            id,
            command: Box::new(Command::Master(0.73)),
        })
        .unwrap();
    gui.rt.process(&mut []);
    assert_ne!(gui.app.engine.undo.checkpoint(), captured_checkpoint);
    assert_eq!(
        gui.app.engine.undo.view().cursor,
        1,
        "the drag remains one history entry"
    );
    resume.send(()).unwrap();
    gui.settle();
    assert!(gui.app.project_dirty());
    let saved =
        crate::project_file::load::<Document>(&path, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    assert_eq!(saved.state.engine.master, 0.23);
    assert_eq!(gui.rt.master, 0.73);
}

#[test]
fn undo_to_same_content_during_open_still_invalidates_prior_discard_authorization() {
    let files = Files::new();
    let path = files.path("reopen.omat");
    let mut gui = Gui::new();
    gui.save_as_ui(&path);
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Prepared);
    gui.app.begin_project_action(Action::Open(path));
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app.send(Command::Master(0.35));
    gui.rt.process(&mut []);
    gui.app.send(Command::Undo);
    gui.rt.process(&mut []);
    assert!(!gui.app.project_dirty());
    let epoch = gui.app.engine.undo.checkpoint().epoch;
    resume.send(()).unwrap();
    gui.settle();
    assert_eq!(gui.app.engine.undo.checkpoint().epoch, epoch);
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("current session changed"));
}

#[test]
fn new_and_open_reset_old_history_and_preserve_only_the_history_panel_view() {
    let files = Files::new();
    let path = files.path("history-view.omat");
    let mut gui = Gui::new();
    gui.edited();
    gui.app.undo_history.open = true;
    gui.save_as_ui(&path);
    let old_epoch = gui.app.engine.undo.checkpoint().epoch;
    gui.menu("New project");
    gui.settle();
    assert_ne!(gui.app.engine.undo.checkpoint().epoch, old_epoch);
    assert_eq!(gui.app.engine.undo.view().cursor, 0);
    assert!(gui.app.engine.undo.view().items.iter().all(Option::is_none));
    assert!(!gui.app.undo_history.open);
    gui.menu("Open project…");
    gui.enter_path(&path);
    gui.click_label("Open");
    gui.settle();
    assert!(gui.app.undo_history.open);
    assert!(gui.app.engine.undo.view().items.iter().all(Option::is_none));
    history_key(&mut gui, false);
    assert_eq!(
        gui.rt.tracks[0].clips[0].notes[0].pitch, 61,
        "a prior project's history cannot undo reopened content"
    );
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-project-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
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
    time: f64,
    close_commands: usize,
    render: bool,
}
impl Gui {
    fn new() -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, 256);
        Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx: egui::Context::default(),
            time: 0.0,
            close_commands: 0,
            render: true,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let result = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.close_commands += result.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|command| matches!(command, egui::ViewportCommand::Close))
            .count();
        if self.render {
            self.rt.process(&mut [0.0; 128]);
        }
        result
    }
    fn click(&mut self, pos: Pos2) -> egui::FullOutput {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ])
    }
    fn click_label(&mut self, label: &str) -> egui::FullOutput {
        self.frame(vec![]);
        let out = self.frame(vec![]);
        self.click(test_support::label_center(&out, label))
    }
    fn menu(&mut self, label: &str) {
        self.click_label("Project");
        self.click_label(label);
        self.frame(vec![]);
    }
    fn settle(&mut self) {
        let end = Instant::now() + Duration::from_secs(10);
        while self.app.project.busy() || self.app.project.awaiting_snapshot.is_some() {
            self.frame(vec![]);
            assert!(
                Instant::now() < end,
                "project did not settle: {:?}",
                self.app.project.message
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn edited(&mut self) {
        self.app.send(Command::SetNotes {
            track: 0,
            scene: 0,
            notes: vec![MidiNote {
                pitch: 61,
                start: 0.25,
                len: 0.75,
                vel: 97,
            }],
        });
        self.rt.process(&mut []);
        assert!(self.app.project_dirty());
    }
    fn enter_path(&mut self, path: &PathBuf) {
        let Some(Dialog::Path { text, .. }) = &self.app.project.dialog else {
            panic!("path dialog not open")
        };
        let old = text.clone();
        self.click_label(if old.is_empty() {
            "/path/to/session.omat"
        } else {
            &old
        });
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            },
            egui::Event::Text(path.display().to_string()),
        ]);
    }
    fn save_as_ui(&mut self, path: &PathBuf) {
        self.menu("Save project as…");
        self.enter_path(path);
        self.click_label("Save");
        self.settle();
        assert_eq!(
            self.app.project.current_path.as_ref(),
            Some(path),
            "{:?}",
            self.app.project.message
        );
    }
}

#[test]
fn real_project_menu_save_as_copy_new_and_open_round_trip_native_state_and_view() {
    let files = Files::new();
    let path = files.path("session.omat");
    let copy = files.path("copy.omat");
    let mut gui = Gui::new();
    gui.edited();
    gui.app.lib_filter = "Harmony".into();
    gui.app.keys_open = true;
    gui.app.diagnostics.open = true;
    gui.save_as_ui(&path);
    assert!(!gui.app.project_dirty(), "{:?}", gui.app.project.message);
    let bundle =
        crate::project_file::load::<Document>(&path, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    assert_eq!(bundle.state.engine.tracks[0].clips[0].notes[0].pitch, 61);
    assert_eq!(bundle.state.view.library_filter, "Harmony");
    assert!(bundle.state.view.keys_open);
    assert!(bundle.state.view.diagnostics_open);
    assert!(
        bundle
            .state
            .view
            .deck_identities
            .iter()
            .all(Option::is_some),
        "initial builtins retain verified identities"
    );

    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.31,
    });
    gui.rt.process(&mut []);
    gui.menu("Save project copy…");
    gui.enter_path(&copy);
    gui.click_label("Save");
    gui.settle();
    assert!(copy.exists());
    assert_eq!(gui.app.project.current_path.as_ref(), Some(&path));
    assert!(
        gui.app.project_dirty(),
        "copy did not mark current edits saved"
    );

    gui.menu("New project");
    gui.click_label("Cancel");
    assert_eq!(gui.rt.tracks[0].clips[0].notes[0].pitch, 61);
    gui.menu("New project");
    gui.click_label("Discard changes");
    gui.settle();
    assert!(gui
        .rt
        .tracks
        .iter()
        .all(|track| track.clips.iter().all(|clip| clip.notes.is_empty())));
    assert!(gui.rt.decks.iter().all(|deck| deck.audio.is_none()));
    assert!(gui.app.project.current_path.is_none());
    assert!(!gui.rt.playing);

    gui.menu("Open project…");
    gui.enter_path(&path);
    gui.click_label("Open");
    gui.settle();
    assert_eq!(gui.rt.tracks[0].clips[0].notes[0].pitch, 61);
    assert_ne!(
        gui.rt.tracks[0].gain, 0.31,
        "opened original, not Save Copy"
    );
    assert_eq!(gui.app.lib_filter, "Harmony");
    assert!(gui.app.keys_open);
    assert!(gui.app.diagnostics.open);
    assert!(!gui.rt.playing);
    assert!(gui.rt.decks.iter().all(|deck| !deck.playing));
    assert!(!gui.app.project_dirty());
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("stopped"));
}

#[test]
fn delayed_save_keeps_gui_alive_and_newer_edits_dirty_without_losing_captured_version() {
    let files = Files::new();
    let path = files.path("captured.omat");
    let mut gui = Gui::new();
    gui.edited();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Captured);
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    for _ in 0..20 {
        gui.frame(vec![]);
    }
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.27,
    });
    gui.rt.process(&mut []);
    gui.app.lib_filter = "Drums".into();
    resume.send(()).unwrap();
    gui.settle();
    assert!(gui.app.project_dirty());
    let bundle =
        crate::project_file::load::<Document>(&path, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    assert_ne!(bundle.state.engine.tracks[0].gain, 0.27);
    assert_ne!(bundle.state.view.library_filter, "Drums");
    assert_eq!(gui.rt.tracks[0].gain, 0.27);
}

#[test]
fn cancel_save_before_commit_preserves_existing_file_path_and_dirty_state() {
    let files = Files::new();
    let path = files.path("old.omat");
    std::fs::write(&path, b"previous good destination").unwrap();
    let mut gui = Gui::new();
    gui.edited();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Captured);
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), true);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.click_label("Cancel project operation");
    resume.send(()).unwrap();
    gui.settle();
    assert_eq!(std::fs::read(&path).unwrap(), b"previous good destination");
    assert!(gui.app.project.current_path.is_none());
    assert!(gui.app.project_dirty());
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("cancelled"));
}

#[test]
fn edits_during_open_preparation_and_new_media_intents_prevent_replacement() {
    let files = Files::new();
    let path = files.path("reference.omat");
    let mut gui = Gui::new();
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.settle();
    for media_intent in [false, true] {
        let (entered, resume) = gui
            .app
            .project
            .worker
            .as_ref()
            .unwrap()
            .pause_next(worker::Stage::Prepared);
        gui.app.begin_project_action(Action::Open(path.clone()));
        let end = Instant::now() + Duration::from_secs(5);
        while entered.try_recv().is_err() {
            gui.frame(vec![]);
            assert!(Instant::now() < end);
        }
        if media_intent {
            gui.app.load_file(0, files.path("pending.wav"), "new media");
        } else {
            gui.app.lib_filter = "edited during open".into();
        }
        let revision = gui.app.engine.project.revision();
        resume.send(()).unwrap();
        gui.settle();
        assert_eq!(gui.app.engine.project.revision(), revision);
        assert!(gui
            .app
            .project
            .message
            .as_ref()
            .unwrap()
            .contains("changed while preparing"));
    }
}

#[test]
fn malformed_unsupported_and_missing_projects_leave_engine_and_view_untouched() {
    let files = Files::new();
    let mut gui = Gui::new();
    gui.edited();
    gui.app.lib_filter = "Harmony".into();
    let valid = files.path("valid.omat");
    gui.app
        .begin_project_save(SaveKind::As, None, valid.clone(), false);
    gui.settle();
    let mut bundle =
        crate::project_file::load::<Document>(&valid, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    bundle.state.mapping_schema += 1;
    let unsupported = files.path("unsupported.omat");
    crate::project_file::save(
        &unsupported,
        &bundle,
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let bad = files.path("corrupt.omat");
    std::fs::write(&bad, b"not a project").unwrap();
    for path in [files.path("missing.omat"), unsupported, bad] {
        let revision = gui.app.engine.project.revision();
        gui.app.begin_project_action(Action::Open(path));
        gui.settle();
        assert_eq!(gui.app.engine.project.revision(), revision);
        assert_eq!(gui.app.lib_filter, "Harmony");
        assert_eq!(gui.app.project.current_path.as_ref(), Some(&valid));
        assert!(gui
            .app
            .project
            .message
            .as_ref()
            .unwrap()
            .starts_with("Project operation failed:"));
        gui.frame(vec![]);
        let output = gui.frame(vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().starts_with("Project operation failed:"))));
    }
}

#[test]
fn unsaved_save_continuation_rechecks_new_edits_and_close_cancel_preserves_session() {
    let files = Files::new();
    let path = files.path("session.omat");
    let mut gui = Gui::new();
    gui.edited();
    gui.save_as_ui(&path);
    gui.edited();
    gui.app.request_project_action(Action::New);
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Captured);
    gui.click_label("Save changes");
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.22,
    });
    gui.rt.process(&mut []);
    resume.send(()).unwrap();
    gui.settle();
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::New))
    ));
    assert_eq!(gui.rt.tracks[0].gain, 0.22);
    gui.click_label("Cancel");
    gui.app.request_project_action(Action::Close);
    gui.click_label("Cancel");
    assert!(!gui.app.project.allow_close);
    assert_eq!(gui.rt.tracks[0].gain, 0.22);
}

#[test]
fn recent_projects_persist_off_thread_and_the_real_recent_menu_reopens_them() {
    let files = Files::new();
    let recent = files.path("recent.json");
    let path = files.path("recent-session.omat");
    let mut first = Gui::new();
    first.app.project.worker = Some(
        Worker::start(
            first.app.engine.project.clone(),
            48_000,
            Some(recent.clone()),
        )
        .unwrap(),
    );
    first.edited();
    first
        .app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    first.settle();
    assert_eq!(
        serde_json::from_slice::<Vec<PathBuf>>(&std::fs::read(&recent).unwrap()).unwrap(),
        vec![path.clone()]
    );
    let mut reopened = Gui::new();
    reopened.app.project.worker =
        Some(Worker::start(reopened.app.engine.project.clone(), 48_000, Some(recent)).unwrap());
    let end = Instant::now() + Duration::from_secs(5);
    while reopened.app.project.recent.is_empty() {
        reopened.frame(vec![]);
        assert!(Instant::now() < end);
    }
    reopened.menu("Open recent");
    reopened.click_label(&path.display().to_string());
    reopened.settle();
    assert_eq!(reopened.app.project.current_path.as_ref(), Some(&path));
    assert_eq!(reopened.rt.tracks[0].clips[0].notes[0].pitch, 61);
}

#[test]
fn auxiliary_recent_failure_is_visible_without_reclassifying_a_committed_save() {
    let files = Files::new();
    let obstruction = files.path("not-a-directory");
    std::fs::create_dir(&obstruction).unwrap();
    let path = files.path("saved.omat");
    let mut gui = Gui::new();
    gui.app.project.worker = Some(
        Worker::start(
            gui.app.engine.project.clone(),
            48_000,
            Some(obstruction.join("recent.json")),
        )
        .unwrap(),
    );
    assert!(matches!(
        gui.app
            .project
            .worker
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        Event::Recent(_, None)
    ));
    // The startup read succeeded; only the later auxiliary publication fails.
    std::fs::remove_dir(&obstruction).unwrap();
    std::fs::write(&obstruction, b"keep").unwrap();
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.settle();
    assert!(path.exists());
    assert_eq!(gui.app.project.current_path.as_ref(), Some(&path));
    assert!(!gui.app.project_dirty());
    assert!(gui
        .app
        .project
        .recent_warning
        .as_ref()
        .unwrap()
        .contains("could not be saved"));
    assert_eq!(std::fs::read(obstruction).unwrap(), b"keep");
}

#[test]
fn close_event_requires_a_decision_and_clean_close_checks_already_queued_commands() {
    let mut gui = Gui::new();
    gui.edited();
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
        ..Default::default()
    };
    raw.viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = gui.ctx.run(raw, |ctx| gui.app.update_frame(ctx));
    let root = &output.viewport_output[&egui::ViewportId::ROOT];
    assert!(root
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::Close))
    ));
    gui.click_label("Cancel");
    assert!(!gui.app.project.allow_close);
    gui.app.request_project_action(Action::Close);
    gui.click_label("Discard changes");
    gui.settle();
    assert!(gui.close_commands > 0);
    assert!(gui.app.project.allow_close);

    let mut gui = Gui::new();
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.17,
    });
    // The command has been accepted, but the audio engine has not applied it.
    assert!(!gui.app.project_dirty());
    gui.app.request_project_action(Action::Close);
    gui.settle();
    assert!(!gui.app.project.allow_close);
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::Close))
    ));
    assert_eq!(gui.rt.tracks[0].gain, 0.17);
}

#[test]
fn worker_disconnect_releases_busy_ui_and_preserves_session_and_destination() {
    let files = Files::new();
    let path = files.path("not-committed.omat");
    let mut gui = Gui::new();
    gui.edited();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::Captured);
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app
        .project
        .worker
        .as_mut()
        .unwrap()
        .disconnect_results();
    gui.frame(vec![]);
    assert!(!gui.app.project.busy());
    assert!(gui.app.project.worker.is_none());
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("disconnected"));
    resume.send(()).unwrap();
    assert!(!path.exists());
    assert_eq!(gui.rt.tracks[0].clips[0].notes[0].pitch, 61);
    gui.app.request_project_action(Action::Close);
    gui.click_label("Discard changes");
    assert!(
        gui.app.project.allow_close,
        "failed project worker cannot trap explicit discard"
    );
    assert!(gui.close_commands > 0);
}

#[test]
fn invalid_view_save_preserves_the_previous_project_and_never_claims_clean() {
    let files = Files::new();
    let path = files.path("valid.omat");
    let mut gui = Gui::new();
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.settle();
    let original = std::fs::read(&path).unwrap();
    gui.app.lib_filter = "x".repeat(4097);
    gui.app
        .begin_project_save(SaveKind::Save, None, path.clone(), true);
    gui.settle();
    assert!(gui.app.project_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("Invalid project view"));
}

#[test]
fn new_edits_while_open_path_dialog_is_visible_need_a_fresh_unsaved_decision() {
    let files = Files::new();
    let path = files.path("does-not-need-to-exist.omat");
    let mut gui = Gui::new();
    gui.edited();
    gui.menu("Open project…");
    gui.click_label("Discard changes");
    gui.enter_path(&path);
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.14,
    });
    gui.rt.process(&mut []);
    gui.click_label("Open");
    assert!(!gui.app.project.busy());
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::Open(_)))
    ));
    assert_eq!(gui.rt.tracks[0].gain, 0.14);
    gui.click_label("Cancel");
    assert!(gui.app.project.dialog.is_none());
}

#[test]
fn recent_cache_fifo_and_symlink_fail_promptly_without_blocking_project_io_or_replacing_occupants()
{
    use std::os::unix::ffi::OsStrExt;
    let files = Files::new();
    let fifo = files.path("recent-fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let target = files.path("unrelated.json");
    std::fs::write(&target, b"keep unrelated").unwrap();
    let link = files.path("recent-symlink");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    for (index, cache) in [fifo.clone(), link.clone()].into_iter().enumerate() {
        let mut gui = Gui::new();
        gui.app.project.worker =
            Some(Worker::start(gui.app.engine.project.clone(), 48_000, Some(cache)).unwrap());
        let path = files.path(&format!("still-saves-{index}.omat"));
        gui.app
            .begin_project_save(SaveKind::As, None, path.clone(), false);
        gui.settle();
        assert_eq!(gui.app.project.current_path.as_ref(), Some(&path));
        assert!(gui
            .app
            .project
            .recent_warning
            .as_ref()
            .unwrap()
            .contains(if index == 0 {
                "not a regular file"
            } else {
                "could not be read"
            }));
    }
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep unrelated");
    use std::os::unix::fs::FileTypeExt;
    assert!(std::fs::metadata(fifo).unwrap().file_type().is_fifo());
}

#[test]
fn capture_never_assigns_an_old_watch_identity_to_replaced_deck_media() {
    let files = Files::new();
    let path = files.path("replacement.omat");
    let mut gui = Gui::new();
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.app.load_source(
        0,
        Some(&Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "New harmony".into(),
        }),
    );
    // Capture executes after this admitted load, but the worker's earlier GUI
    // watch list contains only the old default receipt for deck A.
    gui.settle();
    let bundle =
        crate::project_file::load::<Document>(&path, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    assert_eq!(
        bundle.state.engine.decks[0].audio,
        bundle.state.engine.builtin[1]
    );
    assert!(bundle.state.view.deck_identities[0].is_none());
    assert_eq!(
        bundle.state.view.deck_identities[1]
            .as_ref()
            .unwrap()
            .source,
        LibSource::Builtin(BuiltinStem::Harmony)
    );
}

#[test]
fn close_seal_rejects_an_edit_admitted_after_capture_and_cancel_wins_before_terminal_close() {
    let mut gui = Gui::new();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::CloseCaptured);
    gui.app.request_project_action(Action::Close);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.29,
    });
    resume.send(()).unwrap();
    gui.settle();
    assert!(!gui.app.project.allow_close);
    assert_eq!(gui.close_commands, 0);
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::Close))
    ));
    assert_eq!(gui.rt.tracks[0].gain, 0.29);
    gui.click_label("Cancel");

    let mut gui = Gui::new();
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::CloseCaptured);
    gui.app.request_project_action(Action::Close);
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.click_label("Cancel project operation");
    resume.send(()).unwrap();
    gui.settle();
    assert!(!gui.app.project.allow_close);
    assert!(gui.app.project.close_guard.is_none());
    assert!(gui
        .app
        .engine
        .send(Command::TrackGain {
            track: 0,
            value: 0.49
        })
        .is_ok());
}

#[test]
fn project_display_revision_blocks_stale_load_target_until_the_applied_snapshot_arrives() {
    let files = Files::new();
    let path = files.path("deck-b.omat");
    let mut gui = Gui::new();
    gui.app.send(Command::SelectDeck(1));
    gui.rt.process(&mut []);
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.settle();
    gui.app.send(Command::SelectDeck(0));
    gui.rt.process(&mut []);
    gui.rt.publish_for_test();
    let old = gui.app.engine.snapshot();
    assert_eq!(old.selected_deck, 0);
    gui.app.begin_project_action(Action::Open(path));
    let end = Instant::now() + Duration::from_secs(5);
    while gui.app.project.awaiting_snapshot.is_none() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    gui.rt.publish_for_test();
    let installed = gui.app.engine.snapshot();
    assert_eq!(installed.selected_deck, 1);
    assert!(installed.project_revision > old.project_revision);
    // Replay the earlier real publication to deterministically model snapshot
    // lag. No new audio frames reach the next periodic publication boundary.
    *gui.app.engine.snap.lock() = old;
    let before = gui.rt.decks[0].audio.clone().unwrap();
    let press = || egui::Event::Key {
        key: Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    let output = gui.frame(vec![press()]);
    assert!(gui.app.project.awaiting_snapshot.is_some());
    assert!(Arc::ptr_eq(
        &before,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert!(
        gui.app.loads.iter().all(Option::is_none),
        "F was gated during stale display"
    );
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Waiting for project display…")));
    *gui.app.engine.snap.lock() = installed;
    gui.frame(vec![egui::Event::Key {
        key: Key::F,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Default::default(),
    }]);
    gui.frame(vec![]);
    assert!(gui.app.project.awaiting_snapshot.is_none());
    assert_eq!(gui.app.load_target(), 1);
    gui.frame(vec![press()]);
    assert!(gui.app.loads[0].is_none());
    assert!(
        gui.app.loads[1].is_some(),
        "F uses the saved deck B target after publication"
    );
    assert!(Arc::ptr_eq(
        gui.rt.decks[1].audio.as_ref().unwrap(),
        gui.rt.builtin[0].as_ref().unwrap()
    ));
}

#[test]
fn stalled_but_owned_renderer_never_closes_clean_and_explicit_discard_can_exit() {
    let mut gui = Gui::new();
    gui.app
        .engine
        .project
        .set_wait_limit_for_test(Duration::from_millis(25));
    gui.render = false; // RtEngine/CommandReceiver remain owned, but no callback runs.
    assert!(gui.app.engine.cmd.is_connected());
    gui.app.request_project_action(Action::Close);
    gui.settle();
    assert_eq!(gui.close_commands, 0);
    assert!(!gui.app.project.allow_close);
    assert!(matches!(
        gui.app.project.dialog,
        Some(Dialog::Unsaved(Action::Close))
    ));
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("Could not verify a clean close"));
    gui.click_label("Cancel");
    assert!(!gui.app.project.allow_close);
    gui.app.request_project_action(Action::Close);
    gui.settle(); // Previous cancelled task still awaits its non-running renderer.
    gui.click_label("Discard changes");
    gui.settle();
    assert!(gui.app.engine.cmd.is_connected());
    assert!(gui.app.project.allow_close);
    assert!(gui.close_commands > 0);

    let mut gui = Gui::new();
    gui.edited();
    gui.app
        .engine
        .project
        .set_wait_limit_for_test(Duration::from_millis(25));
    gui.render = false;
    gui.app.request_project_action(Action::Close);
    gui.click_label("Discard changes");
    gui.settle(); // Direct discard seal timeout also permits exit, with no clean claim.
    assert!(gui.app.project.allow_close);
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("Closing without saving"));
}

#[test]
fn accepted_controller_load_between_gui_ready_and_audio_install_preserves_old_session() {
    let files = Files::new();
    let path = files.path("new-session.omat");
    let mut gui = Gui::new();
    gui.app
        .begin_project_save(SaveKind::As, None, path.clone(), false);
    gui.settle();
    gui.app.send(Command::TrackGain {
        track: 0,
        value: 0.37,
    });
    gui.rt.process(&mut []);
    let (entered, resume) = gui
        .app
        .project
        .worker
        .as_ref()
        .unwrap()
        .pause_next(worker::Stage::BeforeInstall);
    // Explicitly authorized replacement, held after GUI ready/revision checks.
    gui.app.begin_project_action(Action::Open(path));
    let end = Instant::now() + Duration::from_secs(5);
    while entered.try_recv().is_err() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
    }
    assert!(gui.app.project.committing());
    gui.app
        .engine
        .ui_requests
        .publish_selection(Some(Arc::new(Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "Captured harmony".into(),
        })));
    gui.app
        .engine
        .send(Command::DeckLoadSelected { deck: 0 })
        .unwrap();
    resume.send(()).unwrap();
    gui.settle();
    assert!(gui
        .app
        .project
        .message
        .as_ref()
        .unwrap()
        .contains("Project operation failed"));
    assert_eq!(gui.rt.tracks[0].gain, 0.37, "old graph preserved");
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.app.loads[0].as_ref().is_some_and(|load| load
        .selection
        .as_ref()
        .is_some_and(|selection| selection.title == "Captured harmony")));
    assert!(Arc::ptr_eq(
        gui.rt.decks[0].audio.as_ref().unwrap(),
        gui.rt.builtin[1].as_ref().unwrap()
    ));
}

#[test]
fn unreadable_regular_recent_cache_is_preserved_while_current_projects_and_memory_recent_work() {
    for original in [
        b"[truncated".as_slice(),
        br#"{"version":999,"recent":["/old/project.omat"]}"#.as_slice(),
    ] {
        let files = Files::new();
        let cache = files.path("recent.json");
        let path = files.path("saved.omat");
        std::fs::write(&cache, original).unwrap();
        let mut gui = Gui::new();
        gui.app.project.worker = Some(
            Worker::start(gui.app.engine.project.clone(), 48_000, Some(cache.clone())).unwrap(),
        );
        gui.app
            .begin_project_save(SaveKind::As, None, path.clone(), false);
        gui.settle();
        assert_eq!(std::fs::read(&cache).unwrap(), original);
        assert_eq!(gui.app.project.current_path.as_ref(), Some(&path));
        assert_eq!(gui.app.project.recent.first(), Some(&path));
        assert!(gui
            .app
            .project
            .recent_warning
            .as_ref()
            .unwrap()
            .contains("history is invalid"));
        assert!(!gui.app.project_dirty());
        crate::project_file::load::<Document>(&path, &Limits::default(), &AtomicBool::new(false))
            .unwrap();
    }
}
