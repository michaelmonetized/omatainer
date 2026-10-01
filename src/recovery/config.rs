//! User limits for the durable journal. The edit batching cadence is separate
//! from checkpoint compaction; neither promises synchronous per-command saving.
use serde::{Deserialize, Serialize};

pub const MIB: u64 = 1024 * 1024;
pub const JOURNAL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
pub const MIN_CHECKPOINT_SECONDS: u32 = 5;
pub const MAX_CHECKPOINT_SECONDS: u32 = 3600;
pub const MIN_RETENTION: u8 = 2;
pub const MAX_RETENTION: u8 = 20;
pub const MIN_STORAGE_BYTES: u64 = 64 * MIB;
pub const MAX_STORAGE_BYTES: u64 = 16 * 1024 * MIB;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub checkpoint_seconds: u32,
    pub retention: u8,
    pub max_bytes: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            checkpoint_seconds: 30,
            retention: 3,
            max_bytes: 2 * 1024 * MIB,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_CHECKPOINT_SECONDS..=MAX_CHECKPOINT_SECONDS).contains(&self.checkpoint_seconds) {
            return Err("Recovery checkpoint interval must be 5–3600 seconds".into());
        }
        if !(MIN_RETENTION..=MAX_RETENTION).contains(&self.retention) {
            return Err(
                "Retain 2–20 recovery generations, including a prior valid generation".into(),
            );
        }
        if !(MIN_STORAGE_BYTES..=MAX_STORAGE_BYTES).contains(&self.max_bytes) {
            return Err("Recovery storage limit must be 64 MiB–16 GiB".into());
        }
        Ok(())
    }
}
