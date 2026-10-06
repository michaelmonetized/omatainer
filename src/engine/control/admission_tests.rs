use crate::engine::{SamplerInstrument, SynthInstrument};
use super::*;
use crate::engine::{RtEngine, Snapshot};
use parking_lot::Mutex;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn engine() -> (CommandPort, RtEngine) {
    let (port, receiver) = CommandPort::channel(256);
    let mut rt = RtEngine::new(
        48_000.0,
        receiver,
        Arc::new(Mutex::new(Snapshot::default())),
    );
    rt.selected_track = 1;
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    rt.decks[1].playing = true;
    (port, rt)
}

fn tick(rt: &mut RtEngine) {
    let mut out = [0.0; 256];
    rt.process(&mut out);
    assert!(rt.command_stats.received_last_block <= COMMANDS_PER_BLOCK);
    assert!(out.iter().all(|sample| sample.is_finite()));
}

fn held(rt: &RtEngine) -> bool {
    rt.tracks.iter().any(|track| {
        track
            .poly
            .voices
            .iter()
            .any(|voice| matches!(voice.env.stage, 1..=3))
    }) || rt
        .sampler_poly
        .voices
        .iter()
        .any(|voice| matches!(voice.env.stage, 1..=3))
}

#[test]
fn pressure_pads_reject_invalid_attacks_and_reserve_the_ordinary_release_without_heap_work() {
    let (port,mut rt)=engine();
    for (pad,pressure) in [(16,0.5),(0,f32::NAN),(0,f32::INFINITY),(0,-0.1),(0,1.1)] {
        assert_eq!(port.send(Command::SamplerPadPressure {pad,pressure}),Err(SubmissionError::InvalidTarget));
    }
    assert_eq!(port.len(),0);
    assert_eq!(port.admission.lock().held,0);
    port.send(Command::SamplerPadPressure {pad:0,pressure:0.2}).unwrap();
    tick(&mut rt);
    assert!(held(&rt));
    while port.send(Command::Master(0.8)).is_ok() {}
    assert_eq!(port.send(Command::SamplerPad {pad:0,on:false}),Ok(SubmissionOutcome::Accepted));
    assert_eq!(port.admission.lock().held,0);
    for _ in 0..8 {tick(&mut rt);}
    assert!(!held(&rt));
    for instrument in [SamplerInstrument::Synth(SynthInstrument::Keys),SamplerInstrument::Samples] {
        rt.apply(Command::SamplerInst(instrument));
        let mut output=[0.0;128];
        let counts=crate::engine::test_alloc::measure(|| {
            for _ in 0..256 {
                port.send(Command::SamplerPadPressure {pad:0,pressure:0.25}).unwrap();
                port.send(Command::SamplerPad {pad:0,on:false}).unwrap();
                rt.process(&mut output);
            }
        });
        assert_eq!(counts,crate::engine::test_alloc::Counts::default(),"{instrument:?}");
        assert!(output.iter().all(|value|value.is_finite()));
    }
}

#[test]
fn equal_pitch_devices_reserve_independent_releases_including_zero_velocity() {
    let (port, mut rt) = engine();
    for source in [11, 22] {
        assert_eq!(
            port.send(Command::LiveNoteOn {
                source,
                ch: 3,
                note: 60,
                vel: 100,
            }),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    assert_eq!(port.admission.lock().held, 2);
    tick(&mut rt);
    while port.send(Command::Master(0.8)).is_ok() {}
    assert_eq!(
        port.send(Command::LiveNoteOn {
            source: 11,
            ch: 3,
            note: 60,
            vel: 0,
        }),
        Ok(SubmissionOutcome::Accepted)
    );
    assert_eq!(port.admission.lock().held, 1);
    assert_eq!(
        port.send(Command::LiveNoteOff {
            source: 22,
            ch: 3,
            note: 60,
        }),
        Ok(SubmissionOutcome::Accepted)
    );
    assert_eq!(port.admission.lock().held, 0);
    for _ in 0..8 {
        tick(&mut rt);
    }
    assert!(!held(&rt));
}

#[test]
fn saturated_queue_preserves_fifo_gate_releases_and_every_emergency_stop_lane() {
    let (port, mut rt) = engine();
    for command in [
        Command::LiveNoteOn {
            source: 0,
            ch: 0,
            note: 60,
            vel: 100,
        },
        Command::SamplerPad { pad: 3, on: true },
        Command::DeckTouch { deck: 0, on: true },
    ] {
        assert_eq!(port.send(command), Ok(SubmissionOutcome::Accepted));
    }
    while port.send(Command::Master(0.8)).is_ok() {}
    assert_eq!(port.len(), 228);
    for release in [
        Command::LiveNoteOff {
            source: 0,
            ch: 0,
            note: 60,
        },
        Command::SamplerPad { pad: 3, on: false },
        Command::DeckTouch { deck: 0, on: false },
    ] {
        assert_eq!(port.send(release.clone()), Ok(SubmissionOutcome::Accepted));
        assert_eq!(port.send(release), Ok(SubmissionOutcome::Coalesced));
    }
    assert_eq!(port.send(Command::Stop), Ok(SubmissionOutcome::Accepted));
    for track in 0..super::super::TRACKS {
        assert_eq!(
            port.send(Command::StopTrack { track: track as u8 }),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    for pad in 0..16 {
        assert_eq!(port.send(Command::SamplerSlotStop { pad }), Ok(SubmissionOutcome::Accepted));
    }
    assert_eq!(port.len(), 256);
    assert_eq!(port.send(Command::Stop), Ok(SubmissionOutcome::Coalesced));
    tick(&mut rt);
    assert!(
        held(&rt),
        "releases must not overtake queued note/pad onsets"
    );
    assert!(rt.decks[0].touching);
    for _ in 1..8 {
        tick(&mut rt);
    }
    assert_eq!(port.len(), 0);
    assert!(!held(&rt));
    assert!(!rt.decks[0].touching);
    assert!(!rt.playing);
    assert!(
        rt.decks[1].playing,
        "a track stop must not reset an unrelated live deck"
    );
    assert_eq!(rt.command_stats.received, 256);
    let stats = port.stats();
    assert_eq!(stats.accepted, 256);
    assert_eq!(stats.coalesced, 4);
    assert_eq!(stats.rejected, 1);
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().submissions.accepted, 256);
}

#[test]
fn all_accepted_distinct_onsets_reserve_their_release_even_without_a_consumer() {
    let (port, mut rt) = engine();
    let mut pitches = Vec::new();
    for note in 0..=255 {
        match port.send(Command::LiveNoteOn {
            source: 0,
            ch: 0,
            note,
            vel: 100,
        }) {
            Ok(SubmissionOutcome::Accepted) => pitches.push(note),
            Err(SubmissionError::Full) => break,
            other => panic!("unexpected admission {other:?}"),
        }
    }
    assert_eq!(pitches.len(), 115);
    for note in pitches {
        assert_eq!(
            port.send(Command::LiveNoteOff {
                source: 0,
                ch: 0,
                note
            }),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    assert_eq!(port.send(Command::Stop), Ok(SubmissionOutcome::Accepted));
    for track in 0..super::super::TRACKS {
        assert_eq!(
            port.send(Command::StopTrack { track: track as u8 }),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    for pad in 0..16 { assert_eq!(port.send(Command::SamplerSlotStop { pad }), Ok(SubmissionOutcome::Accepted)); }
    assert_eq!(port.len(), 255);
    for _ in 0..8 {
        tick(&mut rt);
    }
    assert!(!held(&rt));
}

#[test]
fn redundant_release_never_coalesces_across_an_accepted_retrigger() {
    let (port, mut rt) = engine();
    let on = Command::LiveNoteOn {
        source: 0,
        ch: 0,
        note: 60,
        vel: 100,
    };
    let off = Command::LiveNoteOff {
        source: 0,
        ch: 0,
        note: 60,
    };
    assert_eq!(port.send(off.clone()), Ok(SubmissionOutcome::Coalesced));
    for _ in 0..3 {
        assert_eq!(port.send(on.clone()), Ok(SubmissionOutcome::Accepted));
        assert_eq!(port.send(off.clone()), Ok(SubmissionOutcome::Accepted));
        assert_eq!(port.send(off.clone()), Ok(SubmissionOutcome::Coalesced));
    }
    assert_eq!(port.len(), 6);
    let mut commands = Vec::new();
    while let Ok(command) = rt.cmd_rx.try_recv() {
        commands.push(command);
    }
    for pair in commands.chunks(2) {
        assert!(matches!(pair[0], Command::LiveNoteOn { note: 60, .. }));
        assert!(matches!(pair[1], Command::LiveNoteOff { note: 60, .. }));
    }
    for command in commands {
        rt.apply(command);
    }
    assert!(!held(&rt));
    assert_eq!(port.stats().accepted, 6);
    assert_eq!(port.stats().coalesced, 4);
}

#[test]
fn pending_stop_blocks_all_restart_paths_until_callback_acknowledges_it() {
    let (port, mut rt) = engine();
    let clip_starts = [
        Command::LaunchClip { track: 1, scene: 0 },
        Command::FireClip {
            track: 1,
            scene: 0,
            looping: false,
        },
        Command::LaunchScene { scene: 0 },
        Command::AddScene { scene: 0 },
        Command::RestartScene { scene: 0 },
        Command::ToggleScene { scene: 0 },
        Command::TogglePlay,
    ];
    for stop in [Command::Stop, Command::StopTrack { track: 1 }] {
        assert_eq!(port.send(stop.clone()), Ok(SubmissionOutcome::Accepted));
        for command in &clip_starts {
            assert_eq!(
                port.send(command.clone()),
                Err(SubmissionError::StopPending),
                "{command:?}"
            );
        }
        assert_eq!(port.send(stop), Ok(SubmissionOutcome::Coalesced));
        tick(&mut rt);
        assert_eq!(
            port.send(Command::LaunchClip { track: 1, scene: 0 }),
            Ok(SubmissionOutcome::Accepted)
        );
        tick(&mut rt);
        assert!(rt.tracks[1].playing.is_some());
    }
    port.send(Command::Stop).unwrap();
    assert_eq!(port.send(Command::Play), Err(SubmissionError::StopPending));
    assert_eq!(
        port.send(Command::Record),
        Err(SubmissionError::StopPending)
    );
    tick(&mut rt);
    assert_eq!(port.send(Command::Play), Ok(SubmissionOutcome::Accepted));
}

#[test]
fn callback_never_acquires_the_producer_admission_mutex() {
    let (port, mut rt) = engine();
    port.send(Command::Stop).unwrap();
    let guard = port.admission.lock();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let audio = std::thread::spawn(move || {
        tick(&mut rt);
        done_tx.send(()).unwrap();
    });
    let completed = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    drop(guard);
    audio.join().unwrap();
    assert!(
        completed,
        "callback waited on a producer-only admission lock"
    );
}

#[test]
fn an_unfinished_producer_never_stalls_audio_or_exposes_its_command() {
    let (port, rt) = engine();
    let mut rt = Box::new(rt);
    port.send(Command::Master(0.1)).unwrap();
    let (entered, seen) = std::sync::mpsc::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let producer = std::thread::spawn(move || {
        let mut state = port.admission.lock();
        let chunk = state.producer.write_chunk_uninit(1).unwrap();
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(chunk.fill_from_iter([Command::Master(0.37)]), 1);
    });
    seen.recv_timeout(Duration::from_secs(2)).unwrap();
    let (done, completed) = std::sync::mpsc::channel();
    let audio = std::thread::spawn(move || {
        let mut out = [0.0; 256];
        let allocation = crate::engine::test_alloc::measure(|| {
            for _ in 0..128 { rt.process(&mut out); }
        });
        assert_eq!(allocation.allocations, 0);
        assert_eq!(allocation.frees, 0);
        assert!(out.iter().all(|sample| sample.is_finite()));
        assert!(rt.cmd_rx.is_empty());
        assert!((rt.master - 0.1).abs() < 0.00001);
        done.send(rt).unwrap();
    });
    let result = completed.recv_timeout(Duration::from_secs(2));
    resume.send(()).unwrap();
    producer.join().unwrap();
    audio.join().unwrap();
    let mut rt = result.expect("audio waited for an unfinished producer");
    tick(&mut rt);
    assert!((rt.master - 0.37).abs() < 0.00001);
    assert!(rt.cmd_rx.is_empty());
}

#[test]
fn concurrent_producers_cannot_consume_another_gates_release_reservation() {
    let (port, mut rt) = engine();
    let barrier = Arc::new(Barrier::new(5));
    let mut producers = Vec::new();
    for worker in 0..4 {
        let commands = port.clone();
        let barrier = barrier.clone();
        producers.push(std::thread::spawn(move || {
            let note = 60 + worker;
            let on = Command::LiveNoteOn {
                source: 0,
                ch: 0,
                note,
                vel: 100,
            };
            let off = Command::LiveNoteOff {
                source: 0,
                ch: 0,
                note,
            };
            commands.send(on.clone()).unwrap();
            for _ in 0..70 {
                let _ = commands.send(Command::Master(0.8));
            }
            barrier.wait();
            let deadline = Instant::now() + Duration::from_secs(5);
            for cycle in 0..100 {
                if cycle > 0 {
                    loop {
                        match commands.send(on.clone()) {
                            Ok(SubmissionOutcome::Accepted) => break,
                            Err(SubmissionError::Full) => {
                                assert!(Instant::now() < deadline);
                                std::thread::sleep(Duration::from_micros(50));
                            }
                            other => panic!("unexpected onset result {other:?}"),
                        }
                    }
                }
                for _ in 0..8 {
                    let _ = commands.send(Command::Master(0.7));
                }
                assert_eq!(commands.send(off.clone()), Ok(SubmissionOutcome::Accepted));
                assert_eq!(commands.send(off.clone()), Ok(SubmissionOutcome::Coalesced));
            }
        }));
    }
    barrier.wait();
    assert!(port.stats().rejected > 0, "fixture must saturate admission");
    let deadline = Instant::now() + Duration::from_secs(10);
    while producers.iter().any(|producer| !producer.is_finished()) || !rt.cmd_rx.is_empty() {
        assert!(Instant::now() < deadline);
        tick(&mut rt);
    }
    for producer in producers {
        producer.join().unwrap();
    }
    while !rt.cmd_rx.is_empty() {
        tick(&mut rt);
    }
    assert!(!held(&rt));
    assert!(rt.decks[1].playing);
    let stats = port.stats();
    assert_eq!(stats.coalesced, 400);
    assert_eq!(stats.accepted, rt.command_stats.received);
    assert!(stats.accepted >= 800);
}
