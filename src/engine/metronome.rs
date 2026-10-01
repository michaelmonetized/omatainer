//! One persistent, allocation-free click voice. Scheduling uses the same
//! half-open beat interval as MIDI: a boundary at the interval end belongs to
//! the next sample, independent of callback partitioning.
use super::midi_schedule::BEAT_EPSILON;

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
        }
    }

    pub fn reset(&mut self) {
        self.age = self.length;
    }

    pub fn tick(&mut self, enabled: bool, start: f64, end: f64) -> f32 {
        if !enabled {
            self.reset();
            return 0.0;
        }
        let boundary = (start - BEAT_EPSILON).ceil();
        if end > start && boundary < end - BEAT_EPSILON {
            let accent = boundary.rem_euclid(4.0) == 0.0;
            self.phase = 0.0;
            self.increment = std::f64::consts::TAU * if accent { 1200.0 } else { 800.0 } / self.sr;
            self.amplitude = if accent { 0.20 } else { 0.12 };
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
        rt.set_sample_rate(96_000);
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
