use crate::project_file::SaveOutcome;
use std::{io::Write, os::unix::fs::OpenOptionsExt, path::Path};

/// Explain a completed publication's durability.
/// Args: atomic publication result. Returns a warning only when files are committed but directory durability is uncertain.
pub(crate) fn warning(outcome: SaveOutcome) -> Option<String> {
    match outcome {
        SaveOutcome::Durable => None,
        SaveOutcome::CommittedButDirectorySyncFailed(error) => Some(format!(
            "Files committed; parent directory could not sync: {error}. Keep this output; retry requires a new folder"
        )),
    }
}

/// Retain the committed warning beside delivered audio.
/// Args: published folder and optional warning. Returns the same warning, extended if its sidecar could not flush; committed audio is never removed.
pub(crate) fn retain(folder: &Path, warning: Option<String>) -> Option<String> {
    let warning = warning?;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(folder.join("publication-warning.json"))
            .map_err(|e| e.to_string())?;
        file.write_all(
            &serde_json::to_vec_pretty(
                &serde_json::json!({"committed":true,"directory_durable":false,"warning":warning}),
            )
            .map_err(|e| e.to_string())?,
        )
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
    })();
    Some(match result {
        Ok(()) => warning,
        Err(error) => format!("{warning}; warning sidecar could not flush: {error}"),
    })
}
