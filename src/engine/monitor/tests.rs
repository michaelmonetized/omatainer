use super::*;
use crate::engine::{audio, Command, Engine, RtEngine, Sample};
use std::sync::Arc;

fn engine() -> RtEngine {
    let (_, mut rt) = Engine::headless_for_test(48_000, 64);
    rt.apply(Command::Stop);
    rt.master = 0.5;
    rt.xfader = 1.0;
    rt.fx_wet = [0.0; 3];
    for (slot, deck) in rt.decks.iter_mut().enumerate() {
        deck.audio = Some(Arc::new(Sample {
            name: "monitor reference".into(),
            sr: 48_000,
            ch: 2,
            data: [0.1 + slot as f32 * 0.2, 0.2 + slot as f32 * 0.2].repeat(24_000),
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }));
        deck.playing = true;
        deck.sync = false;
        deck.keylock = false;
        deck.gain = 1.0;
    }
    rt.monitor.status.available = true;
    rt
}

fn control(rt: &mut RtEngine, value: Control) {
    rt.apply(Command::Monitor(value));
}

fn block(rt: &mut RtEngine) -> Vec<f32> {
    let mut output = vec![0.0; 4096 * 4];
    rt.process_interleaved(&mut output, 4);
    output
}

#[test]
fn headphone_fader_never_changes_the_audience_pair() {
    let mut actual = engine();
    let mut reference = engine();
    control(&mut actual, Control::Volume(0.5));
    for mix in [0.0, 1.0, 0.5] {
        control(&mut actual, Control::Mix(mix));
        let a = block(&mut actual);
        let b = block(&mut reference);
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            assert_eq!(a[..2], b[..2]);
        }
        let last = &a[a.len() - 4..];
        let expected = if mix == 0.0 {
            [0.05, 0.10]
        } else if mix == 1.0 {
            [0.15, 0.20]
        } else {
            [0.10, 0.15]
        };
        for channel in 0..2 {
            assert!(
                (last[channel + 2] - f32::tanh(expected[channel])).abs() < 0.0002,
                "mix {mix}: {last:?}"
            );
        }
    }
}

#[test]
fn closed_channel_remains_audible_in_headphones_after_trim_changes() {
    let mut rt = engine();
    control(
        &mut rt,
        Control::Fader {
            deck: 0,
            value: 0.0,
        },
    );
    control(
        &mut rt,
        Control::Fader {
            deck: 1,
            value: 0.0,
        },
    );
    control(&mut rt, Control::Mix(0.0));
    control(&mut rt, Control::Volume(0.5));
    rt.apply(Command::DeckGain {
        deck: 0,
        value: 1.5,
    });
    let output = block(&mut rt);
    let last = &output[output.len() - 4..];
    assert_eq!(last[..2], [0.0; 2]);
    assert!(
        (last[2] - 0.075).abs() < 0.002 && (last[3] - 0.15).abs() < 0.002,
        "{last:?}"
    );
    assert_eq!(rt.monitor.status.faders, [0.0; 2]);
}

#[test]
fn master_monitor_matches_the_master_without_opening_closed_channels() {
    let mut rt = engine();
    control(&mut rt, Control::Mix(0.0));
    control(&mut rt, Control::Volume(1.0));
    control(&mut rt, Control::Master(true));
    let output = block(&mut rt);
    for frame in output.chunks_exact(4).skip(1024) {
        assert_eq!(frame[..2], frame[2..]);
    }
    control(
        &mut rt,
        Control::Fader {
            deck: 1,
            value: 0.0,
        },
    );
    let output = block(&mut rt);
    assert_eq!(&output[output.len() - 4..], &[0.0; 4]);
}

#[test]
fn monitor_refuses_other_outputs_and_nonfinite_controls() {
    let mut rt = engine();
    let original = rt.monitor.status;
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        control(&mut rt, Control::Mix(value));
        control(&mut rt, Control::Volume(value));
        control(&mut rt, Control::Fader { deck: 0, value });
    }
    assert_eq!(rt.monitor.status, original);
    control(&mut rt, Control::Volume(1.0));
    for (name, backend, channels, expected) in [
        ("sysdefault:CARD=NS7", "ALSA", 4, true),
        ("hw:CARD=NS7,DEV=0", "ALSA", 4, true),
        ("sysdefault:CARD=NS7II", "ALSA", 4, false),
        ("sysdefault:CARD=NS7", "JACK", 4, false),
        ("sysdefault:CARD=NS7", "ALSA", 2, false),
    ] {
        rt.monitor.output(&audio::config::Plan {
            backend: backend.into(),
            device: name.into(),
            channels,
            rate: 48000,
            format: cpal::SampleFormat::F32,
            buffer: Some(2048),
            warning: None,
            graph: Default::default(),
        });
        assert_eq!(rt.monitor.status.available, expected);
        let output = block(&mut rt);
        assert_eq!(
            output.chunks_exact(4).any(|frame| frame[2] != 0.0),
            expected
        );
    }
}

#[test]
fn explicit_output_aliases_keep_their_channels() {
    use audio::routing::{model::*, prepared::Prepared};
    let mut rt = engine();
    control(&mut rt, Control::Mix(1.0));
    control(&mut rt, Control::Volume(0.5));
    let mut model = Model::default();
    model.next_id = 3;
    model.ports.push(Port {
        id: 2,
        alias: "Explicit pair".into(),
        direction: Direction::Output,
        channels: vec![2, 3],
    });
    model.connections.push(Connection {
        source: Source {
            group: Group::Deck(0),
            tap: Tap::PostFx,
        },
        destination: Group::Output(2),
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
    let output = block(&mut rt);
    let last = &output[output.len() - 4..];
    assert!(
        (last[2] - 0.1_f32.tanh()).abs() < 0.0002 && (last[3] - 0.2_f32.tanh()).abs() < 0.0002,
        "{last:?}"
    );
}

#[test]
fn emergency_silence_mutes_the_monitor_and_warm_rendering_has_no_heap_work() {
    let mut rt = engine();
    control(&mut rt, Control::Mix(0.0));
    control(&mut rt, Control::Volume(1.0));
    block(&mut rt);
    let mut output = [0.0; 1024];
    rt.frames_done = 0;
    let counts = crate::engine::test_alloc::measure(|| rt.process_interleaved(&mut output, 4));
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    rt.apply(Command::SafetyStop(
        crate::engine::performance::Safety::Silence,
    ));
    rt.process_interleaved(&mut output, 4);
    assert_eq!(&output[output.len() - 4..], &[0.0; 4]);
}
