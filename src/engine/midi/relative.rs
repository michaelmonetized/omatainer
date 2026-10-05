//! Relative wire formats are selected by a binding, never inferred from a byte.

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RelativeEncoding {
    /// Pioneer DDJ jogs: 0x40 is stationary, 0x41 is +1, 0x3f is -1.
    OffsetBinary,
    /// Seven-bit signed value: 0 is stationary, 1 is +1, 0x7f is -1.
    /// Defined by Akai APC40 MkII protocol v1.2, p. 37; no new APC action
    /// is assigned here. Available for explicitly configured relative bindings.
    TwosComplement,
}

impl RelativeEncoding {
    fn steps(self, value: u8) -> Option<i16> {
        if value > 127 {
            return None;
        }
        Some(match self {
            Self::OffsetBinary => i16::from(value) - 64,
            Self::TwosComplement if value >= 64 => i16::from(value) - 128,
            Self::TwosComplement => i16::from(value),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelativeSpec {
    pub encoding: RelativeEncoding,
    /// Engine delta per wire step. Positive and finite; direction is decoded
    /// by the encoding. This is application sensitivity, not hardware proof.
    pub scale: f32,
}

impl RelativeSpec {
    pub(super) const PIONEER_JOG: Self = Self {
        encoding: RelativeEncoding::OffsetBinary,
        scale: 0.35,
    };

    pub(super) fn is_valid(self) -> bool {
        self.scale.is_finite() && self.scale > 0.0 && (self.scale * 64.0).is_finite()
    }

    pub(super) fn decode(self, value: u8) -> Option<f32> {
        self.is_valid().then_some(())?;
        Some(f32::from(self.encoding.steps(value)?) * self.scale)
    }
}
