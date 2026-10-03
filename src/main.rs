mod project_file;
mod project_template;
mod project_dependencies;
mod portable_project;
mod recovery;
mod support;
mod startup;
mod licenses;
mod engine;
mod library;
mod music_provider;
mod video;
mod project_versions;
mod media_location;
mod media_tags;
mod midi_file;
mod midi_file_io;
mod performance_history;
mod sampler_bank;
mod track_analysis;
mod preferences;
mod theme;
mod ui;
mod ipc_server;
mod automation;
mod ipc_transport;
mod ipc_request;
mod ipc_schema;
mod ipc_follow;
mod instance;
mod runtime;

#[cfg(test)]
mod scene_index_tests;
#[cfg(test)]
mod shell_scene_tests;
#[cfg(test)]
mod ipc_control_tests;
#[cfg(test)]
mod ipc_error_tests;
#[cfg(test)]
mod ipc_schema_tests;

use crate::engine::Command;
use crate::theme::socket_path;
use anyhow::Context;
use eframe::egui;
#[cfg(test)]
use std::io::{BufRead, Write};
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const STATUS_REQUEST: &str = r#"{"op":"status"}"#;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(|s| s.as_str()) == Some("benchmark-build-info") {
        anyhow::ensure!(args.len() == 1, "usage: omatainer benchmark-build-info");
        println!("{}", serde_json::json!({
            "schema": 1, "debug_assertions": cfg!(debug_assertions),
            "pkg_version": env!("CARGO_PKG_VERSION"),
            "arch": std::env::consts::ARCH, "os": std::env::consts::OS,
        }));
        return Ok(());
    }
    if args.first().map(|s| s.as_str()) == Some("licenses") {
        std::hint::black_box(licenses::PROTOCOL);
        let text = match args.get(1).map(String::as_str) {
            Some("--manifest") if args.len() == 2 => licenses::MANIFEST,
            Some("--notices") if args.len() == 2 => licenses::NOTICES,
            _ => anyhow::bail!("usage: omatainer licenses --manifest | --notices"),
        };
        print!("{text}");
        return Ok(());
    }
    if args.first().map(|s| s.as_str()) == Some("ctl") {
        args.remove(0);
        return ctl(&args);
    }
    let launch = startup::Launch::parse(&args)?;
    let paths = startup::Paths::environment()?;
    let socket = socket_path()?;
    // Ownership is established before opening audio/MIDI or constructing a
    // window. Keep it until the later IPC guard and GUI have both shut down.
    let _instance = match instance::acquire(&socket)
        .with_context(|| format!("single-instance ownership at {}", socket.display()))?
    {
        instance::Acquisition::Owner(guard) => guard,
        instance::Acquisition::Existing => {
            anyhow::ensure!(!launch.safe_mode, "Omatainer already owns this runtime. Safe mode did not focus, control or replace the existing session; close it explicitly first.");
            focus_existing();
            return Ok(());
        }
    };
    let support = support::worker::Session::start(&paths.support,launch.safe_mode).ok();
    if let Some(support)=&support {support.install_panic_hook();}
    let mut startup = preferences::worker::Startup::read(paths.preferences, paths.home);
    let defaults_once = launch.defaults_once;
    if startup.blocked && !defaults_once && !launch.safe_mode {
        if let Some(support)=&support {
            support.port.event(crate::support::Code::PreferencesReadFailed,Some(crate::support::FailureClass::Invalid));
            support.finish(crate::support::Exit::StartupFailed,Duration::from_secs(2));
        }
        let retry = preferences::recovery::show(startup.diagnostic.as_deref().unwrap_or("Preferences are unavailable"), true)?;
        drop(_instance);
        return preferences::recovery::restart(retry, launch);
    }
    let mut profile = startup.preferences.current().expect("validated startup profile").clone();
    if defaults_once {
        profile.audio = preferences::Audio::default();
        let notice = "System-default audio explicitly selected for this launch. Saved preferences are unchanged.";
        startup.diagnostic = Some(startup.diagnostic.map_or(notice.into(), |error| format!("{error}\n{notice}")));
    }
    let initial = if launch.safe_mode { None } else {
        let choice = if launch.empty_once { &project_template::Startup::Empty } else { &profile.startup.session };
        match ui::startup_session(choice, &std::sync::atomic::AtomicBool::new(false)) {
            Ok(session) => session,
            Err(error) => {
                if let Some(support) = &support { support.finish(crate::support::Exit::StartupFailed, Duration::from_secs(2)); }
                let retry = preferences::recovery::show_template(&format!("Startup template could not be loaded: {error}. Saved settings and template are unchanged."))?;
                drop(_instance); return preferences::recovery::restart(retry, launch);
            }
        }
    };
    let (initial_engine, initial_view) = match initial {
        Some(session) => (Some(session.initial), Some((session.view, session.metadata))),
        None => (None, None),
    };
    let running_audio = profile.audio.clone();
    let mut engine = match if launch.safe_mode {engine::Engine::start_safe()} else {engine::Engine::start_with_session(&profile, initial_engine)} {
        Ok(engine) => engine,
        Err(error) => {
            if let Some(support)=&support {
                support.port.event(crate::support::Code::EngineStartupFailed,Some(crate::support::FailureClass::Unavailable));
                support.finish(crate::support::Exit::StartupFailed,Duration::from_secs(2));
            }
            if launch.startup_check {anyhow::bail!("safe startup project service could not be initialized");}
            let retry = preferences::recovery::show(&format!("Could not open the requested setup: {error:#}. Saved preferences were not changed."), false)?;
            drop(_instance);
            return preferences::recovery::restart(retry, launch);
        }
    };
    if let Some(support)=&support {engine.cmd.attach_support(support.port.clone());}
    if launch.startup_check {
        let result = startup::check_safe_engine(&engine,!startup.blocked);
        drop(engine);
        let exit=if result.is_ok(){crate::support::Exit::Clean}else{crate::support::Exit::StartupFailed};
        let marker_clean = support.as_ref().is_some_and(|session|session.finish(exit,Duration::from_secs(2)));
        let mut report = result?;
        report["support_marker_clean"] = marker_clean.into();
        println!("{}",serde_json::to_string(&report)?);
        return Ok(());
    }
    let _ipc = match ipc_server::start_at(&socket, engine.cmd.clone(), engine.snap.clone()) {
        Ok(ipc)=>ipc, Err(error)=>{
            if let Some(support)=&support {
                support.port.event(crate::support::Code::IpcStartupFailed,Some(crate::support::FailureClass::Unavailable));
                support.finish(crate::support::Exit::StartupFailed,Duration::from_secs(2));
            }
            return Err(error).context("could not start the local control service; check the reported socket path and permissions");
        }
    };

    let restart = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let support_client = support.as_ref().map(|session|session.client());
    let support_root = paths.support.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1540.0, 960.0])
            .with_min_inner_size([860.0, 640.0])
            .with_title("omatainer")
            .with_app_id("org.omarchy.omatainer"),
        ..Default::default()
    };
    let gui_result=eframe::run_native(
        "omatainer",
        options,
        Box::new(|cc| {
            let mut app = ui::App::new(cc, engine);
            app.automation_endpoint(socket.clone());
            app.initialize_preferences(&cc.egui_ctx, startup, running_audio);
            if let Some((view, metadata)) = initial_view { app.initialize_startup_session(&cc.egui_ctx, view, metadata); }
            app.initialize_support(support_client,support_root,restart.clone());
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"));
    drop(_ipc);
    if let Some(support)=&support {
        let exit=if gui_result.is_ok(){crate::support::Exit::Clean}else{support.port.event(crate::support::Code::GuiStartupFailed,Some(crate::support::FailureClass::Unavailable));crate::support::Exit::StartupFailed};
        support.finish(exit,Duration::from_secs(2));
    }
    drop(_instance);
    gui_result?;
    if restart.load(std::sync::atomic::Ordering::Acquire) {
        use std::os::unix::process::CommandExt;
        let error=std::process::Command::new(std::env::current_exe()?).exec();
        return Err(error.into());
    }
    Ok(())
}

fn focus_existing() {
    let _ = std::process::Command::new("hyprctl")
        .args([
            "dispatch",
            "focuswindow",
            "class:org.omarchy.omatainer",
        ])
        .status();
}

fn ctl(args: &[String]) -> anyhow::Result<()> {
    let op = args.first().map(|s| s.as_str()).unwrap_or("status");
    if op == "api" {
        let payload=automation_payload(args)?;
        if payload["request"]["op"]=="subscribe" {
            return automation::follow(&socket_path()?,&payload,&mut std::io::stdout().lock());
        }
        println!("{}",send_op(&payload.to_string())?);return Ok(());
    }
    if op == "follow" {
        return ipc_follow::run(&socket_path()?, &mut std::io::stdout().lock());
    }

    if op == "reload-theme" {
        let request = ipc_request::encode(&serde_json::json!({"op": "reload-theme"}))?;
        let stream = instance::connect(&socket_path()?).context("omatainer is not running")?;
        println!("{}", exchange_encoded_with_budget(stream, request, Duration::from_secs(4))?);
        return Ok(());
    }
    if op == "status" {
        let request = ipc_request::encode(&ipc_request::ReadOperation::Status)?;
        let stream = UnixStream::connect(socket_path()?).context("omatainer is not running")?;
        println!("{}", exchange_encoded(stream, request)?);
        return Ok(());
    }
    let payload = match op {
        "play" => r#"{"op":"play"}"#,
        "stop" => r#"{"op":"stop"}"#,
        "togglePlay" | "toggle" | "toggle-play" => r#"{"op":"togglePlay"}"#,
        "record" => r#"{"op":"record"}"#,
        "tap" => r#"{"op":"tap"}"#,
        "scene" => {
            let s = scene_payload(args)?;
            println!("{}", send_op(&s)?);
            return Ok(());
        }
        "deckA" => r#"{"op":"deckPlay","deck":0}"#,
        "deckB" => r#"{"op":"deckPlay","deck":1}"#,
        "cueA" => r#"{"op":"deckCue","deck":0}"#,
        "cueB" => r#"{"op":"deckCue","deck":1}"#,
        other => anyhow::bail!("unknown ctl op {other}"),
    };
    println!("{}", send_op(payload)?);
    Ok(())
}

/// Build a typed version-1 CLI request.
/// Takes ctl arguments; returns the envelope after validating the JSON request and argument count.
fn automation_payload(args: &[String]) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(args.len() == 2, "usage: omatainer ctl api '<request JSON>'");
    anyhow::ensure!(
        args[1].len() <= ipc_transport::REQUEST_BYTES,
        "API request exceeds byte limit"
    );
    let request: serde_json::Value = serde_json::from_str(&args[1])?;
    let _: automation::Request = serde_json::from_value(request.clone())?;
    Ok(serde_json::json!({"op":"api","version":1,"request":request}))
}

fn scene_payload(args: &[String]) -> anyhow::Result<String> {
    let usage = format!("usage: omatainer ctl scene <1-{}>", engine::session::MAX_SCENES);
    anyhow::ensure!(args.len() == 2, "{usage}");
    let n = args[1].parse::<usize>().with_context(|| usage.clone())?;
    anyhow::ensure!((1..=engine::session::MAX_SCENES).contains(&n), "{usage}");
    Ok(serde_json::json!({"op": "scene", "n": n - 1}).to_string())
}


fn send_op(payload: &str) -> anyhow::Result<String> {
    let stream = UnixStream::connect(socket_path()?).context("omatainer is not running")?;
    exchange_request(stream, payload)
}

fn exchange_request(stream: UnixStream, payload: &str) -> anyhow::Result<String> {
    let value: serde_json::Value = serde_json::from_str(payload)?;
    exchange_encoded(stream, ipc_request::encode(&value)?)
}

fn exchange_encoded(stream: UnixStream, request: ipc_request::Encoded) -> anyhow::Result<String> {
    exchange_encoded_with_budget(stream, request, Duration::from_millis(800))
}

fn exchange_encoded_with_budget(mut stream: UnixStream, request: ipc_request::Encoded, read_budget: Duration) -> anyhow::Result<String> {
    let ipc_request::Encoded { id, line: payload } = request;
    ipc_transport::write_all(&mut stream, payload.as_bytes(), Duration::from_millis(800))?;
    let mut reader = BufReader::with_capacity(ipc_transport::RESPONSE_BYTES, stream);
    let mut line = [0; ipc_transport::RESPONSE_BYTES];
    let size = ipc_transport::read_line(&mut reader, &mut line,
        read_budget, read_budget)?
        .context("empty IPC response")?;
    let line = std::str::from_utf8(&line[..size]).context("malformed IPC response")?;
    let response: serde_json::Value = serde_json::from_str(line)
        .context("malformed IPC response")?;
    anyhow::ensure!(response.get("id") == Some(&id), "IPC response request id mismatch");
    match response.get("ok").and_then(serde_json::Value::as_bool) {
        Some(true) => Ok(line.trim().to_string()),
        Some(false) => anyhow::bail!("IPC request rejected: {}",
            response.get("error").and_then(serde_json::Value::as_str)
                .unwrap_or("server returned ok:false")),
        None => anyhow::bail!("IPC response is missing a boolean ok field"),
    }
}

fn ipc_request_id(request: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    use serde_json::Value;
    match request.get("id") {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(id)) if id.len() <= 128 => Ok(Value::String(id.clone())),
        Some(Value::Number(id)) if id.is_u64() || id.is_i64() => Ok(Value::Number(id.clone())),
        _ => anyhow::bail!("request id must be an integer or a string of at most 128 bytes"),
    }
}

#[cfg(test)]
fn ipc_command(v: &serde_json::Value) -> anyhow::Result<Option<Command>> {
    Ok(ipc_schema::Operation::parse(v)?.command())
}

#[cfg(test)]
fn handle_client(
    stream: UnixStream,
    commands: engine::CommandPort,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
) -> anyhow::Result<()> {
    handle_client_with_limits(stream, commands, snap, ipc_transport::Limits::default())
}

fn handle_client_with_limits(
    stream: UnixStream,
    commands: engine::CommandPort,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
    limits: ipc_transport::Limits,
) -> anyhow::Result<()> {
    handle_client_with_stop(stream, commands, snap, limits, None)
}

fn handle_client_with_stop(
    stream: UnixStream,
    commands: engine::CommandPort,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
    limits: ipc_transport::Limits,
    stopped: Option<&std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::with_capacity(ipc_transport::REQUEST_BYTES, stream.try_clone()?);
    let mut writer = stream;
    let mut line = [0; ipc_transport::REQUEST_BYTES];
    for _ in 0..limits.requests {
        let size = match ipc_transport::read_line(&mut reader, &mut line, limits.idle, limits.read) {
            Ok(Some(size)) => size,
            Ok(None) => return Ok(()),
            Err(error) => {
                commands.support_event(support::Code::ParserRejected,Some(support::FailureClass::Invalid));
                let _ = ipc_transport::reject(&mut writer, serde_json::Value::Null,
                    error.code(), &error.to_string(), limits.write);
                return Ok(());
            }
        };
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&line[..size]);
        if let Ok(value) = &parsed {
            if value.get("op").and_then(serde_json::Value::as_str) == Some("api") {
                if automation::serve(&mut writer,value,&commands,&snap,limits,stopped)? { return Ok(()); }
                continue;
            }
        }
        let mut request_id = serde_json::Value::Null;
        let mut follow = false;
        let mut reload_theme = false;
        let submission: Result<Option<&str>, (&str, anyhow::Error)> = (|| {
            let request = parsed.map_err(|error| ("invalid_json", anyhow::Error::from(error)))?;
            request_id = ipc_request_id(&request).map_err(|error| ("invalid_id", error))?;
            let operation = ipc_schema::Operation::parse(&request)
                .map_err(|error| ("invalid_operation", error))?;
            operation.validate_session(&commands).map_err(|error| ("invalid_operation", error))?;
            follow = matches!(operation, ipc_schema::Operation::Follow {});
            reload_theme = matches!(operation, ipc_schema::Operation::ReloadTheme {});
            let command = operation.command();
            match command {
                Some(command) => commands.send(command)
                    .map(|outcome| Some(outcome.name()))
                    .map_err(|error| ("submission_rejected", anyhow::Error::from(error))),
                None => Ok(None),
            }
        })();
        let command_status = match submission {
            Ok(status) => status,
            Err((code, error)) => {
                if matches!(code,"invalid_json"|"invalid_operation") {commands.support_event(support::Code::ParserRejected,Some(support::FailureClass::Invalid));}
                ipc_transport::reject(&mut writer, request_id, code, &error.to_string(), limits.write)?;
                continue;
            }
        };
        if reload_theme {
            let outcome = commands.theme_requests().request(theme::requests::DEADLINE)
                .and_then(|request| request.wait(|| stopped.is_some_and(|stop|
                    stop.load(std::sync::atomic::Ordering::Acquire))));
            match outcome {
                Ok(applied) => ipc_transport::reply(&mut writer, &serde_json::json!({
                    "ok": true, "id": request_id, "event": "theme", "status": "applied",
                    "theme": applied,
                }), limits.write)?,
                Err(error) => ipc_transport::reject(&mut writer, request_id,
                    error.code, &error.message, limits.write)?,
            }
            continue;
        }
        if follow {
            // This is a read-only stream, not an uncapped command connection.
            drop(reader);
            return ipc_follow::serve(writer, commands, snap, request_id, limits);
        }
        let Some(s) = snap.try_lock_for(limits.snapshot) else {
            if let Some(status) = command_status {
                ipc_transport::reply(&mut writer, &serde_json::json!({
                    "ok": true, "id": request_id, "accepted": true,
                    "command_status": status, "event": "receipt", "state_available": false,
                    "warning": "command accepted; snapshot temporarily unavailable",
                }), limits.write)?;
            } else {
                ipc_transport::reject(&mut writer, request_id, "snapshot_unavailable",
                    "snapshot temporarily unavailable", limits.write)?;
            }
            continue;
        };
        let out = serde_json::json!({
            "ok": true,
            "id": request_id,
            // A queued command may not be reflected in this snapshot yet.
            "accepted": command_status.map(|_| true),
            "command_status": command_status,
            "event": "state",
            "state_available": true,
            "playing": s.playing,
            "recording": s.recording,
            "bpm": s.bpm,
            "bar": s.bar,
            "beat": s.beat_in_bar,
            "xfader": s.xfader,
            "midi": s.midi.iter().take(8).map(|name| ipc_transport::short_text(name, ipc_transport::STATUS_MIDI_NAME_BYTES)).collect::<Vec<_>>(),
            "state_truncated": s.midi.len() > 8 || s.midi.iter().take(8).any(|name| name.len() > ipc_transport::STATUS_MIDI_NAME_BYTES)
                || s.decks.iter().take(2).any(|deck| deck.title.len() > ipc_transport::STATUS_DECK_TITLE_BYTES),
            "midi_clock": s.midi_clock,
            "midi_routing": commands.midi_routing().summary(),
            "audio": commands.audio_metrics(),
            "master_fx": { "types": s.fx_kind, "wet": s.fx_wet },
            "commands": s.commands,
            "submissions": commands.stats(),
            "performance": commands.performance().status(),
            "deckA": s.decks.first().map(|d| ipc_transport::short_text(&d.title,ipc_transport::STATUS_DECK_TITLE_BYTES)).unwrap_or_default(),
            "deckB": s.decks.get(1).map(|d| ipc_transport::short_text(&d.title,ipc_transport::STATUS_DECK_TITLE_BYTES)).unwrap_or_default(),
            "deckAPlaying": s.decks.first().map(|d| d.playing).unwrap_or(false),
            "deckBPlaying": s.decks.get(1).map(|d| d.playing).unwrap_or(false),
        });
        drop(s);
        ipc_transport::reply(&mut writer, &out, limits.write)?;
    }
    Ok(())
}
