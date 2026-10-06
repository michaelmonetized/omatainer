use super::control::{Control, Parameter, Request};
use super::*;
use crate::engine::{
    audio::{
        routing::{input::Pipe, model::*, prepared::Prepared, record::delivery},
        OutputCallback,
    },
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};

fn link(destination: Group, tap: Tap) -> Connection {
    Connection {
        source: Source {
            group: Group::Main,
            tap,
        },
        destination,
        map: (0..2)
            .map(|ch| ChannelMap {
                source: ch,
                destination: ch,
                gain: 1.0,
            })
            .collect(),
    }
}
pub(crate) fn model() -> Model {
    let mut model = Model::default();
    model.version = 2;
    model.next_id = 7;
    for (id, name, direction, channels) in [
        (2, "Mic", Direction::Input, vec![0]),
        (3, "Aux", Direction::Input, vec![2, 1]),
        (4, "Booth", Direction::Output, vec![2, 3]),
        (5, "Record mix", Direction::Record, vec![0, 1]),
        (6, "Headphones", Direction::Output, vec![4, 5]),
    ] {
        model.ports.push(Port {
            id,
            alias: name.into(),
            direction,
            channels,
        });
    }
    model.monitor_output = Some(6);
    model.connections.extend([
        link(Group::Output(4), Tap::PostFx),
        link(Group::Record(5), Tap::PostMixer),
    ]);
    model
}
pub(crate) fn configuration() -> Configuration {
    Configuration {
        channels: [
            Channel {
                input: Some(2),
                mute: false,
                master: Some(1),
                ..Default::default()
            },
            Channel {
                input: Some(3),
                gain: 2.0,
                mute: false,
                master: Some(1),
                booth: Some(4),
                record: Some(5),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
fn engine() -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.fx_wet.fill(0.0);
    rt.master = 0.5;
    for t in &mut rt.tracks {
        t.fx.slots.clear();
    }
    for c in &mut rt.scene_fx {
        c.slots.clear();
    }
    rt.routing = Some(Box::new(
        Prepared::new(Arc::new(model()), &rt.session).unwrap(),
    ));
    rt.routing_pipe = Pipe::controlled_for_test(48000);
    (engine, rt)
}
fn prime(rt: &mut RtEngine) {
    for _ in 0..4 {
        rt.routing_pipe
            .capture(&[0.2_f32, -0.3, 0.1].repeat(64), 3, 0);
    }
    for _ in 0..12 {
        rt.routing_pipe
            .capture(&[0.2_f32, -0.3, 0.1].repeat(64), 3, 0);
        rt.process_interleaved(&mut [0.0; 64 * 6], 6);
    }
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> crate::engine::project::Captured {
    let p = engine.project.clone();
    let j = std::thread::spawn(move || p.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !j.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    j.join().unwrap()
}
#[test]
fn selected_inputs_reach_independent_master_booth_record_and_master_headphones_without_callback_heap(
) {
    let (_, mut rt) = engine();
    let cfg = configuration();
    cfg.validate(rt.routing.as_ref().map(|r| r.model.as_ref()))
        .unwrap();
    rt.mic_aux.set(Some(cfg));
    prime(&mut rt);
    rt.apply(Command::Monitor(crate::engine::monitor::Control::Master(
        true,
    )));
    rt.apply(Command::Monitor(crate::engine::monitor::Control::Volume(
        1.0,
    )));
    for _ in 0..8 {
        rt.routing_pipe
            .capture(&[0.2_f32, -0.3, 0.1].repeat(64), 3, 0);
        rt.process_interleaved(&mut [0.0; 64 * 6], 6);
    }
    let mut callback = OutputCallback::new(rt, 6);
    let mut out = [0.0_f32; 64 * 6];
    let source = [0.2_f32, -0.3, 0.1].repeat(64);
    callback
        .renderer_mut_for_test()
        .routing_pipe
        .capture(&source, 3, 0);
    assert_eq!(
        test_alloc::measure(|| callback.render(&mut out)),
        test_alloc::Counts::default()
    );
    for frame in out.chunks_exact(6) {
        for (value, expected) in frame.iter().zip([0.2, -0.2, 0.2, -0.6, 0.2, -0.2]) {
            assert!(
                (*value - crate::engine::limiter(expected)).abs() < 0.00001,
                "{frame:?} vs {expected}"
            );
        }
    }
    let rt = callback.renderer_mut_for_test();
    let record = rt
        .routing
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.group == Group::Record(5))
        .unwrap();
    assert!((record.input[0] - 0.1).abs() < 0.000001);
    assert!((record.input[1] + 0.3).abs() < 0.000001);
    let status = rt.mic_aux.status();
    assert!(status.meters.iter().all(|m| m.available));
    assert!((status.meters[0].input_peak - 0.2).abs() < 0.00001);
    assert!((status.meters[1].output_peak - 0.6).abs() < 0.00001);
    rt.apply(Command::Monitor(crate::engine::monitor::Control::Master(
        false,
    )));
    rt.apply(Command::Monitor(crate::engine::monitor::Control::Source(
        crate::engine::monitor::Source::Pfl,
    )));
    for _ in 0..8 {
        rt.routing_pipe.capture(&source, 3, 0);
        rt.process_interleaved(&mut out, 6);
    }
    assert!(out
        .chunks_exact(6)
        .all(|f| f[4].abs() < 0.000001 && f[5].abs() < 0.000001));
}
#[test]
fn source_and_destination_changes_fade_out_before_fading_in_and_loss_silences() {
    let mut cfg = configuration();
    cfg.channels[1] = Channel::default();
    let mut m = Mixer::new(Some(cfg), 48000.0);
    let mut values = Vec::with_capacity(480);
    let sample = |m: &mut Mixer, alias, value, complete| {
        m.begin();
        let mut raw = [0.0; MAX_PORT_CHANNELS];
        raw[0] = value;
        m.feed(alias, raw, 1, complete);
        m.music_gain();
        let mut out = [0.0; MAX_PORT_CHANNELS];
        let mut valid = true;
        m.add(Group::Output(1), 2, &mut out, &mut valid, 1.0);
        out[0]
    };
    for _ in 0..480 {
        sample(&mut m, 2, 0.2, true);
    }
    assert_eq!(sample(&mut m, 2, 0.2, true), 0.2);
    cfg.channels[0].input = Some(7);
    m.set(Some(cfg));
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..480 {
                m.begin();
                let mut a = [0.0; MAX_PORT_CHANNELS];
                a[0] = 0.2;
                m.feed(2, a, 1, true);
                a[0] = -0.4;
                m.feed(7, a, 1, true);
                m.music_gain();
                let mut out = [0.0; MAX_PORT_CHANNELS];
                m.add(Group::Output(1), 2, &mut out, &mut true, 1.0);
                values.push(out[0]);
            }
        }),
        test_alloc::Counts::default()
    );
    assert!(values[..240].iter().all(|v| *v >= 0.0));
    assert_eq!(values[239], 0.0);
    assert_eq!(values[479], -0.4);
    for pair in values.windows(2) {
        assert!((pair[1] - pair[0]).abs() < 0.0035);
    }
    let first = sample(&mut m, 7, 0.0, false);
    assert!(first < 0.0 && first > -0.4);
    for _ in 0..240 {
        sample(&mut m, 7, 0.0, false);
    }
    assert_eq!(sample(&mut m, 7, 0.0, false), 0.0);
    assert!(!m.status().meters[0].available);
    m.set(None);
    for _ in 0..240 {
        sample(&mut m, 7, 0.0, false);
    }
    assert_eq!(m.configuration(), None);
}
#[test]
fn talkover_envelopes_manual_override_tone_and_rate_preparation_are_finite_and_measured() {
    let mut cfg = configuration();
    cfg.channels[1] = Channel::default();
    cfg.duck.enabled = true;
    cfg.duck.attack_ms = 20.0;
    cfg.duck.release_ms = 250.0;
    cfg.duck.reduction_db = 12.0;
    for rate in [44100.0, 48000.0, 96000.0] {
        let mut m = Mixer::new(Some(cfg), rate);
        for _ in 0..(rate * 0.01) as usize {
            m.begin();
            let mut raw = [0.0; MAX_PORT_CHANNELS];
            raw[0] = 0.1;
            m.feed(2, raw, 1, true);
        }
        let initial = m.duck;
        for _ in 0..(rate * 0.020) as usize {
            m.begin();
            let mut raw = [0.0; MAX_PORT_CHANNELS];
            raw[0] = 0.1;
            m.feed(2, raw, 1, true);
            m.music_gain();
        }
        let reduction = 10_f32.powf(-12.0 / 20.0);
        let expected = reduction + (initial - reduction) * (-1.0_f32).exp();
        assert!((m.duck - expected).abs() < 0.0005, "{} {expected}", m.duck);
        let start = m.duck;
        let mut next = cfg;
        next.duck.mode = Override::Off;
        m.set(Some(next));
        for _ in 0..(rate * 0.250) as usize {
            m.begin();
            let mut raw = [0.0; MAX_PORT_CHANNELS];
            raw[0] = 0.1;
            m.feed(2, raw, 1, true);
            m.music_gain();
        }
        assert!((m.duck - (1.0 + (start - 1.0) * (-1.0_f32).exp())).abs() < 0.0006);
        next.duck.mode = Override::Held;
        next.channels[0].mute = true;
        m.set(Some(next));
        for _ in 0..(rate * 0.2) as usize {
            m.begin();
            m.music_gain();
        }
        assert!((m.duck - reduction).abs() < 0.0001);
        next.duck.enabled = false;
        m.set(Some(next));
        for _ in 0..rate as usize {
            m.begin();
            m.music_gain();
        }
        assert!(m.duck > 0.98);
        m.set_sample_rate(96000.0);
        assert_eq!(m.configuration(), Some(next));
        assert_eq!(m.status().duck_gain, 1.0);
    }
    cfg.duck.enabled = false;
    cfg.channels[0].tone = true;
    cfg.channels[0].eq_db = [6.0, 0.0, -6.0];
    let mut m = Mixer::new(Some(cfg), 48000.0);
    let mut rms = [0.0_f64; 2];
    for (index, hz) in [100.0, 12000.0].into_iter().enumerate() {
        for i in 0..48000 {
            m.begin();
            let mut frame = [0.0; MAX_PORT_CHANNELS];
            frame[0] = (i as f32 * std::f32::consts::TAU * hz / 48000.0).sin() * 0.1;
            m.feed(2, frame, 1, true);
            if i >= 24000 {
                rms[index] += f64::from(m.frames[0][0]).powi(2);
            }
        }
    }
    assert!(rms[0] / rms[1] > 4.0, "{rms:?}");
    m.begin();
    m.feed(2, [f32::NAN; MAX_PORT_CHANNELS], 1, true);
    assert!(m.frames[0].iter().all(|s| s.is_finite()));
    assert!(!m.status().meters[0].available);
}
#[test]
fn reviewed_controls_save_reopen_undo_and_refuse_stale_aliases_protected_changes_and_invalid_values(
) {
    let (engine, mut rt) = engine();
    let captured = capture(&engine, &mut rt);
    let cfg = configuration();
    let (request, ack) = Request::new(&captured, 48000, Some(cfg)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::MicAuxConfigure(request))),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
    let saved = capture(&engine, &mut rt);
    let json = serde_json::to_value(&saved.state).unwrap();
    assert_eq!(json["version"], crate::engine::project::STATE_VERSION);
    assert_eq!(json["mic_aux"]["channels"][1]["gain"], 2.0);
    let mut legacy = json.clone();
    legacy["version"] = 16.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(legacy).is_err());
    let state = serde_json::from_value(json).unwrap();
    let reopened = crate::engine::project::Prepared::from_state(state, saved.media.clone(), 96000)
        .unwrap()
        .into_offline();
    assert_eq!(reopened.mic_aux.configuration(), Some(cfg));
    rt.apply(Command::Undo);
    assert_eq!(rt.mic_aux.configuration(), None);
    rt.apply(Command::Redo);
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
    rt.apply(Command::PerformanceMode(true));
    let control = Control {
        namespace: rt.session.namespace,
        role: 0,
        input: Some(2),
        parameter: Parameter::Gain(0.5),
    };
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::MicAuxControl(control))),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.mic_aux.configuration().unwrap().channels[0].gain, 0.5);
    let current = rt.mic_aux.configuration();
    rt.apply(Command::MicAuxControl(Control {
        input: Some(99),
        ..control
    }));
    rt.apply(Command::MicAuxControl(Control {
        parameter: Parameter::Gain(f32::NAN),
        ..control
    }));
    assert_eq!(rt.mic_aux.configuration(), current);
    let fresh = capture(&engine, &mut rt);
    let (request, ack) = Request::new(&fresh, 48000, None).unwrap();
    rt.apply(Command::MicAuxConfigure(request));
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Rejected);
    assert_eq!(rt.mic_aux.configuration(), current);
    rt.apply(Command::PerformanceMode(false));
    let (stale, ack) = Request::new(&fresh, 48000, None).unwrap();
    rt.apply(Command::Master(0.4));
    rt.apply(Command::MicAuxConfigure(stale));
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Rejected);
    let mut changed = model();
    changed.ports.retain(|p| p.id != 2);
    let fresh = capture(&engine, &mut rt);
    assert!(
        crate::engine::session::Request::routing(fresh, 48000, Some(Arc::new(changed))).is_err()
    );
    for change in 0..6 {
        let mut bad = cfg;
        match change {
            0 => bad.channels[0].input = Some(99),
            1 => bad.channels[0].master = Some(6),
            2 => bad.channels[1].input = Some(2),
            3 => bad.channels[0].booth = Some(1),
            4 => bad.duck.attack_ms = 0.0,
            _ => bad.channels[0].eq_db[2] = f32::INFINITY,
        }
        assert!(bad.validate(Some(&model())).is_err());
    }
}
#[test]
fn actual_input_callback_and_delivery_record_exact_program_and_independent_raw_voice_exclusion() {
    let (_, mut rt) = engine();
    rt.mic_aux.set(Some(configuration()));
    prime(&mut rt);
    let mut callback = OutputCallback::new(rt, 6);
    let recorder = callback
        .renderer_mut_for_test()
        .routing_pipe
        .recorder
        .clone();
    let root = std::env::temp_dir().join(format!(
        "omatainer-mic-aux-delivery-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    for raw in [false, true] {
        let q = delivery::Request {
            alias: if raw { 5 } else { 1 },
            output_channels: (!raw).then(|| vec![1, 0]),
            options: crate::audio_delivery::Options {
                format: crate::audio_delivery::Format::Float32,
                rate: 48000,
                channels: 2,
                dither: false,
                normalize: false,
            },
            seconds: 1,
            epoch: recorder.epoch(),
        };
        let r = recorder.clone();
        let path = root.join(if raw { "raw" } else { "program" });
        let work = std::thread::spawn(move || r.write_delivery(&q, &path, &AtomicBool::new(false)));
        let deadline = Instant::now() + Duration::from_secs(15);
        while recorder.alias() == 0 {
            callback.renderer_mut_for_test().process(&mut []);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut expected = Vec::new();
        let source = [0.2_f32, -0.3, 0.1].repeat(64);
        let mut out = [0.0_f32; 64 * 6];
        for _ in 0..8 {
            callback
                .renderer_mut_for_test()
                .routing_pipe
                .capture(&source, 3, 0);
            assert_eq!(
                test_alloc::measure(|| callback.render(&mut out)),
                test_alloc::Counts::default()
            );
            expected.extend(out.chunks_exact(6).flat_map(|f| [f[1], f[0]]));
        }
        recorder.stop();
        let result = work.join().unwrap().unwrap();
        assert_eq!(result.frames, 512);
        assert!(result.warning.is_none(), "{:?}", result.warning);
        let decoded = crate::engine::decode::decode_audio(&result.files[0])
            .unwrap()
            .sample;
        if raw {
            for frame in decoded.data.chunks_exact(2) {
                assert!((frame[0] - 0.1).abs() < 0.000001 && (frame[1] + 0.3).abs() < 0.000001);
            }
        } else {
            assert_eq!(decoded.data, expected);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn project_container_restore_and_selective_import_retain_current_external_source_roles() {
    let (engine, mut rt) = engine();
    let cfg = configuration();
    rt.mic_aux.set(Some(cfg));
    let saved = capture(&engine, &mut rt);
    let path = std::env::temp_dir().join(format!(
        "omatainer-mic-aux-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let bundle = crate::project_file::Bundle {
        state: saved.state,
        media: saved.media,
    };
    crate::project_file::save(
        &path,
        &bundle,
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut prepared =
        crate::engine::project::Prepared::from_state(reopened.state, reopened.media, 48000)
            .unwrap();
    rt.mic_aux.set(None);
    assert_eq!(
        test_alloc::measure(|| prepared.swap_into(&mut rt)),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
    std::fs::remove_file(path).unwrap();
    let (source_engine, mut source_rt) = self::engine();
    let mut foreign = cfg;
    foreign.channels[0].input = None;
    foreign.channels[0].master = None;
    foreign.channels[0].gain = 0.25;
    source_rt.mic_aux.set(Some(foreign));
    let source = capture(&source_engine, &mut source_rt);
    let current = capture(&engine, &mut rt);
    let layout = source.state.session.as_ref().unwrap();
    let selection = crate::engine::session::ImportSelection {
        tracks: vec![layout.tracks[0].id],
        scenes: vec![layout.scenes[0].id],
        clips: true,
        devices: true,
        keep_timing: true,
    };
    let (request, ack) = crate::engine::session::Request::import(
        current,
        &source.state,
        &source.media,
        &selection,
        48000,
    )
    .unwrap();
    rt.apply(Command::SessionEdit(request));
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
    rt.apply(Command::Undo);
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
    rt.apply(Command::Redo);
    assert_eq!(rt.mic_aux.configuration(), Some(cfg));
}
#[test]
fn missing_capture_channels_and_faulted_input_are_unavailable_and_fade_to_finite_silence() {
    let (_, mut rt) = engine();
    rt.mic_aux.set(Some(configuration()));
    let input = [0.2_f32; 64];
    for _ in 0..4 {
        rt.routing_pipe.capture(&input, 1, 0);
    }
    let mut output = [0.0_f32; 64 * 6];
    for _ in 0..12 {
        rt.routing_pipe.capture(&input, 1, 0);
        rt.process_interleaved(&mut output, 6);
    }
    let status = rt.mic_aux.status();
    assert!(status.meters[0].available);
    assert!(!status.meters[1].available);
    assert!(output
        .chunks_exact(6)
        .all(|f| (f[0] - crate::engine::limiter(0.1)).abs() < 0.000001
            && (f[1] - crate::engine::limiter(0.1)).abs() < 0.000001
            && f[2] == 0.0
            && f[3] == 0.0));
    rt.routing_pipe.capture(&[f32::NAN], 1, 0);
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..8 {
                rt.process_interleaved(&mut output, 6);
            }
        }),
        test_alloc::Counts::default()
    );
    assert!(output.iter().all(|v| v.is_finite() && *v == 0.0));
    assert!(rt.mic_aux.status().meters.iter().all(|m| !m.available));
}
