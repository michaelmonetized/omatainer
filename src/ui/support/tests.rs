use super::*;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "omatainer-support-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        Self(root)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    closes: usize,
    session: model::worker::Session,
    _files: Files,
}
impl Gui {
    fn safe() -> Self {
        Self::new(Engine::start_safe().unwrap())
    }
    fn new(mut engine: Engine) -> Self {
        let files = Files::new();
        let root = files.0.join("support");
        let session = model::worker::Session::start(&root, engine.safe_mode()).unwrap();
        engine.cmd.attach_support(session.port.clone());
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.initialize_support(
            Some(session.client()),
            root,
            Arc::new(AtomicBool::new(false)),
        );
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut result = Self {
            app,
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            closes: 0,
            session,
            _files: files,
        };
        result.frame(vec![]);
        result.frame(vec![]);
        result
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1900.0, 1800.0))),
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.closes += output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, egui::ViewportCommand::Close))
            .count();
        output
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}; labels {:?}",
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
            action,
            target,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click);
    }
    fn text(&mut self, label: &str, text: &str) {
        self.action(label, Action::Focus);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Text(text.into()),
        ]);
        self.frame(vec![egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }]);
    }
    fn wait(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !condition(self) {
            self.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "support UI timeout: {}; project {:?}",
                self.app.support.message,
                self.app.project_message_for_recovery_test()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
    fn open_support(&mut self) {
        self.click("Project");
        self.click("Support and crash reports…");
    }
    fn edit(&mut self) {
        self.text("Search crate", "safe restart view");
        assert_eq!(self.app.lib_filter, "safe restart view");
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        self.app.stop_support_recovery_test();
        self.session
            .finish(model::Exit::Clean, Duration::from_secs(2));
    }
}

#[test]
fn actual_app_inspects_exports_reopens_and_rejects_invalid_files_without_touching_session() {
    let mut gui = Gui::safe();
    gui.open_support();
    gui.click("Inspect current report");
    gui.wait(|g| g.app.support.preview.is_some());
    assert!(!gui.app.support.consent);
    let destination = gui._files.0.join("reviewed.omasupport.json");
    gui.text("Support report file", destination.to_str().unwrap());
    assert_eq!(gui.app.support.path, destination.to_str().unwrap());
    gui.click("I reviewed this report and want to export it locally");
    gui.click("Export reviewed support report");
    gui.wait(|g| g.app.support.message.contains("exported and synced"));
    let exported = std::fs::read(&destination).unwrap();
    let report = model::storage::reopen(&destination, &AtomicBool::new(false)).unwrap();
    assert!(report.safe_mode);
    assert!(!String::from_utf8(exported.clone())
        .unwrap()
        .contains(gui._files.0.to_str().unwrap()));
    gui.click("Export reviewed support report");
    gui.wait(|g| !g.app.support.worker.as_ref().unwrap().busy());
    assert_eq!(std::fs::read(&destination).unwrap(), exported);
    assert!(gui.app.support.message.contains("publish report"));
    gui.click("Reopen support report");
    gui.wait(|g| g.app.support.message.contains("Review the exact"));
    assert_eq!(
        gui.app.support.preview.as_ref().unwrap().report.run,
        report.run
    );
    let corrupt = gui._files.0.join("corrupt.json");
    std::fs::write(&corrupt, b"{\"private path\":\"bad\"").unwrap();
    std::fs::set_permissions(&corrupt, std::fs::Permissions::from_mode(0o600)).unwrap();
    let revision = gui.app.engine.project.revision();
    gui.text("Support report file", corrupt.to_str().unwrap());
    gui.click("Reopen support report");
    gui.wait(|g| g.app.support.message.contains("malformed"));
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert_eq!(std::fs::read(&destination).unwrap(), exported);
}

#[test]
fn safe_restart_cancel_discard_and_save_use_real_unsaved_project_flow() {
    for choice in ["Cancel", "Discard changes", "Save changes"] {
        let mut gui = Gui::safe();
        gui.edit();
        let output = gui.frame(vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.text().contains("SAFE MODE"))));
        gui.click("Restart normally");
        gui.click(choice);
        if choice == "Cancel" {
            assert_eq!(gui.closes, 0);
            assert!(!gui.app.support.restart.load(Ordering::Acquire));
            assert!(gui.app.project.dialog_is_closed());
            assert_eq!(gui.app.lib_filter, "safe restart view");
            continue;
        }
        let save = gui._files.0.join("saved-before-restart.omat");
        if choice == "Save changes" {
            gui.text("Project file path", save.to_str().unwrap());
            gui.click("Save");
        }
        gui.wait(|g| g.closes > 0);
        assert!(gui.app.support.restart.load(Ordering::Acquire), "{choice}");
        if choice == "Save changes" {
            let bundle: crate::project_file::Bundle<project::Document> = crate::project_file::load(
                &save,
                &crate::project_file::Limits::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(bundle.state.view.library_filter, "safe restart view");
        } else {
            assert!(!save.exists());
        }
    }
}

#[test]
fn optional_preview_cancel_and_protection_do_not_change_the_project_or_publish_stale_data() {
    let mut gui = Gui::safe();
    gui.open_support();
    let revision = gui.app.engine.project.revision();
    gui.app.support.request(Job::Preview(
        gui.session.view().report.clone(),
        Selection::default(),
    ));
    gui.app.support.worker.as_ref().unwrap().cancel();
    gui.wait(|g| !g.app.support.worker.as_ref().unwrap().busy());
    assert!(gui.app.support.preview.is_none());
    assert!(gui.app.support.message.to_lowercase().contains("cancel"));
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.click("Inspect current report");
    assert!(gui.app.support.preview.is_none());
    assert_eq!(gui.app.engine.project.revision(), revision);
}

#[test]
fn actual_parser_failure_collects_only_a_code_and_not_the_rejected_payload() {
    use std::io::{BufRead, Write};
    let mut gui = Gui::safe();
    let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let commands = gui.app.engine.cmd.clone();
    let snapshot = gui.app.engine.snap.clone();
    let thread =
        std::thread::spawn(move || crate::handle_client(server, commands, snapshot).unwrap());
    client
        .write_all(b"{\"op\":\"private-api-key-and-media-path\"}\n")
        .unwrap();
    let mut reply = String::new();
    std::io::BufReader::new(client.try_clone().unwrap())
        .read_line(&mut reply)
        .unwrap();
    assert!(reply.contains("invalid_operation"));
    client.shutdown(std::net::Shutdown::Both).unwrap();
    thread.join().unwrap();
    gui.wait(|g| {
        g.session
            .view()
            .report
            .events
            .iter()
            .any(|e| e.code == model::Code::ParserRejected)
    });
    let report = gui.session.view().report.clone();
    let json = String::from_utf8(model::storage::encode(&report).unwrap()).unwrap();
    assert!(!json.contains("private-api-key-and-media-path"));
}

#[test]
fn actual_audio_owner_fault_is_collected_without_device_names_or_backend_error_text() {
    let (engine, controls) = crate::engine::audio::owner::tests::engine_fixture();
    let mut gui = Gui::new(engine);
    assert!(controls.renders.load(Ordering::Acquire) > 0);
    controls
        .active_fault
        .lock()
        .as_ref()
        .unwrap()
        .store(true, Ordering::Release);
    gui.wait(|g| {
        g.session
            .view()
            .report
            .events
            .iter()
            .any(|e| e.code == model::Code::AudioOutputOffline)
    });
    assert!(gui.app.engine.output_info().is_none());
    let json =
        String::from_utf8(model::storage::encode(&gui.session.view().report).unwrap()).unwrap();
    assert!(!json.contains("Fixture"));
    assert!(!json.contains("Injected backend"));
    assert!(gui
        .session
        .view()
        .report
        .samples
        .iter()
        .any(|sample| sample.audio.callbacks > 0));
    assert_eq!(
        gui.app.engine.cmd.send(Command::Play).unwrap_err(),
        crate::engine::SubmissionError::AudioUnavailable
    );
}

#[test]
fn actual_recovery_enospc_reports_typed_failure_and_links_only_a_confirmed_durable_record() {
    let mut gui = Gui::safe();
    let root = gui._files.0.join("recovery");
    let full = crate::recovery::testing::enospc(&root);
    gui.app.start_recovery(root.clone());
    gui.edit();
    gui.app.force_support_recovery_test();
    gui.wait(|g| {
        g.session.view().report.events.iter().any(|e| {
            e.code == model::Code::RecoveryWriteFailed
                && e.failure == Some(model::FailureClass::NoSpace)
        })
    });
    assert!(gui.session.view().report.recovery.is_empty());
    drop(full);
    gui.app.force_support_recovery_test();
    gui.wait(|g| !g.session.view().report.recovery.is_empty());
    let reference = gui.session.view().report.recovery.last().unwrap().clone();
    assert_eq!(
        gui.app.support_recovery_reference().unwrap().session,
        reference.session
    );
    gui.app.stop_support_recovery_test();
    let candidate = crate::recovery::lookup_exact(
        &root,
        reference.session,
        reference.epoch,
        reference.sequence,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(candidate.metadata.revision, reference.revision);
    assert_eq!(candidate.metadata.view_revision, reference.view_revision);
    let json =
        String::from_utf8(model::storage::encode(&gui.session.view().report).unwrap()).unwrap();
    assert!(!json.contains(root.to_str().unwrap()));
    assert!(!json.contains("safe restart view"));
}

#[test]
fn support_link_finds_exact_recovery_and_actual_preview_restores_stopped_untitled_copy() {
    let mut gui = Gui::safe();
    let root = gui._files.0.join("recovery");
    let captured = gui
        .app
        .engine
        .project
        .capture(&AtomicBool::new(false))
        .unwrap();
    let mut document = project::Document {
        engine: captured.state,
        view: gui.app.project_view(),
        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
    };
    document.engine.bpm = 137.0;
    document.view.library_filter = "recovered local view".into();
    let metadata = crate::recovery::RecordMeta {
        epoch: 44,
        revision: 91,
        view_revision: 12,
        saved_path: Some(gui._files.0.join("private-original.omat")),
        captured_unix_ms: 100,
    };
    let mut store = crate::recovery::Store::open(&root).unwrap();
    let committed = store
        .append(
            &crate::project_file::Bundle {
                state: document,
                media: captured.media,
            },
            metadata.clone(),
            &crate::recovery::Config::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
    let reference = model::RecoveryRef {
        session: crate::recovery::session_digest(store.session_id()),
        epoch: metadata.epoch,
        sequence: committed.sequence,
        revision: metadata.revision,
        view_revision: metadata.view_revision,
        captured_unix_ms: metadata.captured_unix_ms,
        committed_unix_ms: committed.committed_unix_ms,
    };
    drop(store);
    gui.app.start_recovery(root);
    gui.session.port.recovery(reference);
    gui.wait(|g| !g.session.view().report.recovery.is_empty());
    gui.open_support();
    gui.click("Inspect current report");
    gui.wait(|g| g.app.support.preview.is_some());
    gui.click("Find exact recovery record 1");
    gui.wait(|g| g.app.support.found.is_some());
    gui.click("Preview linked recovery");
    gui.wait(|g| {
        g.nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Restore as untitled copy"))
    });
    gui.click("Restore as untitled copy");
    gui.wait(|g| {
        g.app.lib_filter == "recovered local view"
            && g.app
                .project_message_for_recovery_test()
                .is_some_and(|m| m.contains("Recovered as an unsaved untitled copy"))
    });
    let actual = gui
        .app
        .engine
        .project
        .capture(&AtomicBool::new(false))
        .unwrap();
    assert_eq!(actual.state.bpm, 137.0);
    assert!(!gui.app.snap.playing);
    assert!(gui.app.recovery_project_path().is_none());
    assert!(!gui._files.0.join("private-original.omat").exists());
}
