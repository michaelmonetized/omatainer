use super::super::{clip_launch::Grid, midi_data::TimingSettings, midi_schedule::BEAT_EPSILON};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Empty {
    #[default]
    Stop,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Signature {
    pub numerator: u8,
    pub denominator_power: u8,
}
impl Default for Signature {
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator_power: 2,
        }
    }
}
impl Signature {
    /// Validate a scene's time signature.
    /// Takes its numerator and denominator power; returns whether it fits the native meter range.
    pub(crate) fn valid(self) -> bool {
        self.numerator > 0 && self.denominator_power <= 7
    }
    /// Read the quarter-note length of a notated beat.
    /// Takes a validated signature; returns four divided by its denominator.
    pub(crate) fn unit(self) -> f64 {
        4.0 / f64::from(1_u32 << self.denominator_power)
    }
    /// Read the quarter-note length of a bar.
    /// Takes a validated signature; returns numerator times the notated beat length.
    pub(crate) fn length(self) -> f64 {
        f64::from(self.numerator) * self.unit()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Properties {
    pub tempo_micros: Option<u32>,
    pub meter: Option<Signature>,
    pub empty: Empty,
    pub grid: Grid,
}
impl Properties {
    /// Identify an omitted legacy scene policy.
    /// Takes this policy; returns true when timing is inherited, empty slots stop and launch uses the global grid.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
    /// Validate native scene timing and launch policy.
    /// Takes this policy; returns whether tempo and meter stay in their supported ranges.
    pub(crate) fn valid(self) -> bool {
        self.tempo_micros
            .is_none_or(|tempo| (250_000..=1_500_000).contains(&tempo))
            && self.meter.is_none_or(Signature::valid)
    }
    /// Read an explicitly selected tempo.
    /// Takes this policy; returns optional beats per minute derived from integer microseconds per quarter.
    pub(crate) fn bpm(self) -> Option<f64> {
        self.tempo_micros
            .map(|tempo| 60_000_000.0 / f64::from(tempo))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Timing {
    pub signature: Signature,
    pub anchor: f64,
    pub first_bar: u32,
    pub click: TimingSettings,
}
impl Timing {
    /// Validate the retained flat Session meter and click settings.
    /// Takes a saved or prepared timing anchor; returns a refusal before renderer admission.
    pub(crate) fn validate(self) -> Result<(), String> {
        if !self.signature.valid()
            || !self.anchor.is_finite()
            || !(0.0..=262144.0).contains(&self.anchor)
            || self.first_bar == 0
            || self.click.pickup != 0.0
        {
            return Err("Invalid scene meter or downbeat anchor".into());
        }
        self.click.validate(self.signature.length(), None)
    }
    /// Read meter labels around the retained scene downbeat.
    /// Takes an exact quarter-note position; returns its bar number and zero-based notated beat.
    pub(crate) fn position(self, beat: f64) -> (u32, f32) {
        let relative = beat - self.anchor;
        let bars = (relative / self.signature.length()).floor();
        (
            (f64::from(self.first_bar) + bars).clamp(0.0, f64::from(u32::MAX)) as u32,
            (relative.rem_euclid(self.signature.length()) / self.signature.unit()) as f32,
        )
    }
    /// Find the next group of actual scene bars.
    /// Takes the current musical position and positive group size; returns the first matching boundary at or after it.
    pub(crate) fn boundary(self, beat: f64, group: u32) -> f64 {
        let length = self.signature.length();
        let first = ((beat - self.anchor - BEAT_EPSILON) / length).ceil();
        let aligned =
            first + (1.0 - f64::from(self.first_bar) - first).rem_euclid(f64::from(group.max(1)));
        self.anchor + aligned * length
    }
    /// Find one sample's metronome event without allocation.
    /// Takes its half-open beat span; returns an optional downbeat/ordinary click using the retained subdivision.
    pub(crate) fn click_between(self, start: f64, end: f64) -> Option<bool> {
        if !start.is_finite() || end <= start {
            return None;
        }
        let unit = self.signature.unit() / f64::from(self.click.subdivision);
        let number = ((start - self.anchor - BEAT_EPSILON) / unit).ceil();
        let boundary = self.anchor + number * unit;
        (boundary < end - BEAT_EPSILON).then(|| {
            number
                .rem_euclid(f64::from(self.signature.numerator) * f64::from(self.click.subdivision))
                == 0.0
        })
    }
}
