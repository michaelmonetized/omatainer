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
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mount {
    id: u64,
    device: Device,
    access: Device,
    btrfs: bool,
    qualified: bool,
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
/// Opaque current mount-view guard for worker searches, including File roots.
/// It carries no persistent identity and performs no filesystem work itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Access {
    namespace: (u64, u64),
    mount: Mount,
}
impl Access {
    pub fn check(&self, snapshot: &Snapshot, path: &Path) -> Result<(), Failure> {
        if snapshot.namespace != self.namespace || snapshot.mount_at(path)? != &self.mount {
            return Err(Failure::Changed);
        }
        Ok(())
    }
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
    access: Device,
    btrfs: bool,
    block: Device,
    uuid: String,
    root: PathBuf,
    point: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    pub source: LibSource,
    pub path: PathBuf,
    stamp: Option<Stamp>,
}
impl Location {
    /// A worker resolves typed identity to a transient path. Local paths need
    /// no mount enumeration; removable paths always require current discovery.
    pub fn resolve(source: &LibSource) -> Result<Self, Failure> {
        validate_root_source(source)?;
        match source {
            LibSource::File(path) => Ok(Self {
                source: source.clone(),
                path: path.clone(),
                stamp: None,
            }),
            LibSource::Removable { relative_path, .. } if !relative_path.as_os_str().is_empty() => {
                let snapshot = Snapshot::discover()?;
                let location = snapshot.resolve(source)?;
                snapshot.inspect(&location)?;
                Ok(location)
            }
            _ => Err(Failure::Unsupported),
        }
    }
    /// Descriptor and currently visible path must still identify the opened
    /// regular file. Volume discovery is worker-only and guards mount reuse.
    pub fn verify_file(
        &self,
        file: &std::fs::File,
        expected: crate::engine::media_source::FileFingerprint,
    ) -> Result<(), Failure> {
        use crate::engine::media_source::FileFingerprint;
        let meta = file.metadata().map_err(io)?;
        if !meta.is_file()
            || FileFingerprint::from_metadata(&meta) != expected
            || FileFingerprint::read(&self.path) != Some(expected)
        {
            return Err(Failure::Changed);
        }
        self.recheck()
    }
    /// Preserve a directory input's typed identity for descendants. The
    /// snapshot's inspect checks reject nested foreign mounts and symlinks.
    pub fn child(&self, path: &Path) -> Result<Self, Failure> {
        if !absolute(path) {
            return Err(Failure::Invalid);
        }
        let tail = path
            .strip_prefix(&self.path)
            .map_err(|_| Failure::Invalid)?;
        if !relative(tail, true) {
            return Err(Failure::Invalid);
        }
        let source = match &self.source {
            LibSource::File(_) => LibSource::File(path.into()),
            LibSource::Removable {
                volume_id,
                relative_path,
            } => {
                let relative_path = if tail.as_os_str().is_empty() {
                    relative_path.clone()
                } else {
                    relative_path.join(tail)
                };
                if !relative(&relative_path, true) {
                    return Err(Failure::Invalid);
                }
                LibSource::Removable {
                    volume_id: volume_id.clone(),
                    relative_path,
                }
            }
            _ => return Err(Failure::Unsupported),
        };
        Ok(Self {
            source,
            path: path.into(),
            stamp: self.stamp.clone(),
        })
    }
    /// Re-discovery is deliberately worker-only. The caller also checks its
    /// opened descriptor's fingerprint before/after hashing or decoding.
    pub fn recheck(&self) -> Result<(), Failure> {
        if self.stamp.is_none() {
            return Ok(());
        }
        self.recheck_with(&Snapshot::discover()?)
    }
    pub fn recheck_with(&self, snapshot: &Snapshot) -> Result<(), Failure> {
        if self.stamp.is_none() {
            return Ok(());
        }
        snapshot.inspect(self)?;
        // The saved UUID is authoritative after enrollment. A bus/removable
        // classification change must not silently turn an existing reference
        // back into a File source. Inspect already checked this exact mount;
        // derive its filesystem-relative path without selecting another alias.
        let stamp = self.stamp.as_ref().ok_or(Failure::Changed)?;
        let mount = snapshot.mount_at(&self.path)?;
        let prefix = mount.root.strip_prefix("/").map_err(|_| Failure::Changed)?;
        let tail = self
            .path
            .strip_prefix(&mount.point)
            .map_err(|_| Failure::Changed)?;
        let relative_path = if tail.as_os_str().is_empty() {
            prefix.into()
        } else {
            prefix.join(tail)
        };
        let current = LibSource::Removable {
            volume_id: stamp.uuid.clone(),
            relative_path,
        };
        if self.source != current {
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
    use std::os::unix::ffi::OsStrExt;
    path.is_absolute()
        && path.as_os_str().len() <= MAX_PATH
        && !path.as_os_str().as_bytes().contains(&0)
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
pub(crate) fn validate_root_source(source: &LibSource) -> Result<(), Failure> {
    let valid = match source {
        LibSource::File(path) => absolute(path),
        LibSource::Removable {
            volume_id,
            relative_path,
        } => valid_uuid(volume_id) && relative(relative_path, true),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Failure::Invalid)
    }
}
impl Snapshot {
    pub fn discover() -> Result<Self, Failure> {
        linux::discover()
    }
    /// Software-only removable classification of an actual mounted local
    /// block filesystem. The real UUID, namespace and mount guards stay intact.
    /// This is not physical USB discovery or an unplug qualification.
    #[cfg(test)]
    pub(crate) fn fixture_local_volume(path: &Path) -> Result<Self, Failure> {
        let mut snapshot = Self::discover()?;
        let path = path.canonicalize().map_err(io)?;
        let block = snapshot
            .mount_at(&path)?
            .block
            .ok_or(Failure::Unsupported)?;
        let device = snapshot
            .blocks
            .iter_mut()
            .find(|b| b.device == block && !b.uuid.is_empty())
            .ok_or(Failure::Unsupported)?;
        device.removable = true;
        Ok(snapshot)
    }
    #[cfg(test)]
    pub(crate) fn fixture_volume(point: &Path, uuid: &str, id: u64) -> Self {
        use std::os::unix::fs::MetadataExt;
        let device = Device::from_raw(point.metadata().unwrap().dev());
        Self {
            namespace: (1, 2),
            mounts: vec![Mount {
                id,
                device,
                access: device,
                btrfs: false,
                qualified: true,
                root: "/".into(),
                point: point.into(),
                source: "/dev/software-fixture".into(),
                block: Some(device),
            }],
            blocks: vec![Block {
                device,
                uuid: uuid.into(),
                removable: true,
            }],
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture_offline() -> Self {
        Self {
            namespace: (1, 2),
            mounts: Vec::new(),
            blocks: Vec::new(),
        }
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
            access: mount.access,
            btrfs: mount.btrfs,
            block,
            uuid: uuid.into(),
            root: mount.root.clone(),
            point: mount.point.clone(),
        }
    }
    /// Capture the namespace and mounted location for a worker-side path access.
    pub fn access(&self, path: &Path) -> Result<Access, Failure> {
        Ok(Access { namespace: self.namespace, mount: self.mount_at(path)?.clone() })
    }
    /// List visible filesystem entry points for a background discovery process.
    /// Takes this bounded mount snapshot; returns distinct mountpoints without traversing them or changing their access mode.
    pub fn mountpoints(&self)->Vec<PathBuf> {let mut points=Vec::new();for mount in &self.mounts {if !points.contains(&mount.point) {points.push(mount.point.clone());}}points}
    /// Resolve a source's fully visible filesystem root.
    /// Takes an identified source; returns its mountpoint or refuses a partial bind/subvolume view that cannot resolve root-relative vendor paths.
    pub fn volume_root(&self, location: &Location) -> Result<PathBuf, Failure> {
        location.recheck_with(self)?;
        let mount = self.mount_at(&location.path)?;
        if mount.root != Path::new("/") { return Err(Failure::NotVisible); }
        Ok(mount.point.clone())
    }
    /// Explicit imports adopt canonical identity; internal files remain local paths.
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
        if !mount.qualified {
            return Err(Failure::Changed);
        }
        if !valid_uuid(&identity.uuid) {
            return Err(Failure::Unsupported);
        }
        self.unique_block(&identity.uuid)?;
        let tail = path
            .strip_prefix(&mount.point)
            .map_err(|_| Failure::Changed)?;
        let prefix = mount.root.strip_prefix("/").map_err(|_| Failure::Invalid)?;
        let within_volume = if tail.as_os_str().is_empty() {
            prefix.into()
        } else {
            prefix.join(tail)
        };
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
                if !mount.qualified {
                    return Err(Failure::Changed);
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
                || !mount.qualified
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
                    Ok(meta) if Device::from_raw(meta.dev()) != mount.access => {
                        if !mount.btrfs
                            || !linux::btrfs_uuid(&path).is_ok_and(|uuid| uuid == stamp.uuid)
                        {
                            return Err(Failure::Changed);
                        }
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
        if location.stamp.as_ref().is_some_and(|stamp| {
            Device::from_raw(metadata.dev()) != stamp.access
                && (!stamp.btrfs
                    || !linux::btrfs_uuid(&location.path).is_ok_and(|uuid| uuid == stamp.uuid))
        }) {
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
        if (!absolute(&root) && !(fields[separator + 1] == b"nsfs" && relative(&root, false)))
            || !absolute(&point)
        {
            return Err(Failure::Invalid);
        }
        mounts.push(Mount {
            id,
            device: Device(major, minor),
            access: Device(major, minor),
            btrfs: fields[separator + 1] == b"btrfs",
            qualified: true,
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
