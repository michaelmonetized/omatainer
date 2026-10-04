use super::*;
use crate::theme::reload::test_support::{self as sources, Fixture as Sources};
use crate::ui::test_support::Fixture;
use crate::ui::theme_reload_tests::{frame, installed};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

fn request(path: &std::path::Path, id: u64) -> Receiver<serde_json::Value> {
    let path = path.to_owned();
    let (send, result) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        writeln!(stream, "{{\"op\":\"reload-theme\",\"id\":{id}}}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let _ = send.send(serde_json::from_str(&line).unwrap());
    });
    result
}
fn server(source: &Sources, fixture: &Fixture) -> (crate::ipc_server::IpcServer, PathBuf) {
    let path = source.root.join("private.sock");
    let server = crate::ipc_server::start_at(
        &path,
        fixture.app.engine.cmd.clone(),
        fixture.app.engine.snap.clone(),
    )
    .unwrap();
    (server, path)
}
fn settle(
    ctx: &egui::Context,
    fixture: &mut Fixture,
    time: &mut f64,
    result: &Receiver<serde_json::Value>,
) -> serde_json::Value {
    let mut received = None;
    sources::until(|| {
        frame(ctx, fixture, time);
        received = result.try_recv().ok();
        received.is_some()
    });
    received.unwrap()
}

#[test]
fn private_ipc_acknowledges_installed_font_and_shell_changes_after_the_gui_frame() {
    let source = Sources::new();
    let unchanged = std::fs::metadata(&source.theme.path)
        .unwrap()
        .modified()
        .unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.quiet_loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    let original = installed(&ctx);
    let (_server, path) = server(&source, &fixture);
    let admissions = fixture.app.engine.cmd.stats();
    source.shell("[font]\nbase-size = 19\n");
    source.select("Ubuntu", "ubuntu.ttf");
    let result = request(&path, 91);
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture
            .app
            .theme_request
            .as_ref()
            .is_some_and(|pending| pending.installed.is_some())
    });
    assert!(
        result.try_recv().is_err(),
        "setting fonts is not completed font installation"
    );
    frame(&ctx, &mut fixture, &mut time);
    let response = result.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["id"], 91);
    assert_eq!(response["ok"], true);
    assert_eq!(response["event"], "theme");
    assert_eq!(response["status"], "applied");
    assert!(
        response.get("accepted").is_none(),
        "not an audio queue receipt"
    );
    assert_eq!(response["theme"]["font"], "Ubuntu");
    assert_eq!(response["theme"]["font_size"], 19.0);
    assert_ne!(original.font, installed(&ctx).font);
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 19.0);
    assert_eq!(
        std::fs::metadata(&source.theme.path)
            .unwrap()
            .modified()
            .unwrap(),
        unchanged
    );
    assert_eq!(fixture.app.engine.cmd.stats(), admissions);
}

#[test]
fn invalid_forced_bundles_preserve_the_actual_style_and_retry_recovers() {
    let source = Sources::new();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.quiet_loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    let theme = fixture.app.theme.clone();
    let font = installed(&ctx);
    let (_server, path) = server(&source, &fixture);
    for bad in 0..3 {
        source.colors(if bad == 0 {
            "background = 'broken'\n"
        } else {
            "background = '#abcdef'\n"
        });
        source.shell(if bad == 1 {
            "[font]\nbase-size = 'broken'\n"
        } else {
            "[font]\nbase-size = 20\n"
        });
        source.select(
            "Ubuntu",
            if bad == 2 {
                "missing.ttf"
            } else {
                "ubuntu.ttf"
            },
        );
        let response = settle(&ctx, &mut fixture, &mut time, &request(&path, bad));
        assert_eq!(response["ok"], false);
        assert_eq!(response["error_code"], "invalid_theme");
        assert!(response["error"].as_str().unwrap().len() > 20);
        assert_eq!(fixture.app.theme, theme);
        assert!(Arc::ptr_eq(&font, &installed(&ctx)));
        assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 12.0);
    }
    source.select("Ubuntu", "ubuntu.ttf");
    let response = settle(&ctx, &mut fixture, &mut time, &request(&path, 4));
    assert_eq!(response["ok"], true);
    assert_eq!(fixture.app.theme.font, "Ubuntu");
    assert_eq!(fixture.app.theme.font_size, 20.0);
}

#[test]
fn reload_respects_the_latest_follow_theme_and_size_scale_profile() {
    let source = Sources::new();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.quiet_loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    let (_server, path) = server(&source, &fixture);
    source.select("Ubuntu", "ubuntu.ttf");
    let result = request(&path, 10);
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture
            .app
            .theme_request
            .as_ref()
            .is_some_and(|p| p.installed.is_some())
    });
    let active = fixture.app.settings.applied.active.clone();
    let appearance = &mut fixture
        .app
        .settings
        .applied
        .profiles
        .get_mut(&active)
        .unwrap()
        .appearance;
    appearance.follow_theme = false;
    appearance.font_size = Some(17.0);
    appearance.scale = 1.5;
    frame(&ctx, &mut fixture, &mut time);
    assert!(
        result.try_recv().is_err(),
        "new profile also needs its installation frame"
    );
    let response = settle(&ctx, &mut fixture, &mut time, &result);
    assert_eq!(response["theme"]["follow_theme"], false);
    assert_eq!(response["theme"]["font"], "bundled default");
    assert_eq!(response["theme"]["font_size"], 17.0);
    assert_eq!(response["theme"]["scale"], 1.5);
    assert_eq!(ctx.zoom_factor(), 1.5);
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 17.0);
    assert_eq!(
        fixture
            .app
            .settings
            .theme_update
            .as_ref()
            .unwrap()
            .theme
            .font,
        "Ubuntu"
    );
    ctx.fonts(|fonts| {
        assert!(!fonts
            .lock()
            .fonts
            .definitions()
            .font_data
            .contains_key("omatainer-selected"))
    });
    fixture
        .app
        .settings
        .applied
        .profiles
        .get_mut(&active)
        .unwrap()
        .appearance
        .follow_theme = true;
    let response = settle(&ctx, &mut fixture, &mut time, &request(&path, 11));
    assert_eq!(response["theme"]["follow_theme"], true);
    assert_eq!(response["theme"]["font"], "Ubuntu");
    assert_eq!(response["theme"]["font_size"], 17.0);
    assert_eq!(response["theme"]["scale"], 1.5);
    installed(&ctx);
}

#[test]
fn held_worker_keeps_gui_controls_live_and_server_shutdown_cancels_before_apply() {
    let source = Sources::new();
    let (loader, control) = sources::held(&source);
    control
        .started
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(loader);
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let (server, path) = server(&source, &fixture);
    // Keep this socket unread after shutdown; the server owns cancellation.
    let mut stream = UnixStream::connect(path).unwrap();
    writeln!(stream, "{{\"op\":\"reload-theme\",\"id\":1}}").unwrap();
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme_request.is_some()
    });
    let start = Instant::now();
    for i in 0..24 {
        fixture
            .app
            .engine
            .cmd
            .send(Command::Master(i as f32 / 24.0))
            .unwrap();
        frame(&ctx, &mut fixture, &mut time);
        assert_eq!(fixture.rt.master, i as f32 / 24.0);
    }
    drop(server);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(!source.root.join("private.sock").exists());
    control.release.send(()).unwrap();
    control
        .started
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    control.release.send(()).unwrap();
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme_request.is_none()
    });
    assert_eq!(
        fixture.app.theme.font, "bundled default",
        "cancelled request did not apply its late result"
    );
}

#[test]
fn unavailable_gui_and_strict_reload_schema_fail_without_audio_admission() {
    let (commands, _receiver) = crate::engine::CommandPort::channel(80);
    let snapshot = Arc::new(parking_lot::Mutex::new(crate::engine::Snapshot::default()));
    let (client, server) = UnixStream::pair().unwrap();
    let port = commands.clone();
    let handler = std::thread::spawn(move || crate::handle_client(server, port, snapshot));
    let mut client = BufReader::new(client);
    client
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    for (payload, error) in [
        (
            r#"{"op":"reload-theme","id":"bad","unexpected":1}"#,
            "invalid_operation",
        ),
        (
            r#"{"op":"reload-theme","id":"unavailable"}"#,
            "theme_unavailable",
        ),
    ] {
        writeln!(client.get_mut(), "{payload}").unwrap();
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["error_code"], error);
        assert_eq!(commands.len(), 0);
    }
    drop(client);
    handler.join().unwrap().unwrap();
}

#[test]
#[ignore = "requires OMATAINER_TEST_BINARY pointing to a freshly built native CLI"]
fn native_cli_reload_waits_past_normal_status_budget_and_reports_real_gui_failure() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command as Process, Stdio};
    let binary = std::env::var_os("OMATAINER_TEST_BINARY").expect("fresh native binary path");
    let source = Sources::new();
    std::fs::set_permissions(&source.root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (loader, control) = sources::held(&source);
    control
        .started
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    control.release.send(()).unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(loader);
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    let _server = crate::ipc_server::start_at(
        &source.root.join("omatainer.sock"),
        fixture.app.engine.cmd.clone(),
        fixture.app.engine.snap.clone(),
    )
    .unwrap();
    for invalid in [false, true] {
        source.shell(if invalid {
            "[font]\nbase-size = 'broken'\n"
        } else {
            "[font]\nbase-size = 18\n"
        });
        source.select("Ubuntu", "ubuntu.ttf");
        let mut child = Process::new(&binary)
            .args(["ctl", "reload-theme"])
            .env("XDG_RUNTIME_DIR", &source.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        sources::until(|| {
            frame(&ctx, &mut fixture, &mut time);
            control.started.try_recv().is_ok()
        });
        // A valid slow font resolution must outlive the normal 800 ms status budget.
        let started = Instant::now();
        while !invalid && started.elapsed() < Duration::from_millis(950) {
            frame(&ctx, &mut fixture, &mut time);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.try_wait().unwrap().is_none());
        control.release.send(()).unwrap();
        sources::until(|| {
            frame(&ctx, &mut fixture, &mut time);
            child.try_wait().unwrap().is_some()
        });
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            !invalid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if invalid {
            assert!(String::from_utf8_lossy(&output.stderr).contains("font.base-size"));
        } else {
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["event"], "theme");
            assert_eq!(response["status"], "applied");
            assert_eq!(response["theme"]["font"], "Ubuntu");
            assert_eq!(fixture.app.theme.font, "Ubuntu");
            assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 18.0);
            installed(&ctx);
        }
        assert_eq!(fixture.app.theme.font_size, 18.0);
    }
}

#[test]
fn private_ipc_deadline_cancels_unstarted_gui_work_and_reports_started_work_as_unknown() {
    let source = Sources::new();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.quiet_loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    let (_server, path) = server(&source, &fixture);
    source.select("Ubuntu", "ubuntu.ttf");
    let response = request(&path, 31)
        .recv_timeout(Duration::from_secs(4))
        .unwrap();
    assert_eq!(response["error_code"], "theme_timeout");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("before it began"));
    frame(&ctx, &mut fixture, &mut time);
    assert!(fixture.app.theme_request.is_none());
    assert_eq!(fixture.app.theme.font, "Hack");
    let result = request(&path, 32);
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture
            .app
            .theme_request
            .as_ref()
            .is_some_and(|p| p.installed.is_some())
    });
    let response = result.recv_timeout(Duration::from_secs(4)).unwrap();
    assert_eq!(response["error_code"], "theme_timeout");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("outcome is unknown"));
    frame(&ctx, &mut fixture, &mut time);
    assert_eq!(fixture.app.theme.font, "Ubuntu");
    installed(&ctx);
}

#[test]
fn performance_private_ipc_rejects_before_apply_and_preserves_already_installing_ack() {
    let source = Sources::new();
    let performance = crate::engine::performance::Handle::default();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.quiet_loader_with_performance(performance.clone()));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    let original = fixture.app.theme.clone();
    let font = installed(&ctx);
    let (_server, path) = server(&source, &fixture);
    performance.set_enabled(true).unwrap();
    source.shell("[font]\nbase-size = 19\n");
    source.select("Ubuntu", "ubuntu.ttf");
    let response = settle(&ctx, &mut fixture, &mut time, &request(&path, 201));
    assert_eq!(response["error_code"], "performance_protected");
    assert_eq!(response["ok"], false);
    assert_eq!(fixture.app.theme, original);
    assert!(Arc::ptr_eq(&font, &installed(&ctx)));
    performance.set_enabled(false).unwrap();
    let result = request(&path, 202);
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture
            .app
            .theme_request
            .as_ref()
            .is_some_and(|pending| pending.installed.is_some())
    });
    assert_eq!(
        performance.set_enabled(true),
        Err(crate::engine::performance::Error::Changing)
    );
    assert!(result.try_recv().is_err());
    frame(&ctx, &mut fixture, &mut time);
    let response = result.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["status"], "applied");
    assert_eq!(
        installed(&ctx).font,
        fixture
            .app
            .settings
            .theme_update
            .as_ref()
            .unwrap()
            .fonts
            .font_data["omatainer-selected"]
            .font
    );
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 19.0);
    performance.set_enabled(true).unwrap();
}

#[test]
fn performance_private_ipc_cancels_a_force_held_in_font_resolution_without_style_change() {
    let source = Sources::new();
    let performance = crate::engine::performance::Handle::default();
    let (loader, control) = sources::held_with_performance(&source, performance.clone());
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    control.release.send(()).unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(loader);
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    let original = fixture.app.theme.clone();
    let font = installed(&ctx);
    let (_server, path) = server(&source, &fixture);
    source.select("Ubuntu", "ubuntu.ttf");
    let result = request(&path, 203);
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        control.started.try_recv().is_ok()
    });
    performance.set_enabled(true).unwrap();
    control.release.send(()).unwrap();
    let response = settle(&ctx, &mut fixture, &mut time, &result);
    assert_eq!(response["error_code"], "performance_protected");
    assert_eq!(fixture.app.theme, original);
    assert!(Arc::ptr_eq(&font, &installed(&ctx)));
}
