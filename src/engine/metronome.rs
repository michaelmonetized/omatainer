//! One persistent, allocation-free click voice. Scheduling uses the same
//! half-open beat interval as MIDI: a boundary at the interval end belongs to
//! the next sample, independent of callback partitioning.
use super::midi_schedule::BEAT_EPSILON;

pub(super) struct CountIn {
    frame: u64,
    frames: u64,
    sr: u32,
    bpm: f64,
    unit: f64,
    accents: f64,
}

impl CountIn {
    /// Prepare the lead-in at the current meter and tempo.
    /// `map`, `beat` and `sr` select the musical position and output rate.
    /// Returns no lead-in when its configured bar count is zero.
    pub fn new(map: &super::midi_data::Conductor, beat: f64, sr: u32) -> Option<Self> {
        let settings = map.native?;
        if settings.count_in == 0 { return None; }
        let meter = map.position(beat).2;
        let unit = 4.0 / f64::from(1u32 << meter.denominator_power);
        let bpm = 60000000.0 / map.micros_exact_at(beat);
        Some(Self {
            frame: 0,
            frames: (f64::from(settings.count_in) * f64::from(meter.numerator) * unit * 60.0 / bpm * f64::from(sr)).round() as u64,
            sr, bpm, unit: unit / f64::from(settings.subdivision),
            accents: f64::from(meter.numerator) * f64::from(settings.subdivision),
        })
    }
    /// Prepare a flat scene lead-in.
    /// Takes meter, click settings, tempo and output rate; returns an allocation-free count-in when enabled.
    pub(crate) fn constant(signature: super::scene::Signature, settings: super::midi_data::TimingSettings, bpm: f64, sr: u32) -> Option<Self> {
        if settings.count_in == 0 { return None; }
        Some(Self { frame: 0, frames: (f64::from(settings.count_in) * signature.length() * 60.0 / bpm * f64::from(sr)).round() as u64, sr, bpm, unit: signature.unit() / f64::from(settings.subdivision), accents: f64::from(signature.numerator) * f64::from(settings.subdivision) })
    }
    pub fn finished(&self) -> bool { self.frame >= self.frames }
    pub fn remaining(&self) -> f32 { (self.frames.saturating_sub(self.frame) as f64 / f64::from(self.sr)) as f32 }
    pub fn tick(&mut self, sr: u32) -> Option<bool> {
        if sr != self.sr {
            self.frame = (self.frame as f64 * f64::from(sr) / f64::from(self.sr)).round() as u64;
            self.frames = (self.frames as f64 * f64::from(sr) / f64::from(self.sr)).round() as u64;
            self.sr = sr;
        }
        let step = self.bpm / 60.0 / f64::from(sr);
        let start = self.frame as f64 * step;
        let end = (self.frame + 1) as f64 * step;
        self.frame += 1;
        let number = ((start - BEAT_EPSILON) / self.unit).ceil();
        (number * self.unit < end - BEAT_EPSILON).then(|| number.rem_euclid(self.accents) == 0.0)
    }
}

pub(super) struct Click {
    sr: f64,
    length: u32,
    attack: u32,
    age: u32,
    phase: f64,
    increment: f64,
    amplitude: f32,
    #[cfg(test)]
    triggers: u64,
    #[cfg(test)]
    pub trace: Option<Vec<(u64, bool)>>,
}

impl Click {
    pub fn new(sr: f32) -> Self {
        let length = (sr * 0.020).round().max(3.0) as u32;
        Self {
            sr: sr as f64,
            length,
            attack: (sr * 0.001).round().clamp(1.0, (length - 2) as f32) as u32,
            age: length,
            phase: 0.0,
            increment: 0.0,
            amplitude: 0.0,
            #[cfg(test)]
            triggers: 0,
            #[cfg(test)]
            trace: None,
        }
    }

    pub fn reset(&mut self) {
        self.age = self.length;
    }

    pub fn tick(&mut self, enabled: bool, start: f64, end: f64) -> f32 {
        let boundary = (start - BEAT_EPSILON).ceil();
        let accent = (end > start && boundary < end - BEAT_EPSILON)
            .then(|| boundary.rem_euclid(4.0) == 0.0);
        self.tick_event(enabled, accent)
    }
    pub fn tick_event(&mut self, enabled: bool, accent: Option<bool>) -> f32 {
        self.tick_with_gains(enabled, accent, 1.0, 1.0)
    }
    pub fn tick_with_gains(&mut self, enabled: bool, accent: Option<bool>, accent_gain: f32, beat_gain: f32) -> f32 {
        if !enabled {
            self.reset();
            return 0.0;
        }
        if let Some(accent) = accent {
            self.phase = 0.0;
            self.increment = std::f64::consts::TAU * if accent { 1200.0 } else { 800.0 } / self.sr;
            self.amplitude = if accent { 0.20 * accent_gain } else { 0.12 * beat_gain };
            self.age = 0;
            #[cfg(test)]
            {
                self.triggers += 1;
            }
        }
        if self.age >= self.length {
            return 0.0;
        }
        // A 1ms linear attack and the remaining linear decay meet zero at the
        // two endpoints. The oscillator phase is voice-local, never block-local.
        let envelope = if self.age < self.attack {
            self.age as f32 / self.attack as f32
        } else {
            (self.length - 1 - self.age) as f32 / (self.length - 1 - self.attack) as f32
        };
        let sample = self.phase.sin() as f32 * envelope * self.amplitude;
        self.phase = (self.phase + self.increment) % std::f64::consts::TAU;
        self.age += 1;
        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::*;

    #[test]
    fn count_in_holds_clip_recording_and_transport_then_starts_at_the_exact_output_sample() {
        use midi_data::{Conductor, Meter, Tempo, TimingSettings};
        let mut rt = renderer();
        rt.apply(Command::Stop);
        rt.conductor = Some(Conductor::native(960, vec![Tempo::new(0, 120.0, false).unwrap()], vec![Meter { tick: 0, numerator: 7, denominator_power: 3, clocks: 12, thirty_seconds: 8 }], TimingSettings { count_in: 1, subdivision: 2, ..Default::default() }).unwrap());
        rt.metronome = false;
        rt.tracks[2].clips[0].region = Some(midi_edit::Region::full(16.0));
        rt.tracks[2].clips[0].notes.truncate(1);
        rt.tracks[2].clips[0].notes[0].start = 0.0;
        rt.tracks[2].midi_schedule.sample_trace = Some(Vec::with_capacity(8));
        rt.metro.trace = Some(Vec::with_capacity(16));
        rt.apply(Command::FireClip { track: 2, scene: 0, looping: false });
        let clock = rt.note_recording.clock;
        let mut output = vec![0.0; 84000 * 2];
        assert_eq!(test_alloc::measure(|| rt.process(&mut output)), test_alloc::Counts::default());
        assert_eq!(rt.beat, 0.0);
        assert_eq!(rt.note_recording.clock, clock);
        assert!(rt.recording_position(2, 0).is_none());
        assert!(rt.tracks[2].midi_schedule.sample_trace.as_ref().unwrap().is_empty());
        assert_eq!(rt.metro.trace.as_ref().unwrap().len(), 14);
        assert_eq!(rt.metro.trace.as_ref().unwrap()[0], (0, true));
        assert!(output.iter().any(|x| x.abs() > 0.001));
        rt.process(&mut [0.0; 2]);
        assert!(rt.count_in.is_none());
        assert_eq!(rt.tracks[2].midi_schedule.sample_trace.as_ref().unwrap()[0].0, 84000);
        assert!((rt.beat - 1.0 / 24000.0).abs() < 1e-12);
        rt.apply(Command::Stop); rt.apply(Command::Play);
        assert!(rt.count_in.is_some()); rt.apply(Command::Stop);
        assert!(rt.count_in.is_none());
    }

    #[test]
    fn actual_odd_meter_ramp_clicks_match_independent_timestamps_with_no_callback_heap() {
        use midi_data::{Conductor, Meter, Tempo, TimingSettings};
        let map = Conductor::native(960, vec![Tempo::new(0, 120.0, true).unwrap(), Tempo::new(8160, 180.0, false).unwrap()], vec![
            Meter { tick: 0, numerator: 7, denominator_power: 3, clocks: 12, thirty_seconds: 8 },
            Meter { tick: 3360, numerator: 5, denominator_power: 2, clocks: 24, thirty_seconds: 8 },
            Meter { tick: 8160, numerator: 4, denominator_power: 2, clocks: 24, thirty_seconds: 8 },
        ], TimingSettings { pickup: 0.5, subdivision: 2, ..Default::default() }).unwrap();
        let b = 60000000.0 / f64::from(map.tempos[1].micros);
        let timestamp = |beat: f64| {
            let length = beat.min(8.5); let n = 10000; let h = length / f64::from(n);
            let f = |x: f64| 60.0 / (120.0 + (b - 120.0) * x / 8.5);
            let mut sum = f(0.0) + f(length);
            for i in 1..n { sum += f(f64::from(i) * h) * if i % 2 == 0 { 2.0 } else { 4.0 }; }
            sum * h / 3.0 + (beat - 8.5).max(0.0) * 60.0 / b
        };
        let boundaries: Vec<_> = (0..14).map(|i| f64::from(i) * 0.25).chain((0..10).map(|i| 3.5 + f64::from(i) * 0.5)).chain((0..9).map(|i| 8.5 + f64::from(i) * 0.5)).collect();
        for sr in [44100, 48000, 96000] {
            let (_, mut rt) = Engine::headless_for_test(sr, 48);
            rt.apply(Command::Stop); rt.conductor = Some(map.clone()); rt.metronome = true;
            rt.metro.trace = Some(Vec::with_capacity(64)); rt.apply(Command::Play);
            let mut output = vec![0.0; 514]; let frames = (timestamp(12.6) * f64::from(sr)).ceil() as usize;
            let counts = test_alloc::measure(|| { for begin in (0..frames).step_by(257) { rt.process(&mut output[..(frames - begin).min(257) * 2]); } });
            assert_eq!(counts, test_alloc::Counts::default());
            let trace = rt.metro.trace.take().unwrap(); assert_eq!(trace.len(), boundaries.len());
            for ((frame, accent), beat) in trace.into_iter().zip(&boundaries) {
                let expected = (timestamp(*beat) * f64::from(sr)).floor() as u64;
                assert!(frame.abs_diff(expected) <= 1, "{sr} beat{beat}: {frame} vs {expected}");
                assert_eq!(accent, [0.5, 3.5, 8.5, 12.5].contains(beat));
            }
        }
    }

    #[test]
    fn click_gain_changes_are_applied_to_the_next_voice_and_zero_means_silent() {
        let energy = |accent, a, b| {
            let mut voice = Click::new(48000.0);
            voice.tick_with_gains(true, Some(accent), a, b);
            (0..960).map(|_| voice.tick_with_gains(true, None, a, b).powi(2)).sum::<f32>()
        };
        assert_eq!(energy(true, 0.0, 1.0), 0.0);
        assert_eq!(energy(false, 1.0, 0.0), 0.0);
        assert!((energy(true, 0.5, 1.0) / energy(true, 1.0, 1.0) - 0.25).abs() < 1e-6);
        assert!((energy(false, 1.0, 2.0) / energy(false, 1.0, 1.0) - 4.0).abs() < 1e-6);
    }

    fn render_voice(sr: f32, bpm: f64, start: f64, beats: f64) -> (Vec<f32>, Vec<usize>) {
        let mut click = Click::new(sr);
        let step = bpm / (60.0 * sr as f64);
        let mut beat = start;
        let mut triggers = Vec::new();
        let mut output = Vec::new();
        for frame in 0..(beats / step).ceil() as usize {
            let next = beat + step;
            let before = click.triggers;
            output.push(click.tick(true, beat, next));
            if click.triggers != before {
                triggers.push(frame);
            }
            beat = next;
        }
        (output, triggers)
    }

    #[test]
    fn beat_positions_duration_and_downbeat_accent_are_sample_timed() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            for bpm in [73.0, 120.0, 199.0] {
                let (output, starts) = render_voice(sr, bpm, 0.0, 5.1);
                assert_eq!(starts.len(), 6);
                let step = bpm / (60.0 * sr as f64);
                let duration = (sr * 0.02).round() as usize;
                let mut energies = Vec::new();
                for (beat, &frame) in starts.iter().enumerate() {
                    let expected = ((beat as f64 - BEAT_EPSILON) / step).floor().max(0.0) as usize;
                    assert!(
                        frame.abs_diff(expected) <= 1,
                        "sr={sr} bpm={bpm} beat={beat} frame={frame} expected={expected}"
                    );
                    let click = &output[frame..frame + duration];
                    assert_eq!(click[0], 0.0);
                    assert_eq!(click[duration - 1], 0.0);
                    assert!(
                        click.iter().filter(|sample| sample.abs() > 1e-6).count() > duration / 2
                    );
                    energies.push(click.iter().map(|sample| sample * sample).sum::<f32>());
                    let stop = starts.get(beat + 1).copied().unwrap_or(output.len());
                    assert!(output[frame + duration..stop]
                        .iter()
                        .all(|sample| *sample == 0.0));
                }
                assert!(energies[0] > energies[1] * 2.0);
                assert_eq!(energies[0], energies[4]);
                assert_eq!(energies[1], energies[2]);
            }
        }
        let (_, starts) = render_voice(48_000.0, 120.0, 255.5, 2.1);
        assert_eq!(
            starts,
            [12_000, 36_000],
            "beat identities must not wrap at u8 limits"
        );
    }

    fn renderer() -> RtEngine {
        let (_tx, rx) = crossbeam_channel::bounded(32);
        let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
        rt.apply(Command::Stop);
        for deck in 0..DECKS {
            rt.apply(Command::DeckUnload { deck: deck as u8 });
        }
        rt.master = 1.0;
        rt.fx_wet = [0.0; 3];
        rt.bpm = 120.0;
        rt.apply(Command::Metronome);
        rt.apply(Command::Play);
        rt
    }

    fn timeline(mut rt: RtEngine, chunks: &[usize]) -> Vec<f32> {
        let frames = 100_000;
        let mut output = vec![0.0; frames * 2];
        let mut offset = 0;
        let mut chunk = 0;
        while offset < frames {
            let count = chunks[chunk % chunks.len()].min(frames - offset);
            rt.process(&mut output[offset * 2..(offset + count) * 2]);
            offset += count;
            chunk += 1;
        }
        output
    }

    #[test]
    fn actual_renderer_clicks_are_identical_across_callback_partitioning() {
        let a = timeline(renderer(), &[64]);
        let b = timeline(renderer(), &[1, 17, 127, 512, 1024]);
        assert_eq!(a, b);
        assert!(a.iter().filter(|sample| sample.abs() > 1e-6).count() > 1000);
        for pair in a.chunks_exact(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    #[test]
    fn enable_stop_resume_and_sample_rate_changes_have_explicit_no_ghost_click_policy() {
        let mut rt = renderer();
        rt.process(&mut [0.0; 96]);
        assert!(rt.metro.age < rt.metro.length);
        rt.apply(Command::Metronome);
        assert_eq!(rt.metro.age, rt.metro.length);
        let mut silence = [1.0; 64];
        rt.process(&mut silence);
        assert_eq!(silence, [0.0; 64]);
        rt.apply(Command::Metronome);
        rt.process(&mut silence);
        assert_eq!(
            silence, [0.0; 64],
            "enabling between beats waits for the next boundary"
        );
        rt.apply(Command::Stop);
        rt.beat = 4.0;
        rt.process(&mut silence);
        assert_eq!(silence, [0.0; 64]);
        rt.apply(Command::Play);
        rt.process(&mut silence);
        assert!(silence.iter().any(|sample| sample.abs() > 1e-6));
        rt.set_sample_rate(96_000).unwrap();
        assert_eq!(rt.metro.length, 1920);
        assert_eq!(rt.metro.age, rt.metro.length);
        let mut click = Click::new(48_000.0);
        let counts = test_alloc::measure(|| {
            let mut beat = 0.0;
            for _ in 0..50_000 {
                let next = beat + 1.0 / 24_000.0;
                std::hint::black_box(click.tick(true, beat, next));
                beat = next;
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
    }
}
