mod engine;
mod theme;
mod ui;
mod ipc_server;
mod ipc_transport;
mod instance;
mod runtime;

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
#[cfg(test)]
use std::io::BufRead;
use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const STATUS_REQUEST: &str = r#"{"op":"status"}"#;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(|s| s.as_str()) == Some("ctl") {
        args.remove(0);
        return ctl(&args);
    }
    let socket = socket_path()?;
    // Ownership is established before opening audio/MIDI or constructing a
    // window. Keep it until the later IPC guard and GUI have both shut down.
    let _instance = match instance::acquire(&socket)
        .with_context(|| format!("single-instance ownership at {}", socket.display()))?
    {
        instance::Acquisition::Owner(guard) => guard,
        instance::Acquisition::Existing => {
            focus_existing();
            return Ok(());
        }
    };
    let engine = engine::Engine::start().context("audio engine")?;
    let _ipc = ipc_server::start_at(&socket, engine.cmd.clone(), engine.snap.clone())
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
    let stream = UnixStream::connect(socket_path()?).context("omatainer is not running")?;
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
    let payload = format!("{request}\n");
    anyhow::ensure!(payload.len() - 1 <= ipc_transport::REQUEST_BYTES, "IPC request exceeds byte limit");
    ipc_transport::write_all(&mut stream, payload.as_bytes(), Duration::from_millis(800))?;
    let mut reader = BufReader::with_capacity(ipc_transport::RESPONSE_BYTES, stream);
    let mut line = [0; ipc_transport::RESPONSE_BYTES];
    let size = ipc_transport::read_line(&mut reader, &mut line,
        Duration::from_millis(800), Duration::from_millis(800))?
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
    let mut reader = BufReader::with_capacity(ipc_transport::REQUEST_BYTES, stream.try_clone()?);
    let mut writer = stream;
    let mut line = [0; ipc_transport::REQUEST_BYTES];
    for _ in 0..limits.requests {
        let size = match ipc_transport::read_line(&mut reader, &mut line, limits.idle, limits.read) {
            Ok(Some(size)) => size,
            Ok(None) => return Ok(()),
            Err(error) => {
                let _ = ipc_transport::reject(&mut writer, serde_json::Value::Null,
                    error.code(), &error.to_string(), limits.write);
                return Ok(());
            }
        };
        let mut request_id = serde_json::Value::Null;
        let submission: Result<Option<&str>, (&str, anyhow::Error)> = (|| {
            let request: serde_json::Value = serde_json::from_slice(&line[..size])
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
                ipc_transport::reject(&mut writer, request_id, code, &error.to_string(), limits.write)?;
                continue;
            }
        };
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
            "midi": s.midi.iter().take(8).map(|name| ipc_transport::short_text(name, 64)).collect::<Vec<_>>(),
            "state_truncated": s.midi.len() > 8 || s.midi.iter().take(8).any(|name| name.len() > 64)
                || s.decks.iter().take(2).any(|deck| deck.title.len() > 256),
            "commands": s.commands,
            "submissions": commands.stats(),
            "deckA": s.decks.first().map(|d| ipc_transport::text(&d.title)).unwrap_or_default(),
            "deckB": s.decks.get(1).map(|d| ipc_transport::text(&d.title)).unwrap_or_default(),
            "deckAPlaying": s.decks.first().map(|d| d.playing).unwrap_or(false),
            "deckBPlaying": s.decks.get(1).map(|d| d.playing).unwrap_or(false),
        });
        drop(s);
        ipc_transport::reply(&mut writer, &out, limits.write)?;
    }
    Ok(())
}
