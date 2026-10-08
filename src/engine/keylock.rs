//! Original, stereo-linked waveform-similarity overlap-add implementation.
//! MIT, like the rest of Omatainer. No third-party DSP code or foreign allocator.
//!
//! The source is already resident PCM: overlap analysis reads ahead rather than
//! adding an output FIFO. That does not imply zero content displacement. Search
//! and overlap displacement bounds are exposed separately for qualification.
use super::dsp::Sample;
use serde::Serialize;

pub const MIN_RATIO: f32 = 0.5;
pub const MAX_RATIO: f32 = 1.5;
/// Minimum numerical convergence tolerance, used only for exact targets at
/// the supported endpoints or unity while key lock is requested. Higher output
/// rates have a smaller smoothing factor and a wider f32 rounding deadband.
pub const BOUNDARY_SNAP_EPSILON: f32 = 1e-6;
pub(super) fn boundary_tolerance(rate_smoothing: f32) -> f32 {
    // At 1..1.5, half a f32 ULP divided by the blend is the largest update
    // that can round away. This also conservatively covers the smaller ULP at
    // 0.5. Include one ULP for the deadband endpoint itself.
    BOUNDARY_SNAP_EPSILON.max(f32::EPSILON * 0.5 / rate_smoothing + f32::EPSILON)
}
pub const WINDOW_SECONDS: f64 = 2048.0 / 48_000.0;
// A ±25 ms search spans a full phase cycle at 20 Hz; the narrower initial
// search lost sub-bass fundamentals even when overall RMS remained strong.
pub const SEARCH_SECONDS: f64 = 0.025;
pub const OUTPUT_FIFO_FRAMES: usize = 0;
const CORRELATION_POINTS: usize = 128;
// Spend a bounded score budget on enough sub-frame refinement to avoid
// accumulating high-frequency phase error at each hop. Nominal and exact
// continuation candidates plus 45 coarse and 6*9 refined scores total 101.
const COARSE_STEPS: i32 = 22;
const REFINE_LEVELS: usize = 6;

/// Configuration bounds, not measured latency or a perceptual quality score.
/// Seconds refer to unwrapped source time; looping reads are cyclic. Resident
/// PCM may be read ahead; no output FIFO is inserted. Actual content displacement
/// must still be measured separately.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Geometry {
    pub window_frames: usize,
    pub hop_frames: usize,
    pub search_seconds: f64,
    pub max_source_lookahead_seconds: f64,
    pub max_content_displacement_seconds: f64,
    pub output_fifo_frames: usize,
    pub boundary_snap_epsilon: f32,
    pub correlation_points: usize,
    pub max_correlation_scores: usize,
    pub dense_reference_frames: usize,
    /// Initial/silent-grain quantization in SOURCE frame units. The seconds
    /// bounds above already include its tighter bound of one OUTPUT frame;
    /// even unusually low source rates cannot cause a large startup skip.
    pub max_start_quantization_source_frames: f64,
    pub max_start_quantization_output_frames: f64,
}
fn hop_frames(output_sr: f64) -> usize {
    (WINDOW_SECONDS * output_sr * 0.5).round().max(1.0) as usize
}
pub fn geometry(output_sr: u32) -> Geometry {
    let sr = output_sr.max(1) as f64;
    let hop = hop_frames(sr);
    Geometry {
        window_frames: 2 * hop,
        hop_frames: hop,
        search_seconds: SEARCH_SECONDS,
        // Conservative bounds cover reference-analysis reads as well as the
        // two emitted overlapping grains throughout the supported ratio range.
        max_source_lookahead_seconds: SEARCH_SECONDS + (2.0 * hop as f64 + 1.0) / sr,
        max_content_displacement_seconds: SEARCH_SECONDS + (hop as f64 + 1.0) / sr,
        output_fifo_frames: OUTPUT_FIFO_FRAMES,
        boundary_snap_epsilon: boundary_tolerance(super::dsp::rate_blend(0.08, sr as f32)),
        correlation_points: CORRELATION_POINTS,
        max_correlation_scores: 2 + (2 * COARSE_STEPS + 1) as usize + 9 * REFINE_LEVELS,
        dense_reference_frames: hop,
        max_start_quantization_source_frames: 1.0,
        max_start_quantization_output_frames: 1.0,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    NoMedia,
    Stopped,
    Unity,
    Locked,
    ScratchBypass,
    UnsupportedRate,
}

/// The active-rate predicate, after the deck checks media/transport state.
/// Requested key lock and actual processing mode are deliberately distinct.
pub fn mode(enabled: bool, touching: bool, ratio: f32) -> Mode {
    if !enabled {
        Mode::Off
    } else if touching {
        Mode::ScratchBypass
    } else if !ratio.is_finite() || !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
        Mode::UnsupportedRate
    } else if ratio == 1.0 {
        Mode::Unity
    } else {
        Mode::Locked
    }
}

#[derive(Clone, Copy)]
pub(super) struct Source<'a> {
    pub audio: &'a Sample,
    pub loop_on: bool,
    pub loop_start: f64,
    pub loop_len: f64,
}
impl Source<'_> {
    pub fn at(&self, pos: f64) -> (f32, f32) {
        if !(self.loop_on && self.loop_len > 1.0) {
            return self.audio.at(pos);
        }
        let pos = self.loop_start + (pos - self.loop_start).rem_euclid(self.loop_len);
        let next = pos.floor() + 1.0;
        if next < self.loop_start + self.loop_len {
            return self.audio.at(pos);
        }
        // Preserve the ordinary deck interpolation at a fractional loop seam.
        let frame = pos.floor() as usize;
        if pos < 0.0 || frame >= self.audio.frames() {
            return (0.0, 0.0);
        }
        let channels = self.audio.ch as usize;
        let left = self.audio.data[frame * channels];
        let right = self.audio.data[frame * channels + usize::from(channels > 1)];
        let wrapped = self.loop_start + (next - self.loop_start).rem_euclid(self.loop_len);
        let (next_left, next_right) = self.audio.at(wrapped);
        let mix = pos.fract() as f32;
        (
            left + (next_left - left) * mix,
            right + (next_right - right) * mix,
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct Processor {
    pub(super) phase: usize,
    pub(super) origin: f64,
    pub(super) previous: f64,
    pub(super) hop: usize,
    weights: Box<[f32]>,
    radius: f64,
    output_sr: f64,
    fence: Option<f64>,
    started: bool,
    #[cfg(test)]
    analysis_count: u64,
    #[cfg(test)]
    full_search_count: u64,
}
impl Processor {
    /// Off-callback construction/rate preparation; reset and render never resize.
    pub fn new(output_sr: f32) -> Self {
        let hop = hop_frames(output_sr as f64);
        let weights = (0..hop)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::PI * i as f64 / hop as f64).cos()) as f32)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            phase: 0,
            origin: 0.0,
            previous: 0.0,
            hop,
            weights,
            radius: SEARCH_SECONDS * output_sr as f64,
            output_sr: output_sr as f64,
            fence: Some(0.0),
            started: false,
            #[cfg(test)]
            analysis_count: 0,
            #[cfg(test)]
            full_search_count: 0,
        }
    }
    #[cfg(test)]
    pub(super) fn analysis_count(&self) -> u64 {
        self.analysis_count
    }
    /// Counts scoring-loop entries, excluding the silent-reference shortcut.
    /// Like the hop count, this remains cumulative across logical resets.
    #[cfg(test)]
    pub(super) fn full_search_count(&self) -> u64 {
        self.full_search_count
    }
    pub fn reset(&mut self, pos: f64, source_step: f64) {
        self.phase = 0;
        self.origin = pos;
        self.previous = pos - self.hop as f64 * source_step;
        self.fence = Some(pos);
        self.started = false;
    }
    /// A valid natural wrap is the same repeating source, not an explicit seek.
    /// Wrapped interpolation keeps both channels inside that cyclic source,
    /// including forward lookahead before transport wraps. This natural wrap
    /// retires the explicit-seek lower bound on candidate grain origins.
    pub fn natural_wrap(&mut self) {
        self.fence = None;
    }
    fn safe_phase_origin(&self, pos: f64, step: f64) -> f64 {
        // Standard same-rate WSOLA grains start on source sample boundaries.
        // A half-frame start otherwise turns an alternating Nyquist burst into
        // almost silence under linear interpolation. For upsampling, use the
        // output sampling lattice: this bounds displacement by one OUTPUT frame
        // too, including unusual very-low-rate source assets. Never round below
        // an explicit seek/load/cue fence.
        let quantum = step.min(1.0);
        let rounded = (pos / quantum).round() * quantum;
        let minimum = (self.fence.unwrap_or(f64::NEG_INFINITY) / quantum).ceil() * quantum;
        rounded.max(minimum)
    }
    pub fn render(&mut self, source: Source<'_>, pos: f64, source_step: f64) -> (f32, f32) {
        if !self.started {
            // The deck advances before rendering. Quantize both contributions
            // from that current position, with no pre-seek overlap or FIFO.
            self.origin = self.safe_phase_origin(pos, source_step);
            self.previous = self.origin - self.hop as f64 * source_step;
            self.phase = 0;
            self.started = true;
        } else if self.phase == self.hop {
            self.previous = self.origin;
            self.origin = self.align(source, pos, source_step);
            self.phase = 0;
        }
        let phase = self.phase as f64;
        let current = source.at(self.origin + phase * source_step);
        let previous = source.at(self.previous + (phase + self.hop as f64) * source_step);
        let weight = self.weights[self.phase];
        self.phase += 1;
        (
            current.0 * weight + previous.0 * (1.0 - weight),
            current.1 * weight + previous.1 * (1.0 - weight),
        )
    }
    fn align(&mut self, source: Source<'_>, nominal: f64, step: f64) -> f64 {
        #[cfg(test)]
        {
            self.analysis_count = self.analysis_count.wrapping_add(1);
        }
        let mut reference = [[0.0_f32; 2]; CORRELATION_POINTS];
        let mut energy = 0.0_f64;
        let spacing = self.hop as f64 * step / CORRELATION_POINTS as f64;
        let reference_start = self.previous + self.hop as f64 * step;
        // One deterministic stratified point in each overlap bin. Uniform
        // spacing aliases tones at multiples of the effective probe rate (for
        // example 6 kHz at the nominal window), leaving correlation blind to
        // their phase. The coprime permutation covers every fractional stratum
        // without changing the fixed point/score budget or allocating memory.
        let mut offsets: [f64; CORRELATION_POINTS] = std::array::from_fn(|index| {
            let fraction =
                ((index * 73 + 19) % CORRELATION_POINTS) as f64 / CORRELATION_POINTS as f64;
            (index as f64 + fraction) * spacing
        });
        // Stratified probes alone can miss an isolated one-frame onset. One
        // bounded scan of the overlap retains each channel's actual peak in
        // the correlation set, without adding candidate scores or heap work.
        let mut peaks = [(0.0_f32, 0.0_f64); 2];
        for index in 0..self.hop {
            let offset = index as f64 * step;
            let frame = source.at(reference_start + offset);
            for (channel, value) in [frame.0, frame.1].into_iter().enumerate() {
                if value.abs() > peaks[channel].0 {
                    peaks[channel] = (value.abs(), offset);
                }
            }
        }
        for channel in 0..2 {
            offsets[channel] = peaks[channel].1;
        }
        for (frame, offset) in reference.iter_mut().zip(&offsets) {
            let (l, r) = source.at(reference_start + offset);
            *frame = [l, r];
            energy += l as f64 * l as f64 + r as f64 * r as f64;
        }
        if energy < 1e-20 {
            return self.safe_phase_origin(nominal, step);
        }
        #[cfg(test)]
        {
            self.full_search_count = self.full_search_count.wrapping_add(1);
        }
        let radius = self.radius * step.max(f64::from(source.audio.sr) / self.output_sr);
        let score = |candidate: f64| -> f64 {
            if self.fence.is_some_and(|fence| candidate < fence)
                || (!(source.loop_on && source.loop_len > 1.0)
                    && !(0.0..source.audio.frames() as f64).contains(&candidate))
            {
                return f64::NEG_INFINITY;
            }
            let mut dot = 0.0_f64;
            let mut candidate_energy = 0.0_f64;
            for (frame, offset) in reference.iter().zip(&offsets) {
                let (l, r) = source.at(candidate + offset);
                // Sum channel energies/correlations, never L+R samples. A
                // perfectly anti-phase stereo source must not look like silence.
                dot += frame[0] as f64 * l as f64 + frame[1] as f64 * r as f64;
                candidate_energy += l as f64 * l as f64 + r as f64 * r as f64;
            }
            if candidate_energy < 1e-20 {
                return f64::NEG_INFINITY;
            }
            // Unlike cosine-only correlation, this also penalizes amplitude
            // loss: a near-silent fractional-phase copy is not a perfect match.
            2.0 * dot / (energy + candidate_energy)
        };
        let mut best = nominal;
        let mut best_score = score(nominal);
        let consider = |candidate: f64, best: &mut f64, best_score: &mut f64| {
            let candidate = candidate.clamp(nominal - radius, nominal + radius);
            let value = score(candidate);
            if value > *best_score + 1e-9
                || ((value - *best_score).abs() <= 1e-9
                    && (candidate - nominal).abs() < (*best - nominal).abs())
            {
                *best = candidate;
                *best_score = value;
            }
        };
        let mut spacing = radius / COARSE_STEPS as f64;
        // Sparse/nonperiodic attacks may have only one matching origin. Retain
        // the exact continuation when within the same bounded search interval.
        consider(reference_start, &mut best, &mut best_score);
        for offset in -COARSE_STEPS..=COARSE_STEPS {
            // Stratify the coarse candidates too: a uniform candidate spacing
            // equal to half a tone period can otherwise sample only two phases
            // and send refinement toward a clipped search boundary.
            let count = 2 * COARSE_STEPS + 1;
            let fraction = (((offset + COARSE_STEPS) * 17 + 7) % count) as f64 / count as f64;
            let jitter = (fraction - 0.5) * 0.5;
            consider(
                nominal + (offset as f64 + jitter) * spacing,
                &mut best,
                &mut best_score,
            );
        }
        for _ in 0..REFINE_LEVELS {
            let center = best;
            spacing *= 0.25;
            for offset in -4..=4 {
                consider(center + offset as f64 * spacing, &mut best, &mut best_score);
            }
        }
        best
    }
}
