//! Worker-side musical-key estimates. Scores describe profile correlation,
//! not a calibrated probability. Saved tag edits remain separate and authoritative.
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::f64::consts::{PI, TAU};

pub(crate) const ALGORITHM: u32 = 1;
const MIN_FRAMES: u64 = 16;
const MIN_SCORE: f64 = 0.70;
const MIN_MARGIN: f64 = 0.08;
const WHEEL: [[u8; 12]; 2] = [
    [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1],
    [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10],
];
const PROFILES: [[f64; 12]; 2] = [
    [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ],
    [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Key {
    pub tonic: u8,
    pub minor: bool,
}
impl Key {
    /// Check a saved pitch class.
    /// Takes this key; returns whether its tonic is one of the twelve pitch classes.
    pub fn valid(self) -> bool {
        self.tonic < 12
    }

    /// Format conventional notation.
    /// Takes this key; returns a canonical enharmonic spelling, or unknown for an invalid key.
    pub fn conventional(self) -> String {
        const NOTES: [&str; 12] = [
            "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
        ];
        NOTES.get(usize::from(self.tonic)).map_or_else(
            || "—".into(),
            |note| format!("{note}{}", if self.minor { "m" } else { "" }),
        )
    }

    /// Format harmonic-wheel notation.
    /// Takes this key; returns its Camelot number and major B or minor A suffix.
    pub fn harmonic(self) -> String {
        let numbers = WHEEL[usize::from(self.minor)];
        numbers.get(usize::from(self.tonic)).map_or_else(
            || "—".into(),
            |number| format!("{number}{}", if self.minor { "A" } else { "B" }),
        )
    }

    /// Compare source keys without changing audio.
    /// Takes both keys; returns whether they are equal, relative major/minor, or adjacent on the harmonic wheel.
    pub fn compatible(self, other: Self) -> bool {
        if !self.valid() || !other.valid() {
            return false;
        }
        if self.minor != other.minor {
            return (if self.minor {
                self.tonic + 3
            } else {
                self.tonic + 9
            }) % 12
                == other.tonic;
        }
        matches!((self.tonic + 12 - other.tonic) % 12, 0 | 5 | 7)
    }

    /// Read a complete conventional or harmonic key label.
    /// Takes saved text; returns an exact pitch class and mode, refusing malformed or partial labels.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(suffix) = text
            .chars()
            .last()
            .filter(|c| matches!(c, 'A' | 'B' | 'a' | 'b'))
        {
            if let Ok(number) = text[..text.len() - 1].parse::<u8>() {
                let minor = suffix.eq_ignore_ascii_case(&'A');
                return WHEEL[usize::from(minor)]
                    .iter()
                    .position(|value| *value == number)
                    .map(|tonic| Self {
                        tonic: tonic as u8,
                        minor,
                    });
            }
        }
        let mut chars = text.chars();
        let tonic = match chars.next()?.to_ascii_uppercase() {
            'C' => 0i8,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let tail = chars.as_str();
        let (shift, tail) = match tail.chars().next() {
            Some('#' | '♯') => (1, &tail[tail.chars().next()?.len_utf8()..]),
            Some('b' | '♭') => (-1, &tail[tail.chars().next()?.len_utf8()..]),
            _ => (0, tail),
        };
        let minor = match tail.trim() {
            "" | "M" => false,
            "m" => true,
            word if word.eq_ignore_ascii_case("minor") || word.eq_ignore_ascii_case("min") => true,
            word if word.eq_ignore_ascii_case("major") || word.eq_ignore_ascii_case("maj") => false,
            _ => return None,
        };
        Some(Self {
            tonic: (tonic + shift).rem_euclid(12) as u8,
            minor,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Analysis {
    pub key: Option<Key>,
    pub score: f64,
    pub margin: f64,
    pub frames: u64,
}
impl Analysis {
    /// Represent completed analysis without tonal evidence.
    /// Takes no arguments; returns an unknown result distinct from an unanalyzed field.
    pub fn unknown() -> Self {
        Self {
            key: None,
            score: 0.0,
            margin: 0.0,
            frames: 0,
        }
    }

    /// Validate a persisted estimate and its unknown decision.
    /// Takes this record; returns whether finite scores and decision thresholds agree.
    pub fn valid(self) -> bool {
        self.score.is_finite()
            && (-1.0..=1.0).contains(&self.score)
            && self.margin.is_finite()
            && (0.0..=2.0).contains(&self.margin)
            && self.frames <= crate::track_analysis::MAX_PCM_BYTES / 4
            && (self.frames > 0 || self == Self::unknown())
            && self.key.is_none_or(Key::valid)
            && self.key.is_some()
                == (self.frames >= MIN_FRAMES
                    && self.score >= MIN_SCORE
                    && self.margin >= MIN_MARGIN)
    }
}

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}
impl Complex {
    /// Multiply spectral values.
    /// Takes both complex values; returns their complex product.
    fn times(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
    /// Read spectral power.
    /// Takes this value; returns its squared magnitude.
    fn power(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}
/// Transform one power-of-two frame in place.
/// Takes fixed complex storage; returns its forward spectrum without allocating.
fn fft(frame: &mut [Complex]) {
    let n = frame.len();
    let mut reversed = 0;
    for i in 1..n {
        let mut bit = n / 2;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit /= 2;
        }
        reversed ^= bit;
        if i < reversed {
            frame.swap(i, reversed);
        }
    }
    let mut width = 2;
    while width <= n {
        let angle = -TAU / width as f64;
        let step = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for block in frame.chunks_exact_mut(width) {
            let mut weight = Complex { re: 1.0, im: 0.0 };
            for j in 0..width / 2 {
                let left = block[j];
                let right = block[j + width / 2].times(weight);
                block[j] = Complex {
                    re: left.re + right.re,
                    im: left.im + right.im,
                };
                block[j + width / 2] = Complex {
                    re: left.re - right.re,
                    im: left.im - right.im,
                };
                weight = weight.times(step);
            }
        }
        width *= 2;
    }
}
/// Compare accumulated chroma to a transposed cognitive profile.
/// Takes twelve pitch weights, tonic and mode; returns Pearson correlation, or zero for a flat distribution.
fn correlation(chroma: &[f64; 12], tonic: usize, mode: usize) -> f64 {
    let x_mean = chroma.iter().sum::<f64>() / 12.0;
    let y_mean = PROFILES[mode].iter().sum::<f64>() / 12.0;
    let (mut cross, mut x_energy, mut y_energy) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let x = chroma[i] - x_mean;
        let y = PROFILES[mode][(i + 12 - tonic) % 12] - y_mean;
        cross += x * y;
        x_energy += x * x;
        y_energy += y * y;
    }
    let scale = (x_energy * y_energy).sqrt();
    if scale > 1e-20 {
        (cross / scale).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Analyze source PCM on a cancellable background worker.
/// Takes interleaved PCM, channels, native rate, cancellation and progress callbacks; returns a whole-source estimate or a refusal without changing samples.
/// The first stereo pair or mono channel is analyzed; all channels must contain finite samples.
pub(crate) fn analyze(
    pcm: &[f32],
    channels: u16,
    rate: u32,
    cancelled: impl Fn() -> bool,
    progress: impl Fn(u64, u64),
) -> Result<Analysis, String> {
    let channels = usize::from(channels);
    if channels == 0
        || channels > usize::from(crate::project_file::MAX_CHANNELS)
        || rate == 0
        || rate > crate::project_file::MAX_SAMPLE_RATE
        || pcm.is_empty()
        || pcm.len() % channels != 0
        || pcm.len() as u64 * 4 > crate::track_analysis::MAX_PCM_BYTES
    {
        return Err("Invalid or oversized musical-key source PCM".into());
    }
    for chunk in pcm.chunks(4096) {
        if cancelled() {
            return Err("Musical-key analysis cancelled".into());
        }
        if chunk.iter().any(|s| !s.is_finite()) {
            return Err("Musical-key source contains non-finite samples".into());
        }
    }
    if rate < 8000 {
        return Ok(Analysis::unknown());
    }
    let n = ((u64::from(rate) * 512 / 1000) as usize)
        .next_power_of_two()
        .max(4096);
    let source_frames = pcm.len() / channels;
    if source_frames < n {
        return Ok(Analysis::unknown());
    }
    let window: Vec<_> = (0..n)
        .map(|i| 0.5 - 0.5 * (TAU * i as f64 / (n - 1) as f64).cos())
        .collect();
    let mut frame = vec![Complex::default(); n];
    let mut power = vec![0.0; n / 2 + 1];
    let mut chroma = [0.0f64; 12];
    let mut frames = 0u64;
    for start in (0..=source_frames - n).step_by(n / 2) {
        if cancelled() {
            return Err("Musical-key analysis cancelled".into());
        }
        power.fill(0.0);
        for channel in 0..channels.min(2) {
            for i in 0..n {
                frame[i] = Complex {
                    re: f64::from(pcm[(start + i) * channels + channel]) * window[i],
                    im: 0.0,
                };
            }
            fft(&mut frame);
            if cancelled() {
                return Err("Musical-key analysis cancelled".into());
            }
            for i in 0..=n / 2 {
                power[i] += frame[i].power();
            }
        }
        let strongest = power.iter().copied().fold(0.0f64, f64::max);
        let mut local = [0.0f64; 12];
        if strongest >= 1e-8 {
            for bin in 2..n / 2 - 1 {
                let p = power[bin];
                if p < strongest * 1e-5 || p <= power[bin - 1] || p <= power[bin + 1] {
                    continue;
                }
                let (left, center, right) = (
                    power[bin - 1].max(1e-30).ln(),
                    p.ln(),
                    power[bin + 1].max(1e-30).ln(),
                );
                let denominator = left - 2.0 * center + right;
                let offset = if denominator.abs() > 1e-12 {
                    (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
                } else {
                    0.0
                };
                let frequency = (bin as f64 + offset) * f64::from(rate) / n as f64;
                if !(55.0..=3500.0).contains(&frequency) {
                    continue;
                }
                let note = 69.0 + 12.0 * (frequency / 440.0).log2();
                let nearest = note.round();
                let weight = (PI * (note - nearest).abs()).cos().powi(2);
                local[(nearest as i32).rem_euclid(12) as usize] += p.sqrt() * weight;
            }
            let total = local.iter().sum::<f64>();
            if total > 0.0 {
                for i in 0..12 {
                    chroma[i] += local[i] / total;
                }
                frames += 1;
            }
        }
        progress((start + n) as u64, source_frames as u64);
    }
    if cancelled() {
        return Err("Musical-key analysis cancelled".into());
    }
    progress(source_frames as u64, source_frames as u64);
    if frames == 0 {
        return Ok(Analysis::unknown());
    }
    let mut candidates = [(
        0.0,
        Key {
            tonic: 0,
            minor: false,
        },
    ); 24];
    for mode in 0..2 {
        for tonic in 0..12 {
            candidates[mode * 12 + tonic] = (
                correlation(&chroma, tonic, mode),
                Key {
                    tonic: tonic as u8,
                    minor: mode == 1,
                },
            );
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let score = candidates[0].0;
    let margin = score - candidates[1].0;
    Ok(Analysis {
        key: (frames >= MIN_FRAMES && score >= MIN_SCORE && margin >= MIN_MARGIN)
            .then_some(candidates[0].1),
        score,
        margin,
        frames,
    })
}

#[cfg(test)]
mod tests;

/// Describe a completed estimate without suggesting a probability.
/// Takes the measured result; returns localized conventional/harmonic notation, correlation and ambiguity margin.
pub(crate) fn description(analysis: Analysis) -> String {
    let key = analysis.key.map_or_else(
        || crate::localization::text("Unknown musical key").into(),
        |key| format!("{} · {}", key.conventional(), key.harmonic()),
    );
    crate::localization::format("Analyzed key: {}; correlation {}; margin {}; tonal frames {} (scores are not probabilities)", &[
        key,crate::localization::number(analysis.score,3),crate::localization::number(analysis.margin,3),analysis.frames.to_string()])
}

/// Select the effective source key without changing saved metadata.
/// Takes the exact version, unsaved fallback and metadata lock; returns borrowed saved text or an analyzed label with its provenance.
pub(crate) fn effective<'a>(
    version: Option<&'a crate::library::Version>,
    fallback: &'a str,
    protected: bool,
) -> (Cow<'a, str>, &'static str) {
    let saved = version.map_or(fallback, |version| version.metadata.key.as_str());
    let tags = version.and_then(|version| version.tags.as_ref());
    let analysis = version
        .and_then(|version| version.analysis.as_ref())
        .and_then(|record| record.key.as_ref());
    let authoritative =
        tags.is_some_and(|tags| tags.overrides.key.is_some() || tags.observed.key.is_some());
    if protected {
        (Cow::Borrowed(saved), "Locked saved key")
    } else if authoritative {
        (Cow::Borrowed(saved), tags.unwrap().key_source())
    } else if let Some(key) = analysis.and_then(|measurement| measurement.value.key) {
        (Cow::Owned(key.conventional()), "Analyzed musical key")
    } else {
        (Cow::Borrowed(saved), "Filename or catalog key hint")
    }
}

/// Format a visible key with retained provenance.
/// Takes the exact version, unsaved fallback and metadata lock; returns conventional/harmonic text and the separate analysis explanation.
pub(crate) fn display(
    version: Option<&crate::library::Version>,
    fallback: &str,
    protected: bool,
) -> (String, String) {
    let (value, source) = effective(version, fallback, protected);
    let analysis = version
        .and_then(|version| version.analysis.as_ref())
        .and_then(|record| record.key.as_ref());
    let cell = Key::parse(&value).map_or_else(
        || {
            if value.is_empty() {
                "—".into()
            } else {
                value.to_string()
            }
        },
        |key| format!("{} · {}", key.conventional(), key.harmonic()),
    );
    let mut detail = crate::localization::format(
        "Key: {} · {}",
        &[value.into_owned(), crate::localization::text(source).into()],
    );
    if let Some(analysis) = analysis {
        detail.push_str("; ");
        detail.push_str(&description(analysis.value));
    }
    (cell, detail)
}
