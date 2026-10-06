use serde::{Deserialize, Serialize};
pub(crate) mod edit;
pub(crate) mod preset;

/// Keep clip appearance and activation with its musical content.
/// Moving or copying a clip retains these values; disabled clips retain their media and notes without accepting a launch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Properties {
    pub color: Option<[u8; 3]>,
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "super::clip_launch::Policy::is_default")]
    pub launch: super::clip_launch::Policy,
}
impl Properties {
    /// Omit unchanged legacy clip properties.
    /// Takes these properties; returns true when older project representations can remain unchanged.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
#[cfg(test)]
mod tests;
