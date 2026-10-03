//! Saved recovery limits use the same direct numeric and contextual-help path
//! as the other preference editors. Applying settings does not write a journal.
use super::*;

pub(super) fn edit(ui: &mut Ui, config: &mut crate::recovery::Config) {
    use crate::recovery::*;
    ui.label(tr!("Dirty edit state is batched about every 2 seconds. Checkpoints compact that journal separately; the Recovery window reports the age of the last confirmed durable record."));
    let mut interval = config.checkpoint_seconds as f32;
    preferences::float_control(
        ui,
        "Recovery checkpoint interval",
        &mut interval,
        MIN_CHECKPOINT_SECONDS as f32,
        MAX_CHECKPOINT_SECONDS as f32,
        1.0,
        " seconds",
        HelpControl::RecoveryInterval,
    );
    config.checkpoint_seconds = interval.round() as u32;
    let mut retention = config.retention as f32;
    preferences::float_control(
        ui,
        "Recovery generations per session",
        &mut retention,
        MIN_RETENTION as f32,
        MAX_RETENTION as f32,
        1.0,
        " generations",
        HelpControl::RecoveryRetention,
    );
    config.retention = retention.round() as u8;
    let mut storage = config.max_bytes as f32 / MIB as f32;
    let displayed_storage = storage;
    preferences::float_control(
        ui,
        "Recovery storage limit",
        &mut storage,
        (MIN_STORAGE_BYTES / MIB) as f32,
        (MAX_STORAGE_BYTES / MIB) as f32,
        1.0,
        " MiB",
        HelpControl::RecoveryStorage,
    );
    if storage != displayed_storage {
        config.max_bytes = storage.round() as u64 * MIB;
    }
    ui.label(tr!("The global limit includes embedded media, journals and staging files. At the limit, new writes fail visibly; the only usable recovery is never deleted automatically to make room. Retention applies to generations within a session, not other sessions."));
}
