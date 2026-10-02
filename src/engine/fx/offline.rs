//! Retained device identity and serialized state while its processor is absent.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeviceState {
    pub schema: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OfflineDevice {
    pub identifier: String,
    pub state: Option<DeviceState>,
    label: String,
    bytes: usize,
}
impl OfflineDevice {
    /// Retain an unavailable device without constructing or executing it.
    /// `identifier` is its persisted ID; `state` holds opaque schema-bound data.
    /// Returns a bounded placeholder or an error for invalid retained metadata.
    pub fn new(identifier: String, state: Option<DeviceState>) -> Result<Self, String> {
        if identifier.is_empty() || identifier.len() > 1024 || identifier.chars().any(char::is_control)
            || state.as_ref().is_some_and(|state| state.schema == 0 || state.data.len() > 1024 * 1024)
        {
            return Err("Device identity or serialized state exceeds supported bounds".into());
        }
        let label = format!("Unavailable · {identifier}");
        let bytes = identifier.capacity() + label.capacity()
            + state.as_ref().map_or(0, |state| state.data.capacity())
            + std::mem::size_of::<Self>();
        Ok(Self { identifier, state, label, bytes })
    }
    pub fn label(&self) -> &str { &self.label }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
