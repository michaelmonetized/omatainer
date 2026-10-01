//! One bounded, correlated JSON encoder for ordinary and streaming requests.
use anyhow::Context;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum ReadOperation {
    Status,
    Follow,
}

pub(crate) struct Encoded {
    pub id: Value,
    pub line: String,
}

pub(crate) fn encode(request: &impl Serialize) -> anyhow::Result<Encoded> {
    static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
    let mut request = serde_json::to_value(request)?;
    let object = request
        .as_object_mut()
        .context("IPC request must be an object")?;
    let id = Value::String(format!(
        "{}-{}",
        std::process::id(),
        NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
    ));
    object.insert("id".into(), id.clone());
    let line = format!("{request}\n");
    anyhow::ensure!(
        line.len() - 1 <= crate::ipc_transport::REQUEST_BYTES,
        "IPC request exceeds byte limit"
    );
    Ok(Encoded { id, line })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_operations_share_correlated_bounded_json_envelope() {
        let status = encode(&ReadOperation::Status).unwrap();
        let follow = encode(&ReadOperation::Follow).unwrap();
        assert_ne!(status.id, follow.id);
        for (request, operation) in [(status, "status"), (follow, "follow")] {
            assert!(request.line.ends_with('\n'));
            let wire: Value = serde_json::from_str(&request.line).unwrap();
            assert_eq!(wire, serde_json::json!({"op":operation,"id":request.id}));
        }
        assert!(encode(&"status").is_err());
        assert!(encode(&serde_json::json!({"op":"status", "extra":"x".repeat(crate::ipc_transport::REQUEST_BYTES)})).is_err());
    }
}
