use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum LibSource {
    Builtin(BuiltinStem),
    File(PathBuf),
    Removable {
        volume_id: String,
        relative_path: PathBuf,
    },
    Provider {
        provider: String,
        media_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum BuiltinStem {
    Drums,
    Harmony,
}

impl BuiltinStem {
    pub fn index(self) -> u8 {
        match self {
            Self::Drums => 0,
            Self::Harmony => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub source: LibSource,
    pub title: String,
}

/// Identity of the bytes inspected by a scan/decode, not just their pathname.
/// This is cache invalidation metadata, not a cryptographic content identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileFingerprint {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileFingerprint {
    pub fn byte_len(self) -> u64 { self.length }
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
    pub fn read(path: &std::path::Path) -> Option<Self> {
        std::fs::metadata(path)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| Self::from_metadata(&m))
    }
}
