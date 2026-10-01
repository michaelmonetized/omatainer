//! Opt-in loopback evidence. Callbacks only fill preallocated buffers; signal
//! matching and quality decisions execute on the audio management worker.
//! Times are common host-monotonic callback-entry timestamps, never subtraction
//! of CPAL StreamInstants (ALSA stream origins are independent).
use std::time::Duration;

pub(crate) mod native;
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub profile: String,
    pub input: super::config::Plan,
    pub output: super::config::Plan,
    pub input_channel: u16,
    pub output_channel: u16,
    pub level_db: f32,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.input.rate != self.output.rate
            || self.input.backend != self.output.backend
            || self.input_channel >= self.input.channels
            || self.output_channel >= self.output.channels
        {
            return Err("Calibration requires matching logical rates/backend and valid input/output channels".into());
        }
        validate_probe(self.output.rate, self.level_db)
    }
}
#[derive(Clone, Debug)]
pub struct Evidence {
    pub identity: Request,
    pub measured: Measurement,
}

pub const MAX_RATE: u32 = 192_000;
pub const SECONDS: usize = 3;
pub const PROBES: usize = 3;
const CHIPS: usize = 127;
const CHIP_RATE: u32 = 4_000;
const MAX_DELAY_NS: u64 = 500_000_000;
const MAX_STAMPS: usize = 40_000;

#[derive(Clone)]
pub struct Probe {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub starts: [usize; PROBES],
    patterns: [[f32; CHIPS]; PROBES],
    chip_frames: usize,
}
fn validate_probe(sample_rate: u32, level_db: f32) -> Result<(), String> {
    if !(8_000..=MAX_RATE).contains(&sample_rate)
        || !level_db.is_finite()
        || !(-60.0..=-24.0).contains(&level_db)
    {
        return Err(
            "Calibration needs 8–192 kHz and a probe level between −60 and −24 dBFS".into(),
        );
    }
    Ok(())
}
impl Probe {
    pub fn new(sample_rate: u32, level_db: f32) -> Result<Self, String> {
        validate_probe(sample_rate, level_db)?;
        let chip_frames = (sample_rate / CHIP_RATE).max(1) as usize;
        let starts = [
            sample_rate as usize / 4,
            sample_rate as usize * 3 / 4,
            sample_rate as usize * 5 / 4,
        ];
        let patterns = std::array::from_fn(|probe| {
            let mut seed = 0xa42f_3917u32.wrapping_mul(probe as u32 + 1);
            std::array::from_fn(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                if seed & 1 == 0 {
                    -1.0
                } else {
                    1.0
                }
            })
        });
        let mut samples = vec![0.0; sample_rate as usize * 2];
        let amplitude = 10.0f32.powf(level_db / 20.0);
        for (start, pattern) in starts.iter().zip(&patterns) {
            for (chip, value) in pattern.iter().enumerate() {
                samples[*start + chip * chip_frames..*start + (chip + 1) * chip_frames]
                    .fill(*value * amplitude);
            }
        }
        Ok(Self {
            sample_rate,
            samples,
            starts,
            patterns,
            chip_frames,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Stamp {
    pub first: usize,
    pub frames: usize,
    pub callback_ns: u64,
}

/// Unique callback-owned capture. `push` never allocates or grows a buffer.
pub struct Capture {
    pub samples: Vec<f32>,
    pub stamps: Vec<Stamp>,
    pub invalid: bool,
    pub backend_error: bool,
    capacity: usize,
}
impl Capture {
    pub fn new(sample_rate: u32) -> Self {
        let capacity = sample_rate.min(MAX_RATE) as usize * SECONDS;
        Self {
            samples: Vec::with_capacity(capacity),
            stamps: Vec::with_capacity(MAX_STAMPS),
            invalid: false,
            backend_error: false,
            capacity,
        }
    }
    pub fn full(&self) -> bool {
        self.samples.len() == self.capacity
    }
    pub fn push<T: Copy>(
        &mut self,
        input: &[T],
        channels: usize,
        channel: usize,
        callback_ns: u64,
        convert: impl Fn(T) -> f32,
    ) {
        if channels == 0 || channel >= channels || input.len() % channels != 0 {
            self.invalid = true;
            return;
        }
        if self.full() {
            return;
        }
        if self.stamps.len() == self.stamps.capacity() {
            self.invalid = true;
            return;
        }
        let first = self.samples.len();
        for frame in input.chunks_exact(channels).take(self.capacity - first) {
            let value = convert(frame[channel]);
            if !value.is_finite() || value.abs() >= 0.999 {
                self.invalid = true;
            }
            self.samples.push(value);
        }
        self.stamps.push(Stamp {
            first,
            frames: self.samples.len() - first,
            callback_ns,
        });
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// Median actual host callback-entry elapsed time for the three matched
    /// output→input probe deliveries. Buffer batching remains part of this value.
    pub host_return_ns: u64,
    pub minimum_ns: u64,
    pub maximum_ns: u64,
    pub callback_resolution_ns: u64,
    pub correlations: [f64; PROBES],
    pub input_frames: [usize; PROBES],
}
impl Measurement {
    pub fn spread(&self) -> Duration {
        Duration::from_nanos(self.maximum_ns - self.minimum_ns)
    }
}
fn stamp_at(stamps: &[Stamp], frame: usize) -> Option<&Stamp> {
    let index = stamps
        .partition_point(|stamp| stamp.first <= frame)
        .checked_sub(1)?;
    stamps
        .get(index)
        .filter(|stamp| frame < stamp.first + stamp.frames)
}
fn valid_stamps(stamps: &[Stamp], frames: usize) -> bool {
    let mut first = 0;
    let mut previous = None;
    for stamp in stamps {
        if stamp.first != first
            || stamp.frames == 0
            || previous.is_some_and(|time| stamp.callback_ns < time)
        {
            return false;
        }
        first += stamp.frames;
        previous = Some(stamp.callback_ns);
    }
    first == frames
}
fn correlation(
    samples: &[f32],
    start: usize,
    chip_frames: usize,
    pattern: &[f32; CHIPS],
) -> Option<(f64, f64)> {
    let length = chip_frames * CHIPS;
    let samples = samples.get(start..start + length)?;
    let mut sum = 0.0;
    let mut squares = 0.0;
    let mut dot = 0.0;
    let mut pattern_sum = 0.0;
    for (chip, expected) in samples.chunks_exact(chip_frames).zip(pattern) {
        let value = chip.iter().map(|s| *s as f64).sum::<f64>() / chip_frames as f64;
        sum += value;
        squares += value * value;
        dot += value * (*expected as f64);
        pattern_sum += *expected as f64;
    }
    let variance = (squares - sum * sum / CHIPS as f64).max(0.0);
    let pattern_variance = CHIPS as f64 - pattern_sum * pattern_sum / CHIPS as f64;
    let corr =
        (dot - sum * pattern_sum / CHIPS as f64) / (variance * pattern_variance).sqrt().max(1e-20);
    Some((corr.abs(), (variance / CHIPS as f64).sqrt()))
}

pub fn analyze(
    probe: &Probe,
    capture: &Capture,
    output_stamps: &[Stamp],
) -> Result<Measurement, String> {
    if capture.invalid || capture.backend_error {
        return Err("No measurement: clipped, nonfinite, overflowing or failed capture".into());
    }
    if !valid_stamps(&capture.stamps, capture.samples.len())
        || !valid_stamps(output_stamps, probe.samples.len())
    {
        return Err("No measurement: incomplete or inconsistent callback timing".into());
    }
    let length = probe.chip_frames * CHIPS;
    if capture.samples.len() < length {
        return Err("No measurement: incomplete input capture".into());
    }
    let mut delays = [0u64; PROBES];
    let mut correlations = [0.0; PROBES];
    let mut matches = [0usize; PROBES];
    let mut resolution = 0;
    for n in 0..PROBES {
        let output = stamp_at(output_stamps, probe.starts[n])
            .ok_or("No measurement: output probe was not submitted")?;
        let mut candidates = Vec::new();
        for start in (0..=capture.samples.len() - length).step_by(probe.chip_frames) {
            let Some(stamp) = stamp_at(&capture.stamps, start) else {
                continue;
            };
            if stamp.callback_ns < output.callback_ns
                || stamp.callback_ns - output.callback_ns > MAX_DELAY_NS
            {
                continue;
            }
            let (score, _) = correlation(
                &capture.samples,
                start,
                probe.chip_frames,
                &probe.patterns[n],
            )
            .unwrap();
            if score >= 0.5 {
                candidates.push((score, start));
            }
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let Some((_, coarse)) = candidates.first().copied() else {
            return Err("No measurement: a reliable loopback probe was not found".into());
        };
        let mut best = (0.0, coarse, 0.0);
        for start in coarse.saturating_sub(probe.chip_frames)
            ..=(coarse + probe.chip_frames).min(capture.samples.len() - length)
        {
            let (score, rms) = correlation(
                &capture.samples,
                start,
                probe.chip_frames,
                &probe.patterns[n],
            )
            .unwrap();
            if score > best.0 {
                best = (score, start, rms);
            }
        }
        let second = candidates
            .iter()
            .filter(|(_, start)| start.abs_diff(best.1) > probe.chip_frames * 2)
            .map(|(score, _)| *score)
            .fold(0.0, f64::max);
        let noise_end = best.1.saturating_sub(probe.chip_frames * 2);
        let noise_start = noise_end.saturating_sub(length);
        let noise = &capture.samples[noise_start..noise_end];
        let mean = noise.iter().map(|x| *x as f64).sum::<f64>() / noise.len().max(1) as f64;
        let noise_rms = (noise
            .iter()
            .map(|x| (*x as f64 - mean).powi(2))
            .sum::<f64>()
            / noise.len().max(1) as f64)
            .sqrt();
        if best.0 < 0.82
            || best.0 < second * 1.2
            || best.2 < 1e-5
            || noise.len() < length / 2
            || best.2 < noise_rms * 2.0
        {
            return Err("No measurement: loopback is weak, noisy or ambiguous".into());
        }
        let input = stamp_at(&capture.stamps, best.1).unwrap();
        let Some(delay) = input
            .callback_ns
            .checked_sub(output.callback_ns)
            .filter(|delay| *delay <= MAX_DELAY_NS)
        else {
            return Err("No measurement: invalid loopback timing".into());
        };
        delays[n] = delay;
        correlations[n] = best.0;
        matches[n] = best.1;
        resolution = resolution.max(
            (input.frames.max(output.frames) as u128 * 1_000_000_000 / probe.sample_rate as u128)
                as u64,
        );
    }
    if !(matches[0] < matches[1] && matches[1] < matches[2]) {
        return Err("No measurement: probe order did not match".into());
    }
    delays.sort_unstable();
    if delays[2] - delays[0] > resolution * 2 + 5_000_000 {
        return Err("No measurement: repeated loopback timing was inconsistent".into());
    }
    Ok(Measurement {
        host_return_ns: delays[1],
        minimum_ns: delays[0],
        maximum_ns: delays[2],
        callback_resolution_ns: resolution,
        correlations,
        input_frames: matches,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn stamps(frames: usize, block: usize, rate: u32, offset: u64) -> Vec<Stamp> {
        (0..frames)
            .step_by(block)
            .map(|first| Stamp {
                first,
                frames: block.min(frames - first),
                callback_ns: offset + (first as u128 * 1_000_000_000 / rate as u128) as u64,
            })
            .collect()
    }
    fn loopback(probe: &Probe, delay: usize) -> Capture {
        let mut capture = Capture::new(probe.sample_rate);
        let mut signal = vec![0.0; probe.sample_rate as usize * SECONDS];
        signal[delay..delay + probe.samples.len()].copy_from_slice(&probe.samples);
        for (i, block) in signal.chunks(128).enumerate() {
            capture.push(
                block,
                1,
                0,
                (i as u128 * 128 * 1_000_000_000 / probe.sample_rate as u128) as u64,
                |s| s,
            );
        }
        capture
    }
    pub(crate) fn synthetic(
        plan: &super::super::config::Plan,
        delay_ns: u64,
    ) -> Result<Measurement, String> {
        let probe = Probe::new(plan.rate, -40.0)?;
        let delay = (plan.rate as u64 * delay_ns / 1_000_000_000) as usize;
        analyze(
            &probe,
            &loopback(&probe, delay),
            &stamps(probe.samples.len(), 128, plan.rate, 0),
        )
    }
    #[test]
    fn real_signal_correlation_reports_qualified_host_roundtrip_at_supported_fixture_rates() {
        for rate in [44100, 48000, 96000, 192000] {
            let probe = Probe::new(rate, -40.0).unwrap();
            let delay = rate as usize / 50;
            let capture = loopback(&probe, delay);
            let output = stamps(probe.samples.len(), 128, rate, 0);
            let result = analyze(&probe, &capture, &output).unwrap();
            assert!(result.host_return_ns.abs_diff(20_000_000) <= result.callback_resolution_ns);
            for n in 0..PROBES {
                assert_eq!(result.input_frames[n], probe.starts[n] + delay);
                assert!(result.correlations[n] > 0.999);
            }
        }
    }
    #[test]
    fn silence_noise_clipping_gaps_and_incomplete_capture_never_produce_a_measurement() {
        let probe = Probe::new(48000, -40.0).unwrap();
        let output = stamps(probe.samples.len(), 128, 48000, 0);
        let mut input = loopback(&probe, 960);
        input.samples.fill(0.0);
        assert!(analyze(&probe, &input, &output).is_err());
        let mut seed = 345u32;
        for sample in &mut input.samples {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *sample = (seed as f32 / u32::MAX as f32 - 0.5) * 0.1;
        }
        assert!(analyze(&probe, &input, &output).is_err());
        let mut input = loopback(&probe, 960);
        input.invalid = true;
        assert!(analyze(&probe, &input, &output).is_err());
        input.invalid = false;
        input.stamps[3].first += 1;
        assert!(analyze(&probe, &input, &output).is_err());
        let input = Capture::new(48000);
        assert!(analyze(&probe, &input, &output).is_err());
    }
    #[test]
    fn duplicate_paths_and_inconsistent_repeat_timing_reject_instead_of_guessing() {
        let probe = Probe::new(48000, -40.0).unwrap();
        let output = stamps(probe.samples.len(), 128, 48000, 0);
        let mut echo = loopback(&probe, 960);
        for (i, value) in probe.samples.iter().enumerate() {
            echo.samples[i + 5760] += value;
        }
        assert!(analyze(&probe, &echo, &output)
            .unwrap_err()
            .contains("ambiguous"));
        let mut jitter = loopback(&probe, 960);
        let from = probe.starts[1] + 960;
        for stamp in &mut jitter.stamps {
            if stamp.first + stamp.frames > from {
                stamp.callback_ns += 100_000_000;
            }
        }
        assert!(analyze(&probe, &jitter, &output)
            .unwrap_err()
            .contains("inconsistent"));
    }
    #[test]
    fn bounded_callback_capture_allocates_nothing_and_marks_corrupt_frames() {
        let mut input = Capture::new(48000);
        assert_eq!(
            crate::engine::test_alloc::measure(|| input.push(&[0.1f32; 256], 2, 1, 5, |s| s)),
            crate::engine::test_alloc::Counts::default()
        );
        assert_eq!(input.samples.len(), 128);
        input.push(&[f32::NAN], 1, 0, 10, |s| s);
        assert!(input.invalid);
        for clipped in [-1.0_f32, 1.0] {
            let mut input = Capture::new(48000);
            input.push(&[0.0, clipped, 0.0], 1, 0, 0, |s| s);
            assert!(input.invalid, "clipped PCM must invalidate the capture");
        }
    }
}
