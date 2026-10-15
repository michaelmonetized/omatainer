use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Keep the next qualification's small native producer files.
/// Takes a stable case name; returns a new persistent directory under the configured /home receipt root, refusing collisions.
pub(crate) fn producer(case: &str) -> Option<PathBuf> {
    assert!(!case.is_empty() && case.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    let root = PathBuf::from(std::env::var_os("OMATAINER_DELIVERY_RECEIPTS")?);
    assert!(root.is_absolute() && root.starts_with("/home"));
    let parent = root.join("producers");
    std::fs::create_dir_all(&parent).unwrap();
    let directory = parent.join(case);
    std::fs::create_dir(&directory).unwrap();
    Some(directory)
}

/// Bind a retained native fixture to its current bytes.
/// Takes an absolute small regular producer path; returns its explicit path, byte length and SHA256 for independent consumers.
pub(crate) fn file(path: &Path) -> serde_json::Value {
    assert!(path.is_absolute() && path.starts_with("/home"));
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_file() && metadata.len() <= 16 * 1024 * 1024);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len() as u64, metadata.len());
    serde_json::json!({"path":path,"size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))})
}

/// Retain an exact native fixture receipt when qualification requests one.
/// Takes a stable case identifier and measured JSON; writes source-bound software proof under the caller's /home receipt directory.
pub(crate) fn retain(case: &str, mut value: serde_json::Value) {
    if let Some(root) = std::env::var_os("OMATAINER_DELIVERY_RECEIPTS") {
        let root = PathBuf::from(root);
        assert!(root.starts_with("/home"));
        std::fs::create_dir_all(&root).unwrap();
        value["schema"] = serde_json::json!(1);
        value["case"] = serde_json::json!(case);
        value["source_revision"] =
            serde_json::json!(std::env::var("OMATAINER_DELIVERY_REVISION").ok());
        value["boundaries"] = serde_json::json!([
            "headless native software and encoded files",
            "no native window/device/listening/physical proof",
            "not long-session endurance"
        ]);
        std::fs::write(
            root.join(format!("{case}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
