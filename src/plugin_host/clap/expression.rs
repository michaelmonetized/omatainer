use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub note_id: i32,
    pub port: i16,
    pub channel: i16,
    pub key: i16,
}
impl Target {
    /// Select a global parameter or every note. Args: none. Returns CLAP's wildcard address.
    pub const fn all() -> Self {
        Self {
            note_id: -1,
            port: -1,
            channel: -1,
            key: -1,
        }
    }
    fn valid(self) -> bool {
        self.note_id >= -1
            && (-1..8).contains(&self.port)
            && (-1..16).contains(&self.channel)
            && (-1..128).contains(&self.key)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", deny_unknown_fields)]
pub(crate) enum NativeEvent {
    Note {
        target: Target,
        on: bool,
        velocity: f64,
    },
    Expression {
        target: Target,
        kind: i32,
        value: f64,
    },
    Modulation {
        target: Target,
        parameter: u32,
        amount: f64,
    },
}
impl NativeEvent {
    /// Check a native event before queue admission. Args: event. Returns whether its address, values and expression type are within CLAP's bounds.
    pub fn valid(self) -> bool {
        match self {
            Self::Note {
                target, velocity, ..
            } => {
                target.valid()
                    && target.port >= 0
                    && target.channel >= 0
                    && target.key >= 0
                    && velocity.is_finite()
                    && (0.0..=1.).contains(&velocity)
            }
            Self::Expression {
                target,
                kind,
                value,
            } => {
                target.valid()
                    && value.is_finite()
                    && match kind {
                        0 => (0.0..=4.).contains(&value),
                        1 | 3..=6 => (0.0..=1.).contains(&value),
                        2 => (-120.0..=120.).contains(&value),
                        _ => false,
                    }
            }
            Self::Modulation { target, amount, .. } => target.valid() && amount.is_finite(),
        }
    }
}
