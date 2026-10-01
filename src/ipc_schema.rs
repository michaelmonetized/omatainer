//! Typed schema for every shipped IPC opcode; conversion never wraps a target.
use crate::engine::{Command, DECKS, SCENES};
use anyhow::Context;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

macro_rules! index {
    ($type:ident, $count:ident, $field:literal) => {
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct $type(u8);
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use serde::de::Error;
                let message = || {
                    format!(
                        "{} must be a zero-based integer from 0 through {}",
                        $field,
                        $count - 1
                    )
                };
                let value =
                    u64::deserialize(deserializer).map_err(|_| D::Error::custom(message()))?;
                if value >= $count as u64 {
                    return Err(D::Error::custom(message()));
                }
                Ok(Self(value as u8))
            }
        }
    };
}
index!(DeckIndex, DECKS, "deck");
index!(SceneIndex, SCENES, "n");

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Operation {
    Ping {},
    Status {},
    Follow {},
    Play {},
    Stop {},
    TogglePlay {},
    Record {},
    Tap {},
    Scene { n: SceneIndex },
    DeckPlay { deck: DeckIndex },
    DeckCue { deck: DeckIndex },
}

impl Operation {
    pub fn parse(value: &Value) -> anyhow::Result<Self> {
        let mut object = value
            .as_object()
            .context("IPC request must be an object")?
            .clone();
        // Correlation is independently checked before this schema is parsed.
        object.remove("id");
        Ok(serde_json::from_value(Value::Object(object))?)
    }

    pub fn command(self) -> Option<Command> {
        Some(match self {
            Self::Ping {} | Self::Status {} | Self::Follow {} => return None,
            Self::Play {} => Command::Play,
            Self::Stop {} => Command::Stop,
            Self::TogglePlay {} => Command::TogglePlay,
            Self::Record {} => Command::Record,
            Self::Tap {} => Command::Tap(std::time::Instant::now()),
            Self::Scene { n } => Command::LaunchScene { scene: n.0 },
            Self::DeckPlay { deck } => Command::DeckPlay { deck: deck.0 },
            Self::DeckCue { deck } => Command::DeckCue { deck: deck.0 },
        })
    }
}
