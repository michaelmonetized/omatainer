mod engine;
mod theme;
mod ui;
mod ipc_server;

#[cfg(test)]
mod scene_index_tests;
#[cfg(test)]
mod ipc_control_tests;
#[cfg(test)]
mod ipc_error_tests;

use crate::engine::Command;
use crate::theme::socket_path;
use anyhow::Context;
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const STATUS_REQUEST: &str = r#"{"op":"status"}"#;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(|s| s.as_str()) == Some("ctl") {
        args.remove(0);
        return ctl(&args);
    }
    if single_instance_running() {
        focus_existing();
        return Ok(());
    }
    let engine = engine::Engine::start().context("audio engine")?;
    let _ipc = ipc_server::start(engine.cmd.clone(), engine.snap.clone())
        .context("could not start the local control service; check the reported socket path and permissions")?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1540.0, 960.0])
            .with_min_inner_size([860.0, 640.0])
            .with_title("omatainer")
            .with_app_id("org.omarchy.omatainer"),
        ..Default::default()
    };
    eframe::run_native(
        "omatainer",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc, engine)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn single_instance_running() -> bool {
    // Successful connection proves a listener exists even if it is slow. A
    // failed probe never authorizes deleting an endpoint owned by another run.
    ipc_server::has_listener(&socket_path())
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
    if op == "follow" {
        loop {
            match send_op(STATUS_REQUEST) {
                Ok(s) => {
                    println!("{s}");
                    let _ = std::io::stdout().flush();
                }
                Err(e) => {
                    println!(
                        "{}",
                        serde_json::json!({"ok":false,"error":e.to_string(),"playing":false})
                    );
                    let _ = std::io::stdout().flush();
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    let payload = match op {
        "play" => r#"{"op":"play"}"#,
        "stop" => r#"{"op":"stop"}"#,
        "togglePlay" | "toggle" | "toggle-play" => r#"{"op":"togglePlay"}"#,
        "record" => r#"{"op":"record"}"#,
        "tap" => r#"{"op":"tap"}"#,
        "status" => STATUS_REQUEST,
        "scene" => {
            let s = scene_payload(args)?;
            println!("{}", send_op(&s)?);
            return Ok(());
        }
        "deckA" => r#"{"op":"deckPlay","deck":0}"#,
        "deckB" => r#"{"op":"deckPlay","deck":1}"#,
        "cueA" => r#"{"op":"deckCue","deck":0}"#,
        "cueB" => r#"{"op":"deckCue","deck":1}"#,
        "reload-theme" => STATUS_REQUEST,
        other => anyhow::bail!("unknown ctl op {other}"),
    };
    println!("{}", send_op(payload)?);
    Ok(())
}

fn scene_payload(args: &[String]) -> anyhow::Result<String> {
    let usage = format!("usage: omatainer ctl scene <1-{}>", engine::SCENES);
    anyhow::ensure!(args.len() == 2, "{usage}");
    let n = args[1].parse::<usize>().with_context(|| usage.clone())?;
    anyhow::ensure!((1..=engine::SCENES).contains(&n), "{usage}");
    Ok(serde_json::json!({"op": "scene", "n": n - 1}).to_string())
}

fn ipc_scene_index(v: &serde_json::Value) -> anyhow::Result<u8> {
    let n = v.get("n").and_then(serde_json::Value::as_u64);
    match n {
        Some(n) if n < engine::SCENES as u64 => Ok(u8::try_from(n)?),
        _ => anyhow::bail!(
            "scene n must be a zero-based integer from 0 through {}",
            engine::SCENES - 1
        ),
    }
}

fn send_op(payload: &str) -> anyhow::Result<String> {
    let stream = UnixStream::connect(socket_path()).context("omatainer is not running")?;
    exchange_request(stream, payload)
}

fn exchange_request(mut stream: UnixStream, payload: &str) -> anyhow::Result<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
    let mut request: serde_json::Value = serde_json::from_str(payload)?;
    let object = request.as_object_mut().context("IPC request must be an object")?;
    let id = serde_json::Value::String(format!(
        "{}-{}", std::process::id(), NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
    ));
    object.insert("id".into(), id.clone());
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    writeln!(stream, "{request}")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    anyhow::ensure!(reader.read_line(&mut line)? > 0, "empty IPC response");
    let response: serde_json::Value = serde_json::from_str(&line)
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

fn ipc_deck_index(request: &serde_json::Value) -> anyhow::Result<u8> {
    match request.get("deck").and_then(serde_json::Value::as_u64) {
        Some(deck) if deck < engine::DECKS as u64 => Ok(deck as u8),
        _ => anyhow::bail!("deck must be an integer from 0 through {}", engine::DECKS - 1),
    }
}


fn ipc_command(v: &serde_json::Value) -> anyhow::Result<Option<Command>> {
    let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("");
    Ok(Some(match op {
        "play" => Command::Play,
        "stop" => Command::Stop,
        "togglePlay" => Command::TogglePlay,
        "record" => Command::Record,
        "tap" => Command::Tap(std::time::Instant::now()),
        "scene" => Command::LaunchScene { scene: ipc_scene_index(v)? },
        "deckPlay" => Command::DeckPlay {
            deck: ipc_deck_index(v)?,
        },
        "deckCue" => Command::DeckCue {
            deck: ipc_deck_index(v)?,
        },
        "ping" | "status" => return Ok(None),
        _ => anyhow::bail!("missing or unsupported IPC operation {:?}", op.chars().take(64).collect::<String>()),
    }))
}

fn handle_client(
    stream: UnixStream,
    commands: engine::CommandPort,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let mut request_id = serde_json::Value::Null;
        let submission: Result<Option<&str>, (&str, anyhow::Error)> = (|| {
            let request: serde_json::Value = serde_json::from_str(line.trim())
                .map_err(|error| ("invalid_json", anyhow::Error::from(error)))?;
            request_id = ipc_request_id(&request).map_err(|error| ("invalid_id", error))?;
            let command = ipc_command(&request).map_err(|error| ("invalid_operation", error))?;
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
                writeln!(writer, "{}", serde_json::json!({
                    "ok": false,
                    "id": request_id,
                    "error_code": code,
                    "accepted": false,
                    "command_status": "rejected",
                    "error": error.to_string(),
                }))?;
                line.clear();
                continue;
            }
        };
        let s = snap.lock().clone();
        let out = serde_json::json!({
            "ok": true,
            "id": request_id,
            // A queued command may not be reflected in this snapshot yet.
            "accepted": command_status.map(|_| true),
            "command_status": command_status,
            "event": "state",
            "playing": s.playing,
            "recording": s.recording,
            "bpm": s.bpm,
            "bar": s.bar,
            "beat": s.beat_in_bar,
            "xfader": s.xfader,
            "midi": s.midi,
            "commands": s.commands,
            "submissions": commands.stats(),
            "deckA": s.decks.first().map(|d| d.title.clone()).unwrap_or_default(),
            "deckB": s.decks.get(1).map(|d| d.title.clone()).unwrap_or_default(),
            "deckAPlaying": s.decks.first().map(|d| d.playing).unwrap_or(false),
            "deckBPlaying": s.decks.get(1).map(|d| d.playing).unwrap_or(false),
        });
        writeln!(writer, "{out}")?;
        line.clear();
    }
    Ok(())
}
