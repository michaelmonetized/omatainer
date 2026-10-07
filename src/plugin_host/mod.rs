use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub(crate) mod process;
pub(crate) mod realtime;
pub(crate) mod scanner;
pub(crate) mod worker;

pub(crate) const MAX_MESSAGE: usize = 32 * 1024 * 1024;
pub(crate) const MAX_STATE: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CHANNELS: usize = 32;
pub(crate) const MAX_BUSES: usize = 8;
pub(crate) const BLOCK: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BinaryIdentity {
    pub bundle: PathBuf,
    pub binary: PathBuf,
    pub sha256: String,
    pub arch: String,
}

/// Identify a native VST3 bundle without loading code.
/// Takes an absolute bundle path; returns its native ELF path and bounded content digest, or an architecture/format/access diagnostic.
pub(crate) fn identify(path: &Path) -> Result<BinaryIdentity, String> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err("Choose an absolute VST3 bundle path without parent traversal".into());
    }
    if path.extension().and_then(|s| s.to_str()) != Some("vst3") {
        return Err("Native Linux VST3 bundles are supported; AU, VST2 and Windows bundles require an explicit compatible installation or rendered audio".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("Plugin unavailable: {e}"))?;
    if meta.is_symlink() || !meta.is_dir() {
        return Err(
            "Choose the real VST3 bundle directory; symlink/file bundles are not loaded".into(),
        );
    }
    let arch = std::env::consts::ARCH;
    let native = path.join("Contents").join(format!("{arch}-linux"));
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .max_depth(16)
    {
        let entry = entry.map_err(|e| format!("Plugin bundle cannot be read: {e}"))?;
        if entry.file_type().is_symlink() {
            return Err(
                "Plugin bundle contains a symlink; select a self-contained native installation"
                    .into(),
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let size = entry.metadata().map_err(|e| e.to_string())?.len();
        bytes = bytes.checked_add(size).ok_or("Plugin size overflow")?;
        if size > 128 * 1024 * 1024 || bytes > 512 * 1024 * 1024 || files.len() >= 4096 {
            return Err("Plugin bundle exceeds the 4096-file/512 MiB scan limit".into());
        }
        files.push(entry.path().to_path_buf());
    }
    files.sort();
    let binaries: Vec<_> = files
        .iter()
        .filter(|p| {
            p.parent() == Some(native.as_path()) && p.extension().is_some_and(|e| e == "so")
        })
        .cloned()
        .collect();
    if binaries.len() != 1 {
        return Err(format!("Expected one {arch}-linux .so in this bundle; the installed OS/CPU architecture is incompatible or incomplete"));
    }
    let binary = binaries[0].clone();
    let mut digest = Sha256::new();
    for file in files {
        let mut input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&file)
            .map_err(|e| e.to_string())?;
        let before = input.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() {
            return Err("Plugin bundle changed during inspection".into());
        }
        let name = file
            .strip_prefix(path)
            .map_err(|e| e.to_string())?
            .as_os_str()
            .as_encoded_bytes();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name);
        digest.update(before.len().to_le_bytes());
        let mut count = 0;
        let mut buffer = [0u8; 65536];
        loop {
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > 128 * 1024 * 1024 {
                return Err("Plugin grew beyond its file limit".into());
            }
            digest.update(&buffer[..n]);
        }
        use std::os::unix::fs::MetadataExt;
        let after = input.metadata().map_err(|e| e.to_string())?;
        if count != before.len()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err("Plugin bundle changed during inspection".into());
        }
    }
    let mut header = [0u8; 20];
    File::open(&binary)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    let machine = match arch {
        "aarch64" => 183,
        "x86_64" => 62,
        _ => return Err(format!("Native VST3 host is not qualified for {arch}")),
    };
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != machine
    {
        return Err(format!("Plugin binary is not a native 64-bit {arch} Linux ELF; install the matching plugin or use a rendered stem"));
    }
    Ok(BinaryIdentity {
        bundle: path.into(),
        binary,
        sha256: format!("{:x}", digest.finalize()),
        arch: arch.into(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Class {
    pub info: vst3_host::PluginInfo,
    pub layout: vst3_host::AudioBusLayout,
    pub parameters: Vec<vst3_host::Parameter>,
    pub latency: u32,
    pub tail: u32,
}
impl Class {
    pub fn validate(&self) -> Result<(), String> {
        if self.info.uid.len() != 32
            || !self.info.uid.bytes().all(|b| b.is_ascii_hexdigit())
            || self.parameters.len() > 8192
            || self.info.name.len() > 4096
            || self.info.vendor.len() > 4096
            || self.info.version.len() > 1024
        {
            return Err("Plugin identity/parameters exceed host limits".into());
        }
        layout_valid(&self.layout)?;
        let mut ids = std::collections::HashSet::new();
        if self.parameters.iter().any(|p| {
            !ids.insert(p.id)
                || p.name.len() > 4096
                || p.unit.len() > 1024
                || !p.value.is_finite()
                || !(0.0..=1.0).contains(&p.value)
        }) {
            return Err("Plugin parameters are malformed or duplicated".into());
        }
        Ok(())
    }
}
pub(crate) fn layout_valid(layout: &vst3_host::AudioBusLayout) -> Result<(), String> {
    for buses in [&layout.inputs, &layout.outputs] {
        if buses.len() > MAX_BUSES
            || buses.iter().any(|b| b.channel_count > MAX_CHANNELS)
            || buses.iter().map(|b| b.channel_count).sum::<usize>() > MAX_CHANNELS
        {
            return Err("Plugin exceeds eight buses or 32 channels per direction".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub schema: u32,
    pub binary: BinaryIdentity,
    pub class_id: String,
    pub plugin_version: String,
    pub state_codec: String,
    pub state: Vec<u8>,
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.state_codec != "vst3-host-0.9-state"
            || self.state.len() > MAX_STATE
            || self.class_id.len() != 32
            || !self.class_id.bytes().all(|c| c.is_ascii_hexdigit())
            || self.plugin_version.len() > 1024
        {
            return Err("Plugin state identity, version or codec is unsupported".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frame {
    pub inputs: Vec<Vec<Vec<f32>>>,
    pub frames: usize,
    pub bpm: f64,
    pub beat: f64,
    pub sample_position: i64,
    pub playing: bool,
    pub signature: [i32; 2],
    pub parameters: Vec<(u32, f64, i32)>,
    pub midi: Vec<(i32, [u8; 3])>,
}
impl Frame {
    pub fn validate(&self) -> Result<(), String> {
        if self.frames == 0
            || self.frames > BLOCK
            || self.inputs.len() > MAX_BUSES
            || self.inputs.iter().any(|b| b.len() > MAX_CHANNELS)
            || self.inputs.iter().map(Vec::len).sum::<usize>() > MAX_CHANNELS
            || self
                .inputs
                .iter()
                .flatten()
                .any(|c| c.len() != self.frames || c.iter().any(|v| !v.is_finite()))
            || !self.bpm.is_finite()
            || !(1.0..=999.0).contains(&self.bpm)
            || !self.beat.is_finite()
            || self.sample_position < 0
            || !(1..=64).contains(&self.signature[0])
            || ![1, 2, 4, 8, 16, 32, 64].contains(&self.signature[1])
            || self.parameters.len() > 512
            || self.midi.len() > 512
        {
            return Err("Plugin processing frame exceeds audio/event/context limits".into());
        }
        if self.parameters.iter().any(|(_, v, n)| {
            !v.is_finite() || !(0.0..=1.0).contains(v) || *n < 0 || *n >= self.frames as i32
        }) || self.midi.iter().any(|(n, bytes)| {
            *n < 0
                || *n >= self.frames as i32
                || bytes[0] < 0x80
                || bytes[0] >= 0xf0
                || bytes[1] > 127
                || bytes[2] > 127
        }) {
            return Err("Plugin event value or offset is invalid".into());
        }
        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub(crate) enum Request {
    Paths { roots: Vec<PathBuf> },
    Inspect { path: PathBuf },
    Probe { binary: BinaryIdentity },
    Load { saved: Saved, rate: u32 },
    Process { frame: Frame },
    State,
    Editor { open: bool },
    Quit,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum Response {
    Paths {
        paths: Vec<PathBuf>,
    },
    Identity {
        binary: BinaryIdentity,
    },
    Classes {
        classes: Vec<Class>,
    },
    Loaded {
        class: Class,
    },
    Audio {
        outputs: Vec<Vec<Vec<f32>>>,
        latency: u32,
        tail: u32,
        restart: u32,
        midi: Vec<vst3_host::MidiEvent>,
    },
    State {
        saved: Saved,
    },
    Ok,
    Error {
        message: String,
    },
}
