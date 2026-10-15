use super::*;
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    ptr,
};
mod api;
mod editor;
mod features;
mod state;
use api::{Api, Feature, RawInstance, M, P};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Port {
    pub index: u32,
    pub symbol: String,
    pub name: String,
    pub input: bool,
    pub kind: String,
    pub bus: usize,
    pub channel: usize,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub integer: bool,
    pub latency: bool,
    pub midi: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub ports: Vec<Port>,
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub unsupported: Vec<String>,
}
struct World {
    raw: M,
    api: Api,
}
impl World {
    /// Load RDF for one inspected bundle. Args: absolute bundle directory. Returns an owned Lilv world without scanning unrelated plugins.
    fn open(bundle: &Path) -> Result<Self, String> {
        let api = Api::open()?;
        let raw = unsafe { (api.world_new)() };
        if raw.is_null() {
            return Err("LV2 metadata world allocation failed".into());
        }
        let world = Self { raw, api };
        let path = CString::new(format!("{}/", bundle.display())).map_err(|e| e.to_string())?;
        unsafe {
            let node = (world.api.new_file_uri)(raw, ptr::null(), path.as_ptr());
            if node.is_null() {
                return Err("LV2 bundle URI allocation failed".into());
            }
            (world.api.world_load_bundle)(raw, node);
            (world.api.node_free)(node);
        }
        Ok(world)
    }
    fn uri(&self, uri: &str) -> Result<M, String> {
        let uri = CString::new(uri).map_err(|e| e.to_string())?;
        let n = unsafe { (self.api.new_uri)(self.raw, uri.as_ptr()) };
        if n.is_null() {
            Err("LV2 URI allocation failed".into())
        } else {
            Ok(n)
        }
    }
    unsafe fn text(&self, n: P) -> Result<String, String> {
        if n.is_null() {
            return Ok(String::new());
        }
        let p = (self.api.node_as_string)(n);
        if p.is_null() {
            return Err("LV2 metadata string is null".into());
        }
        let s = CStr::from_ptr(p).to_str().map_err(|e| e.to_string())?;
        if s.len() > 4096 {
            return Err("LV2 metadata string exceeds 4096 bytes".into());
        }
        Ok(s.into())
    }
    unsafe fn owned_text(&self, n: M) -> Result<String, String> {
        let result = self.text(n);
        (self.api.node_free)(n);
        result
    }
    unsafe fn list(&self, n: M) -> Result<Vec<String>, String> {
        let result = (|| {
            let mut v = Vec::new();
            let mut it = (self.api.nodes_begin)(n);
            while !(self.api.nodes_is_end)(n, it) {
                if v.len() >= 256 {
                    return Err("LV2 metadata collection exceeds 256 entries".into());
                }
                v.push(self.text((self.api.nodes_get)(n, it))?);
                it = (self.api.nodes_next)(n, it);
            }
            Ok(v)
        })();
        (self.api.nodes_free)(n);
        result
    }
    unsafe fn property(&self, plugin: P, port: P, uri: &str) -> Result<Vec<String>, String> {
        let key = self.uri(uri)?;
        let values = if port.is_null() {
            (self.api.plugin_get_value)(plugin, key)
        } else {
            (self.api.port_get_value)(plugin, port, key)
        };
        (self.api.node_free)(key);
        self.list(values)
    }
    unsafe fn is(&self, plugin: P, port: P, class: &str) -> Result<bool, String> {
        let n = self.uri(class)?;
        let result = (self.api.port_is_a)(plugin, port, n);
        (self.api.node_free)(n);
        Ok(result)
    }
    unsafe fn has(&self, plugin: P, port: P, property: &str) -> Result<bool, String> {
        let n = self.uri(property)?;
        let result = (self.api.port_has_property)(plugin, port, n);
        (self.api.node_free)(n);
        Ok(result)
    }
    unsafe fn plugins(&self) -> Result<Vec<P>, String> {
        let all = (self.api.world_get_all_plugins)(self.raw);
        if (self.api.plugins_size)(all) > 64 {
            return Err("LV2 bundle exceeds 64 plugins".into());
        }
        let mut it = (self.api.plugins_begin)(all);
        let mut result = Vec::new();
        while !(self.api.plugins_is_end)(all, it) {
            result.push((self.api.plugins_get)(all, it));
            it = (self.api.plugins_next)(all, it);
        }
        if result.is_empty() {
            return Err("LV2 bundle contains no discoverable plugin".into());
        }
        Ok(result)
    }
    fn report(&self, plugin: P, binary: &BinaryIdentity) -> Result<Class, String> {
        unsafe {
            let library = (self.api.node_get_path)(
                (self.api.plugin_get_library_uri)(plugin),
                ptr::null_mut(),
            );
            if library.is_null() {
                return Err("LV2 processor binary is not a native local file".into());
            }
            let library_path = PathBuf::from(CStr::from_ptr(library).to_string_lossy().as_ref());
            (self.api.free)(library.cast());
            let bundle_root = binary.bundle.canonicalize().map_err(|e| e.to_string())?;
            let library_path = library_path.canonicalize().map_err(|e| e.to_string())?;
            if !library_path.starts_with(&bundle_root) {
                return Err("LV2 processor binary escapes its inspected bundle".into());
            }
            let count = (self.api.plugin_get_num_ports)(plugin);
            if count > 512 {
                return Err("LV2 plugin exceeds 512 ports".into());
            }
            let mut min = vec![f32::NAN; count as usize];
            let mut max = min.clone();
            let mut defaults = min.clone();
            (self.api.plugin_get_port_ranges_float)(
                plugin,
                min.as_mut_ptr(),
                max.as_mut_ptr(),
                defaults.as_mut_ptr(),
            );
            let required = self.list((self.api.plugin_get_required_features)(plugin))?;
            let optional = self.list((self.api.plugin_get_optional_features)(plugin))?;
            let unsupported = optional
                .iter()
                .filter(|f| !features::SUPPORTED.contains(&f.as_str()))
                .cloned()
                .collect();
            let missing = required
                .iter()
                .filter(|f| !features::SUPPORTED.contains(&f.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(format!(
                    "LV2 required features are unavailable: {}",
                    missing.join(", ")
                ));
            }
            let mut metadata = Metadata {
                ports: Vec::new(),
                required,
                optional,
                unsupported,
            };
            let mut groups: [Vec<String>; 2] = [Vec::new(), Vec::new()];
            let mut layouts: [Vec<vst3_host::AudioBusConfig>; 2] = [Vec::new(), Vec::new()];
            let mut parameters = Vec::new();
            let mut midi = [false, false];
            for i in 0..count {
                let port = (self.api.plugin_get_port_by_index)(plugin, i);
                if port.is_null() {
                    return Err("LV2 port metadata is null".into());
                }
                let input = self.is(plugin, port, "http://lv2plug.in/ns/lv2core#InputPort")?;
                let output = self.is(plugin, port, "http://lv2plug.in/ns/lv2core#OutputPort")?;
                if input == output {
                    return Err("LV2 port direction is ambiguous".into());
                }
                let direction = if input { 0 } else { 1 };
                let audio = self.is(plugin, port, "http://lv2plug.in/ns/lv2core#AudioPort")?;
                let control = self.is(plugin, port, "http://lv2plug.in/ns/lv2core#ControlPort")?;
                let atom = self.is(plugin, port, "http://lv2plug.in/ns/ext/atom#AtomPort")?;
                let symbol = self.text((self.api.port_get_symbol)(plugin, port))?;
                let name = self.owned_text((self.api.port_get_name)(plugin, port))?;
                let integer = self.has(plugin, port, "http://lv2plug.in/ns/lv2core#integer")?;
                let latency =
                    self.has(plugin, port, "http://lv2plug.in/ns/lv2core#reportsLatency")?
                        || self
                            .property(plugin, port, "http://lv2plug.in/ns/lv2core#designation")?
                            .iter()
                            .any(|v| v == "http://lv2plug.in/ns/lv2core#latency");
                let (kind, bus, channel) = if audio {
                    let group = self
                        .property(plugin, port, "http://lv2plug.in/ns/ext/port-groups#group")?
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    let bus = groups[direction]
                        .iter()
                        .position(|g| g == &group)
                        .unwrap_or_else(|| {
                            groups[direction].push(group);
                            layouts[direction].push(vst3_host::AudioBusConfig {
                                channel_count: 0,
                                active: true,
                            });
                            groups[direction].len() - 1
                        });
                    let channel = layouts[direction][bus].channel_count;
                    layouts[direction][bus].channel_count += 1;
                    ("audio", bus, channel)
                } else if control {
                    let a = if min[i as usize].is_finite() {
                        min[i as usize]
                    } else {
                        0.
                    };
                    let b = if max[i as usize].is_finite() {
                        max[i as usize]
                    } else {
                        1.
                    };
                    if a > b {
                        return Err("LV2 parameter range is inverted".into());
                    }
                    let d = if defaults[i as usize].is_finite() {
                        defaults[i as usize].clamp(a, b)
                    } else {
                        a
                    };
                    let value = if b > a {
                        ((d - a) / (b - a)) as f64
                    } else {
                        0.
                    };
                    min[i as usize] = a;
                    max[i as usize] = b;
                    defaults[i as usize] = d;
                    parameters.push(vst3_host::Parameter {
                        id: i,
                        name: name.clone(),
                        value,
                        min: 0.,
                        max: 1.,
                        default: value,
                        unit: String::new(),
                        step_count: if integer {
                            (b - a).min(i32::MAX as f32) as i32
                        } else {
                            0
                        },
                        can_automate: input,
                        is_read_only: output,
                        is_bypass: false,
                        flags: 0,
                    });
                    ("control", 0, 0)
                } else if atom {
                    let types =
                        self.property(plugin, port, "http://lv2plug.in/ns/ext/atom#bufferType")?;
                    if !types
                        .iter()
                        .any(|s| s == "http://lv2plug.in/ns/ext/atom#Sequence")
                    {
                        return Err("LV2 Atom port requires an unsupported buffer type".into());
                    }
                    let events =
                        self.property(plugin, port, "http://lv2plug.in/ns/ext/atom#supports")?;
                    midi[direction] |= events
                        .iter()
                        .any(|s| s == "http://lv2plug.in/ns/ext/midi#MidiEvent");
                    let size = self
                        .property(
                            plugin,
                            port,
                            "http://lv2plug.in/ns/ext/resize-port#minimumSize",
                        )?
                        .first()
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);
                    if size > 65536 {
                        return Err("LV2 Atom port requires more than 64 KiB".into());
                    }
                    ("atom", 0, 0)
                } else if self.has(
                    plugin,
                    port,
                    "http://lv2plug.in/ns/lv2core#connectionOptional",
                )? {
                    ("optional", 0, 0)
                } else {
                    return Err(format!("Unsupported required LV2 port {symbol}"));
                };
                metadata.ports.push(Port {
                    index: i,
                    symbol,
                    name,
                    input,
                    kind: kind.into(),
                    bus,
                    channel,
                    min: if min[i as usize].is_finite() {
                        min[i as usize]
                    } else {
                        0.
                    },
                    max: if max[i as usize].is_finite() {
                        max[i as usize]
                    } else {
                        1.
                    },
                    default: if defaults[i as usize].is_finite() {
                        defaults[i as usize]
                    } else {
                        0.
                    },
                    integer,
                    latency,
                    midi: atom
                        && self
                            .property(plugin, port, "http://lv2plug.in/ns/ext/atom#supports")?
                            .iter()
                            .any(|s| s == "http://lv2plug.in/ns/ext/midi#MidiEvent"),
                });
            }
            let instrument = self
                .property(
                    plugin,
                    ptr::null(),
                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                )?
                .iter()
                .any(|s| s == "http://lv2plug.in/ns/lv2core#InstrumentPlugin");
            let uid = format!("lv2:{}", self.text((self.api.plugin_get_uri)(plugin))?);
            let version = format!(
                "{}.{}",
                self.property(
                    plugin,
                    ptr::null(),
                    "http://lv2plug.in/ns/lv2core#minorVersion"
                )?
                .first()
                .cloned()
                .unwrap_or_else(|| "0".into()),
                self.property(
                    plugin,
                    ptr::null(),
                    "http://lv2plug.in/ns/lv2core#microVersion"
                )?
                .first()
                .cloned()
                .unwrap_or_else(|| "0".into())
            );
            let result = Class {
                info: vst3_host::PluginInfo {
                    path: binary.bundle.clone(),
                    name: self.owned_text((self.api.plugin_get_name)(plugin))?,
                    vendor: self.owned_text((self.api.plugin_get_author_name)(plugin))?,
                    version,
                    category: if instrument { "Instrument" } else { "Fx" }.into(),
                    uid,
                    audio_inputs: layouts[0].len() as u32,
                    audio_outputs: layouts[1].len() as u32,
                    has_midi_input: midi[0],
                    has_midi_output: midi[1],
                    has_gui: {
                        let uis = (self.api.plugin_get_uis)(plugin);
                        let present = !(self.api.uis_is_end)(uis, (self.api.uis_begin)(uis));
                        (self.api.uis_free)(uis);
                        present
                    },
                },
                layout: vst3_host::AudioBusLayout {
                    inputs: layouts[0].clone(),
                    outputs: layouts[1].clone(),
                },
                parameters,
                latency: 0,
                tail: 0,
                native: None,
                lv2: Some(metadata),
            };
            result.validate()?;
            Ok(result)
        }
    }
}
impl Drop for World {
    fn drop(&mut self) {
        self.editor = None;
        unsafe {
            (self.api.world_free)(self.raw);
        }
    }
}

/// Identify a self-contained LV2 bundle. Args: absolute directory. Returns its native ELF identity and a digest covering RDF, code and assets.
pub(super) fn identify(path: &Path) -> Result<BinaryIdentity, String> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Choose an absolute LV2 bundle path without parent traversal".into());
    }
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("LV2 bundle must be a real directory".into());
    }
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .max_depth(16)
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().is_symlink() {
            return Err(
                "LV2 bundle contains a symlink; use a self-contained native installation".into(),
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        total += entry.metadata().map_err(|e| e.to_string())?.len();
        if total > 512 * 1024 * 1024 || files.len() >= 4096 {
            return Err("LV2 bundle exceeds its file/size limit".into());
        }
        files.push(entry.path().to_path_buf());
    }
    files.sort();
    let mut digest = Sha256::new();
    let mut binary = None;
    for file in files {
        let identity = if file.extension().is_some_and(|e| e == "so") {
            let mut input = File::open(&file).map_err(|e| e.to_string())?;
            let mut header = [0u8; 20];
            input.read_exact(&mut header).map_err(|e| e.to_string())?;
            let machine = match std::env::consts::ARCH {
                "aarch64" => 183,
                "x86_64" => 62,
                _ => return Err("LV2 host requires native Linux aarch64 or x86_64".into()),
            };
            if &header[..4] != b"\x7fELF"
                || header[4] != 2
                || header[5] != 1
                || u16::from_le_bytes([header[18], header[19]]) != machine
            {
                return Err("LV2 bundle contains an incompatible native binary".into());
            }
            Some(file.clone())
        } else {
            None
        };
        if binary.is_none() {
            binary = identity;
        }
        let mut input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&file)
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        input
            .by_ref()
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("LV2 file exceeds 128 MiB".into());
        }
        let name = file
            .strip_prefix(path)
            .map_err(|e| e.to_string())?
            .as_os_str()
            .as_encoded_bytes();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(BinaryIdentity {
        bundle: path.into(),
        binary: binary.ok_or("LV2 bundle has no native binary")?,
        sha256: format!("{:x}", digest.finalize()),
        arch: std::env::consts::ARCH.into(),
    })
}

pub(super) struct Instance {
    raw: *mut RawInstance,
    world: World,
    features: features::Features,
    pub class: Class,
    saved: Saved,
    controls: Vec<f32>,
    atoms: Vec<Vec<u64>>,
    active: bool,
    audio: Vec<Vec<f32>>,
    editor: Option<editor::Editor>,
    pending_atoms: Vec<(u32, Vec<u8>)>,
    output_atoms: Vec<(u32, Vec<u8>)>,
}
impl Instance {
    /// Restore an LV2 processor. Args: saved class and rate. Returns an active instance whose feature requirements were checked before native instantiation.
    pub fn load(saved: Saved, rate: u32) -> Result<Self, String> {
        if !(8000..=384000).contains(&rate) {
            return Err("Unsupported LV2 sample rate".into());
        }
        let world = World::open(&saved.binary.bundle)?;
        let uri = world.uri(
            saved
                .class_id
                .strip_prefix("lv2:")
                .ok_or("Invalid LV2 class URI")?,
        )?;
        let plugin = unsafe {
            (world.api.plugins_get_by_uri)((world.api.world_get_all_plugins)(world.raw), uri)
        };
        unsafe {
            (world.api.node_free)(uri);
        }
        if plugin.is_null() {
            return Err("LV2 saved plugin URI is unavailable".into());
        }
        let class = world.report(plugin, &saved.binary)?;
        if class.info.version != saved.plugin_version {
            return Err("LV2 plugin version changed; review state compatibility".into());
        }
        let base = std::env::var_os("OMATAINER_PLUGIN_SCRATCH")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".local/share/omatainer/plugin-work"))
            })
            .ok_or("LV2 requires a private home data directory")?;
        if !base.is_absolute() || !base.starts_with("/home") {
            return Err("LV2 work directory must be under /home".into());
        }
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let root = base.join(format!(
            "lv2-{}-{}",
            std::process::id(),
            crate::sampler_bank::BankId::new()?
        ));
        std::fs::create_dir(&root).map_err(|e| e.to_string())?;
        let features = features::Features::new(rate, root)?;
        let raw = unsafe {
            (world.api.plugin_instantiate)(plugin, rate as f64, features.pointers.as_ptr())
        };
        if raw.is_null() {
            return Err("LV2 native instantiate refused supported features".into());
        }
        let controls = class
            .lv2
            .as_ref()
            .unwrap()
            .ports
            .iter()
            .map(|p| p.default)
            .collect();
        let atoms = class
            .lv2
            .as_ref()
            .unwrap()
            .ports
            .iter()
            .map(|p| {
                if p.kind == "atom" {
                    vec![0u64; 8192]
                } else {
                    vec![]
                }
            })
            .collect();
        let audio = class
            .lv2
            .as_ref()
            .unwrap()
            .ports
            .iter()
            .map(|p| {
                if p.kind == "audio" {
                    vec![0.; BLOCK]
                } else {
                    vec![]
                }
            })
            .collect();
        let mut instance = Self {
            raw,
            world,
            features,
            class,
            saved,
            controls,
            atoms,
            active: false,
            audio,
            editor: None,
            pending_atoms: Vec::with_capacity(64),
            output_atoms: Vec::with_capacity(64),
        };
        unsafe {
            if (*raw).descriptor.is_null()
                || (*(*raw).descriptor).connect.is_none()
                || (*(*raw).descriptor).run.is_none()
            {
                return Err("LV2 descriptor vtable is incomplete".into());
            }
        }
        for port in &instance.class.lv2.as_ref().unwrap().ports {
            let index = port.index as usize;
            let data = match port.kind.as_str() {
                "audio" => instance.audio[index].as_mut_ptr().cast(),
                "control" => (&mut instance.controls[index] as *mut f32).cast(),
                "atom" => {
                    fill_atom(
                        &mut instance.atoms[index],
                        instance
                            .features
                            .id("http://lv2plug.in/ns/ext/atom#Sequence")?,
                        instance
                            .features
                            .id("http://lv2plug.in/ns/ext/midi#MidiEvent")?,
                        port.input,
                        0,
                        BLOCK,
                        &[],
                    )?;
                    instance.atoms[index].as_mut_ptr().cast()
                }
                _ => ptr::null_mut(),
            };
            unsafe {
                (*(*raw).descriptor).connect.unwrap()((*raw).handle, port.index, data);
            }
        }
        instance.restore()?;
        instance.activate();
        Ok(instance)
    }
    fn activate(&mut self) {
        unsafe {
            if let Some(f) = (*(*self.raw).descriptor).activate {
                f((*self.raw).handle);
            }
        }
        self.active = true;
    }
    fn values(&self) -> Result<Vec<(u32, f64)>, String> {
        self.class
            .writable_parameters()
            .map(|p| {
                let port = &self.class.lv2.as_ref().unwrap().ports[p.id as usize];
                let value = self.controls[p.id as usize];
                let value = if port.max > port.min {
                    ((value - port.min) / (port.max - port.min)) as f64
                } else {
                    0.
                };
                if !value.is_finite() || !(0.0..=1.).contains(&value) {
                    return Err("LV2 parameter returned an invalid value".into());
                }
                Ok((p.id, value))
            })
            .collect()
    }
    pub fn process(&mut self, frame: Frame) -> Result<Response, String> {
        frame.validate()?;
        if !frame.native.is_empty() {
            return Err("CLAP native expression events cannot be sent to LV2; use supported MIDI/Atom events".into());
        }
        if frame.inputs.len() != self.class.layout.inputs.len()
            || frame
                .inputs
                .iter()
                .zip(&self.class.layout.inputs)
                .any(|(b, l)| b.len() != l.channel_count)
        {
            return Err("LV2 input bus layout changed".into());
        }
        let mut outputs: Vec<Vec<Vec<f32>>> = self
            .class
            .layout
            .outputs
            .iter()
            .map(|b| vec![vec![0.; frame.frames]; b.channel_count])
            .collect();
        let mut inputs = frame.inputs.clone();
        let mut midi_output = Vec::new();
        let mut cuts = vec![0, frame.frames];
        cuts.extend(frame.parameters.iter().map(|p| p.2 as usize));
        cuts.sort_unstable();
        cuts.dedup();
        for segment in cuts.windows(2) {
            let start = segment[0];
            let end = segment[1];
            for &(id, value, time) in &frame.parameters {
                if time as usize != start {
                    continue;
                }
                let port = self
                    .class
                    .lv2
                    .as_ref()
                    .unwrap()
                    .ports
                    .get(id as usize)
                    .filter(|p| p.kind == "control" && p.input)
                    .ok_or("LV2 automation references an unavailable writable parameter")?;
                let mut value = port.min + value as f32 * (port.max - port.min);
                if port.integer {
                    value = value.round().clamp(port.min, port.max);
                }
                self.controls[id as usize] = value;
            }
            for port in &self.class.lv2.as_ref().unwrap().ports {
                let index = port.index as usize;
                let data = match port.kind.as_str() {
                    "audio" => {
                        if port.input {
                            inputs[port.bus][port.channel][start..].as_mut_ptr().cast()
                        } else {
                            outputs[port.bus][port.channel][start..].as_mut_ptr().cast()
                        }
                    }
                    "control" => (&mut self.controls[index] as *mut f32).cast(),
                    "atom" => {
                        let sequence =
                            self.features.id("http://lv2plug.in/ns/ext/atom#Sequence")?;
                        let midi = self
                            .features
                            .id("http://lv2plug.in/ns/ext/midi#MidiEvent")?;
                        fill_atom(
                            &mut self.atoms[index],
                            sequence,
                            midi,
                            port.input,
                            start,
                            end,
                            if port.midi { &frame.midi } else { &[] },
                        )?;
                        if port.input && start == 0 {
                            append_atoms(&mut self.atoms[index], port.index, &self.pending_atoms)?;
                        }
                        self.atoms[index].as_mut_ptr().cast()
                    }
                    _ => ptr::null_mut(),
                };
                unsafe {
                    (*(*self.raw).descriptor).connect.unwrap()(
                        (*self.raw).handle,
                        port.index,
                        data,
                    );
                }
            }
            unsafe {
                (*(*self.raw).descriptor).run.unwrap()((*self.raw).handle, (end - start) as u32);
            }
            for port in self
                .class
                .lv2
                .as_ref()
                .unwrap()
                .ports
                .iter()
                .filter(|p| !p.input && p.kind == "atom")
            {
                parse_atoms(
                    &self.atoms[port.index as usize],
                    port.index,
                    end - start,
                    self.features.id("http://lv2plug.in/ns/ext/atom#Sequence")?,
                    self.features
                        .id("http://lv2plug.in/ns/ext/midi#MidiEvent")?,
                    &mut midi_output,
                    &mut self.output_atoms,
                )?;
            }
        }
        self.pending_atoms.clear();
        if outputs.iter().flatten().flatten().any(|v| !v.is_finite()) {
            return Err("LV2 processor returned non-finite audio".into());
        }
        let latency = self
            .class
            .lv2
            .as_ref()
            .unwrap()
            .ports
            .iter()
            .find(|p| p.latency)
            .map_or(0, |p| self.controls[p.index as usize].max(0.) as u32);
        Ok(Response::Audio {
            outputs,
            latency,
            tail: 0,
            restart: 0,
            editor_open: self.editor.is_some(),
            parameters: self.values()?,
            midi: midi_output,
        })
    }
    pub fn editor(&mut self, open: bool) -> Result<Response, String> {
        if !open {
            self.editor = None;
        } else if self.editor.is_none() {
            self.editor = Some(editor::Editor::open(self)?);
        }
        Ok(Response::Ok)
    }
    pub fn service(&mut self) -> Result<(), String> {
        if let Some(editor) = &mut self.editor {
            if editor.service(
                &mut self.controls,
                &self.class.lv2.as_ref().unwrap().ports,
                &mut self.pending_atoms,
                &mut self.output_atoms,
            )? {
                self.editor = None;
            }
        } else {
            self.output_atoms.clear();
        }
        Ok(())
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            if !self.raw.is_null() {
                if self.active {
                    if let Some(f) = (*(*self.raw).descriptor).deactivate {
                        f((*self.raw).handle);
                    }
                }
                (self.world.api.instance_free)(self.raw);
            }
        }
    }
}
/// Read native LV2 metadata and verify instantiation. Args: inspected bundle. Returns supported class reports; unsupported required features fail before native activation.
pub(super) fn probe(binary: &BinaryIdentity) -> Result<Vec<Class>, String> {
    let world = World::open(&binary.bundle)?;
    let classes = unsafe { world.plugins()? }
        .iter()
        .map(|p| world.report(*p, binary))
        .collect::<Result<Vec<_>, _>>()?;
    classes
        .into_iter()
        .map(|class| {
            let saved = Saved {
                schema: 1,
                binary: binary.clone(),
                class_id: class.info.uid.clone(),
                plugin_version: class.info.version.clone(),
                state_codec: "lv2-state-1".into(),
                state: vec![],
            };
            Instance::load(saved, 48000).map(|i| i.class.clone())
        })
        .collect()
}
fn fill_atom(
    buffer: &mut [u64],
    sequence: u32,
    midi: u32,
    input: bool,
    start: usize,
    end: usize,
    events: &[(i32, [u8; 3])],
) -> Result<(), String> {
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 8)
    };
    bytes.fill(0);
    bytes[4..8].copy_from_slice(&sequence.to_ne_bytes());
    if !input {
        let capacity = (bytes.len() - 8) as u32;
        bytes[..4].copy_from_slice(&capacity.to_ne_bytes());
        return Ok(());
    }
    let mut position = 16;
    for &(time, data) in events {
        let time = time as usize;
        if time < start || time >= end {
            continue;
        }
        if position + 24 > bytes.len() {
            return Err("LV2 MIDI Atom sequence capacity exceeded".into());
        }
        bytes[position..position + 8].copy_from_slice(&((time - start) as i64).to_ne_bytes());
        bytes[position + 8..position + 12].copy_from_slice(&3u32.to_ne_bytes());
        bytes[position + 12..position + 16].copy_from_slice(&midi.to_ne_bytes());
        bytes[position + 16..position + 19].copy_from_slice(&data);
        position += 24;
    }
    bytes[..4].copy_from_slice(&((position - 8) as u32).to_ne_bytes());
    Ok(())
}

#[cfg(test)]
mod tests;

/// Append UI patch atoms to an input sequence. Args: aligned port buffer, port index and bounded native messages. Returns a capacity/shape diagnostic on malformed atoms.
fn append_atoms(buffer: &mut [u64], port: u32, pending: &[(u32, Vec<u8>)]) -> Result<(), String> {
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 8)
    };
    let mut position = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize + 8;
    for (_, atom) in pending.iter().filter(|(p, _)| *p == port).rev() {
        if atom.len() < 8
            || u32::from_ne_bytes(atom[..4].try_into().unwrap()) as usize + 8 != atom.len()
        {
            return Err("LV2 UI sent a malformed patch Atom".into());
        }
        let length = (atom.len() + 8 + 7) & !7;
        if position + length > bytes.len() {
            return Err("LV2 UI patch queue exceeds the Atom port capacity".into());
        }
        bytes.copy_within(16..position, 16 + length);
        bytes[16..24].copy_from_slice(&0i64.to_ne_bytes());
        bytes[24..24 + atom.len()].copy_from_slice(atom);
        position += length;
    }
    bytes[..4].copy_from_slice(&((position - 8) as u32).to_ne_bytes());
    Ok(())
}
/// Read native output events. Args: aligned Atom sequence, port, segment length and URI IDs. Returns bounded MIDI and patch notifications or an invalid-output diagnostic.
fn parse_atoms(
    buffer: &[u64],
    port: u32,
    frames: usize,
    sequence: u32,
    midi: u32,
    notes: &mut Vec<vst3_host::MidiEvent>,
    atoms: &mut Vec<(u32, Vec<u8>)>,
) -> Result<(), String> {
    let bytes =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) };
    let size = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
    let kind = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
    if kind != sequence {
        return Ok(());
    }
    if size < 8 || size + 8 > bytes.len() || bytes[8..12] != [0, 0, 0, 0] {
        return Err("LV2 processor returned an invalid Atom sequence".into());
    }
    let mut pos = 16;
    let end = size + 8;
    let mut last = 0;
    while pos < end {
        if pos + 16 > end {
            return Err("LV2 processor truncated an Atom event".into());
        }
        let time = i64::from_ne_bytes(bytes[pos..pos + 8].try_into().unwrap());
        let length = u32::from_ne_bytes(bytes[pos + 8..pos + 12].try_into().unwrap()) as usize;
        let kind = u32::from_ne_bytes(bytes[pos + 12..pos + 16].try_into().unwrap());
        if time < last || time < 0 || time as usize >= frames || length > end - pos - 16 {
            return Err("LV2 output event has invalid time or size".into());
        }
        last = time;
        if kind == midi {
            if length != 3 || notes.len() >= 512 {
                return Err("LV2 MIDI output exceeds its supported event bound".into());
            }
            let data = bytes[pos + 16..pos + 19].try_into().unwrap();
            notes.push(
                super::clap::events::midi(data).ok_or("LV2 output MIDI message is malformed")?,
            );
        } else if atoms.len() < 64 {
            atoms.push((port, bytes[pos + 8..pos + 16 + length].to_vec()));
        }
        pos = pos
            .checked_add((16 + length + 7) & !7)
            .ok_or("LV2 Atom event length overflow")?;
    }
    if pos != end {
        return Err("LV2 Atom output is misaligned".into());
    }
    Ok(())
}

impl Metadata {
    /// Validate native LV2 metadata. Args: shared class buses. Returns an error for unsupported cached port shapes or unbounded metadata.
    pub(super) fn validate(&self, layout: &vst3_host::AudioBusLayout) -> Result<(), String> {
        if self.ports.len() > 512
            || self.required.len() > 256
            || self.optional.len() > 256
            || self.unsupported.len() > 256
            || self
                .required
                .iter()
                .chain(&self.optional)
                .chain(&self.unsupported)
                .any(|s| s.len() > 4096)
        {
            return Err("LV2 metadata exceeds its bounds".into());
        }
        for (index, port) in self.ports.iter().enumerate() {
            if port.index as usize != index
                || port.name.len() > 4096
                || port.symbol.len() > 4096
                || !port.min.is_finite()
                || !port.max.is_finite()
                || port.min > port.max
                || !port.default.is_finite()
            {
                return Err("LV2 cached port metadata is malformed".into());
            }
            match port.kind.as_str() {
                "audio" => {
                    let buses = if port.input {
                        &layout.inputs
                    } else {
                        &layout.outputs
                    };
                    if buses
                        .get(port.bus)
                        .is_none_or(|b| port.channel >= b.channel_count)
                    {
                        return Err("LV2 cached audio port escapes its bus".into());
                    }
                }
                "control" | "atom" | "optional" => {}
                _ => return Err("Unknown LV2 cached port class".into()),
            }
        }
        Ok(())
    }
    /// Measure retained LV2 metadata. Args: this metadata. Returns owned bytes for graph admission.
    pub(super) fn bytes(&self) -> usize {
        self.ports.capacity() * std::mem::size_of::<Port>()
            + self
                .ports
                .iter()
                .map(|p| p.name.capacity() + p.symbol.capacity() + p.kind.capacity())
                .sum::<usize>()
            + [&self.required, &self.optional, &self.unsupported]
                .iter()
                .map(|v| {
                    v.capacity() * std::mem::size_of::<String>()
                        + v.iter().map(String::capacity).sum::<usize>()
                })
                .sum::<usize>()
    }
}
