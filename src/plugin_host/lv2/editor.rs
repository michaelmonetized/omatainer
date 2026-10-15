use super::*;
use std::sync::Mutex;
#[repr(C)]
struct Descriptor {
    uri: *const c_char,
    instantiate: Option<
        unsafe extern "C" fn(
            *const Descriptor,
            *const c_char,
            *const c_char,
            unsafe extern "C" fn(M, u32, u32, u32, P),
            M,
            *mut M,
            *const *const Feature,
        ) -> M,
    >,
    cleanup: Option<unsafe extern "C" fn(M)>,
    port_event: Option<unsafe extern "C" fn(M, u32, u32, u32, P)>,
    extension: Option<unsafe extern "C" fn(*const c_char) -> P>,
}
#[repr(C)]
struct Idle {
    idle: Option<unsafe extern "C" fn(M) -> i32>,
}
#[repr(C)]
struct Show {
    show: Option<unsafe extern "C" fn(M) -> i32>,
    hide: Option<unsafe extern "C" fn(M) -> i32>,
}
struct Controller {
    edits: Mutex<Vec<(u32, f32)>>,
    atoms: Mutex<Vec<(u32, Vec<u8>)>>,
    event_transfer: u32,
}
pub(super) struct Editor {
    raw: M,
    descriptor: *const Descriptor,
    controller: Box<Controller>,
    _idle_feature: Box<Feature>,
    _features: Vec<*const Feature>,
    _library: libloading::Library,
}
impl Editor {
    /// Open an LV2 native floating UI. Args: loaded processor. Returns an editor with verified idle/show interfaces, or a generic-controls diagnostic.
    pub fn open(instance: &Instance) -> Result<Self, String> {
        let world = &instance.world;
        let uri = world.uri(
            instance
                .saved
                .class_id
                .strip_prefix("lv2:")
                .ok_or("Invalid LV2 URI")?,
        )?;
        let plugin = unsafe {
            (world.api.plugins_get_by_uri)((world.api.world_get_all_plugins)(world.raw), uri)
        };
        unsafe {
            (world.api.node_free)(uri);
        }
        if plugin.is_null() {
            return Err("LV2 processor metadata disappeared".into());
        }
        let uis = unsafe { (world.api.plugin_get_uis)(plugin) };
        let result = (|| {
            let mut it = unsafe { (world.api.uis_begin)(uis) };
            let mut attempts = Vec::new();
            for _ in 0..64 {
                if unsafe { (world.api.uis_is_end)(uis, it) } {
                    break;
                }
                let ui = unsafe { (world.api.uis_get)(uis, it) };
                it = unsafe { (world.api.uis_next)(uis, it) };
                match Self::from_ui(instance, ui) {
                    Ok(editor) => return Ok(editor),
                    Err(error) => attempts.push(error),
                }
            }
            Err(format!(
                "No compatible LV2 floating editor: {}. Use generic parameter controls",
                attempts.join("; ")
            ))
        })();
        unsafe {
            (world.api.uis_free)(uis);
        }
        result
    }
    fn from_ui(instance: &Instance, ui: P) -> Result<Self, String> {
        let world = &instance.world;
        unsafe {
            let uri = (world.api.ui_get_uri)(ui);
            (world.api.world_load_resource)(world.raw, uri);
            let required_key = world.uri("http://lv2plug.in/ns/lv2core#requiredFeature")?;
            let required = world.list((world.api.world_find_nodes)(
                world.raw,
                uri,
                required_key,
                ptr::null(),
            ))?;
            (world.api.node_free)(required_key);
            for feature in required {
                if !features::SUPPORTED.contains(&feature.as_str())
                    && feature != "http://lv2plug.in/ns/extensions/ui#idleInterface"
                {
                    return Err(format!("UI requires unsupported feature {feature}"));
                }
            }
            let path =
                (world.api.node_get_path)((world.api.ui_get_binary_uri)(ui), ptr::null_mut());
            if path.is_null() {
                return Err("LV2 UI binary is not a local native file".into());
            }
            let path_string = CStr::from_ptr(path).to_string_lossy().into_owned();
            (world.api.free)(path.cast());
            let path = PathBuf::from(path_string)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !path.starts_with(
                instance
                    .saved
                    .binary
                    .bundle
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            ) {
                return Err("LV2 UI binary escapes its inspected bundle".into());
            }
            let library = libloading::Library::new(&path).map_err(|e| e.to_string())?;
            let descriptors = *library
                .get::<unsafe extern "C" fn(u32) -> *const Descriptor>(b"lv2ui_descriptor\0")
                .map_err(|e| e.to_string())?;
            let wanted = world.text(uri)?;
            let mut descriptor = ptr::null();
            for i in 0..64 {
                let d = descriptors(i);
                if d.is_null() {
                    break;
                }
                if !(*d).uri.is_null() && CStr::from_ptr((*d).uri).to_bytes() == wanted.as_bytes() {
                    descriptor = d;
                    break;
                }
            }
            let d = descriptor
                .as_ref()
                .ok_or("LV2 UI descriptor is unavailable")?;
            let extension = d.extension.ok_or("LV2 UI has no floating interfaces")?;
            let idle = extension(c"http://lv2plug.in/ns/extensions/ui#idleInterface".as_ptr())
                .cast::<Idle>()
                .as_ref()
                .and_then(|i| i.idle)
                .ok_or("LV2 UI has no idle interface")?;
            let show = extension(c"http://lv2plug.in/ns/extensions/ui#showInterface".as_ptr())
                .cast::<Show>()
                .as_ref()
                .ok_or("LV2 UI has no floating show interface")?;
            let show_fn = show.show.ok_or("LV2 floating show is unavailable")?;
            if show.hide.is_none() || d.cleanup.is_none() {
                return Err("LV2 UI lifecycle is incomplete".into());
            }
            let mut controller = Box::new(Controller {
                edits: Mutex::new(Vec::with_capacity(512)),
                atoms: Mutex::new(Vec::with_capacity(64)),
                event_transfer: instance
                    .features
                    .id("http://lv2plug.in/ns/ext/atom#eventTransfer")?,
            });
            let plugin = CString::new(instance.saved.class_id.strip_prefix("lv2:").unwrap())
                .map_err(|e| e.to_string())?;
            let bundle_path =
                (world.api.node_get_path)((world.api.ui_get_bundle_uri)(ui), ptr::null_mut());
            if bundle_path.is_null() {
                return Err("LV2 UI bundle path is unavailable".into());
            }
            let idle_feature = Box::new(Feature {
                uri: c"http://lv2plug.in/ns/extensions/ui#idleInterface".as_ptr(),
                data: ptr::null_mut(),
            });
            let mut features = instance
                .features
                .pointers
                .iter()
                .copied()
                .filter(|p| !p.is_null())
                .collect::<Vec<_>>();
            features.push(&*idle_feature);
            features.push(ptr::null());
            let mut widget = ptr::null_mut();
            let raw = d.instantiate.ok_or("Missing LV2 UI instantiate")?(
                descriptor,
                plugin.as_ptr(),
                bundle_path,
                write,
                (&mut *controller as *mut Controller).cast(),
                &mut widget,
                features.as_ptr(),
            );
            (world.api.free)(bundle_path.cast());
            if raw.is_null() {
                return Err("LV2 UI instantiation refused".into());
            }
            let editor = Self {
                raw,
                descriptor,
                controller,
                _idle_feature: idle_feature,
                _features: features,
                _library: library,
            };
            if show_fn(raw) != 0 {
                return Err("LV2 UI could not show its floating window".into());
            }
            let _ = idle;
            Ok(editor)
        }
    }
    /// Synchronize native UI edits and meters. Args: live editor and instance. Returns true when the plugin closes its window.
    pub fn service(
        &mut self,
        controls: &mut [f32],
        ports: &[Port],
        pending: &mut Vec<(u32, Vec<u8>)>,
        output: &mut Vec<(u32, Vec<u8>)>,
    ) -> Result<bool, String> {
        let edits = std::mem::take(&mut *self.controller.edits.lock().map_err(|e| e.to_string())?);
        for (id, value) in edits {
            if let Some(port) = ports
                .get(id as usize)
                .filter(|p| p.input && p.kind == "control")
            {
                if value.is_finite() && (port.min..=port.max).contains(&value) {
                    controls[id as usize] = value;
                }
            }
        }
        let mut atoms = self.controller.atoms.lock().map_err(|e| e.to_string())?;
        for (port, data) in atoms.drain(..) {
            if ports
                .get(port as usize)
                .is_some_and(|p| p.input && p.kind == "atom")
                && pending.len() < 64
            {
                pending.push((port, data));
            }
        }
        drop(atoms);
        unsafe {
            let d = &*self.descriptor;
            if let Some(event) = d.port_event {
                for (port, bytes) in output.drain(..) {
                    event(
                        self.raw,
                        port,
                        bytes.len() as u32,
                        self.controller.event_transfer,
                        bytes.as_ptr().cast(),
                    );
                }
                for port in ports.iter().filter(|p| p.kind == "control") {
                    event(
                        self.raw,
                        port.index,
                        4,
                        0,
                        (&controls[port.index as usize] as *const f32).cast(),
                    );
                }
            }
            let extension = d.extension.ok_or("LV2 UI extension accessor disappeared")?;
            let idle = extension(c"http://lv2plug.in/ns/extensions/ui#idleInterface".as_ptr())
                .cast::<Idle>()
                .as_ref()
                .and_then(|i| i.idle)
                .ok_or("LV2 UI idle interface disappeared")?;
            Ok(idle(self.raw) != 0)
        }
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            let d = &*self.descriptor;
            if let Some(extension) = d.extension {
                if let Some(show) =
                    extension(c"http://lv2plug.in/ns/extensions/ui#showInterface".as_ptr())
                        .cast::<Show>()
                        .as_ref()
                {
                    if let Some(hide) = show.hide {
                        hide(self.raw);
                    }
                }
            }
            if let Some(cleanup) = d.cleanup {
                cleanup(self.raw);
            }
        }
    }
}
unsafe extern "C" fn write(h: M, port: u32, size: u32, format: u32, data: P) {
    if data.is_null() {
        return;
    }
    let controller = &*(h as *const Controller);
    if format == 0 && size == 4 {
        let value = ptr::read_unaligned(data.cast::<f32>());
        if !value.is_finite() {
            return;
        }
        if let Ok(mut edits) = controller.edits.lock() {
            if edits.len() < 512 {
                edits.push((port, value));
            }
        }
    } else if format == controller.event_transfer && (8..=65536).contains(&size) {
        if let Ok(mut atoms) = controller.atoms.lock() {
            if atoms.len() < 64 {
                atoms.push((
                    port,
                    std::slice::from_raw_parts(data.cast::<u8>(), size as usize).to_vec(),
                ));
            }
        }
    }
}
