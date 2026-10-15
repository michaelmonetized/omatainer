use super::*;
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::*,
    ext::{audio_ports::*, latency::*, note_ports::*, params::*, state::*, tail::*},
    factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID},
    host::clap_host,
    plugin::clap_plugin,
    process::{clap_process, CLAP_PROCESS_ERROR},
    stream::{clap_istream, clap_ostream},
    version::{clap_version_is_compatible, CLAP_VERSION},
};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    ptr,
    sync::atomic::Ordering,
};
mod callbacks;
mod editor;
pub(super) mod events;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Port {
    pub id: u32,
    pub name: String,
    pub channels: u32,
    pub flags: u32,
    pub dialects: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Parameter {
    pub id: u32,
    pub module: String,
    pub flags: u32,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub audio_inputs: Vec<Port>,
    pub audio_outputs: Vec<Port>,
    pub note_inputs: Vec<Port>,
    pub note_outputs: Vec<Port>,
    pub parameters: Vec<Parameter>,
    pub extensions: Vec<String>,
    pub unsupported_host_extensions: Vec<String>,
}

/// Identify a Linux CLAP module without loading it.
/// Args: absolute module path. Returns a bounded ELF identity or an installation diagnostic.
pub(super) fn identify(path: &Path) -> Result<BinaryIdentity, String> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Choose an absolute CLAP module path without parent traversal".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > 128 * 1024 * 1024 {
        return Err("CLAP module must be a regular file within 128 MiB".into());
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    use std::os::unix::fs::MetadataExt;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != bytes.len() as u64
        || before.len() != after.len()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.mtime() != after.mtime()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err("CLAP module changed during inspection".into());
    }
    let machine = match std::env::consts::ARCH {
        "aarch64" => 183,
        "x86_64" => 62,
        _ => return Err("CLAP host requires native aarch64 or x86_64 Linux".into()),
    };
    if bytes.len() < 20
        || &bytes[..4] != b"\x7fELF"
        || bytes[4] != 2
        || bytes[5] != 1
        || u16::from_le_bytes([bytes[18], bytes[19]]) != machine
    {
        return Err("CLAP module is not a native 64-bit Linux ELF".into());
    }
    Ok(BinaryIdentity {
        bundle: path.into(),
        binary: path.into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        arch: std::env::consts::ARCH.into(),
    })
}

struct Module {
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    _library: libloading::Library,
}
impl Module {
    /// Load the CLAP entry. Args: inspected path. Returns the initialized factory, retained until every instance is destroyed.
    fn open(path: &Path) -> Result<Self, String> {
        unsafe {
            let library = libloading::Library::new(path).map_err(|e| e.to_string())?;
            let entry = *library
                .get::<*const clap_plugin_entry>(b"clap_entry\0")
                .map_err(|e| e.to_string())?;
            let e = entry.as_ref().ok_or("CLAP entry is null")?;
            if !clap_version_is_compatible(e.clap_version) {
                return Err("Unsupported CLAP ABI version".into());
            }
            let name =
                CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
            let init = e.init.ok_or("Missing CLAP entry init")?;
            let deinit = e.deinit.ok_or("Missing CLAP entry deinit")?;
            let get = e.get_factory.ok_or("Missing CLAP factory accessor")?;
            if !init(name.as_ptr()) {
                return Err("CLAP entry initialization refused".into());
            }
            let factory = get(CLAP_PLUGIN_FACTORY_ID.as_ptr()).cast::<clap_plugin_factory>();
            if factory.is_null() {
                deinit();
                return Err("Missing CLAP plugin factory".into());
            }
            Ok(Self {
                entry,
                factory,
                _library: library,
            })
        }
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            if let Some(f) = (*self.entry).deinit {
                f();
            }
        }
    }
}

/// Read a CLAP descriptor string. Args: plugin-owned null-terminated pointer. Returns owned UTF-8 within the host's metadata limit.
unsafe fn string(p: *const c_char) -> Result<String, String> {
    if p.is_null() {
        return Ok(String::new());
    }
    let s = CStr::from_ptr(p).to_str().map_err(|e| e.to_string())?;
    if s.len() > 4096 {
        return Err("CLAP metadata string exceeds 4096 bytes".into());
    }
    Ok(s.into())
}
fn name(buf: &[c_char]) -> String {
    String::from_utf8_lossy(
        &buf.iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect::<Vec<_>>(),
    )
    .into_owned()
}

pub(super) struct Instance {
    plugin: *const clap_plugin,
    host: Box<clap_host>,
    callbacks: Box<callbacks::State>,
    saved: Saved,
    pub(super) class: Class,
    rate: u32,
    active: bool,
    started: bool,
    steady: i64,
    editor: Option<editor::Editor>,
    _module: Module,
}
impl Instance {
    /// Create one native processor. Args: inspected identity, CLAP descriptor id, saved state and rate. Returns an initialized, inactive processor.
    fn create(binary: &BinaryIdentity, id: &str, state: &[u8], rate: u32) -> Result<Self, String> {
        let module = Module::open(&binary.binary)?;
        let mut callbacks = Box::new(callbacks::State::default());
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *callbacks as *mut callbacks::State).cast(),
            name: c"Omatainer".as_ptr(),
            vendor: c"Omatainer".as_ptr(),
            url: c"https://github.com/michaelmonetized/omatainer".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(callbacks::extension),
            request_restart: Some(callbacks::restart),
            request_process: Some(callbacks::process),
            request_callback: Some(callbacks::callback),
        });
        let cid = CString::new(id).map_err(|e| e.to_string())?;
        unsafe {
            let create = (*module.factory)
                .create_plugin
                .ok_or("Missing CLAP create_plugin")?;
            let plugin = create(module.factory, &*host, cid.as_ptr());
            let p = plugin.as_ref().ok_or("CLAP factory refused processor")?;
            let destroy = p.destroy.ok_or("Missing CLAP destroy")?;
            if !p.init.ok_or("Missing CLAP init")?(plugin) {
                destroy(plugin);
                return Err("CLAP processor initialization refused".into());
            }
            let saved = Saved {
                schema: 1,
                binary: binary.clone(),
                class_id: format!("clap:{id}"),
                plugin_version: String::new(),
                state_codec: "clap-state-1".into(),
                state: state.into(),
            };
            let mut instance = Self {
                plugin,
                host,
                callbacks,
                saved,
                class: empty_class(binary),
                rate,
                active: false,
                started: false,
                steady: 0,
                editor: None,
                _module: module,
            };
            if p.activate.is_none()
                || p.deactivate.is_none()
                || p.start_processing.is_none()
                || p.stop_processing.is_none()
                || p.process.is_none()
                || p.get_extension.is_none()
            {
                return Err("CLAP lifecycle vtable is incomplete".into());
            }
            if !state.is_empty() {
                instance.restore(state)?;
            }
            instance.class = instance.report()?;
            instance.callbacks.restart.store(false, Ordering::Release);
            if instance.class.info.uid != format!("clap:{id}") {
                return Err("CLAP descriptor identity changed".into());
            }
            instance.saved.plugin_version = instance.class.info.version.clone();
            Ok(instance)
        }
    }
    /// Query an optional plugin extension. Args: extension identifier. Returns its vtable only when the plugin supplies it.
    unsafe fn extension<T>(&self, id: &CStr) -> Option<&T> {
        ((*self.plugin).get_extension?)(self.plugin, id.as_ptr())
            .cast::<T>()
            .as_ref()
    }
    fn report(&self) -> Result<Class, String> {
        unsafe {
            let d = (*self.plugin)
                .desc
                .as_ref()
                .ok_or("Missing CLAP descriptor")?;
            let mut metadata = Metadata {
                audio_inputs: self.audio_ports(true)?,
                audio_outputs: self.audio_ports(false)?,
                note_inputs: self.note_ports(true)?,
                note_outputs: self.note_ports(false)?,
                parameters: Vec::new(),
                extensions: Vec::new(),
                unsupported_host_extensions: self
                    .callbacks
                    .unsupported
                    .lock()
                    .map_err(|e| e.to_string())?
                    .clone(),
            };
            for id in [
                CLAP_EXT_AUDIO_PORTS,
                CLAP_EXT_NOTE_PORTS,
                CLAP_EXT_PARAMS,
                CLAP_EXT_STATE,
                CLAP_EXT_LATENCY,
                CLAP_EXT_TAIL,
                clap_sys::ext::gui::CLAP_EXT_GUI,
            ] {
                if self.extension::<u8>(id).is_some() {
                    metadata.extensions.push(id.to_string_lossy().into());
                }
            }
            let mut parameters = Vec::new();
            if let Some(p) = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) {
                let count = p.count.ok_or("Missing CLAP parameter count")?(self.plugin);
                if count > 8192 {
                    return Err("CLAP parameter count exceeds 8192".into());
                }
                for i in 0..count {
                    let mut info: clap_param_info = std::mem::zeroed();
                    if !p.get_info.ok_or("Missing CLAP parameter metadata")?(
                        self.plugin,
                        i,
                        &mut info,
                    ) || info.id == u32::MAX
                        || !info.min_value.is_finite()
                        || !info.max_value.is_finite()
                        || info.min_value > info.max_value
                        || !info.default_value.is_finite()
                        || !(info.min_value..=info.max_value).contains(&info.default_value)
                    {
                        return Err("Malformed CLAP parameter metadata".into());
                    }
                    let mut value = 0.;
                    if !p.get_value.ok_or("Missing CLAP parameter value")?(
                        self.plugin,
                        info.id,
                        &mut value,
                    ) || !value.is_finite()
                        || !(info.min_value..=info.max_value).contains(&value)
                    {
                        return Err("Malformed CLAP parameter value".into());
                    }
                    let normalize = |v: f64| {
                        if info.max_value > info.min_value {
                            (v - info.min_value) / (info.max_value - info.min_value)
                        } else {
                            0.
                        }
                    };
                    parameters.push(vst3_host::Parameter {
                        id: info.id,
                        name: name(&info.name),
                        value: normalize(value),
                        min: 0.,
                        max: 1.,
                        default: normalize(info.default_value),
                        unit: String::new(),
                        step_count: if info.flags & CLAP_PARAM_IS_STEPPED != 0 {
                            (info.max_value - info.min_value).min(i32::MAX as f64) as i32
                        } else {
                            0
                        },
                        can_automate: info.flags & CLAP_PARAM_IS_AUTOMATABLE != 0,
                        is_read_only: info.flags & CLAP_PARAM_IS_READONLY != 0,
                        is_bypass: info.flags & CLAP_PARAM_IS_BYPASS != 0,
                        flags: if info.flags & CLAP_PARAM_IS_HIDDEN != 0 {
                            1 << 4
                        } else {
                            0
                        },
                    });
                    metadata.parameters.push(Parameter {
                        id: info.id,
                        module: name(&info.module),
                        flags: info.flags,
                        min: info.min_value,
                        max: info.max_value,
                        default: info.default_value,
                    });
                }
            }
            let mut instrument = false;
            if !d.features.is_null() {
                for i in 0..64 {
                    let f = *d.features.add(i);
                    if f.is_null() {
                        break;
                    }
                    instrument |= string(f)? == "instrument";
                    if i == 63 {
                        return Err("Too many CLAP feature strings".into());
                    }
                }
            }
            let bus = |p: &Port| vst3_host::AudioBusConfig {
                channel_count: p.channels as usize,
                active: true,
            };
            let result = Class {
                info: vst3_host::PluginInfo {
                    path: self.saved.binary.bundle.clone(),
                    name: string(d.name)?,
                    vendor: string(d.vendor)?,
                    version: string(d.version)?,
                    category: if instrument { "Instrument" } else { "Fx" }.into(),
                    uid: format!("clap:{}", string(d.id)?),
                    audio_inputs: metadata.audio_inputs.len() as u32,
                    audio_outputs: metadata.audio_outputs.len() as u32,
                    has_midi_input: !metadata.note_inputs.is_empty(),
                    has_midi_output: !metadata.note_outputs.is_empty(),
                    has_gui: self
                        .extension::<u8>(clap_sys::ext::gui::CLAP_EXT_GUI)
                        .is_some(),
                },
                layout: vst3_host::AudioBusLayout {
                    inputs: metadata.audio_inputs.iter().map(bus).collect(),
                    outputs: metadata.audio_outputs.iter().map(bus).collect(),
                },
                parameters,
                latency: if self.active { self.latency() } else { 0 },
                tail: if self.active { self.tail() } else { 0 },
                native: Some(metadata),
                lv2: None,
            };
            result.validate()?;
            Ok(result)
        }
    }
    unsafe fn audio_ports(&self, input: bool) -> Result<Vec<Port>, String> {
        let Some(p) = self.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS) else {
            return Ok(Vec::new());
        };
        let count = p.count.ok_or("Missing CLAP audio port count")?(self.plugin, input);
        if count > MAX_BUSES as u32 {
            return Err("CLAP audio port count exceeds eight".into());
        }
        (0..count)
            .map(|i| {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                if !p.get.ok_or("Missing CLAP audio port metadata")?(
                    self.plugin,
                    i,
                    input,
                    &mut info,
                ) || info.id == u32::MAX
                    || info.channel_count == 0
                    || info.channel_count > MAX_CHANNELS as u32
                {
                    return Err("Malformed CLAP audio port".into());
                }
                Ok(Port {
                    id: info.id,
                    name: name(&info.name),
                    channels: info.channel_count,
                    flags: info.flags,
                    dialects: 0,
                })
            })
            .collect()
    }
    unsafe fn note_ports(&self, input: bool) -> Result<Vec<Port>, String> {
        let Some(p) = self.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS) else {
            return Ok(Vec::new());
        };
        let count = p.count.ok_or("Missing CLAP note port count")?(self.plugin, input);
        if count > 8 {
            return Err("CLAP note port count exceeds eight".into());
        }
        (0..count)
            .map(|i| {
                let mut info: clap_note_port_info = std::mem::zeroed();
                if !p.get.ok_or("Missing CLAP note port metadata")?(
                    self.plugin,
                    i,
                    input,
                    &mut info,
                ) || info.id == u32::MAX
                {
                    return Err("Malformed CLAP note port".into());
                }
                Ok(Port {
                    id: info.id,
                    name: name(&info.name),
                    channels: 0,
                    flags: 0,
                    dialects: info.supported_dialects,
                })
            })
            .collect()
    }
    fn latency(&self) -> u32 {
        unsafe {
            self.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY)
                .and_then(|e| e.get)
                .map_or(0, |f| f(self.plugin))
        }
    }
    fn tail(&self) -> u32 {
        unsafe {
            self.extension::<clap_plugin_tail>(CLAP_EXT_TAIL)
                .and_then(|e| e.get)
                .map_or(0, |f| f(self.plugin))
        }
    }
    fn activate(&mut self) -> Result<(), String> {
        unsafe {
            if !(*self.plugin).activate.unwrap()(self.plugin, self.rate as f64, 1, BLOCK as u32) {
                return Err("CLAP activation refused".into());
            }
        }
        self.active = true;
        self.callbacks.audio.store(true, Ordering::Release);
        let started = unsafe { (*self.plugin).start_processing.unwrap()(self.plugin) };
        self.callbacks.audio.store(false, Ordering::Release);
        self.started = started;
        if !started {
            return Err("CLAP start_processing refused".into());
        }
        Ok(())
    }
    fn deactivate(&mut self) {
        unsafe {
            if self.started {
                self.callbacks.audio.store(true, Ordering::Release);
                (*self.plugin).stop_processing.unwrap()(self.plugin);
                self.callbacks.audio.store(false, Ordering::Release);
                self.started = false;
            }
            if self.active {
                (*self.plugin).deactivate.unwrap()(self.plugin);
                self.active = false;
            }
        }
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut cursor = std::io::Cursor::new(bytes);
        let stream = clap_istream {
            ctx: (&mut cursor as *mut std::io::Cursor<&[u8]>).cast(),
            read: Some(read_state),
        };
        unsafe {
            if !self
                .extension::<clap_plugin_state>(CLAP_EXT_STATE)
                .and_then(|s| s.load)
                .ok_or("CLAP processor cannot restore opaque state")?(
                self.plugin, &stream
            ) {
                return Err("CLAP processor refused saved state".into());
            }
        }
        Ok(())
    }
    pub(super) fn state(&mut self) -> Result<Response, String> {
        self.service()?;
        (|| {
            let mut bytes = Vec::<u8>::new();
            let stream = clap_ostream {
                ctx: (&mut bytes as *mut Vec<u8>).cast(),
                write: Some(write_state),
            };
            unsafe {
                if let Some(save) = self
                    .extension::<clap_plugin_state>(CLAP_EXT_STATE)
                    .and_then(|s| s.save)
                {
                    if !save(self.plugin, &stream) {
                        return Err("CLAP state save refused or exceeded 8 MiB".into());
                    }
                } else {
                    return Err(
                        "CLAP state extension is unavailable; project state cannot be captured"
                            .into(),
                    );
                }
            }
            let mut saved = self.saved.clone();
            saved.state = bytes;
            saved.validate()?;
            Ok(Response::State {
                saved,
                parameters: self.values()?,
            })
        })()
    }
    fn values(&self) -> Result<Vec<(u32, f64)>, String> {
        unsafe {
            self.class
                .writable_parameters()
                .map(|p| {
                    let mut v = 0.;
                    let ext = self
                        .extension::<clap_plugin_params>(CLAP_EXT_PARAMS)
                        .ok_or("CLAP parameters disappeared")?;
                    if !ext.get_value.ok_or("Missing CLAP get_value")?(self.plugin, p.id, &mut v) {
                        return Err("CLAP get_value refused".into());
                    }
                    let range = self
                        .class
                        .native
                        .as_ref()
                        .unwrap()
                        .parameters
                        .iter()
                        .find(|r| r.id == p.id)
                        .ok_or("CLAP parameter disappeared")?;
                    let v = if range.max > range.min {
                        (v - range.min) / (range.max - range.min)
                    } else {
                        0.
                    };
                    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                        return Err("CLAP parameter returned an invalid value".into());
                    }
                    Ok((p.id, v))
                })
                .collect()
        }
    }
    pub(super) fn process(&mut self, frame: Frame) -> Result<Response, String> {
        frame.validate()?;
        if !self.started {
            return Err("CLAP processor is stopped".into());
        }
        if frame.inputs.len() != self.class.layout.inputs.len()
            || frame
                .inputs
                .iter()
                .zip(&self.class.layout.inputs)
                .any(|(v, b)| v.len() != b.channel_count)
        {
            return Err("CLAP input buses changed; review the routing graph".into());
        }
        let queue = events::input(&frame, &self.class)?;
        let mut out = events::Output::new(frame.frames);
        let input_events = clap_input_events {
            ctx: (&queue as *const Vec<events::Event>).cast_mut().cast(),
            size: Some(events::size),
            get: Some(events::get),
        };
        let output_events = clap_output_events {
            ctx: (&mut out as *mut events::Output).cast(),
            try_push: Some(events::push),
        };
        let mut inputs = frame.inputs.clone();
        let mut outputs: Vec<Vec<Vec<f32>>> = self
            .class
            .layout
            .outputs
            .iter()
            .map(|b| vec![vec![0.; frame.frames]; b.channel_count])
            .collect();
        let mut in_ptrs: Vec<Vec<*mut f32>> = inputs
            .iter_mut()
            .map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect())
            .collect();
        let mut out_ptrs: Vec<Vec<*mut f32>> = outputs
            .iter_mut()
            .map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect())
            .collect();
        let buffers = |b: &mut Vec<Vec<*mut f32>>| -> Vec<clap_audio_buffer> {
            b.iter_mut()
                .map(|p| clap_audio_buffer {
                    data32: p.as_mut_ptr(),
                    data64: ptr::null_mut(),
                    channel_count: p.len() as u32,
                    latency: 0,
                    constant_mask: 0,
                })
                .collect()
        };
        let ins = buffers(&mut in_ptrs);
        let mut outs = buffers(&mut out_ptrs);
        let transport = events::transport(&frame, self.rate);
        let process = clap_process {
            steady_time: self.steady,
            frames_count: frame.frames as u32,
            transport: &transport,
            audio_inputs: ins.as_ptr(),
            audio_outputs: outs.as_mut_ptr(),
            audio_inputs_count: ins.len() as u32,
            audio_outputs_count: outs.len() as u32,
            in_events: &input_events,
            out_events: &output_events,
        };
        self.callbacks.audio.store(true, Ordering::Release);
        let status = unsafe { (*self.plugin).process.unwrap()(self.plugin, &process) };
        self.callbacks.audio.store(false, Ordering::Release);
        self.steady = self.steady.saturating_add(frame.frames as i64);
        if status == CLAP_PROCESS_ERROR
            || out.invalid
            || outputs.iter().flatten().flatten().any(|v| !v.is_finite())
        {
            return Err("CLAP processor returned invalid events, audio or process status".into());
        }
        let restart = self.callbacks.restart.swap(false, Ordering::AcqRel);
        if restart {
            return Err("CLAP processor requested a restart or port/parameter rescan; capture state and review the processor before retrying".into());
        }
        Ok(Response::Audio {
            outputs,
            latency: self.latency(),
            tail: self.tail(),
            restart: 0,
            editor_open: self.editor.is_some(),
            parameters: self.values()?,
            midi: out.midi,
        })
    }
    pub(super) fn service(&mut self) -> Result<(), String> {
        if self.callbacks.flush.swap(false, Ordering::AcqRel) {
            let queue = Vec::<events::Event>::new();
            let mut output = events::Output::new(BLOCK);
            let input = clap_input_events {
                ctx: (&queue as *const Vec<events::Event>).cast_mut().cast(),
                size: Some(events::size),
                get: Some(events::get),
            };
            let out = clap_output_events {
                ctx: (&mut output as *mut events::Output).cast(),
                try_push: Some(events::push),
            };
            self.callbacks.audio.store(self.active, Ordering::Release);
            unsafe {
                if let Some(flush) = self
                    .extension::<clap_plugin_params>(CLAP_EXT_PARAMS)
                    .and_then(|p| p.flush)
                {
                    flush(self.plugin, &input, &out);
                }
            }
            self.callbacks.audio.store(false, Ordering::Release);
            if output.invalid {
                return Err("CLAP parameter flush returned invalid events".into());
            }
        }
        if self.callbacks.callback.swap(false, Ordering::AcqRel) {
            unsafe {
                if let Some(f) = (*self.plugin).on_main_thread {
                    f(self.plugin);
                }
            }
        }
        callbacks::service(&self.callbacks, self.plugin)?;
        if let Some(editor) = &mut self.editor {
            if editor.service(self.plugin, &self.callbacks)? {
                self.close_editor();
            }
        }
        Ok(())
    }
    pub(super) fn editor(&mut self, open: bool) -> Result<Response, String> {
        if !open {
            self.close_editor();
        } else if self.editor.is_none() {
            self.callbacks.closed.store(false, Ordering::Release);
            self.callbacks.destroyed.store(false, Ordering::Release);
            self.editor = Some(editor::Editor::open(self.plugin)?);
        }
        Ok(Response::Ok)
    }
    fn close_editor(&mut self) {
        if let Some(mut editor) = self.editor.take() {
            editor.close(
                self.plugin,
                self.callbacks.destroyed.swap(false, Ordering::AcqRel),
            );
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        self.close_editor();
        self.deactivate();
        unsafe {
            if let Some(f) = (*self.plugin).destroy {
                f(self.plugin);
            }
        }
        let _ = &self.host;
    }
}

fn empty_class(binary: &BinaryIdentity) -> Class {
    Class {
        info: vst3_host::PluginInfo {
            path: binary.bundle.clone(),
            name: String::new(),
            vendor: String::new(),
            version: String::new(),
            category: String::new(),
            uid: String::new(),
            audio_inputs: 0,
            audio_outputs: 0,
            has_midi_input: false,
            has_midi_output: false,
            has_gui: false,
        },
        layout: vst3_host::AudioBusLayout {
            inputs: Vec::new(),
            outputs: Vec::new(),
        },
        parameters: Vec::new(),
        latency: 0,
        tail: 0,
        native: None,
        lv2: None,
    }
}

/// Inspect all classes in a module. Args: verified binary identity. Returns native metadata after constructing each class in this isolated worker.
pub(super) fn probe(binary: &BinaryIdentity) -> Result<Vec<Class>, String> {
    let ids = {
        let module = Module::open(&binary.binary)?;
        unsafe {
            let f = &*module.factory;
            let count = f.get_plugin_count.ok_or("Missing CLAP factory count")?(module.factory);
            if count == 0 || count > 64 {
                return Err("CLAP factory must expose one to 64 processors".into());
            }
            (0..count)
                .map(|i| {
                    let d = f.get_plugin_descriptor.ok_or("Missing CLAP descriptors")?(
                        module.factory,
                        i,
                    )
                    .as_ref()
                    .ok_or("CLAP descriptor is null")?;
                    if !clap_version_is_compatible(d.clap_version) {
                        return Err("Unsupported CLAP descriptor ABI".into());
                    }
                    string(d.id)
                })
                .collect::<Result<Vec<_>, String>>()?
        }
    };
    ids.iter()
        .map(|id| {
            Instance::create(binary, id, &[], 48000).and_then(|mut i| {
                i.activate()?;
                i.class.latency = i.latency();
                i.class.tail = i.tail();
                Ok(i.class.clone())
            })
        })
        .collect()
}
/// Restore and activate a CLAP processor. Args: saved identity/state and rate. Returns the live instance with the exact saved version.
pub(super) fn load(saved: Saved, rate: u32) -> Result<Instance, String> {
    if !(8000..=384000).contains(&rate) {
        return Err("Unsupported CLAP sample rate".into());
    }
    let id = saved
        .class_id
        .strip_prefix("clap:")
        .ok_or("Invalid CLAP class identifier")?;
    let mut instance = Instance::create(&saved.binary, id, &saved.state, rate)?;
    if instance.class.info.version != saved.plugin_version {
        return Err("CLAP version changed; review state compatibility".into());
    }
    instance.saved = saved;
    instance.activate()?;
    instance.class.latency = instance.latency();
    instance.class.tail = instance.tail();
    Ok(instance)
}
unsafe extern "C" fn read_state(s: *const clap_istream, b: *mut c_void, n: u64) -> i64 {
    if b.is_null() {
        return -1;
    }
    let c = &mut *((*s).ctx as *mut std::io::Cursor<&[u8]>);
    let remaining = c.get_ref().len().saturating_sub(c.position() as usize);
    let n = remaining.min(n as usize);
    let dest = std::slice::from_raw_parts_mut(b.cast::<u8>(), n);
    c.read(dest).map_or(-1, |n| n as i64)
}
unsafe extern "C" fn write_state(s: *const clap_ostream, b: *const c_void, n: u64) -> i64 {
    if b.is_null() || n > MAX_STATE as u64 {
        return -1;
    }
    let bytes = &mut *((*s).ctx as *mut Vec<u8>);
    if bytes.len().saturating_add(n as usize) > MAX_STATE {
        return -1;
    }
    bytes.extend_from_slice(std::slice::from_raw_parts(b.cast::<u8>(), n as usize));
    n as i64
}

#[cfg(test)]
mod tests;
