//! Provenance is explicit; neither filename numbers nor the tempo heuristic
//! provide a calibrated confidence score. User values always win reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Origin {
    Unknown,
    FilenameHint,
    Heuristic,
    EmbeddedTag,
    User,
    Builtin,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bpm {
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
    /// A reviewed empty value suppresses tags, estimates and filename hints.
    pub const USER_CLEARED: Self = Self {
        value: None,
        origin: Origin::User,
    };
    pub fn valid(self) -> bool {
        match self.value {
            Some(value) => self.origin != Origin::Unknown && value.is_finite() && value > 1.0,
            None => matches!(self.origin, Origin::Unknown | Origin::User),
        }
    }
    pub fn hint(value: f32) -> Self {
        Self::new(value, Origin::FilenameHint)
    }
    pub fn value(self) -> Option<f32> {
        self.value
    }
    pub fn reconcile(self, analysis: Self) -> Self {
        let priority = |origin| match origin {
            Origin::Unknown => 0,
            Origin::FilenameHint => 1,
            Origin::Heuristic => 2,
            Origin::EmbeddedTag => 3,
            Origin::Builtin => 4,
            Origin::User => 5,
        };
        // An explicit failed/unknown analysis invalidates prior automatic
        // estimates and filename hints, but cannot erase tags or user values.
        if priority(self.origin) > priority(analysis.origin)
            && !(analysis.origin == Origin::Unknown
                && matches!(self.origin, Origin::FilenameHint | Origin::Heuristic))
        {
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
            Origin::EmbeddedTag => "embedded tag",
            Origin::User => "user correction",
            Origin::Builtin => "built-in tempo",
        }
    }
    pub fn cell(self) -> String {
        let Some(value) = self.value else {
            return if self.origin == Origin::User {
                "— user cleared"
            } else {
                "— unknown"
            }
            .into();
        };
        let label = match self.origin {
            Origin::FilenameHint => "hint",
            Origin::Heuristic => "est",
            Origin::EmbeddedTag => "tag",
            Origin::User => "user",
            Origin::Builtin => "built-in",
            Origin::Unknown => "unknown",
        };
        format!("{value:.1} {label}")
    }
}
