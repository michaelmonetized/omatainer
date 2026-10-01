//! Shared regression checks, also run as an isolated panic=abort executable by
//! scripts/check-scene-inputs.py. No desktop, audio device, or live socket is used.

use super::*;
use engine::{ClipKind, MidiNote, RtEngine, Snapshot, SCENES};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;
use std::io::BufRead;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    RtEngine::new(48000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn scene_state(rt: &RtEngine) -> Value {
    json!({
        "playing": rt.playing,
        "recording": rt.recording,
        "beat": rt.beat,
        "selected": [rt.selected_track, rt.selected_scene],
        "compose_target": rt.compose_target,
        "fx_view": rt.fx_view,
        "tracks": rt.tracks.iter().map(|track| json!({
            "clips": track.clips,
            "playing": track.playing.map(|clip| json!({
                "scene": clip.scene,
                "start": clip.start_beat,
                "last": clip.last_beat,
                "looping": clip.looping,
            })),
            "voices": track.poly.voices.iter().map(|voice|
                (voice.note(), voice.env.stage)
            ).collect::<Vec<_>>(),
            "arp_note": track.arp_note,
            "drum_pos": track.drum_pos,
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn check_cli_scene_arguments() {
    for n in 1..=SCENES {
        let payload = scene_payload(&["scene".into(), n.to_string()]).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap(),
            json!({"op": "scene", "n": n - 1})
        );
    }
    let mut invalid = vec![
        vec!["scene".into()],
        vec!["scene".into(), "1".into(), "2".into()],
    ];
    for n in [
        "0",
        "9",
        "255",
        "256",
        "257",
        "-1",
        "-255",
        "",
        "nonnumeric",
        "1.0",
        "1e0",
        "18446744073709551616",
    ] {
        invalid.push(vec!["scene".into(), n.into()]);
    }
    for args in invalid {
        let error = scene_payload(&args).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("usage: omatainer ctl scene <1-8>"),
            "{args:?}: {error}"
        );
        // Exercise the actual CLI dispatch too: it must reject before opening
        // the user's socket, even if a running app happens to be available.
        assert_eq!(ctl(&args).unwrap_err().to_string(), error.to_string());
    }
}

pub(super) fn check_engine_rejects_invalid_scenes() {
    let mut rt = engine();
    for playing in [false, true] {
        if playing {
            rt.apply(Command::LaunchScene { scene: 0 });
            rt.recording = true;
        }
        let before = scene_state(&rt);
        for scene in SCENES as u8..=u8::MAX {
            let commands = [
                Command::LaunchScene { scene },
                Command::ToggleScene { scene },
                Command::RestartScene { scene },
                Command::AddScene { scene },
                Command::LaunchClip { track: 0, scene },
                Command::FireClip {
                    track: 0,
                    scene,
                    looping: false,
                },
                Command::SetNotes {
                    track: 0,
                    scene,
                    notes: vec![],
                },
                Command::Select {
                    track: 0,
                    scene: scene as usize,
                },
                Command::ComposeArm { track: 0, scene: scene as usize },
                Command::OpenFxScene(scene),
            ];
            for command in commands {
                let label = format!("{command:?}, playing={playing}");
                rt.apply(command);
                assert_eq!(scene_state(&rt), before, "{label}");
            }
        }
        for scene in [256, 257, usize::MAX] {
            rt.apply(Command::Select { track: 0, scene });
            assert_eq!(scene_state(&rt), before, "Select scene {scene}");
        }
        rt.apply(Command::Select {
            track: usize::MAX,
            scene: 0,
        });
        assert_eq!(scene_state(&rt), before, "invalid selection track");
    }
}

pub(super) fn check_valid_scene_operations() {
    let mut rt = engine();
    rt.quant = 0.0;
    for scene in 0..SCENES as u8 {
        for track in &mut rt.tracks {
            track.clips[scene as usize].kind = ClipKind::Midi;
        }
        rt.apply(Command::SetNotes {
            track: 0,
            scene,
            notes: vec![MidiNote {
                pitch: 60,
                start: 0.0,
                len: 1.0,
                vel: 100,
            }],
        });
        assert_eq!(rt.tracks[0].clips[scene as usize].notes[0].pitch, 60);
        rt.apply(Command::Select {
            track: 0,
            scene: scene as usize,
        });
        assert_eq!(rt.selected_scene, scene as usize);
        assert_eq!(rt.compose_target, None);
        rt.apply(Command::ComposeArm { track: 0, scene: scene as usize });
        assert_eq!(rt.compose_target, Some(engine::ComposeTarget { track: 0, scene: scene as usize }));
        rt.apply(Command::OpenFxScene(scene));
        assert_eq!(rt.fx_view, 100 + scene as i16);

        rt.apply(Command::LaunchScene { scene });
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().scene == scene));
        rt.apply(Command::ToggleScene { scene });
        assert!(rt.tracks.iter().all(|track| track.playing.is_none()));
        rt.apply(Command::ToggleScene { scene });
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().scene == scene));
        rt.beat += 2.0;
        rt.apply(Command::RestartScene { scene });
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().start_beat == rt.beat));

        rt.apply(Command::Stop);
        rt.apply(Command::AddScene { scene });
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().scene == scene));
        rt.apply(Command::FireClip {
            track: 0,
            scene,
            looping: false,
        });
        assert_eq!(rt.tracks[0].playing.unwrap().scene, scene);
        assert!(!rt.tracks[0].playing.unwrap().looping);
        rt.apply(Command::LaunchClip { track: 0, scene });
        assert_eq!(rt.tracks[0].playing.unwrap().scene, scene);
        assert!(rt.tracks[0].playing.unwrap().looping);
    }
}

pub(super) fn check_ipc_scene_requests() {
    let (tx, rx) = engine::CommandPort::channel(16);
    let commands = tx.clone();
    let snap = Arc::new(Mutex::new(Snapshot::default()));
    let mut rt = RtEngine::new(48000.0, rx, snap.clone());
    rt.apply(Command::LaunchScene { scene: 0 });
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let server_thread = std::thread::spawn(move || handle_client_with_limits(server, commands, snap, ipc_transport::Limits::default()));
    let mut client = BufReader::new(client);
    let before = scene_state(&rt);
    let mut invalid = vec![json!({"op": "scene"})];
    for n in [
        json!(8),
        json!(9),
        json!(255),
        json!(256),
        json!(257),
        json!(-1),
        json!(-255),
        json!(u64::MAX),
        json!(null),
        json!(true),
        json!(false),
        json!(0.0),
        json!(7.0),
        json!("0"),
        json!("nonnumeric"),
        json!([0]),
        json!({}),
    ] {
        invalid.push(json!({"op": "scene", "n": n}));
    }
    for request in invalid {
        writeln!(client.get_mut(), "{request}").unwrap();
        let mut response = String::new();
        assert!(client.read_line(&mut response).unwrap() > 0);
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["ok"], false, "{request}: {response}");
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("zero-based integer"));
        assert_eq!(scene_state(&rt), before, "{request}");
        assert_eq!(tx.len(), 0, "invalid scene was enqueued: {request}");
    }
    for scene in 0..SCENES {
        // Ensure both documented endpoints and every intervening scene still
        // work on the same connection after invalid messages.
        for track in &mut rt.tracks {
            track.clips[scene].kind = ClipKind::Midi;
        }
        let request = scene_payload(&["scene".into(), (scene + 1).to_string()]).unwrap();
        writeln!(client.get_mut(), "{request}").unwrap();
        let mut response = String::new();
        assert!(client.read_line(&mut response).unwrap() > 0);
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["accepted"], true);
        assert_eq!(response["command_status"], "accepted");
        assert_eq!(tx.len(), 1, "acknowledgment must mean queued, not executed");
        // Only the renderer applies the accepted command, at its next block.
        rt.process(&mut []);
        assert_eq!(tx.len(), 0);
        assert!(rt
            .tracks
            .iter()
            .all(|track| track.playing.unwrap().scene as usize == scene));
    }
    drop(client);
    server_thread.join().unwrap().unwrap();
}

#[test]
fn cli_scene_arguments() {
    check_cli_scene_arguments();
}

#[test]
fn engine_rejects_invalid_scenes() {
    check_engine_rejects_invalid_scenes();
}

#[test]
fn valid_scene_operations() {
    check_valid_scene_operations();
}

#[test]
fn ipc_scene_requests() {
    check_ipc_scene_requests();
}
