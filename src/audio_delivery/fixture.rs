use std::path::PathBuf;

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
