use super::*;
use crate::engine::dsp::Sample;
use crate::engine::{Command, Snapshot};
use crate::engine::{CommandPort, SubmissionError, SubmissionOutcome};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn fixture() -> (OutputCallback, CommandPort) {
    let (tx, rx) = CommandPort::channel(256);
    let snap = Arc::new(Mutex::new(Snapshot::default()));
    let mut rt = RtEngine::new(48_000.0, rx, snap);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample {
            name: "continuous signal".into(),
            sr: 48_000,
            ch: 2,
            data: vec![0.25; 200_000],
            peaks: vec![],
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.decks[1].audio = None;
    rt.decks[0].playing = true;
    rt.xfader = 0.0;
    rt.bpm = 120.0;
    (OutputCallback::new(rt, 2), tx)
}

#[test]
fn control_port_reports_queue_acceptance_full_and_disconnected() {
    let (commands, rx) = CommandPort::channel(12);
    for _ in 0..3 {
        assert_eq!(
            commands.send(Command::Play),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    assert_eq!(commands.send(Command::Play), Err(SubmissionError::Full));
    // Dedicated capacity still accepts Stop when ordinary admission is full.
    assert_eq!(
        commands.send(Command::Stop),
        Ok(SubmissionOutcome::Accepted)
    );
    assert_eq!(
        commands.send(Command::Stop),
        Ok(SubmissionOutcome::Coalesced)
    );
    for _ in 0..3 {
        assert!(matches!(rx.try_recv(), Ok(Command::Play)));
    }
    assert!(matches!(
        rx.try_recv(),
        Ok(Command::ReservedStop { lane: 0, .. })
    ));
    drop(rx);
    assert_eq!(
        commands.send(Command::Stop),
        Err(SubmissionError::Disconnected)
    );
    assert_eq!(
        commands.send(Command::LiveNoteOff {
            source: 0,
            ch: 0,
            note: 60
        }),
        Err(SubmissionError::Disconnected)
    );
}

#[test]
fn production_callback_keeps_rendering_during_delayed_gui_and_ipc_work() {
    let (mut callback, tx) = fixture();
    let commands = tx.clone();
    let snap = callback.rt.snap.clone();
    let mut warm = [0.0f32; 1024];
    callback.render(&mut warm);
    let initial_pos = callback.rt.decks[0].pos;

    let (gui_ready_tx, gui_ready_rx) = mpsc::channel();
    let (gui_release_tx, gui_release_rx) = mpsc::channel();
    let gui_snap = snap.clone();
    let gui_commands = commands.clone();
    let gui = std::thread::spawn(move || {
        // Simulate a delayed GUI reader with the exact publication mutex held.
        let _snapshot = gui_snap.lock();
        gui_commands
            .send(Command::DeckGain {
                deck: 0,
                value: 0.85,
            })
            .unwrap();
        gui_ready_tx.send(()).unwrap();
        gui_release_rx.recv().unwrap();
    });
    gui_ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let server_snap = snap.clone();
    let server = std::thread::spawn(move || crate::handle_client(server, commands, server_snap));
    let client = std::thread::spawn(move || {
        let mut client = BufReader::new(client);
        writeln!(client.get_mut(), "{{\"op\":\"play\"}}").unwrap();
        let mut response = String::new();
        client.read_line(&mut response).unwrap();
        serde_json::from_str::<serde_json::Value>(&response).unwrap()
    });
    // The IPC command is queued before its response waits on the GUI reader.
    let deadline = Instant::now() + Duration::from_secs(2);
    while tx.len() < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let both_queued = tx.len() == 2;
    let (done_tx, done_rx) = mpsc::channel();
    let audio = std::thread::spawn(move || {
        let mut data = [0.0f32; 1024];
        let mut continuous = true;
        let mut audible = true;
        for block in 1..=48 {
            callback.render(&mut data);
            continuous &= callback.rt.decks[0].pos == initial_pos + block as f64 * 512.0;
            audible &= data.iter().any(|sample| sample.abs() > 0.001);
        }
        done_tx.send(()).unwrap();
        (callback, continuous, audible)
    });
    // No audio-device timing claim: a two-second bound only detects dependence
    // on the deliberately held reader. Always release it before joining.
    let completed_while_reader_held = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    gui_release_tx.send(()).unwrap();
    gui.join().unwrap();
    let (mut callback, continuous, audible) = audio.join().unwrap();
    let response = client.join().unwrap();
    server.join().unwrap().unwrap();

    assert!(
        both_queued,
        "GUI and IPC commands did not reach the control queue"
    );
    assert!(
        completed_while_reader_held,
        "audio waited for a GUI/IPC snapshot reader"
    );
    assert!(continuous, "sample position stopped advancing");
    assert!(audible, "callback substituted an all-zero buffer");
    assert!(
        callback.rt.frames_done >= 24_000,
        "test missed periodic snapshot publication"
    );
    assert!(callback.rt.playing, "IPC command was not applied by audio");
    assert_eq!(callback.rt.command_stats.received, 2);
    assert_eq!(response["ok"], true);
    assert_eq!(response["accepted"], true);
    assert_eq!(response["command_status"], "accepted");
    assert_eq!(
        response["playing"], false,
        "a queued acknowledgment must not invent current state"
    );

    // After the slow reader leaves, a later normal publication catches up.
    for _ in 0..12 {
        callback.render(&mut warm);
    }
    let state = snap.lock();
    assert!(state.playing);
    assert_eq!(state.commands.received, 2);
    assert!(state.decks[0].pos > initial_pos + 24_000.0);
}

#[test]
fn production_callback_converts_all_supported_sample_formats() {
    let (mut callback, _tx) = fixture();
    let mut floats = [0.0f32; 128];
    let mut signed = [0i16; 128];
    let mut unsigned = [32768u16; 128];
    callback.render(&mut floats);
    callback.render(&mut signed);
    callback.render(&mut unsigned);
    assert!(floats.iter().any(|sample| *sample > 0.0));
    assert!(signed.iter().any(|sample| *sample > 0));
    assert!(unsigned.iter().any(|sample| *sample > 32768));
    assert_eq!(callback.rt.decks[0].pos, 192.0);
}
