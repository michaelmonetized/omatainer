use super::*;
use crate::engine::{Command, CommandPort, RtEngine, SamplerInstrument, Snapshot};
use parking_lot::Mutex;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

/// Independent former rendering path: pitch exponential and detune multiply
/// are intentionally evaluated for every sample, never via cached increments.
#[derive(Clone)]
struct LegacyVoice {
    note: u8,
    tuning: f32,
    kind: SynthInstrument,
    velocity: f32,
    phase: f32,
    phase2: f32,
    envelope: Env,
    filter: Svf,
}

impl LegacyVoice {
    fn new(sr: f32, kind: SynthInstrument, note: u8) -> Self {
        let settings = match kind {
            SynthInstrument::Analog => [0.005, 0.18, 0.35, 0.12],
            SynthInstrument::Keys => [0.008, 0.22, 0.45, 0.28],
            SynthInstrument::Pad => [0.04, 0.4, 0.7, 0.8],
        };
        let mut envelope = Env::adsr(sr, settings[0], settings[1], settings[2], settings[3]);
        envelope.on();
        Self {
            note,
            tuning: 440.0,
            kind,
            velocity: 0.8,
            phase: 0.0,
            phase2: 0.0,
            envelope,
            filter: Svf::default(),
        }
    }

    fn tick(&mut self, sr: f32, cutoff: f32) -> f32 {
        if !self.envelope.active() {
            return 0.0;
        }
        let hz = self.tuning * 2f32.powf((self.note as f32 - 69.0) / 12.0);
        let increment = hz / sr;
        self.phase = (self.phase + increment) % 1.0;
        self.phase2 = (self.phase2 + increment * 0.997) % 1.0;
        let saw = self.phase * 2.0 - 1.0;
        let square = if self.phase < 0.5 { 0.7 } else { -0.7 };
        let sine = (self.phase * std::f32::consts::TAU).sin();
        let oscillator = match self.kind {
            SynthInstrument::Analog => saw * 0.7 + square * 0.3,
            SynthInstrument::Keys => saw * 0.35 + sine * 0.65,
            SynthInstrument::Pad => sine * 0.6 + (self.phase2 * 2.0 - 1.0) * 0.4,
        };
        let level = self.envelope.tick();
        let cutoff = (cutoff + level * 1800.0).clamp(80.0, sr * 0.42);
        self.filter
            .process(oscillator * level * self.velocity, cutoff, 0.35, sr, 0.0)
            * 0.35
    }
}

fn compare(
    actual: &mut Voice,
    filter: &mut Svf,
    reference: &mut LegacyVoice,
    sr: f32,
    count: usize,
) {
    for frame in 0..count {
        let expected = reference.tick(sr, 1700.0);
        let sample = actual.tick(sr, 1700.0, filter);
        assert_eq!(
            sample.to_bits(),
            expected.to_bits(),
            "kind={:?} note={} sr={sr} frame={frame}",
            actual.kind,
            actual.note()
        );
        assert_eq!(actual.phase.to_bits(), reference.phase.to_bits());
        assert_eq!(actual.phase2.to_bits(), reference.phase2.to_bits());
    }
}

#[test]
fn cached_pitch_is_bit_exact_to_legacy_waveforms_for_notes_tuning_and_rate_changes() {
    for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for kind in SynthInstrument::ALL {
            for note in [0, 12, 36, 57, 69, 84, 108, 127] {
                let mut voice = Voice::new(sr, kind);
                voice.trig(note, 0.8);
                let mut filter = Svf::default();
                let mut reference = LegacyVoice::new(sr, kind, note);
                compare(&mut voice, &mut filter, &mut reference, sr, 4096);
                // Tuning changes neither phase nor the held envelope.
                let before = (
                    voice.phase,
                    voice.phase2,
                    voice.env.stage,
                    voice.env.level,
                    voice.kind,
                );
                assert!(voice.set_tuning_hz(432.0));
                reference.tuning = 432.0;
                assert_eq!(
                    (
                        voice.phase,
                        voice.phase2,
                        voice.env.stage,
                        voice.env.level,
                        voice.kind
                    ),
                    before
                );
                compare(&mut voice, &mut filter, &mut reference, sr, 1024);
                // Direct Voice callers may change sample rate while held. The
                // pitch cache refreshes on that boundary without retriggering.
                compare(&mut voice, &mut filter, &mut reference, 88_200.0, 1024);
            }
        }
    }
}

#[test]
fn steady_voice_rendering_never_recomputes_pitch_and_frequency_matches_reference() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for note in 0..=127 {
            let mut voice = Voice::new(sr, SynthInstrument::Pad);
            voice.trig(note, 0.8);
            let expected = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
            assert_eq!(voice.phase_increment.to_bits(), (expected / sr).to_bits());
            assert_eq!(
                voice.detuned_increment.to_bits(),
                ((expected / sr) * 0.997).to_bits()
            );
            let updates = voice.pitch_updates;
            let mut filter = Svf::default();
            let mut wraps = 0u32;
            for _ in 0..sr as usize {
                let phase = voice.phase;
                voice.tick(sr, 1200.0, &mut filter);
                if voice.phase < phase {
                    wraps += 1;
                }
            }
            let measured = wraps as f64 + voice.phase as f64;
            assert!(
                (measured - expected as f64).abs() < 0.02,
                "note={note} sr={sr}: {measured} != {expected} Hz"
            );
            assert_eq!(voice.pitch_updates, updates, "samples recomputed pitch");
        }
    }
}

#[test]
fn held_transpose_and_retrigger_refresh_cache_without_resetting_instrument_or_phase() {
    let sr = 48_000.0;
    for kind in SynthInstrument::ALL {
        let mut poly = Poly::new(sr, kind, 8);
        let input = InputKey::Pad(0);
        poly.note_on_input(60, 0.8, input);
        let mut reference = LegacyVoice::new(sr, kind, 60);
        let mut filter = Svf::default();
        let voice = poly
            .voices
            .iter_mut()
            .find(|voice| voice.input == Some(input))
            .unwrap();
        compare(voice, &mut filter, &mut reference, sr, 1024);
        let before = (
            voice.phase,
            voice.phase2,
            voice.env.stage,
            voice.env.level,
            voice.kind,
            voice.pitch_updates,
        );
        poly.transpose_input(input, 12);
        reference.note = 72;
        let voice = poly
            .voices
            .iter_mut()
            .find(|voice| voice.input == Some(input))
            .unwrap();
        assert_eq!(
            (
                voice.phase,
                voice.phase2,
                voice.env.stage,
                voice.env.level,
                voice.kind
            ),
            (before.0, before.1, before.2, before.3, before.4)
        );
        assert_eq!(voice.pitch_updates, before.5 + 1);
        compare(voice, &mut filter, &mut reference, sr, 2048);
        let before = voice.pitch_updates;
        poly.note_on_input(72, 0.8, input);
        reference.envelope.on();
        let voice = poly
            .voices
            .iter_mut()
            .find(|voice| voice.input == Some(input))
            .unwrap();
        assert_eq!(voice.pitch_updates, before + 1);
        compare(voice, &mut filter, &mut reference, sr, 1024);
        poly.transpose_input(input, 127);
        assert_eq!(
            poly.voices
                .iter()
                .find(|voice| voice.input == Some(input))
                .unwrap()
                .note(),
            127
        );
        let count = poly
            .voices
            .iter()
            .find(|voice| voice.input == Some(input))
            .unwrap()
            .pitch_updates;
        poly.transpose_input(input, 127);
        assert_eq!(
            poly.voices
                .iter()
                .find(|voice| voice.input == Some(input))
                .unwrap()
                .pitch_updates,
            count,
            "clamped no-op recomputed pitch"
        );
    }
}

#[test]
fn actual_sampler_octave_tuning_and_rate_preparation_keep_pitch_cache_current() {
    let (_commands, receiver) = CommandPort::channel(256);
    let mut rt = RtEngine::new(
        48_000.0,
        receiver,
        Arc::new(Mutex::new(Snapshot::default())),
    );
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Analog,
    )));
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    rt.process(&mut [0.0; 128]);
    let before = *rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Pad,
    )));
    rt.apply(Command::SamplerOct(1));
    let after = rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    assert_eq!(after.kind, SynthInstrument::Analog);
    assert_eq!(
        (after.env.stage, after.env.level, after.phase),
        (before.env.stage, before.env.level, before.phase)
    );
    assert_eq!(after.phase_increment, before.phase_increment * 2.0);
    assert!(rt.sampler_poly.set_tuning_hz(442.0));
    let after = rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    assert_eq!(after.phase_increment, 442.0 / 48_000.0);
    for invalid in [f32::NAN, f32::INFINITY, 0.0, -440.0, 20_001.0] {
        assert!(!rt.sampler_poly.set_tuning_hz(invalid));
    }
    rt.apply(Command::SamplerPad { pad: 1, on: true });
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .filter(|voice| voice.env.active())
        .all(|voice| voice.tuning_hz == 442.0));
    rt.set_sample_rate(96_000).unwrap();
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let voice = rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    assert_eq!(voice.sample_rate, 96_000.0);
    assert_eq!(voice.tuning_hz, 442.0);
    assert_eq!(voice.phase_increment, 442.0 / 96_000.0);
    assert_eq!(voice.kind, SynthInstrument::Pad);
}

#[test]
#[ignore = "comparative dense-polyphony timing; run locally with --release --ignored --nocapture"]
fn dense_polyphony_pitch_cache_benchmark() {
    const VOICES: usize = 64;
    const FRAMES: usize = 48_000;
    const SR: f32 = 48_000.0;
    fn cached() -> (std::time::Duration, f64) {
        let mut voices: Vec<_> = (0..VOICES)
            .map(|index| {
                let mut voice = Voice::new(SR, SynthInstrument::ALL[index % 3]);
                voice.trig(36 + (index % 60) as u8, 0.8);
                (voice, Svf::default())
            })
            .collect();
        let start = Instant::now();
        let mut checksum = 0.0f64;
        for _ in 0..FRAMES {
            for (voice, filter) in &mut voices {
                checksum += black_box(voice).tick(SR, 1700.0, black_box(filter)) as f64;
            }
        }
        (start.elapsed(), black_box(checksum))
    }
    fn legacy() -> (std::time::Duration, f64) {
        let mut voices: Vec<_> = (0..VOICES)
            .map(|index| {
                LegacyVoice::new(SR, SynthInstrument::ALL[index % 3], 36 + (index % 60) as u8)
            })
            .collect();
        let start = Instant::now();
        let mut checksum = 0.0f64;
        for _ in 0..FRAMES {
            for voice in &mut voices {
                checksum += black_box(voice).tick(SR, 1700.0) as f64;
            }
        }
        (start.elapsed(), black_box(checksum))
    }
    cached();
    legacy(); // unmeasured warm-up of each path
    let (mut cached_times, mut legacy_times) = (Vec::new(), Vec::new());
    for round in 0..9 {
        let (new, old) = if round % 2 == 0 {
            (cached(), legacy())
        } else {
            let old = legacy();
            (cached(), old)
        };
        assert_eq!(new.1.to_bits(), old.1.to_bits(), "benchmark render differs");
        cached_times.push(new.0.as_secs_f64() * 1000.0);
        legacy_times.push(old.0.as_secs_f64() * 1000.0);
    }
    cached_times.sort_by(f64::total_cmp);
    legacy_times.sort_by(f64::total_cmp);
    println!("{VOICES} voices x {FRAMES} frames @ {SR} Hz; 9 alternating rounds");
    println!("cached ms: {cached_times:?}");
    println!("legacy ms: {legacy_times:?}");
    println!(
        "median cached {:.3} ms; legacy {:.3} ms; ratio {:.4}; improvement {:.2}%",
        cached_times[4],
        legacy_times[4],
        cached_times[4] / legacy_times[4],
        100.0 * (1.0 - cached_times[4] / legacy_times[4])
    );
}
