use super::{input, model::*, prepared::Prepared};
use crate::engine::{audio, midi, Engine};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "NS7 hardware qualification timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "Requires a powered original NS7, rtkit and OMATAINER_NS7_QUALIFY_DIR on /home; output is silent"]
fn original_ns7_recovers_an_underrun_without_retiring_the_output() {
    use audio::owner::Backend;
    let directory = PathBuf::from(
        std::env::var_os("OMATAINER_NS7_QUALIFY_DIR").expect("qualification directory"),
    );
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    let (engine, mut rt) = Engine::headless_for_test(44100, 256);
    rt.master = 0.0;
    let mut callback = audio::OutputCallback::new(rt, 4);
    callback.stall_once = Some((8, Duration::from_millis(40)));
    let settings = crate::preferences::Audio {
        backend: Some("ALSA".into()),
        device: Some("hw:CARD=NS7,DEV=0".into()),
        sample_rate: Some(44100),
        channels: Some(4),
        format: Some(crate::preferences::AudioFormat::I32),
        buffer_frames: Some(512),
        ..Default::default()
    };
    let mut backend = audio::Native;
    let plan = backend.select(&settings).unwrap();
    let fault = Arc::new(AtomicBool::new(false));
    let identity = backend.identity(&plan).expect("physical NS7 identity");
    let stream = backend
        .open(&plan, callback, fault.clone(), Some(&identity))
        .unwrap();
    backend.play(&stream).unwrap();
    wait(|| engine.cmd.audio_metrics().xruns > 0 && engine.cmd.audio_metrics().callbacks > 100);
    let tid = std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| {
            (std::fs::read_to_string(entry.path().join("comm"))
                .ok()?
                .trim()
                == "cpal_alsa_out")
                .then(|| entry.file_name().to_string_lossy().parse::<i32>().unwrap())
        })
        .expect("native ALSA callback thread");
    let policy = unsafe { libc::sched_getscheduler(tid) };
    let base_policy = policy & !libc::SCHED_RESET_ON_FORK;
    let metrics = engine.cmd.audio_metrics();
    let hw = std::fs::read_to_string("/proc/asound/NS7/pcm0p/sub0/hw_params").unwrap();
    let receipt = serde_json::json!({"device":plan.device,"master":0,"injected_stall_ms":40,
        "audio":metrics,"fault":fault.load(Ordering::Acquire),"callback_tid":tid,
        "callback_policy":policy,"callback_base_policy":base_policy,
        "callback_reset_on_fork":policy & libc::SCHED_RESET_ON_FORK != 0,
        "hw_params":hw,"analog_listening":false});
    std::fs::write(
        directory.join("ns7-underrun-recovery.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(metrics.backend_errors, 0, "{receipt}");
    assert_eq!(metrics.device_lost, 0, "{receipt}");
    assert_eq!(metrics.realtime_denied, 0, "{receipt}");
    assert!(!fault.load(Ordering::Acquire), "{receipt}");
    assert!(
        matches!(base_policy, libc::SCHED_FIFO | libc::SCHED_RR),
        "{receipt}"
    );
    assert!(
        hw.contains("buffer_size: 512") && hw.contains("period_size: 256"),
        "{receipt}"
    );
    backend.close(stream);
}

/// Select the three stage controllers.
/// Takes no arguments; returns exact discovered input names, excluding the resetting MPD232.
fn stage_inputs() -> midi::InputPolicy {
    let probe = midir::MidiInput::new("omatainer-stage-qualification").unwrap();
    let names = probe
        .ports()
        .iter()
        .filter_map(|port| probe.port_name(port).ok())
        .filter(|name| {
            name.contains("Numark NS7:")
                || name.contains("Pioneer DDJ-SP1:")
                || name.contains("APC40")
        })
        .collect();
    midi::InputPolicy::Selected(names)
}

#[test]
#[ignore = "Requires a powered original NS7 with snd_ns7 loaded and OMATAINER_NS7_QUALIFY_DIR on /home"]
fn original_ns7_production_audio_and_midi_io() {
    let directory = PathBuf::from(
        std::env::var_os("OMATAINER_NS7_QUALIFY_DIR").expect("qualification directory"),
    );
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !directory.join("ns7-input.wav").exists(),
        "use a fresh NS7 qualification directory"
    );
    let interface = std::fs::read_dir("/sys/bus/usb/drivers/snd_ns7")
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            std::fs::read_to_string(path.join("bInterfaceNumber"))
                .is_ok_and(|number| number.trim() == "00")
        })
        .expect("original NS7 driver");
    let counter = |name: &str| {
        std::fs::read_to_string(interface.join(name))
            .unwrap()
            .trim()
            .parse::<u64>()
            .unwrap()
    };
    let output_before = counter("midi_output_bytes");
    let input_before = counter("midi_input_bytes");
    let playback_before = counter("pcm_playback_frames");
    let capture_before = counter("pcm_capture_frames");
    let feedback_before = counter("pcm_feedback_frames");
    let pcm_errors_before = counter("pcm_errors");
    let partial_before = counter("pcm_feedback_partial");
    let midi_errors_before = counter("midi_errors");
    let (engine, mut rt) = Engine::headless_for_test(44100, 256);
    rt.master = 0.0;
    let mut model = Model::default();
    model.next_id = 4;
    model.ports.extend([
        Port {
            id: 2,
            alias: "NS7 stereo input".into(),
            direction: Direction::Input,
            channels: vec![0, 1],
        },
        Port {
            id: 3,
            alias: "NS7 capture".into(),
            direction: Direction::Record,
            channels: vec![0, 1],
        },
    ]);
    model.connections.push(Connection {
        source: Source {
            group: Group::Input(2),
            tap: Tap::PostFx,
        },
        destination: Group::Record(3),
        map: vec![
            ChannelMap {
                source: 0,
                destination: 0,
                gain: 1.0,
            },
            ChannelMap {
                source: 1,
                destination: 1,
                gain: 1.0,
            },
        ],
    });
    rt.routing = Some(Box::new(
        Prepared::new(Arc::new(model), &rt.session).unwrap(),
    ));
    let settings = crate::preferences::Audio {
        backend: Some("ALSA".into()),
        device: Some("sysdefault:CARD=NS7".into()),
        sample_rate: Some(44100),
        channels: Some(4),
        format: Some(crate::preferences::AudioFormat::I32),
        buffer_frames: Some(2048),
        ..Default::default()
    };
    let audio = audio::start_with_settings(rt, &settings).unwrap();
    let midi =
        midi::MidiHub::start_with_policy(engine.cmd.clone(), engine.snap.clone(), stage_inputs())
            .unwrap();
    wait(|| {
        engine.cmd.audio_metrics().callbacks > 20 && counter("midi_output_bytes") > output_before
    });
    let status = audio.handle.status();
    let output = &status.active.as_ref().unwrap().plan;
    let saved = InputConfig {
        backend: output.backend.clone(),
        device: output.device.clone(),
        channels: 2,
        format: crate::preferences::AudioFormat::I32,
        buffer_frames: Some(2048),
    };
    let plan = input::preview(&saved, &audio::config::discover().unwrap(), output).unwrap();
    audio
        .input
        .apply(
            Some(plan),
            status.generation,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    wait(|| engine.routing.shared.captured.load(Ordering::Relaxed) > 44100);
    let recorder = engine.routing.recorder.clone();
    let epoch = recorder.epoch();
    let destination = directory.join("ns7-input.wav");
    let capture = std::thread::spawn(move || {
        recorder.write(
            3,
            2,
            44100,
            10,
            &destination,
            &AtomicBool::new(false),
            epoch,
        )
    });
    wait(|| engine.routing.recorder.alias() == 3);
    std::thread::sleep(Duration::from_secs(5));
    engine.routing.recorder.stop();
    capture.join().unwrap().unwrap();
    let decoded = crate::engine::decode::decode_audio(&directory.join("ns7-input.wav")).unwrap();
    assert_eq!(decoded.sample.ch, 2);
    assert!(decoded.sample.data.len() >= 44100 * 2 * 4);
    assert!(decoded.sample.data.iter().all(|value| value.is_finite()));
    let metrics = engine.cmd.audio_metrics();
    let snapshot = engine.snap.lock().clone();
    let input_status = audio.input.status().message.clone();
    drop(audio);
    audio::owner::finish_shutdown();
    let pcm_closed = || {
        ["pcm0p", "pcm0c"].iter().all(|direction| {
            std::fs::read_to_string(format!("/proc/asound/NS7/{direction}/sub0/status"))
                .is_ok_and(|status| status.trim() == "closed")
        })
    };
    wait(pcm_closed);
    let silent_before = counter("pcm_playback_frames");
    wait(|| counter("pcm_playback_frames") > silent_before + 4410);
    let receipt = serde_json::json!({
        "output":format!("{:?}",output),"input_status":input_status,"audio":metrics,
        "input_captured_frames":engine.routing.shared.captured.load(Ordering::Relaxed),
        "input_overflow":engine.routing.shared.overflow.load(Ordering::Relaxed),
        "input_underrun":engine.routing.shared.underrun.load(Ordering::Relaxed),
        "recorded_frames":decoded.sample.data.len()/2,"midi_input":snapshot.midi_input,"midi_feedback":snapshot.midi_feedback,"monitor":snapshot.monitor,
        "ns7_midi_output_bytes":counter("midi_output_bytes")-output_before,
        "ns7_midi_input_bytes":counter("midi_input_bytes")-input_before,"ns7_midi_errors":counter("midi_errors")-midi_errors_before,
        "ns7_pcm_playback_frames":counter("pcm_playback_frames")-playback_before,"ns7_pcm_capture_frames":counter("pcm_capture_frames")-capture_before,
        "ns7_pcm_feedback_frames":counter("pcm_feedback_frames")-feedback_before,"ns7_pcm_errors":counter("pcm_errors")-pcm_errors_before,
        "ns7_pcm_startup_partial":counter("pcm_feedback_partial")-partial_before,
        "midi_clock_without_pcm":{"playback_frames":counter("pcm_playback_frames")-silent_before,"pcm_closed":pcm_closed()}
    });
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(metrics.backend_errors, 0, "{receipt}");
    assert!(snapshot.monitor.available, "{receipt}");
    assert_eq!(receipt["ns7_pcm_errors"], 0, "{receipt}");
    assert!(
        !engine.routing.shared.fault.load(Ordering::Acquire),
        "{receipt}"
    );
    drop(midi);
}

#[test]
#[ignore = "Requires NS7 headphones, muted stage amps, and OMATAINER_NS7_LISTEN_DIR on /home"]
fn original_ns7_headphone_listening() {
    use crate::engine::{monitor::Control, Command, Sample};
    let directory =
        PathBuf::from(std::env::var_os("OMATAINER_NS7_LISTEN_DIR").expect("listening directory"));
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !directory.join("stop").exists(),
        "use a fresh listening directory"
    );
    let (engine, mut rt) = Engine::headless_for_test(44100, 256);
    rt.apply(Command::Stop);
    rt.master = 0.0;
    rt.fx_wet = [0.0; 3];
    for (slot, deck) in rt.decks.iter_mut().enumerate() {
        let data = (0..88200)
            .flat_map(|frame| {
                let time = frame as f32 / 44100.0;
                let phase = time.fract();
                let envelope = (phase / 0.005)
                    .min(1.0)
                    .min(((0.4 - phase) / 0.005).clamp(0.0, 1.0));
                let sample =
                    0.02 * envelope * (std::f32::consts::TAU * [440.0, 660.0][slot] * time).sin();
                [sample; 2]
            })
            .collect();
        deck.audio = Some(Arc::new(Sample {
            name: format!("Quiet headphone check {}", slot + 1),
            sr: 44100,
            ch: 2,
            data,
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }));
        deck.playing = true;
        deck.sync = false;
        deck.keylock = false;
        deck.gain = 1.0;
        deck.loop_on = true;
        deck.loop_start = 0.0;
        deck.loop_len = 88200.0;
        rt.monitor.apply(Control::Fader {
            deck: slot as u8,
            value: 0.0,
        });
    }
    rt.monitor.apply(Control::Mix(0.5));
    rt.monitor.apply(Control::Master(false));
    rt.monitor.apply(Control::Volume(0.5));
    let settings = crate::preferences::Audio {
        backend: Some("ALSA".into()),
        device: Some("sysdefault:CARD=NS7".into()),
        sample_rate: Some(44100),
        channels: Some(4),
        format: Some(crate::preferences::AudioFormat::I32),
        buffer_frames: Some(2048),
        ..Default::default()
    };
    let midi =
        midi::MidiHub::start_with_policy(engine.cmd.clone(), engine.snap.clone(), stage_inputs())
            .unwrap();
    let audio = audio::start_with_settings(rt, &settings).unwrap();
    wait(|| engine.cmd.audio_metrics().callbacks > 20);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(180) && !directory.join("stop").exists() {
        let snapshot = engine.snap.lock().clone();
        let receipt = serde_json::json!({"elapsed_seconds":started.elapsed().as_secs_f32(),"source_peak":0.02,
            "audience_gain":0.0,"frequencies_hz":[440,660],"monitor":snapshot.monitor,
            "audio":engine.cmd.audio_metrics(),"midi_input":snapshot.midi_input,"midi_feedback":snapshot.midi_feedback});
        std::fs::write(
            directory.join("status.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(250));
    }
    let metrics = engine.cmd.audio_metrics();
    drop(midi);
    drop(audio);
    audio::owner::finish_shutdown();
    assert_eq!(metrics.backend_errors, 0);
}
