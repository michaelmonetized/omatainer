//! Immutable release records. Parsed off the GUI thread; never touch user media.
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

pub const PROTOCOL: &str = "omatainer:offline-license-records:v1";
pub const MANIFEST: &str = include_str!("../licenses/manifest.json");
pub const NOTICES: &str = include_str!("../licenses/notices.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub target: String,
    pub application: String,
    pub scope: String,
    pub entries: Vec<Entry>,
    pub cargo: serde_json::Value,
    pub toolchain: serde_json::Value,
    pub package: BTreeMap<String, String>,
    pub source_files: BTreeMap<String, String>,
    pub external: Vec<String>,
    pub absent: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub category: String,
    pub delivery: String,
    pub license: String,
    pub commercial_use: String,
    pub redistribution: String,
    pub sources: Vec<Record>,
    pub notices: Vec<Record>,
    pub members: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub location: String,
    pub sha256: Option<String>,
}
pub struct Catalog {
    pub manifest: Manifest,
    pub notices: BTreeMap<String, String>,
    pub notice_lines: BTreeMap<String, Vec<std::ops::Range<usize>>>,
}
impl Catalog {
    pub fn parse(manifest: &str, notices: &str) -> Result<Self, String> {
        if manifest.len() > 4 * 1024 * 1024 || notices.len() > 8 * 1024 * 1024 {
            return Err("license records exceed the supported size".into());
        }
        let manifest: Manifest = serde_json::from_str(manifest).map_err(|e| e.to_string())?;
        let notices: BTreeMap<String, String> =
            serde_json::from_str(notices).map_err(|e| e.to_string())?;
        if manifest.schema != 1
            || !matches!(
                manifest.target.as_str(),
                "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
            )
        {
            return Err("unsupported license record version or platform".into());
        }
        let mut ids = HashSet::new();
        if manifest.entries.is_empty() || manifest.entries.len() > 4096 {
            return Err("invalid license index size".into());
        }
        for entry in &manifest.entries {
            if !ids.insert(&entry.id)
                || entry.id.is_empty()
                || entry.notices.is_empty()
                || entry.sources.is_empty()
                || entry.license.is_empty()
                || entry.commercial_use.is_empty()
                || entry.redistribution.is_empty()
                || entry.notices.iter().any(|record| {
                    record
                        .sha256
                        .as_ref()
                        .is_none_or(|id| !notices.contains_key(id))
                })
            {
                return Err(format!(
                    "incomplete or duplicate license entry: {}",
                    entry.id
                ));
            }
        }
        let notice_lines = notices
            .iter()
            .map(|(id, text)| {
                let mut ranges = Vec::new();
                let mut base = 0;
                for line in text.split_inclusive('\n') {
                    let mut start = 0;
                    for (count, (offset, _)) in line.char_indices().enumerate() {
                        if count != 0 && count % 100 == 0 {
                            ranges.push(base + start..base + offset);
                            start = offset;
                        }
                    }
                    ranges.push(base + start..base + line.len());
                    base += line.len();
                }
                (id.clone(), ranges)
            })
            .collect();
        Ok(Self {
            manifest,
            notices,
            notice_lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_index_covers_factory_fonts_components_and_explicit_absences() {
        let catalog = Catalog::parse(MANIFEST, NOTICES).unwrap();
        for (id, members) in [
            ("factory:drum-kit", 6),
            ("factory:pad-banks", 48),
            ("factory:session-stems", 2),
            ("factory:instruments", 3),
        ] {
            assert_eq!(
                catalog
                    .manifest
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .unwrap()
                    .members
                    .len(),
                members
            );
        }
        assert_eq!(
            catalog
                .manifest
                .entries
                .iter()
                .filter(|e| e.category == "font")
                .count(),
            4
        );
        assert_eq!(
            catalog
                .manifest
                .entries
                .iter()
                .filter(|e| e.category == "rust-component")
                .count(),
            catalog.manifest.cargo.as_array().unwrap().len()
        );
        assert_eq!(catalog.manifest.package.len(), 6);
        assert!(!catalog.manifest.source_files.is_empty());
        assert_eq!(catalog.manifest.absent.len(), 3);
        for id in [
            "font:hack",
            "font:ubuntu",
            "font:noto-emoji",
            "font:emoji-icon",
            "crate:symphonia@0.5.5",
        ] {
            let entry = catalog
                .manifest
                .entries
                .iter()
                .find(|e| e.id == id)
                .unwrap();
            assert!(entry
                .notices
                .iter()
                .all(|r| !catalog.notices[r.sha256.as_ref().unwrap()]
                    .trim()
                    .is_empty()));
            assert!(entry
                .notices
                .iter()
                .any(|r| catalog.notices[r.sha256.as_ref().unwrap()].len() > 100));
        }
    }
    #[test]
    fn corrupt_or_future_records_fail_without_invented_permissions() {
        assert!(Catalog::parse("not json", NOTICES).is_err());
        assert!(Catalog::parse(
            &MANIFEST.replace("\"schema\": 1", "\"schema\": 99"),
            NOTICES
        )
        .is_err());
        assert!(Catalog::parse(MANIFEST, "{}").is_err());
    }
}
