//! Worker-only Linux media locations. Filesystem UUID is persistent identity;
//! namespace, mount ID and device numbers are access-time guards, not identity.
//! Nothing in this module belongs on the GUI or audio callback.
use crate::engine::media_source::LibSource;
use std::os::unix::fs::MetadataExt;
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

mod linux;
#[cfg(test)]
mod tests;

const MAX_MOUNTS: usize = 8192;
const MAX_BLOCKS: usize = 4096;
const MAX_PATH: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Device(pub u32, pub u32);
impl Device {
    fn from_raw(raw: u64) -> Self {
        Self(libc::major(raw), libc::minor(raw))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Offline,
    NotVisible,
    Missing,
    Ambiguous,
    Changed,
    Invalid,
    Unsupported,
    Capacity,
    Unreadable(String),
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => f.write_str("Volume is offline or not mounted"),
            Self::NotVisible => {
                f.write_str("Volume is mounted, but this path is outside its visible mounts")
            }
            Self::Missing => f.write_str("File is missing on the mounted volume"),
            Self::Ambiguous => f.write_str("Volume identity or visible mount is ambiguous"),
            Self::Changed => f.write_str("Media mount or source changed during access; retry"),
            Self::Invalid => f.write_str("Invalid volume identity or relative media path"),
            Self::Unsupported => f.write_str("This media namespace cannot be resolved locally"),
            Self::Capacity => f.write_str("Media discovery exceeds its bounded inventory"),
            Self::Unreadable(e) => write!(f, "Cannot inspect media: {e}"),
        }
    }
}
fn io(error: std::io::Error) -> Failure {
    Failure::Unreadable(error.to_string().chars().take(256).collect())
}
#[derive(Clone, Debug)]
struct Mount {
    id: u64,
    device: Device,
    root: PathBuf,
    point: PathBuf,
    source: PathBuf,
    block: Option<Device>,
}
#[derive(Clone, Debug)]
struct Block {
    device: Device,
    uuid: String,
    removable: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    namespace: (u64, u64),
    mounts: Vec<Mount>,
    blocks: Vec<Block>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    namespace: (u64, u64),
    mount: u64,
    device: Device,
    block: Device,
    uuid: String,
    root: PathBuf,
    point: PathBuf,
}
#[derive(Clone, Debug)]
pub(crate) struct Location {
    pub source: LibSource,
    pub path: PathBuf,
    stamp: Option<Stamp>,
}
impl Location {
    /// Re-discovery is deliberately worker-only. The caller also checks its
    /// opened descriptor's fingerprint before/after hashing or decoding.
    pub fn recheck(&self) -> Result<(), Failure> {
        if self.stamp.is_none() {
            return Ok(());
        }
        let snapshot = Snapshot::discover()?;
        snapshot.inspect(self)?;
        let current = snapshot.identify_canonical(self.path.clone())?;
        if self.stamp != current.stamp || self.path != current.path || self.source != current.source
        {
            return Err(Failure::Changed);
        }
        Ok(())
    }
}
fn valid_uuid(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= 128
        && uuid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn relative(path: &Path, empty: bool) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let normalized: PathBuf = path.components().collect();
    path.as_os_str().len() <= MAX_PATH
        && !path.as_os_str().as_bytes().contains(&0)
        && (empty || !path.as_os_str().is_empty())
        && path.as_os_str() == normalized.as_os_str()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= MAX_PATH
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
impl Snapshot {
    pub fn discover() -> Result<Self, Failure> {
        linux::discover()
    }

    fn unique_block(&self, uuid: &str) -> Result<Device, Failure> {
        if !valid_uuid(uuid) {
            return Err(Failure::Invalid);
        }
        let mut matches = self
            .blocks
            .iter()
            .filter(|b| b.uuid == uuid)
            .map(|b| b.device);
        let first = matches.next().ok_or(Failure::Offline)?;
        if matches.any(|next| next != first) {
            return Err(Failure::Ambiguous);
        }
        Ok(first)
    }
    fn mount_at(&self, path: &Path) -> Result<&Mount, Failure> {
        let mount = self
            .mounts
            .iter()
            .filter(|m| path.starts_with(&m.point))
            .max_by_key(|m| m.point.components().count())
            .ok_or(Failure::Changed)?;
        // Mount IDs may be reused. Refuse an overmount ambiguity rather than
        // guessing that the numerically largest ID is the currently visible one.
        if self
            .mounts
            .iter()
            .filter(|m| m.point == mount.point)
            .count()
            != 1
        {
            return Err(Failure::Ambiguous);
        }
        Ok(mount)
    }
    fn stamp(&self, mount: &Mount, block: Device, uuid: &str) -> Stamp {
        Stamp {
            namespace: self.namespace,
            mount: mount.id,
            device: mount.device,
            block,
            uuid: uuid.into(),
            root: mount.root.clone(),
            point: mount.point.clone(),
        }
    }
    /// Explicit path imports adopt a canonical identity once, off the GUI.
    /// Internal/non-block files remain ordinary local-file sources.
    pub fn identify(&self, path: &Path) -> Result<Location, Failure> {
        let path = path.canonicalize().map_err(io)?;
        self.identify_canonical(path)
    }
    fn identify_canonical(&self, path: PathBuf) -> Result<Location, Failure> {
        if !absolute(&path) {
            return Err(Failure::Invalid);
        }
        let mount = self.mount_at(&path)?;
        let Some(block) = mount.block else {
            return Ok(Location {
                source: LibSource::File(path.clone()),
                path,
                stamp: None,
            });
        };
        let Some(identity) = self
            .blocks
            .iter()
            .find(|b| b.device == block && b.removable)
        else {
            return Ok(Location {
                source: LibSource::File(path.clone()),
                path,
                stamp: None,
            });
        };
        if identity.uuid.is_empty() {
            return Err(Failure::Unsupported);
        }
        self.unique_block(&identity.uuid)?;
        let tail = path
            .strip_prefix(&mount.point)
            .map_err(|_| Failure::Changed)?;
        let within_volume = mount
            .root
            .strip_prefix("/")
            .map_err(|_| Failure::Invalid)?
            .join(tail);
        if !relative(&within_volume, true) {
            return Err(Failure::Invalid);
        }
        let source = LibSource::Removable {
            volume_id: identity.uuid.clone(),
            relative_path: within_volume,
        };
        Ok(Location {
            source,
            path,
            stamp: Some(self.stamp(mount, block, &identity.uuid)),
        })
    }
    /// Resolve a persisted identity at the current mountpoint. The empty
    /// relative path is allowed for watched volume roots, never catalog tracks.
    pub fn resolve(&self, source: &LibSource) -> Result<Location, Failure> {
        match source {
            LibSource::File(path) if absolute(path) => Ok(Location {
                source: source.clone(),
                path: path.clone(),
                stamp: None,
            }),
            LibSource::Removable {
                volume_id,
                relative_path,
            } => {
                if !relative(relative_path, true) {
                    return Err(Failure::Invalid);
                }
                let block = self.unique_block(volume_id)?;
                let mut candidates = Vec::new();
                let mut mounted = false;
                let mut ambiguous = false;
                for mount in self.mounts.iter().filter(|m| m.block == Some(block)) {
                    mounted = true;
                    let root = mount.root.strip_prefix("/").map_err(|_| Failure::Invalid)?;
                    let Ok(tail) = relative_path.strip_prefix(root) else {
                        continue;
                    };
                    let path = mount.point.join(tail);
                    match self.mount_at(&path) {
                        Ok(visible) if visible.id == mount.id => candidates.push((mount, path)),
                        Err(Failure::Ambiguous) => ambiguous = true,
                        _ => {}
                    }
                }
                // Full filesystem views precede bind/subvolume views; aliases
                // of the same physical device are not duplicate volumes.
                candidates.sort_by_key(|(m, _)| (m.root.components().count(), m.id));
                let (mount, path) = candidates.into_iter().next().ok_or(if ambiguous {
                    Failure::Ambiguous
                } else if mounted {
                    Failure::NotVisible
                } else {
                    Failure::Offline
                })?;
                if !absolute(&path) {
                    return Err(Failure::Invalid);
                }
                Ok(Location {
                    source: source.clone(),
                    path,
                    stamp: Some(self.stamp(mount, block, volume_id)),
                })
            }
            LibSource::File(_) => Err(Failure::Invalid),
            LibSource::Builtin(_) | LibSource::Provider { .. } => Err(Failure::Unsupported),
        }
    }
    /// Missing is only established after the same visible mounted volume is
    /// selected. An absent mount never means deleted. Symlink traversal and
    /// nested foreign mounts refuse association instead of following new media.
    pub fn inspect(&self, location: &Location) -> Result<std::fs::Metadata, Failure> {
        if let Some(stamp) = &location.stamp {
            if stamp.namespace != self.namespace || self.unique_block(&stamp.uuid)? != stamp.block {
                return Err(Failure::Changed);
            }
            let mount = self.mount_at(&location.path)?;
            if *stamp != self.stamp(mount, stamp.block, &stamp.uuid)
                || mount.block != Some(stamp.block)
            {
                return Err(Failure::Changed);
            }
            let mut path = mount.point.clone();
            for component in location
                .path
                .strip_prefix(&mount.point)
                .map_err(|_| Failure::Changed)?
                .components()
            {
                path.push(component);
                match path.symlink_metadata() {
                    Ok(meta) if meta.file_type().is_symlink() => return Err(Failure::Unsupported),
                    Ok(meta) if Device::from_raw(meta.dev()) != mount.device => {
                        return Err(Failure::Changed)
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Err(Failure::Missing)
                    }
                    Err(e) => return Err(io(e)),
                }
            }
        }
        let metadata = location.path.metadata().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Failure::Missing
            } else {
                io(e)
            }
        })?;
        if location
            .stamp
            .as_ref()
            .is_some_and(|stamp| Device::from_raw(metadata.dev()) != stamp.device)
        {
            return Err(Failure::Changed);
        }
        Ok(metadata)
    }
}

fn unescape(field: &[u8]) -> Result<PathBuf, Failure> {
    use std::os::unix::ffi::OsStringExt;
    let mut decoded = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        if field[i] == b'\\' {
            let code = field.get(i + 1..i + 4).ok_or(Failure::Invalid)?;
            decoded.push(match code {
                b"040" => b' ',
                b"011" => b'\t',
                b"012" => b'\n',
                b"134" => b'\\',
                _ => return Err(Failure::Invalid),
            });
            i += 4;
        } else {
            if field[i] == 0 {
                return Err(Failure::Invalid);
            }
            decoded.push(field[i]);
            i += 1;
        }
    }
    Ok(std::ffi::OsString::from_vec(decoded).into())
}
fn parse_mounts(bytes: &[u8]) -> Result<Vec<Mount>, Failure> {
    let mut mounts = Vec::new();
    let mut ids = HashSet::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        if mounts.len() == MAX_MOUNTS {
            return Err(Failure::Capacity);
        }
        let fields: Vec<_> = line.split(|b| *b == b' ').take(65).collect();
        if fields.len() > 64 {
            return Err(Failure::Capacity);
        }
        let separator = fields
            .iter()
            .position(|f| *f == b"-")
            .ok_or(Failure::Invalid)?;
        if separator < 6 || fields.len() != separator + 4 || fields.iter().any(|f| f.is_empty()) {
            return Err(Failure::Invalid);
        }
        let number = |v: &[u8]| {
            std::str::from_utf8(v)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(Failure::Invalid)
        };
        let id = number(fields[0])?;
        number(fields[1])?;
        if id == 0 || !ids.insert(id) {
            return Err(Failure::Invalid);
        }
        let mut device = fields[2].split(|b| *b == b':');
        let major = u32::try_from(number(device.next().ok_or(Failure::Invalid)?)?)
            .map_err(|_| Failure::Invalid)?;
        let minor = u32::try_from(number(device.next().ok_or(Failure::Invalid)?)?)
            .map_err(|_| Failure::Invalid)?;
        if device.next().is_some() {
            return Err(Failure::Invalid);
        }
        let root = unescape(fields[3])?;
        let point = unescape(fields[4])?;
        let source = unescape(fields[separator + 2])?;
        if !absolute(&root) || !absolute(&point) {
            return Err(Failure::Invalid);
        }
        mounts.push(Mount {
            id,
            device: Device(major, minor),
            root,
            point,
            source,
            block: None,
        });
    }
    if mounts.is_empty() {
        return Err(Failure::Invalid);
    }
    Ok(mounts)
}
