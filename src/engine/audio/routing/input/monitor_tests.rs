use super::*;
use crate::engine::{
    audio::routing::{model::*, prepared::Prepared},
    input_monitor::Mode,
    test_alloc, Command, Engine,
};

fn link(source: Group, destination: Group) -> Connection {
    Connection {
        source: Source {
            group: source,
            tap: Tap::PostMixer,
        },
        destination,
        map: (0..2)
            .map(|channel| ChannelMap {
                source: channel,
                destination: channel,
                gain: 1.0,
            })
            .collect(),
    }
}

#[test]
fn controlled_input_records_exact_source_frames_with_monitoring_off_and_source_loss_stays_silent() {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.fx_wet.fill(0.0);
    for track in &mut rt.tracks {
        track.fx.slots.clear();
    }
    for chain in &mut rt.scene_fx {
        chain.slots.clear();
    }
    rt.apply(Command::TrackMonitor {
        track: 0,
        mode: Mode::Off,
    });
    let id = rt.session.tracks[0].id;
    let mut model = Model::default();
    model.next_id = 4;
    model.ports.push(Port {
        id: 2,
        alias: "Controlled stereo input".into(),
        direction: Direction::Input,
        channels: vec![0, 1],
    });
    model.ports.push(Port {
        id: 3,
        alias: "Raw input record".into(),
        direction: Direction::Record,
        channels: vec![0, 1],
    });
    model.connections.extend([
        link(Group::Input(2), Group::Track(id)),
        link(Group::Input(2), Group::Record(3)),
    ]);
    rt.routing = Some(Box::new(
        Prepared::new(Arc::new(model), &rt.session).unwrap(),
    ));
    rt.routing_pipe.shared.rate.store(48000, Ordering::Release);
    rt.routing_pipe
        .shared
        .enabled
        .store(true, Ordering::Release);
    let input: Vec<f32> = (0..1024)
        .flat_map(|index| [0.1 + index as f32 / 8192.0, -0.2 - index as f32 / 16384.0])
        .collect();
    for chunk in input.chunks(64 * 2) {
        rt.routing_pipe.capture(chunk, 2, 0);
    }
    let path = std::env::temp_dir().join(format!(
        "omatainer-monitor-off-{}.wav",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let destination = path.clone();
    let recorder = rt.routing_pipe.recorder.clone();
    let writer = recorder.clone();
    let epoch = recorder.epoch();
    let work = std::thread::spawn(move || {
        writer.write(3, 2, 48000, 1, &destination, &AtomicBool::new(false), epoch)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while recorder.alias() != 3 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut output = [0.0; 128 * 2];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut output)),
        test_alloc::Counts::default()
    );
    assert!(output.iter().all(|value| value.abs() < 0.000001));
    assert_eq!(recorder.frames(), 128);
    recorder.stop();
    assert_eq!(work.join().unwrap().unwrap(), path);
    let saved = crate::engine::decode::decode_audio(&path).unwrap().sample;
    assert_eq!((saved.sr, saved.ch, saved.frames()), (48000, 2, 128));
    assert_eq!(saved.data, input[..256]);
    std::fs::remove_file(path).unwrap();
    rt.apply(Command::TrackMonitor {
        track: 0,
        mode: Mode::In,
    });
    let mut monitored = [0.0; 512 * 2];
    rt.process(&mut monitored);
    assert!(monitored.iter().any(|value| value.abs() > 0.01));
    rt.routing_pipe.stop();
    rt.process(&mut monitored);
    assert!(monitored[monitored.len() - 2..]
        .iter()
        .all(|value| value.abs() < 0.000001));
    assert!(
        !rt.routing
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.group == Group::Track(id))
            .unwrap()
            .valid
    );
    rt.apply(Command::TrackMonitor {
        track: 0,
        mode: Mode::Off,
    });
    rt.process(&mut monitored);
    assert!(
        rt.routing
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.group == Group::Track(id))
            .unwrap()
            .valid
    );
    assert!(
        !rt.routing
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.group == Group::Input(2))
            .unwrap()
            .valid
    );
}
