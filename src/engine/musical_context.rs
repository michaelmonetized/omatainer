use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scale {
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    HarmonicMinor,
    MelodicMinor,
    MajorPentatonic,
    MinorPentatonic,
    WholeTone,
    Chromatic,
}

impl Scale {
    pub const ALL: [Self; 13] = [
        Self::Major,
        Self::Minor,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Locrian,
        Self::HarmonicMinor,
        Self::MelodicMinor,
        Self::MajorPentatonic,
        Self::MinorPentatonic,
        Self::WholeTone,
        Self::Chromatic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::Dorian => "Dorian",
            Self::Phrygian => "Phrygian",
            Self::Lydian => "Lydian",
            Self::Mixolydian => "Mixolydian",
            Self::Locrian => "Locrian",
            Self::HarmonicMinor => "Harmonic minor",
            Self::MelodicMinor => "Melodic minor",
            Self::MajorPentatonic => "Major pentatonic",
            Self::MinorPentatonic => "Minor pentatonic",
            Self::WholeTone => "Whole tone",
            Self::Chromatic => "Chromatic",
        }
    }

    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Self::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Self::MajorPentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPentatonic => &[0, 3, 5, 7, 10],
            Self::WholeTone => &[0, 2, 4, 6, 8, 10],
            Self::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Context {
    pub tonic: u8,
    pub scale: Scale,
}

impl Context {
    /// Validate a saved musical key.
    /// Takes this tonic and scale; returns whether the tonic is a MIDI pitch class.
    pub fn valid(self) -> bool {
        self.tonic < 12
    }

    /// Test an absolute MIDI pitch against this scale.
    /// Takes the pitch; returns false for invalid contexts, invalid MIDI pitches or chromatic exceptions.
    pub fn contains(self, pitch: u8) -> bool {
        self.valid()
            && pitch < 128
            && self
                .scale
                .intervals()
                .contains(&((pitch + 12 - self.tonic) % 12))
    }

    /// Move by scale degrees without silently changing chromatic notes.
    /// Takes an absolute MIDI pitch, signed degree count and explicit chromatic permission; returns a valid pitch or refuses the whole out-of-range edit.
    pub fn transpose(self, pitch: u8, degrees: i16, include_chromatic: bool) -> Result<u8, String> {
        if !self.valid() || pitch > 127 || !(-128..=128).contains(&degrees) {
            return Err(
                "Set a valid tonic, MIDI pitch and degree interval before transposing".into(),
            );
        }
        if !self.contains(pitch) && !include_chromatic {
            return Ok(pitch);
        }
        let pitch = if self.contains(pitch) {
            pitch
        } else {
            self.nearest(pitch)?
        };
        let relative = i32::from(pitch) - i32::from(self.tonic);
        let intervals = self.scale.intervals();
        let octave = relative.div_euclid(12);
        let index = intervals
            .iter()
            .position(|v| i32::from(*v) == relative.rem_euclid(12))
            .unwrap() as i32;
        let degree = octave * intervals.len() as i32 + index + i32::from(degrees);
        let octave = degree.div_euclid(intervals.len() as i32);
        let interval = intervals[degree.rem_euclid(intervals.len() as i32) as usize];
        let next = i32::from(self.tonic) + octave * 12 + i32::from(interval);
        u8::try_from(next)
            .ok()
            .filter(|p| *p < 128)
            .ok_or_else(|| "Scale transpose would move a note outside MIDI pitches 0–127".into())
    }

    /// Find the closest valid scale pitch.
    /// Takes an absolute MIDI pitch; returns the nearest pitch in this scale, choosing the lower pitch for equal distances.
    pub fn nearest(self, pitch: u8) -> Result<u8, String> {
        if !self.valid() || pitch > 127 {
            return Err(
                "Choose a valid tonic and MIDI pitch before fitting notes to a scale".into(),
            );
        }
        (0..=127_u8)
            .filter(|p| self.contains(*p))
            .min_by_key(|p| (p.abs_diff(pitch), *p))
            .ok_or_else(|| "This scale contains no MIDI pitches".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Clip,
    Song,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub context: Option<Context>,
    pub origin: Origin,
}

/// Resolve a captured clip's saved musical context.
/// Takes the clip override and song context; returns the unchanged explicit owner and its provenance.
pub(crate) fn resolve(clip: Option<Context>, song: Option<Context>) -> Resolved {
    if let Some(context) = clip {
        Resolved {
            context: Some(context),
            origin: Origin::Clip,
        }
    } else if let Some(context) = song {
        Resolved {
            context: Some(context),
            origin: Origin::Song,
        }
    } else {
        Resolved {
            context: None,
            origin: Origin::None,
        }
    }
}

/// Resolve one opted-in instrument pad to a scale degree.
/// Takes the explicit context, octave and pad identity; returns a bounded pitch or refuses an invalid or out-of-range gate.
pub(crate) fn pad_pitch(context: Context, octave: i8, pad: u8) -> Option<u8> {
    if !context.valid() || !(1..=7).contains(&octave) || pad >= 16 {
        return None;
    }
    let root = (i16::from(octave) + 1) * 12 + i16::from(context.tonic);
    let root = u8::try_from(root).ok().filter(|root| *root < 128)?;
    context.transpose(root, i16::from(pad), false).ok()
}

impl super::Snapshot {
    /// Resolve the displayed instrument pad to its actual gate pitch.
    /// Takes one pad identity; returns the same saved-context pitch used by the renderer, refusing unavailable scale notes.
    pub(crate) fn sampler_gate_pitch(&self, pad: u8) -> Option<u8> {
        if self.sampler_scale && self.sampler_inst.synth().is_some() {
            self.sampler_context
                .and_then(|context| pad_pitch(context, self.sampler_oct, pad))
        } else if pad < 16 {
            Some(super::sampler_pitch(
                self.sampler_inst,
                self.sampler_oct,
                pad,
            ))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Active {
    pub context: Option<Context>,
    pub conflicted: bool,
    pub unspecified: bool,
}

/// Observe the keys of concurrently playing clips.
/// Takes the saved song key and overrides of active clips; returns explicit disagreement or unspecified ownership without choosing a new song key or changing pitches.
pub(crate) fn active(
    song: Option<Context>,
    clips: impl IntoIterator<Item = Option<Context>>,
) -> Active {
    let mut status = Active::default();
    let mut any = false;
    for clip in clips {
        any = true;
        match resolve(clip, song).context {
            Some(context) => {
                if let Some(first) = status.context {
                    status.conflicted |= first != context;
                } else {
                    status.context = Some(context);
                }
            }
            None => status.unspecified = true,
        }
    }
    if !any {
        status.context = song;
        status.unspecified = song.is_none();
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_major_scales_keep_chromatic_exceptions_and_move_exact_degrees() {
        for (scale, expected) in [
            (Scale::Dorian, [60, 62, 63, 65, 67, 69, 70]),
            (Scale::Phrygian, [60, 61, 63, 65, 67, 68, 70]),
            (Scale::HarmonicMinor, [60, 62, 63, 65, 67, 68, 71]),
        ] {
            let context = Context { tonic: 0, scale };
            for degree in 0..7 {
                assert_eq!(
                    context.transpose(60, degree, false).unwrap(),
                    expected[degree as usize]
                );
            }
            assert_eq!(context.transpose(60, 7, false).unwrap(), 72);
            assert_eq!(context.transpose(72, -7, false).unwrap(), 60);
            for pitch in 0..=127 {
                let next = context.transpose(pitch, 0, false).unwrap();
                assert_eq!(next, pitch);
                if !context.contains(pitch) {
                    assert_eq!(context.transpose(pitch, 3, false).unwrap(), pitch);
                    assert!(context.contains(context.transpose(pitch, 0, true).unwrap()));
                }
            }
        }
    }

    #[test]
    fn all_saved_contexts_roundtrip_and_pitch_boundaries_refuse_instead_of_clamping() {
        for scale in Scale::ALL {
            for tonic in 0..12 {
                let context = Context { tonic, scale };
                let decoded: Context =
                    serde_json::from_slice(&serde_json::to_vec(&context).unwrap()).unwrap();
                assert_eq!(decoded, context);
                for pitch in 0..=127 {
                    if context.contains(pitch) {
                        if let Ok(next) = context.transpose(pitch, 1, false) {
                            assert!(next > pitch && context.contains(next));
                            assert_eq!(context.transpose(next, -1, false).unwrap(), pitch);
                        }
                    }
                }
            }
        }
        let context = Context {
            tonic: 0,
            scale: Scale::Chromatic,
        };
        assert!(context.transpose(0, -1, false).is_err());
        assert!(context.transpose(127, 1, false).is_err());
        assert!(!Context {
            tonic: 12,
            scale: Scale::Major
        }
        .valid());
    }

    #[test]
    fn clip_overrides_have_explicit_provenance_and_never_rewrite_song_context() {
        let song = Context {
            tonic: 2,
            scale: Scale::Dorian,
        };
        let clip = Context {
            tonic: 4,
            scale: Scale::Phrygian,
        };
        assert_eq!(
            resolve(None, Some(song)),
            Resolved {
                context: Some(song),
                origin: Origin::Song
            }
        );
        assert_eq!(
            resolve(Some(clip), Some(song)),
            Resolved {
                context: Some(clip),
                origin: Origin::Clip
            }
        );
        assert_eq!(resolve(None, None).origin, Origin::None);
        assert_eq!(song.tonic, 2);
    }
    #[test]
    fn simultaneous_clip_keys_report_conflicts_without_changing_the_song_or_unknown_owners() {
        let dorian = Context {
            tonic: 0,
            scale: Scale::Dorian,
        };
        let phrygian = Context {
            tonic: 2,
            scale: Scale::Phrygian,
        };
        assert_eq!(
            active(Some(dorian), [None, Some(dorian)]),
            Active {
                context: Some(dorian),
                conflicted: false,
                unspecified: false
            }
        );
        assert_eq!(
            active(Some(dorian), [None, Some(phrygian)]),
            Active {
                context: Some(dorian),
                conflicted: true,
                unspecified: false
            }
        );
        assert_eq!(
            active(None, [None, Some(phrygian)]),
            Active {
                context: Some(phrygian),
                conflicted: false,
                unspecified: true
            }
        );
        assert_eq!(
            active(Some(dorian), []),
            Active {
                context: Some(dorian),
                conflicted: false,
                unspecified: false
            }
        );
        assert_eq!(
            active(None, []),
            Active {
                context: None,
                conflicted: false,
                unspecified: true
            }
        );
    }
}

#[cfg(test)]
mod integration_tests;
