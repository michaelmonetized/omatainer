use super::*;
use crate::engine::{Engine, MidiClockInput};

fn send(bytes: &[u8], source: u64, commands: &crate::engine::CommandPort) {
    handle_msg(
        bytes,
        source,
        &class_compliant(),
        commands,
        &Arc::new(Mutex::new(Vec::new())),
        &Arc::new(Mutex::new([false; 4])),
        "synthetic realtime input",
    );
}

#[test]
fn one_byte_clock_and_transport_reach_their_command_handlers() {
    // Positive replacement for the issue's audit_one_byte_* regression.
    for byte in [0xfa, 0xfc, 0xf8] {
        let (commands, receiver) = crate::engine::CommandPort::channel(32);
        send(&[byte], 71, &commands);
        let received: Vec<_> = receiver.try_iter().collect();
        assert_eq!(received.len(), 1, "status {byte:#04x}: {received:?}");
        assert!(match (byte, &received[0]) {
            (0xfa, Command::Play) => true,
            (0xfc, Command::ReservedStop { lane: 0, .. }) => true,
            (0xf8, Command::MidiClock { source: 71 }) => true,
            _ => false,
        });
    }
}

#[test]
fn realtime_can_appear_at_every_channel_byte_boundary() {
    use framing::Message::{Channel, Realtime};
    for status in [0x80, 0x9f, 0xa2, 0xb5, 0xe7] {
        let original = [status, 60, 100];
        for realtime in 0xf8..=0xff {
            for insertion in 0..=3 {
                let mut packet = original.to_vec();
                packet.insert(insertion, realtime);
                let received: Vec<_> = framing::messages(&packet).collect();
                let expected = if insertion == 3 {
                    vec![Channel(original), Realtime(realtime)]
                } else {
                    vec![Realtime(realtime), Channel(original)]
                };
                assert_eq!(received, expected, "{packet:02x?}");
            }
        }
    }
    assert_eq!(
        framing::messages(&[0x90, 0xf8, 60, 0xfa, 100, 0xfc]).collect::<Vec<_>>(),
        vec![
            Realtime(0xf8),
            Realtime(0xfa),
            Channel([0x90, 60, 100]),
            Realtime(0xfc)
        ]
    );
}

#[test]
fn renderer_consumes_clock_source_and_transport_without_claiming_tempo_sync() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 48);
    let original_bpm = rt.bpm;
    rt.selected_track = 1; // Synth input gate; default track 0 is a drum kit.
    assert!(!rt.playing);
    send(&[0xfa], 71, &engine.cmd);
    rt.process(&mut [0.0; 256]);
    assert!(rt.playing && rt.beat > 0.0);

    send(&[0x93, 0xf8, 60, 0xf8, 100, 0xf8], 71, &engine.cmd);
    rt.process(&mut []);
    assert_eq!(
        rt.midi_clock,
        MidiClockInput {
            ticks: 3,
            last_source: Some(71)
        }
    );
    assert!(
        rt.tracks
            .iter()
            .flat_map(|track| &track.poly.voices)
            .any(|voice| voice.input
                == Some(crate::engine::dsp::InputKey::Midi {
                    source: 71,
                    ch: 3,
                    note: 60
                }))
    );
    send(&[0xf8], 92, &engine.cmd);
    let allocations = crate::engine::test_alloc::measure(|| rt.process(&mut []));
    assert_eq!(allocations, crate::engine::test_alloc::Counts::default());
    assert_eq!(
        rt.midi_clock,
        MidiClockInput {
            ticks: 4,
            last_source: Some(92)
        }
    );

    send(&[0xfc], 71, &engine.cmd);
    rt.process(&mut [0.0; 256]);
    assert!(!rt.playing);
    let stopped_beat = rt.beat;
    send(&[0xf8, 0xf8], 93, &engine.cmd);
    rt.process(&mut [0.0; 256]);
    assert_eq!(rt.beat, stopped_beat);
    assert_eq!(rt.bpm, original_bpm);
    assert_eq!(
        rt.midi_clock,
        MidiClockInput {
            ticks: 6,
            last_source: Some(93)
        }
    );
    rt.publish_for_test();
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.midi_clock, rt.midi_clock);
    let serialized = serde_json::to_value(snapshot).unwrap();
    assert_eq!(serialized["midi_clock"]["ticks"], 6);
    assert_eq!(serialized["midi_clock"]["last_source"], 93);
    // Check the actual IPC status surface too, not only Snapshot's serializer.
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;
    let (client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let commands = engine.cmd.clone();
    let snapshot = rt.snap.clone();
    let handler =
        std::thread::spawn(move || crate::handle_client(server, commands, snapshot).unwrap());
    let mut reader = std::io::BufReader::new(client);
    writeln!(reader.get_mut(), "{}", crate::STATUS_REQUEST).unwrap();
    reader
        .get_mut()
        .shutdown(std::net::Shutdown::Write)
        .unwrap();
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    handler.join().unwrap();
    let status: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(status["midi_clock"]["ticks"], 6);
    assert_eq!(status["midi_clock"]["last_source"], 93);
}

#[test]
fn truncated_channel_frames_never_invent_data_or_carry_into_the_next_packet() {
    let (commands, receiver) = crate::engine::CommandPort::channel(32);
    for status in 0x80..=0xef {
        send(&[status], 1, &commands);
        send(&[status, 60], 1, &commands);
        assert!(receiver.is_empty(), "short/unsupported {status:#04x}");
    }
    for packet in [
        &[][..],
        &[60, 100],
        &[0x90, 60],
        &[100],
        &[0xb0, 7],
        &[0xe0, 0],
        &[0x90, 60, 0x80],
        &[0xb0, 7, 0xf4, 64],
        &[0x90, 0xf0, 60, 100, 0xf7],
        &[0xc0, 5, 60, 100],
        &[0xd0, 5, 60, 100],
    ] {
        send(packet, 1, &commands);
        assert!(receiver.is_empty(), "malformed/unsupported {packet:02x?}");
    }
    // A new valid status resynchronizes after a truncated frame.
    send(&[0x90, 60, 0xb1, 7, 100, 0x91, 61, 110], 1, &commands);
    let received: Vec<_> = receiver.try_iter().collect();
    assert!(matches!(
        received.as_slice(),
        [Command::LiveNoteOn {
            source: 1,
            ch: 1,
            note: 61,
            vel: 110
        }]
    ));
}

#[test]
fn realtime_bypasses_learn_but_complete_channel_capture_is_preserved() {
    let (commands, receiver) = crate::engine::CommandPort::channel(48);
    let hub=MidiHub::without_devices();
    let mut input=hub.open_for_test(&commands,71,class_compliant(),"learning input","fixture:learn");
    commands.midi_learn().begin(cbind(0,0,Action::Master,0,0),None).unwrap();
    input.push(&[0xfa,0xb2,0xf8,7,0xf8,99]);
    let received:Vec<_>=receiver.try_iter().collect();
    assert!(matches!(received.as_slice(),[Command::Play,Command::MidiClock{source:71},Command::MidiClock{source:71}]));
    let view=commands.midi_learn().view();assert!(!view.armed);
    assert_eq!(view.capture.unwrap().bytes,[0xb2,7,99]);
    for packet in [&[0xb2,7][..],&[0xc0,1],&[0xd0,1],&[0x90,60]] {input.push(packet);assert!(receiver.is_empty());}
    input.push(&[0xfc]);assert!(matches!(receiver.try_recv(),Ok(Command::ReservedStop{lane:0,..})));
}

#[test]
fn unsupported_system_statuses_are_safe_and_do_not_swallow_realtime() {
    let (commands, receiver) = crate::engine::CommandPort::channel(48);
    for status in 0xf0..=0xff {
        if [0xfa, 0xfc, 0xf8].contains(&status) {
            continue;
        }
        send(&[status], 1, &commands);
        assert!(
            receiver.is_empty(),
            "unsupported system status {status:#04x}"
        );
    }
    send(
        &[0xf0, 1, 2, 0xf8, 3, 0xfa, 4, 0xf7, 0xf1, 0xf8, 1],
        17,
        &commands,
    );
    let received: Vec<_> = receiver.try_iter().collect();
    assert!(matches!(
        received.as_slice(),
        [
            Command::MidiClock { source: 17 },
            Command::Play,
            Command::MidiClock { source: 17 }
        ]
    ));
}

#[test]
fn actual_input_worker_hands_single_byte_messages_to_the_renderer() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 48);
    let counters = Arc::new(handoff::InputCounters::default());
    let (mut input, guard) = handoff::start(
        88,
        class_compliant(),
        engine.cmd.clone(),
        Arc::new(Mutex::new(Vec::new())),
        "realtime worker fixture".into(),
        counters.clone(),
    )
    .unwrap();
    let wait = |expected| {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while counters.snapshot().dispatched < expected {
            assert!(Instant::now() < deadline, "MIDI worker did not advance");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    input.push(&[0xfa]);
    input.push(&[0xf8]);
    wait(2);
    rt.process(&mut [0.0; 256]);
    assert!(rt.playing && rt.beat > 0.0);
    assert_eq!(
        rt.midi_clock,
        MidiClockInput {
            ticks: 1,
            last_source: Some(88)
        }
    );
    input.push(&[0xfc]);
    wait(3);
    rt.process(&mut [0.0; 256]);
    assert!(!rt.playing);
    drop(input);
    drop(guard);
}
