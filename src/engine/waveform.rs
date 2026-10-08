//! Immutable source peaks and frequency energy prepared away from audio callbacks.
use std::mem::size_of;

pub(crate) const BANDS: usize = 8;
pub(crate) const EDGES_HZ: [f64; BANDS + 1] = [
    20.0, 60.0, 150.0, 400.0, 1000.0, 2500.0, 6000.0, 12000.0, 24000.0,
];
const MAX_BINS: usize = 65_536;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Bin {
    pub peak: [f32; 2],
    pub energy: [f32; BANDS],
}

#[derive(Clone, Copy, Debug, Default)]
struct StoredBin {
    peak: [f32; 2],
    energy: [u8; BANDS],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Waveform {
    frames: usize,
    hop: usize,
    levels: Vec<Vec<StoredBin>>,
}

#[derive(Clone, Copy, Default)]
struct Bandpass {
    b: f64,
    a: [f64; 2],
    state: [[f64; 2]; 2],
}

impl Bandpass {
    /// Prepare a frequency band.
    /// Takes native sample rate and band edges in Hz; returns a unity-peak biquad or silence above Nyquist.
    fn new(rate: u32, low: f64, high: f64) -> Self {
        let high = high.min(f64::from(rate) * 0.49);
        if low >= high {
            return Self::default();
        }
        let center = (low * high).sqrt();
        let angle = std::f64::consts::TAU * center / f64::from(rate);
        let alpha = angle.sin() * ((high / low).ln() * 0.5 * angle / angle.sin()).sinh();
        Self {
            b: alpha / (1.0 + alpha),
            a: [
                -2.0 * angle.cos() / (1.0 + alpha),
                (1.0 - alpha) / (1.0 + alpha),
            ],
            ..Self::default()
        }
    }

    /// Measure one channel without mixing stereo phases.
    /// Takes a finite source value and channel index; advances the filter and returns its band sample.
    fn tick(&mut self, value: f64, channel: usize) -> f64 {
        let state = &mut self.state[channel];
        let output = self.b * value + state[0];
        state[0] = -self.a[0] * output + state[1];
        state[1] = -self.b * value - self.a[1] * output;
        output
    }
}

impl Waveform {
    /// Analyze source audio once on a worker.
    /// Takes interleaved PCM, channels, native rate and cancellation; returns bounded multiresolution peaks and overlapping frequency-band energy.
    pub(crate) fn analyze(
        pcm: &[f32],
        channels: u16,
        rate: u32,
        cancelled: impl Fn() -> bool,
    ) -> Option<Self> {
        if cancelled() {
            return None;
        }
        let channels = usize::from(channels);
        if channels == 0 || rate == 0 || pcm.is_empty() {
            return Some(Self::default());
        }
        if pcm.len() % channels != 0 {
            return None;
        }
        let frames = pcm.len() / channels;
        let hop = (rate as usize / 400).max(1).max(frames.div_ceil(MAX_BINS));
        let mut filters: [Bandpass; BANDS] =
            std::array::from_fn(|band| Bandpass::new(rate, EDGES_HZ[band], EDGES_HZ[band + 1]));
        let mut base = Vec::with_capacity(frames.div_ceil(hop));
        for start in (0..frames).step_by(hop) {
            if cancelled() {
                return None;
            }
            let end = (start + hop).min(frames);
            let mut bin = StoredBin::default();
            let mut energy = [0.0f64; BANDS];
            for frame in start..end {
                if frame % 4096 == 0 && cancelled() {
                    return None;
                }
                let left = pcm[frame * channels];
                let right = pcm[frame * channels + usize::from(channels > 1)];
                if !left.is_finite() || !right.is_finite() {
                    return None;
                }
                bin.peak[0] = bin.peak[0].max(left.abs());
                bin.peak[1] = bin.peak[1].max(right.abs());
                for (band, filter) in filters.iter_mut().enumerate() {
                    let l = filter.tick(f64::from(left), 0);
                    let r = filter.tick(f64::from(right), 1);
                    energy[band] += l * l + r * r;
                }
            }
            for (out, power) in bin.energy.iter_mut().zip(energy) {
                let rms = (power / (2 * (end - start)) as f64).sqrt();
                *out = (rms.min(1.0).sqrt() * 255.0).round() as u8;
            }
            base.push(bin);
        }
        let mut levels = vec![base];
        while levels.last()?.len() > 1 {
            if cancelled() {
                return None;
            }
            let previous = levels.last()?;
            let mut next = Vec::with_capacity(previous.len().div_ceil(2));
            for pair in previous.chunks(2) {
                let mut bin = pair[0];
                if let Some(other) = pair.get(1) {
                    for (peak, value) in bin.peak.iter_mut().zip(other.peak) {
                        *peak = peak.max(value);
                    }
                    for (energy, value) in bin.energy.iter_mut().zip(other.energy) {
                        *energy = (*energy).max(value);
                    }
                }
                next.push(bin);
            }
            levels.push(next);
        }
        Some(Self {
            frames,
            hop,
            levels,
        })
    }

    /// Query a screen pixel's source interval.
    /// Takes source-frame bounds; returns preserved stereo peaks and band energy using a bounded number of prepared bins.
    pub(crate) fn range(&self, start: f64, end: f64) -> Bin {
        let mut result = Bin::default();
        if self.levels.is_empty()
            || !start.is_finite()
            || !end.is_finite()
            || end <= 0.0
            || start >= self.frames as f64
            || end <= start
        {
            return result;
        }
        let width = (end - start).max(1.0);
        let level =
            ((width / self.hop as f64).log2().floor().max(0.0) as usize).min(self.levels.len() - 1);
        let hop = self.hop << level;
        let bins = &self.levels[level];
        let first = (start.max(0.0) / hop as f64).floor() as usize;
        let last = ((end.min(self.frames as f64) / hop as f64).ceil() as usize).min(bins.len());
        for bin in &bins[first.min(last)..last] {
            for (peak, value) in result.peak.iter_mut().zip(bin.peak) {
                *peak = peak.max(value);
            }
            for (energy, value) in result.energy.iter_mut().zip(bin.energy) {
                *energy = energy.max((f32::from(value) / 255.0).powi(4));
            }
        }
        result
    }

    /// Account for all retained analysis allocations.
    /// Takes this waveform; returns its owned allocation bytes including the Arc header.
    pub(crate) fn storage_bytes(&self) -> usize {
        size_of::<Self>()
            + 2 * size_of::<usize>()
            + self.levels.capacity() * size_of::<Vec<StoredBin>>()
            + self
                .levels
                .iter()
                .map(|level| level.capacity() * size_of::<StoredBin>())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests;
