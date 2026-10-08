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
        audio: Arc::new(Sample { spectrum: None,
            name: "continuous signal".into(),
            sr: 48_000,
            ch: 2,
            data: vec![0.25; 200_000],
            peaks: vec![].into(),
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
fn captured_clicks_follow_retained_output_positions_during_reverse_and_mapped_tempo() {
    use crate::engine::{beatgrid::Grid, test_alloc};
    for reverse in [false, true] {
        let (mut callback, _commands) = fixture();
        let mut pcm = vec![0.0; 8192 * 2];
        for (index, frame) in (64..8192).step_by(128).enumerate() {
            let frame = frame + index % 13;
            pcm[frame*2] = 0.1; pcm[frame*2+1] = 0.1;
        }
        let sample = Arc::new(Sample { spectrum: None, name: "captured click timeline".into(),sr:48000,ch:2,
            data:pcm.clone(),peaks:vec![].into(),bpm:120.0,path:String::new() });
        callback.rt.playing = false;
        callback.rt.decks[0].audio = Some(sample.clone());
        callback.rt.decks[0].history_key = 77;
        callback.rt.decks[0].transition_remaining = 0;
        callback.rt.decks[0].pos = if reverse { 4000.0 } else { 0.0 };
        callback.rt.decks[0].touching = reverse;
        callback.rt.decks[0].scratch = -0.75;
        if !reverse {
            callback.rt.decks[0].grid = Some(Grid::new(0.0,120.0).unwrap()
                .with_anchor(0.08,0.03).unwrap().with_anchor(0.16,0.09).unwrap());
            callback.rt.decks[0].sync = true;
            callback.rt.decks[0].sync_bpm = 120.0;
        }
        let handle = callback.rt.audible.handle();
        let mut captured_clicks = 0;
        for block in 0..16 {
            let start = 1_000_000_000 + block * 256 * 1_000_000_000 / 48000;
            let mut output = [0.0_f32;512];
            assert_eq!(test_alloc::measure(|| callback.render_at(&mut output,None,Some(start))), test_alloc::Counts::default());
            for frame in 0..256 {
                let positions = handle.positions_at(start + frame as u64 * 1_000_000_000 / 48000).unwrap();
                let position = positions[0];
                assert_eq!(position.media_key,77);
                let expected = sample.at(position.source_frame).0;
                if expected > 0.05 { assert!(output[frame*2].abs()>0.005); captured_clicks += 1; }
                if expected == 0.0 { assert!(output[frame*2].abs()<0.002); }
            }
        }
        assert!(captured_clicks>8);
        assert_eq!(sample.data,pcm);
        assert!(Arc::ptr_eq(&sample,callback.rt.decks[0].audio.as_ref().unwrap()));
        assert!(handle.positions_at(999_999_999).is_none());
        callback.render(&mut [0.0_f32;512]);
        assert!(handle.positions_at(1_000_000_000).is_none());
    }
}

#[test]
fn control_port_reports_queue_acceptance_full_and_disconnected() {
    let (commands, rx) = CommandPort::channel(28);
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
    // The reader can outlast the bounded snapshot wait. A truthful receipt
    // still acknowledges admission; state is included only when available.
    match response["state_available"].as_bool() {
        Some(true) => {
            assert_eq!(response["event"], "state");
            assert!(response["playing"].is_boolean());
        }
        Some(false) => {
            assert_eq!(response["event"], "receipt");
            assert!(response["playing"].is_null());
            assert!(response["warning"].as_str().is_some_and(|text| !text.is_empty()));
        }
        None => panic!("IPC acknowledgment omitted state availability: {response}"),
    }

    // After the slow reader leaves, a later normal publication catches up.
    for _ in 0..12 {
        callback.render(&mut warm);
    }
    callback.rt.publish_for_test();
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
