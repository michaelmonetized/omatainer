//! Whole-source RMS and sample-peak adjustment, independent of the deck fader.
use serde::{Deserialize, Serialize};

pub(crate) const ALGORITHM: u32 = 1;
pub(crate) const WORDS: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Policy {
    #[default]
    Off,
    Manual {
        db: f32,
    },
    Auto {
        target_dbfs: f32,
        peak_dbfs: f32,
    },
}
impl Policy {
    /// Validate a saved source adjustment.
    /// Takes this policy; returns whether its explicit limits fit the supported range.
    pub fn valid(self) -> bool {
        match self {
            Self::Off => true,
            Self::Manual { db } => db.is_finite() && (-120.0..=12.0).contains(&db),
            Self::Auto {
                target_dbfs,
                peak_dbfs,
            } => {
                target_dbfs.is_finite()
                    && (-40.0..=-6.0).contains(&target_dbfs)
                    && peak_dbfs.is_finite()
                    && (-12.0..=-1.0).contains(&peak_dbfs)
            }
        }
    }
    /// Omit disabled gain from serialized preparation.
    /// Takes this policy; returns true when source adjustment is off.
    pub fn is_off(&self) -> bool {
        *self == Self::Off
    }
    /// Encode a validated policy for coherent preparation publication.
    /// Takes this policy; returns three fixed atomic words without allocation.
    pub(crate) fn words(self) -> [u64; WORDS] {
        match self {
            Self::Off => [0; WORDS],
            Self::Manual { db } => [1, u64::from(db.to_bits()), 0],
            Self::Auto {
                target_dbfs,
                peak_dbfs,
            } => [
                2,
                u64::from(target_dbfs.to_bits()),
                u64::from(peak_dbfs.to_bits()),
            ],
        }
    }
    /// Read a coherently captured source policy.
    /// Takes three publication words; returns a valid policy or refuses malformed storage.
    pub(crate) fn from_words(words: [u64; WORDS]) -> Option<Self> {
        if words[1..].iter().any(|word| *word > u64::from(u32::MAX)) {
            return None;
        }
        let policy = match words[0] {
            0 if words[1..] == [0, 0] => Self::Off,
            1 if words[2] == 0 => Self::Manual {
                db: f32::from_bits(words[1] as u32),
            },
            2 => Self::Auto {
                target_dbfs: f32::from_bits(words[1] as u32),
                peak_dbfs: f32::from_bits(words[2] as u32),
            },
            _ => return None,
        };
        policy.valid().then_some(policy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Resolved {
    policy: Policy,
    db: f32,
    linear: f32,
    level: Option<Level>,
}
impl Default for Resolved {
    fn default() -> Self {
        Self {
            policy: Policy::Off,
            db: 0.0,
            linear: 1.0,
            level: None,
        }
    }
}
impl Resolved {
    /// Prepare the callback's source multiplier.
    /// Takes a reviewed policy and actual decoded level; returns bounded dB and a precomputed linear multiplier, or refuses unusable audio.
    pub fn prepare(policy: Policy, level: Option<Level>) -> Result<Self, &'static str> {
        if !policy.valid() || level.is_some_and(|level| !level.valid()) {
            return Err("Invalid source gain or measurement");
        }
        let db = match policy {
            Policy::Off => 0.0,
            Policy::Manual { db } => db,
            Policy::Auto {
                target_dbfs,
                peak_dbfs,
            } => recommend(
                level.ok_or("Auto gain needs a current decoded level")?,
                f64::from(target_dbfs),
                f64::from(peak_dbfs),
            )?,
        };
        let exact = 10.0f64.powf(f64::from(db) / 20.0);
        if level
            .and_then(|level| level.peak_dbfs)
            .is_some_and(|peak| peak + f64::from(db) > 20.0 * f64::from(f32::MAX).log10())
        {
            return Err("Source trim would exceed finite sample storage");
        }
        let mut linear = exact as f32;
        if f64::from(linear) > exact {
            linear = linear.next_down();
        }
        Ok(Self {
            policy,
            db,
            linear,
            level,
        })
    }
    /// Read prepared source gain without callback math.
    /// Takes this resolved adjustment; returns its finite linear multiplier.
    pub fn linear(self) -> f32 {
        self.linear
    }
    /// Read the reviewed source policy.
    /// Takes this resolved adjustment; returns its saved policy without recalculation.
    pub fn policy(self) -> Policy {
        self.policy
    }
    /// Read the prepared source trim.
    /// Takes this adjustment; returns its finite gain in dB.
    pub fn db(self) -> f32 {
        self.db
    }
    /// Read the decoded measurement used for review.
    /// Takes this adjustment; returns its source level when measured.
    pub fn level(self) -> Option<Level> {
        self.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Analysis {
    pub level: Level,
    pub recommended_db: Option<f32>,
}
impl Analysis {
    /// Retain the measured level and default recommendation.
    /// Takes actual whole-track level; returns its -18 dBFS RMS / -3 dBFS sample-peak recommendation when usable.
    pub fn new(level: Level) -> Self {
        Self {
            level,
            recommended_db: recommend(level, -18.0, -3.0).ok(),
        }
    }
    /// Validate a stored measurement and its recommendation.
    /// Takes this record; returns true only when the recommendation matches the documented defaults.
    pub fn valid(self) -> bool {
        self.level.valid() && self.recommended_db == recommend(self.level, -18.0, -3.0).ok()
    }
}

/// Describe source levels for review.
/// Takes a measured whole-track level; returns RMS, sample peak and near-full-scale sample count with explicit units.
pub(crate) fn description(level: Level) -> String {
    let db = |value: Option<f64>| {
        value.map_or_else(
            || tr!("silence").into(),
            |value| format!("{} dBFS", crate::localization::number(value, 2)),
        )
    };
    crate::localization::format("Whole-track RMS: {}; sample peak: {}; near-full-scale samples: {} of {}. First stereo pair or mono; not LUFS or true peak.", &[db(level.rms_dbfs), db(level.peak_dbfs), level.near_full_scale.to_string(), level.samples.to_string()])
}

const MAX_SAMPLES: u64 = 1024 * 1024 * 1024 / 4;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Level {
    pub samples: u64,
    pub rms_dbfs: Option<f64>,
    pub peak_dbfs: Option<f64>,
    pub near_full_scale: u64,
}
impl Level {
    /// Validate a whole-source level measurement.
    /// Takes this stored measurement; returns whether its sample count and finite levels are consistent.
    pub fn valid(self) -> bool {
        self.samples > 0
            && self.samples <= MAX_SAMPLES
            && self.near_full_scale <= self.samples
            && match (self.rms_dbfs, self.peak_dbfs) {
                (None, None) => self.near_full_scale == 0,
                (Some(rms), Some(peak)) => {
                    rms.is_finite()
                        && peak.is_finite()
                        && (-1000.0..=1000.0).contains(&rms)
                        && (-1000.0..=1000.0).contains(&peak)
                        && rms <= peak + 1e-7
                }
                _ => false,
            }
    }
}

/// Measure unchanged whole-track PCM.
/// Takes interleaved normalized samples and a cancellation check; returns arithmetic all-channel RMS, sample peak and near-full-scale count without allocating. This is not LUFS or true peak.
#[cfg(test)]
fn measure(pcm: &[f32], cancelled: impl FnMut() -> bool) -> Result<Level, &'static str> {
    if pcm.is_empty() || pcm.len() as u64 > MAX_SAMPLES {
        return Err("Level analysis needs 1–268435456 decoded samples");
    }
    measure_channels(pcm, 1, cancelled)
}

/// Measure the source channels the deck actually renders.
/// Takes whole interleaved PCM, source channel count and cancellation; returns first-pair or mono level without a PCM copy.
pub(crate) fn measure_channels(
    pcm: &[f32],
    channels: u16,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Level, &'static str> {
    let channels = usize::from(channels);
    if pcm.is_empty()
        || pcm.len() as u64 > MAX_SAMPLES
        || !(1..=128).contains(&channels)
        || pcm.len() % channels != 0
    {
        return Err("Invalid decoded channel layout for gain analysis");
    }
    let rendered_channels = channels.min(2);
    let samples = (pcm.len() / channels * rendered_channels) as u64;
    let mut energy = 0.0f64;
    let mut peak = 0.0f64;
    let mut near_full_scale = 0u64;
    for chunk in pcm.chunks(4096 * channels) {
        if cancelled() {
            return Err("Level analysis cancelled");
        }
        for frame in chunk.chunks_exact(channels) {
            if frame.iter().any(|sample| !sample.is_finite()) {
                return Err("Level analysis refuses non-finite PCM");
            }
            for &sample in &frame[..rendered_channels] {
                let sample = f64::from(sample).abs();
                energy += sample * sample;
                peak = peak.max(sample);
                near_full_scale += u64::from(sample >= 0.9999);
            }
        }
    }
    if cancelled() {
        return Err("Level analysis cancelled");
    }
    let db = |value: f64| (value > 0.0).then(|| 20.0 * value.log10());
    let result = Level {
        samples,
        rms_dbfs: db((energy / samples as f64).sqrt()),
        peak_dbfs: db(peak),
        near_full_scale,
    };
    if !result.valid() {
        return Err("Invalid source level measurement");
    }
    Ok(result)
}

/// Recommend a bounded source adjustment.
/// Takes measured PCM, target RMS dBFS and sample-peak headroom dBFS; returns dB that meet peak headroom with at most 12 dB boost, or refuses silence and unrepresentable attenuation.
pub(crate) fn recommend(
    level: Level,
    target_rms_dbfs: f64,
    headroom_dbfs: f64,
) -> Result<f32, &'static str> {
    if !level.valid()
        || !target_rms_dbfs.is_finite()
        || !(-40.0..=-6.0).contains(&target_rms_dbfs)
        || !headroom_dbfs.is_finite()
        || !(-12.0..=-1.0).contains(&headroom_dbfs)
    {
        return Err("Review a -40 to -6 dBFS RMS target and -12 to -1 dBFS sample-peak limit");
    }
    let (Some(rms), Some(peak)) = (level.rms_dbfs, level.peak_dbfs) else {
        return Err("Silent audio has no gain recommendation");
    };
    let db = (target_rms_dbfs - rms).min(headroom_dbfs - peak).min(12.0);
    if db < -120.0 {
        return Err("Required attenuation exceeds the safe adjustment range");
    }
    let stored = db as f32;
    Ok(if f64::from(stored) > db {
        stored.next_down()
    } else {
        stored
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_loud_and_clipped_sources_keep_pcm_and_peak_headroom() {
        for amplitude in [0.001f32, 0.5, 2.0] {
            let pcm = vec![amplitude; 16000];
            let original = pcm.clone();
            let level = measure(&pcm, || false).unwrap();
            let adjustment = recommend(level, -18.0, -3.0).unwrap();
            let gain = 10.0f64.powf(f64::from(adjustment) / 20.0);
            assert!(f64::from(amplitude) * gain <= 10.0f64.powf(-3.0 / 20.0) + 1e-7);
            assert!(adjustment <= 12.0);
            if amplitude == 0.001 {
                assert_eq!(adjustment, 12.0);
            } else {
                assert!((level.rms_dbfs.unwrap() + f64::from(adjustment) + 18.0).abs() < 1e-5);
            }
            assert_eq!(pcm, original);
            assert_eq!(
                level.near_full_scale,
                if amplitude >= 1.0 { 16000 } else { 0 }
            );
        }
    }
    #[test]
    fn transient_peak_caps_the_average_target_and_stereo_uses_all_samples() {
        let mut pcm = vec![0.0f32; 20000];
        pcm[0] = 1.0;
        let level = measure(&pcm, || false).unwrap();
        assert!((level.rms_dbfs.unwrap() + 43.01029995663981).abs() < 1e-8);
        assert_eq!(recommend(level, -18.0, -3.0).unwrap(), -3.0);
        let stereo = measure(&[0.5, 0.0, -0.5, 0.0], || false).unwrap();
        assert!((stereo.rms_dbfs.unwrap() + 9.030899869919436).abs() < 1e-8);
    }
    #[test]
    fn rear_channels_do_not_invent_an_audible_level_and_layout_damage_is_refused() {
        let level = measure_channels(&[0.0, 0.0, 2.0, 2.0], 4, || false).unwrap();
        assert_eq!(level.samples, 2);
        assert_eq!(level.rms_dbfs, None);
        assert_eq!(level.peak_dbfs, None);
        assert_eq!(level.near_full_scale, 0);
        assert!(recommend(level, -18.0, -3.0).is_err());
        let actual = measure_channels(&[0.25, 0.25, 2.0, 2.0], 4, || false).unwrap();
        assert!((actual.rms_dbfs.unwrap() + 12.041199826559248).abs() < 1e-8);
        assert!(measure_channels(&[0.25; 3], 2, || false).is_err());
        assert!(measure_channels(&[0.25; 4], 0, || false).is_err());
        assert!(measure_channels(&[0.0, 0.0, f32::NAN, 0.0], 4, || false).is_err());
    }

    #[test]
    fn silence_damage_cancellation_and_extreme_values_fail_without_partial_measurement() {
        let silent = measure(&[0.0; 32], || false).unwrap();
        assert!(silent.valid());
        assert!(recommend(silent, -18.0, -3.0).is_err());
        assert!(measure(&[], || false).is_err());
        for bad in [f32::NAN, f32::INFINITY] {
            assert!(measure(&[bad], || false).is_err());
        }
        let mut checks = 0;
        assert!(measure(&[0.1; 16000], || {
            checks += 1;
            checks == 2
        })
        .is_err());
        let loud = measure(&[f32::MAX; 32], || false).unwrap();
        assert!(loud.valid());
        assert!(recommend(loud, -18.0, -3.0).is_err());
        assert!(Resolved::prepare(Policy::Manual { db: 12.0 }, Some(loud)).is_err());
        let quiet = measure(&[f32::from_bits(1); 32], || false).unwrap();
        assert!(quiet.valid());
        assert_eq!(recommend(quiet, -18.0, -3.0).unwrap(), 12.0);
        let level = measure(&[0.1; 32], || false).unwrap();
        for (target, headroom) in [
            (f64::NAN, -3.0),
            (-18.0, f64::INFINITY),
            (-100.0, -3.0),
            (-18.0, 0.0),
        ] {
            assert!(recommend(level, target, headroom).is_err());
        }
    }
}
