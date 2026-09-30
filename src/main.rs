mod engine;
mod theme;
mod ui;

use crate::engine::Command;
use crate::theme::socket_path;
use anyhow::Context;
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

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
    start_ipc(engine.rt.clone(), engine.snap.clone());

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
    let _ = std::fs::remove_file(socket_path());
    Ok(())
}

fn single_instance_running() -> bool {
    let p = socket_path();
    if !p.exists() {
        return false;
    }
    UnixStream::connect(&p)
        .and_then(|mut s| {
            s.set_read_timeout(Some(Duration::from_millis(200)))?;
            writeln!(s, r#"{{"op":"ping"}}"#)?;
            let mut r = BufReader::new(s);
            let mut line = String::new();
            r.read_line(&mut line)?;
            Ok(line.contains("ok"))
        })
        .unwrap_or_else(|_| {
            let _ = std::fs::remove_file(&p);
            false
        })
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
            match send_op("status") {
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
        "status" => r#"{"op":"status"}"#,
        "scene" => {
            let n = args.get(1).and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
            let s = format!(r#"{{"op":"scene","n":{}}}"#, n.saturating_sub(1));
            println!("{}", send_op(&s)?);
            return Ok(());
        }
        "deckA" => r#"{"op":"deckPlay","deck":0}"#,
        "deckB" => r#"{"op":"deckPlay","deck":1}"#,
        "cueA" => r#"{"op":"deckCue","deck":0}"#,
        "cueB" => r#"{"op":"deckCue","deck":1}"#,
        "reload-theme" => r#"{"op":"status"}"#,
        other => anyhow::bail!("unknown ctl op {other}"),
    };
    println!("{}", send_op(payload)?);
    Ok(())
}

fn send_op(payload: &str) -> anyhow::Result<String> {
    let mut s = UnixStream::connect(socket_path()).context("omatainer is not running")?;
    s.set_read_timeout(Some(Duration::from_millis(800)))?;
    writeln!(s, "{payload}")?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn start_ipc(
    rt: std::sync::Arc<parking_lot::Mutex<engine::RtEngine>>,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
) {
    std::thread::Builder::new()
        .name("omatainer-ipc".into())
        .spawn(move || {
            let path = socket_path();
            let _ = std::fs::remove_file(&path);
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("omatainer ipc: {e}");
                    return;
                }
            };
            for stream in listener.incoming().flatten() {
                let rt = rt.clone();
                let snap = snap.clone();
                std::thread::spawn(move || {
                    let _ = handle_client(stream, rt, snap);
                });
            }
        })
        .ok();
}

fn apply_now(rt: &std::sync::Arc<parking_lot::Mutex<engine::RtEngine>>, c: Command) {
    let mut e = rt.lock();
    e.apply(c);
    e.publish();
}

fn handle_client(
    stream: UnixStream,
    rt: std::sync::Arc<parking_lot::Mutex<engine::RtEngine>>,
    snap: std::sync::Arc<parking_lot::Mutex<engine::Snapshot>>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap_or(serde_json::json!({}));
        let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("");
        match op {
            "play" => apply_now(&rt, Command::Play),
            "stop" => apply_now(&rt, Command::Stop),
            "togglePlay" => apply_now(&rt, Command::TogglePlay),
            "record" => apply_now(&rt, Command::Record),
            "tap" => apply_now(&rt, Command::Tap(std::time::Instant::now())),
            "scene" => {
                let n = v.get("n").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
                apply_now(&rt, Command::LaunchScene { scene: n });
            }
            "deckPlay" => {
                let d = v.get("deck").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
                apply_now(&rt, Command::DeckPlay { deck: d });
            }
            "deckCue" => {
                let d = v.get("deck").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
                apply_now(&rt, Command::DeckCue { deck: d });
            }
            "ping" | "status" => {}
            _ => {}
        }
        let s = snap.lock().clone();
        let out = serde_json::json!({
            "ok": true,
            "event": "state",
            "playing": s.playing,
            "recording": s.recording,
            "bpm": s.bpm,
            "bar": s.bar,
            "beat": s.beat_in_bar,
            "xfader": s.xfader,
            "midi": s.midi,
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
