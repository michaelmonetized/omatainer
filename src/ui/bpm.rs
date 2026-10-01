//! Provenance is explicit; neither filename numbers nor the tempo heuristic
//! provide a calibrated confidence score. User values always win reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Unknown,
    FilenameHint,
    Heuristic,
    User,
    Builtin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bpm {
    value: Option<f32>,
    pub origin: Origin,
}

impl Bpm {
    pub const UNKNOWN: Self = Self {
        value: None,
        origin: Origin::Unknown,
    };
    pub fn new(value: f32, origin: Origin) -> Self {
        if origin != Origin::Unknown && value.is_finite() && value > 1.0 {
            Self {
                value: Some(value),
                origin,
            }
        } else {
            Self::UNKNOWN
        }
    }
    pub fn hint(value: f32) -> Self {
        Self::new(value, Origin::FilenameHint)
    }
    pub fn value(self) -> Option<f32> {
        self.value
    }
    pub fn reconcile(self, analysis: Self) -> Self {
        if self.origin == Origin::User {
            self
        } else {
            analysis
        }
    }
    pub fn label(self) -> &'static str {
        match self.origin {
            Origin::Unknown => "unknown",
            Origin::FilenameHint => "filename hint · unverified",
            Origin::Heuristic => "heuristic estimate · unverified",
            Origin::User => "user correction",
            Origin::Builtin => "built-in tempo",
        }
    }
    pub fn cell(self) -> String {
        let Some(value) = self.value else {
            return "— unknown".into();
        };
        let label = match self.origin {
            Origin::FilenameHint => "hint",
            Origin::Heuristic => "est",
            Origin::User => "user",
            Origin::Builtin => "built-in",
            Origin::Unknown => "unknown",
        };
        format!("{value:.1} {label}")
    }
}
