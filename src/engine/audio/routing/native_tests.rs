//! The external driver owns a private PipeWire server and all port links.
use super::{input, model::*, prepared::Prepared};
use crate::engine::{audio, Command, Engine};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Native routing fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "Requires scripts/check-audio-routing.py and its private 32-channel PipeWire server"]
fn private_native_32_channel_loopback_captures_each_physical_output() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("OMATAINER_NATIVE_ROUTING_DIR").expect("private fixture directory"),
    );
    assert_eq!(
        std::env::var_os("ALSA_CONFIG_PATH").unwrap(),
        directory.join("alsa.conf")
    );
    assert_eq!(
        std::env::var_os("XDG_RUNTIME_DIR").unwrap(),
        directory.join("runtime")
    );
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let mut model = Model::default();
    model.next_id = 6;
    for half in 0..2_u64 {
        let start = half as u16 * 26;
        let end = (start + 26).min(32);
        let source = 2 + half * 2;
        let record = source + 1;
        model.ports.push(Port {
            id: source,
            alias: format!("Loopback input {half}"),
            direction: Direction::Input,
            channels: (start..end).collect(),
        });
        model.ports.push(Port {
            id: record,
            alias: format!("Loopback record {half}"),
            direction: Direction::Record,
            channels: (0..end - start).collect(),
        });
        model.connections.push(Connection {
            source: Source {
                group: Group::Input(source),
                tap: Tap::PostFx,
            },
            destination: Group::Record(record),
            map: (0..(end - start) as u8)
                .map(|channel| ChannelMap {
                    source: channel,
                    destination: channel,
                    gain: 1.0,
                })
                .collect(),
        });
    }
    rt.routing = Some(Box::new(
        Prepared::new(Arc::new(model), &rt.session).unwrap(),
    ));
    let settings = crate::preferences::Audio {
        sample_rate: Some(48000),
        channels: Some(32),
        format: Some(crate::preferences::AudioFormat::F32),
        buffer_frames: Some(128),
        ..Default::default()
    };
    let audio = audio::start_with_settings(rt, &settings).unwrap();
    wait(|| engine.cmd.audio_metrics().callbacks > 20);
    let active = audio.handle.status();
    let output = &active.active.as_ref().unwrap().plan;
    assert_eq!(output.channels, 32);
    let saved = InputConfig {
        backend: output.backend.clone(),
        device: output.device.clone(),
        channels: 32,
        format: crate::preferences::AudioFormat::F32,
        buffer_frames: Some(128),
    };
    let plan = input::preview(&saved, &audio::config::discover().unwrap(), output).unwrap();
    audio
        .input
        .apply(
            Some(plan),
            active.generation,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    std::fs::write(
        directory.join("streams.ready"),
        b"Private native input and output opened\n",
    )
    .unwrap();
    wait(|| directory.join("links.ready").exists());
    std::thread::sleep(Duration::from_millis(200));
    let mut receipts = Vec::new();
    for half in 0..2_usize {
        let start = half * 26;
        let end = (start + 26).min(32);
        let width = end - start;
        let path = directory.join(format!("loopback-{half}.wav"));
        let recorder = engine.routing.recorder.clone();
        let epoch = recorder.epoch();
        let destination = path.clone();
        let capture = std::thread::spawn(move || {
            recorder.write(
                (3 + half * 2) as u64,
                width as u16,
                48000,
                15,
                &destination,
                &AtomicBool::new(false),
                epoch,
            )
        });
        wait(|| engine.routing.recorder.alias() != 0);
        let underruns_before = engine.routing.shared.underrun.load(Ordering::Relaxed);
        let overflow_before = engine.routing.shared.overflow.load(Ordering::Relaxed);
        for channel in start..end {
            engine
                .routing
                .shared
                .probe
                .store(channel as u32 + 1, Ordering::Release);
            std::thread::sleep(Duration::from_millis(250));
            engine.routing.shared.probe.store(0, Ordering::Release);
            std::thread::sleep(Duration::from_millis(30));
        }
        std::thread::sleep(Duration::from_millis(200));
        engine.routing.recorder.stop();
        let input_underrun_frames = engine.routing.shared.underrun.load(Ordering::Relaxed).saturating_sub(underruns_before);
        let input_overflow_frames = engine.routing.shared.overflow.load(Ordering::Relaxed).saturating_sub(overflow_before);
        capture.join().unwrap().unwrap();
        let decoded = crate::engine::decode::decode_audio(&path).unwrap();
        assert_eq!(decoded.sample.ch, width as u16);
        let mut energy = [0.0_f64; 32];
        let mut peak = [0.0_f32; 32];
        let mut overlap = 0_usize;
        for frame in decoded.sample.data.chunks_exact(width) {
            if frame.iter().filter(|value| value.abs() > 0.0002).count() > 1 {
                overlap += 1;
            }
            for (channel, value) in frame.iter().enumerate() {
                energy[channel] += f64::from(*value).powi(2);
                peak[channel] = peak[channel].max(value.abs());
            }
        }
        for channel in 0..width {
            assert!(
                energy[channel] > 0.2 && (0.009..=0.011).contains(&peak[channel]),
                "Output {}: energy {} peak {}",
                start + channel + 1,
                energy[channel],
                peak[channel]
            );
        }
        assert_eq!(
            overlap, 0,
            "Physical channel tests crossed into another channel"
        );
        receipts.push(serde_json::json!({"physical_channels": [start + 1, end], "frames": decoded.sample.frames(), "energy": energy, "peak": peak, "cross_channel_overlap_frames": overlap, "input_underrun_frames": input_underrun_frames, "input_overflow_frames": input_overflow_frames, "wav": path}));
    }
    engine.cmd.send(Command::Stop).unwrap();
    std::fs::write(directory.join("done.json"), serde_json::to_vec_pretty(&serde_json::json!({"channels":32,"rate":48000,"input_counter_unit":"frames","captures":receipts,"input_underruns":engine.routing.shared.underrun.load(Ordering::Relaxed),"input_overflow":engine.routing.shared.overflow.load(Ordering::Relaxed),"callbacks":engine.cmd.audio_metrics().callbacks})).unwrap()).unwrap();
    drop(audio);
}
