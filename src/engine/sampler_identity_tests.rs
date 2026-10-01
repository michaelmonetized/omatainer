use super::*;
use dsp::{Env, Svf, VoiceOwner};

pub(crate) fn prepare(rt: &mut RtEngine) {
    rt.apply(Command::Stop);
    for deck in 0..DECKS {
        rt.apply(Command::DeckUnload { deck: deck as u8 });
    }
    for track in &mut rt.tracks {
        track.playing = None;
        track.gain = 1.0;
        track.pan = 0.0;
        track.mute = false;
        track.solo = false;
        track.fx.slots.clear();
    }
    for chain in &mut rt.scene_fx {
        chain.slots.clear();
    }
    rt.fx_wet = [0.0; 3];
    rt.master = 1.0;
    rt.cue_mix = 0.0;
    rt.selected_track = 4;
    rt.selected_scene = 0;
}

fn specification(kind: SynthInstrument) -> ([f32; 4], f32) {
    // Independent expected instrument contract, not a call to kind.adsr().
    match kind {
        SynthInstrument::Analog => ([0.005, 0.18, 0.35, 0.12], 700.0),
        SynthInstrument::Keys => ([0.008, 0.22, 0.45, 0.28], 1800.0),
        SynthInstrument::Pad => ([0.04, 0.4, 0.7, 0.8], 1800.0),
    }
}

/// Used after real UI menu selection as well as rate-specific engine tests.
pub(crate) fn assert_selected_sound(
    rt: &mut RtEngine,
    selected: SamplerInstrument,
) -> Vec<[f32; 2]> {
    assert_eq!(rt.sampler_inst, selected);
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let sr = rt.sr;
    let (adsr, cutoff) = selected
        .synth()
        .map(specification)
        .unwrap_or(([0.005, 0.18, 0.35, 0.12], 700.0));
    let mut envelope = Env::adsr(sr, adsr[0], adsr[1], adsr[2], adsr[3]);
    envelope.on();
    if let Some(kind) = selected.synth() {
        assert_eq!(rt.sampler_poly.kind, kind);
        let active: Vec<_> = rt
            .sampler_poly
            .voices
            .iter()
            .filter(|voice| voice.env.active())
            .collect();
        assert_eq!(active.len(), 1);
        let voice = active[0];
        assert_eq!(voice.kind, kind);
        assert_eq!(voice.note(), 57);
        assert_eq!(voice.input, Some(InputKey::Pad(0)));
        assert_eq!(voice.owner, VoiceOwner::Live);
        assert_eq!(
            [voice.env.att, voice.env.dec, voice.env.sus, voice.env.rel],
            [envelope.att, envelope.dec, envelope.sus, envelope.rel]
        );
        assert_eq!(voice.cutoff, cutoff);
        assert_eq!(rt.pad_destinations[0], 4);
    } else {
        assert!(rt
            .sampler_poly
            .voices
            .iter()
            .all(|voice| !voice.env.active()));
        let voice = rt.pad_voices[0].as_ref().unwrap();
        assert!(Arc::ptr_eq(&voice.audio, &rt.sampler_banks[0].data.audio[0].as_ref().unwrap()));
        assert_eq!(voice.track, 4);
    }
    let sample = rt.sampler_banks[0].data.audio[0].as_ref().unwrap().clone();
    let mut filter = Svf::default();
    let (mut phase, mut detuned) = (0.0f32, 0.0f32);
    let increment = 220.0 / sr; // pad A at octave 3: MIDI 57
    let mut output = Vec::new();
    for frame in 0..8192 {
        let expected = if let Some(kind) = selected.synth() {
            phase = (phase + increment) % 1.0;
            detuned = (detuned + increment * 0.997) % 1.0;
            let saw = phase * 2.0 - 1.0;
            let square = if phase < 0.5 { 0.7 } else { -0.7 };
            let sine = (phase * std::f32::consts::TAU).sin();
            let oscillator = match kind {
                SynthInstrument::Analog => saw * 0.7 + square * 0.3,
                SynthInstrument::Keys => saw * 0.35 + sine * 0.65,
                SynthInstrument::Pad => sine * 0.6 + (detuned * 2.0 - 1.0) * 0.4,
            };
            let level = envelope.tick();
            let y = filter.process(
                oscillator * level * 0.9,
                (cutoff + level * 1800.0).clamp(80.0, sr * 0.42),
                0.35,
                sr,
                0.0,
            ) * 0.35;
            [y.tanh(); 2]
        } else {
            let (l, r) = sample.at(frame as f64 * sample.sr as f64 / sr as f64);
            [l.tanh(), r.tanh()]
        };
        let mut actual = [0.0; 2];
        rt.process(&mut actual);
        for channel in 0..2 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 2e-6,
                "{} sr={sr} frame={frame} ch={channel}: {} != {}",
                selected.label(),
                actual[channel],
                expected[channel]
            );
        }
        output.push(actual);
    }
    assert!(output.iter().flatten().any(|sample| sample.abs() > 0.001));
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    if selected.synth().is_some() {
        assert!(rt
            .sampler_poly
            .voices
            .iter()
            .filter(|voice| voice.input == Some(InputKey::Pad(0)))
            .all(|voice| voice.env.stage == 4));
    } else {
        assert!(
            rt.pad_voices[0].is_some(),
            "sample release must preserve the one-shot tail"
        );
    }
    output
}

fn engine(sr: u32) -> RtEngine {
    let (_commands, receiver) = CommandPort::channel(256);
    let mut rt = RtEngine::new(
        sr as f32,
        receiver,
        Arc::new(Mutex::new(Snapshot::default())),
    );
    prepare(&mut rt);
    rt
}

#[test]
fn every_typed_source_matches_its_envelope_and_distinct_rendered_sound_at_each_rate() {
    for sr in [44_100, 48_000, 96_000] {
        let mut sounds = Vec::new();
        for selected in SamplerInstrument::ALL {
            let mut rt = engine(sr);
            rt.apply(Command::SamplerInst(selected));
            sounds.push(assert_selected_sound(&mut rt, selected));
        }
        for left in 0..sounds.len() {
            for right in left + 1..sounds.len() {
                let difference: f32 = sounds[left]
                    .iter()
                    .zip(&sounds[right])
                    .map(|(a, b)| (a[0] - b[0]).abs())
                    .sum();
                assert!(
                    difference > 10.0,
                    "aliased menu sounds {left}/{right} at {sr}"
                );
            }
        }
    }
}

#[test]
fn held_inputs_retain_original_instrument_envelope_cutoff_and_owner_across_selection_changes() {
    let mut rt = engine(48_000);
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Analog,
    )));
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    rt.process(&mut [0.0; 128]);
    let old = *rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    rt.apply(Command::Select { track: 2, scene: 1 });
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Keys,
    )));
    let retained = *rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    assert_eq!(
        (
            retained.kind,
            retained.cutoff,
            retained.env.stage,
            retained.env.level,
            retained.phase
        ),
        (
            old.kind,
            old.cutoff,
            old.env.stage,
            old.env.level,
            old.phase
        )
    );
    rt.apply(Command::SamplerPad { pad: 1, on: true });
    rt.apply(Command::SamplerInst(SamplerInstrument::Samples));
    rt.apply(Command::SamplerBank(2));
    rt.apply(Command::SamplerPad { pad: 2, on: true });
    let sample = rt.pad_voices[2].as_ref().unwrap().audio.clone();
    assert!(Arc::ptr_eq(&sample, &rt.sampler_banks[2].data.audio[2].as_ref().unwrap()));
    rt.apply(Command::SamplerOct(1));
    assert_eq!(rt.pad_targets[0].unwrap().track, 4);
    assert_eq!(rt.pad_targets[1].unwrap().track, 2);
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Pad,
    )));
    rt.apply(Command::SamplerPad { pad: 3, on: true });
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    for (pad, kind, held) in [
        (0, SynthInstrument::Analog, false),
        (1, SynthInstrument::Keys, true),
        (3, SynthInstrument::Pad, true),
    ] {
        let voice = rt
            .sampler_poly
            .voices
            .iter()
            .find(|voice| voice.input == Some(InputKey::Pad(pad)))
            .unwrap();
        assert_eq!(voice.kind, kind);
        assert_eq!(voice.cutoff, specification(kind).1);
        assert_eq!(matches!(voice.env.stage, 1..=3), held);
    }
    rt.apply(Command::SamplerPad { pad: 1, on: false });
    rt.apply(Command::SamplerPad { pad: 3, on: false });
    rt.apply(Command::SamplerPad { pad: 2, on: false });
    assert!(Arc::ptr_eq(&rt.pad_voices[2].as_ref().unwrap().audio, &sample));
    for _ in 0..48_000 {
        rt.tick_pad_sources();
    }
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .all(|voice| !voice.env.active()));
    assert!(rt.pad_voices.iter().all(Option::is_none));
}

#[test]
fn selecting_instruments_and_reusing_synth_voices_has_no_heap_traffic() {
    let mut rt = engine(48_000);
    let counts = test_alloc::measure(|| {
        for _ in 0..32 {
            for kind in SynthInstrument::ALL {
                rt.apply(Command::SamplerInst(SamplerInstrument::Synth(kind)));
                rt.apply(Command::SamplerPad { pad: 0, on: true });
                rt.tick_pad_sources();
                rt.apply(Command::SamplerPad { pad: 0, on: false });
            }
            rt.apply(Command::SamplerInst(SamplerInstrument::Samples));
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
}
