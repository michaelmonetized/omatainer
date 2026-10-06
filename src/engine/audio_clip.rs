use super::Sample;
use serde::{Deserialize, Serialize};
pub(crate) mod edit;
#[cfg(test)]
mod tests;

/// Retain an editable audio source region.
/// Stores half-open source-frame bounds, a musical source tempo, pitch resampling and a loop inside the trim. The immutable source PCM is shared separately.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Region {
    pub start: u64,
    pub end: u64,
    pub loop_start: u64,
    pub loop_end: u64,
    pub loop_enabled: bool,
    pub reverse: bool,
    pub transpose: f32,
    pub tempo: f32,
}
impl Region {
    /// Select a complete source at its intended musical tempo.
    /// Takes decoded audio and BPM; returns a validated region or an unsupported source/tempo refusal.
    pub(crate) fn full(source: &Sample, tempo: f32) -> Result<Self, &'static str> {
        let region = Self {
            start: 0,
            end: source.frames() as u64,
            loop_start: 0,
            loop_end: source.frames() as u64,
            loop_enabled: false,
            reverse: false,
            transpose: 0.0,
            tempo,
        };
        region.prepare(source)?;
        Ok(region)
    }
    /// Validate and prepare immutable source coordinates.
    /// Takes decoded PCM; returns bounded frame/beat conversion without opening a file, or refuses empty/out-of-source/nonfinite settings.
    pub(crate) fn prepare(self, source: &Sample) -> Result<Plan, &'static str> {
        if source.sr == 0
            || !(1..=2).contains(&source.ch)
            || source.data.len() % usize::from(source.ch) != 0
            || self.start >= self.end
            || self.end > source.frames() as u64
            || self.loop_start < self.start
            || self.loop_start >= self.loop_end
            || self.loop_end > self.end
            || !self.transpose.is_finite()
            || !(-48.0..=48.0).contains(&self.transpose)
            || !self.tempo.is_finite()
            || !(30.0..=300.0).contains(&self.tempo)
        {
            return Err("Audio clip needs a finite mono/stereo source, in-source trim/loop, tempo 30–300 BPM and transpose ±48 semitones");
        }
        let frames_per_beat = f64::from(source.sr) * 60.0 / f64::from(self.tempo)
            * 2_f64.powf(f64::from(self.transpose) / 12.0);
        let duration_beats = (self.end - self.start) as f64 / frames_per_beat;
        if !(0.000001..=262144.0).contains(&duration_beats) {
            return Err("Audio clip duration exceeds its supported musical range");
        }
        Ok(Plan {
            region: self,
            frames_per_beat,
            duration_beats,
        })
    }
}
/// Process a validated clip without recalculating pitch coefficients.
/// Stores copyable source/beat conversion; sampling and progress use the same half-open position policy.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan {
    pub region: Region,
    frames_per_beat: f64,
    pub duration_beats: f64,
}
impl Plan {
    /// Resolve source position for a musical clip instance.
    /// Takes elapsed beats and launch repetition; returns a trimmed fractional frame or silence after a one-shot ends.
    pub(crate) fn position(self, beats: f64, repeating: bool) -> Option<f64> {
        self.coordinate(beats, repeating)
            .map(|(position, _)| position)
    }
    fn coordinate(self, beats: f64, repeating: bool) -> Option<(f64, u64)> {
        if !beats.is_finite() || beats < 0.0 {
            return None;
        }
        let mut offset = beats * self.frames_per_beat;
        let r = self.region;
        let length = (r.end - r.start) as f64;
        let mut bounds = (r.start, r.end);
        if repeating {
            let (intro, period) = if r.loop_enabled {
                if r.reverse {
                    (
                        (r.end - r.loop_start) as f64,
                        (r.loop_end - r.loop_start) as f64,
                    )
                } else {
                    (
                        (r.loop_end - r.start) as f64,
                        (r.loop_end - r.loop_start) as f64,
                    )
                }
            } else {
                (length, length)
            };
            if r.loop_enabled {
                bounds = if r.reverse {
                    (r.loop_start, r.end)
                } else {
                    (r.start, r.loop_end)
                };
            }
            if offset >= intro {
                if r.loop_enabled {
                    bounds = (r.loop_start, r.loop_end);
                }
                offset = if r.loop_enabled {
                    if r.reverse {
                        (r.end - r.loop_end) as f64
                    } else {
                        (r.loop_start - r.start) as f64
                    }
                } else {
                    0.0
                } + (offset - intro).rem_euclid(period);
            }
        } else if offset >= length {
            return None;
        }
        Some((
            if r.reverse {
                (r.end - 1) as f64 - offset
            } else {
                r.start as f64 + offset
            }
            .clamp(bounds.0 as f64, (bounds.1 - 1) as f64),
            bounds.1,
        ))
    }
    /// Read a trimmed stereo frame.
    /// Takes shared PCM, elapsed beats and launch repetition; interpolates within the retained region and duplicates mono without reading adjacent trimmed material.
    pub(crate) fn sample(self, source: &Sample, beats: f64, repeating: bool) -> [f32; 2] {
        let Some((position, end)) = self.coordinate(beats, repeating) else {
            return [0.0; 2];
        };
        let frame = position.floor() as usize;
        let next = (frame + 1).min((end - 1) as usize);
        let fraction = (position - frame as f64) as f32;
        let channels = usize::from(source.ch);
        std::array::from_fn(|channel| {
            let channel = channel.min(channels - 1);
            let a = source.data[frame * channels + channel];
            let b = source.data[next * channels + channel];
            a + (b - a) * fraction
        })
    }
    /// Read visible source progress.
    /// Takes elapsed beats and repetition; returns forward/reverse position inside the trim, clamped to its visible width.
    pub(crate) fn progress(self, beats: f64, repeating: bool) -> f32 {
        self.position(beats, repeating)
            .map_or(if self.region.reverse { 0.0 } else { 1.0 }, |p| {
                ((p - self.region.start as f64) / (self.region.end - self.region.start) as f64)
                    as f32
            })
            .clamp(0.0, 1.0)
    }
}
