use super::*;
type Store = unsafe extern "C" fn(M, u32, P, usize, u32, u32) -> u32;
type Retrieve = unsafe extern "C" fn(M, u32, *mut usize, *mut u32, *mut u32) -> P;
#[repr(C)]
struct Interface {
    save: Option<unsafe extern "C" fn(M, Store, M, u32, *const *const Feature) -> u32>,
    restore: Option<unsafe extern "C" fn(M, Retrieve, M, u32, *const *const Feature) -> u32>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Property {
    key: String,
    kind: String,
    flags: u32,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    controls: Vec<(u32, f32)>,
    properties: Vec<Property>,
    assets: BTreeMap<String, Vec<u8>>,
}
struct Saving<'a> {
    features: &'a features::Features,
    properties: Vec<Property>,
    size: usize,
    failed: bool,
}
struct Restoring {
    values: Vec<(u32, u32, u32, Vec<u8>)>,
}
impl Instance {
    unsafe fn state_interface(&self) -> Option<&Interface> {
        (*(*self.raw).descriptor).extension?(c"http://lv2plug.in/ns/ext/state#interface".as_ptr())
            .cast::<Interface>()
            .as_ref()
    }
    /// Capture LV2 state. Args: active instance. Returns controls, URI-keyed native properties and embedded path assets within the project state limit.
    pub fn state(&mut self) -> Result<Response, String> {
        let mut saving = Saving {
            features: &self.features,
            properties: Vec::new(),
            size: 0,
            failed: false,
        };
        unsafe {
            if let Some(interface) = self.state_interface() {
                if let Some(save) = interface.save {
                    let status = save(
                        (*self.raw).handle,
                        store,
                        (&mut saving as *mut Saving<'_>).cast(),
                        3,
                        self.features.pointers.as_ptr(),
                    );
                    if status != 0 || saving.failed {
                        return Err(
                            "LV2 native state save refused a property or exceeded its bounds"
                                .into(),
                        );
                    }
                }
            }
        }
        let controls = self
            .class
            .lv2
            .as_ref()
            .unwrap()
            .ports
            .iter()
            .filter(|p| p.kind == "control" && p.input)
            .map(|p| (p.index, self.controls[p.index as usize]))
            .collect();
        let snapshot = Snapshot {
            schema: 1,
            controls,
            properties: saving.properties,
            assets: self.features.assets()?,
        };
        let mut saved = self.saved.clone();
        saved.state = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
        saved.validate()?;
        Ok(Response::State {
            saved,
            parameters: self.values()?,
        })
    }
    /// Restore LV2 state before activation. Args: saved state in this instance. Returns after URI remapping, path recreation and native property restoration.
    pub(super) fn restore(&mut self) -> Result<(), String> {
        if self.saved.state.is_empty() {
            return Ok(());
        }
        let snapshot: Snapshot =
            serde_json::from_slice(&self.saved.state).map_err(|e| e.to_string())?;
        if snapshot.schema != 1
            || snapshot.controls.len() > 512
            || snapshot.properties.len() > 4096
            || snapshot.assets.len() > 256
        {
            return Err("Unsupported LV2 state schema or collection bounds".into());
        }
        self.features.restore_assets(&snapshot.assets)?;
        let mut seen = std::collections::HashSet::new();
        for (id, value) in snapshot.controls {
            let port = self
                .class
                .lv2
                .as_ref()
                .unwrap()
                .ports
                .get(id as usize)
                .filter(|p| p.kind == "control" && p.input)
                .ok_or("LV2 saved control is unavailable")?;
            if !seen.insert(id) || !value.is_finite() || !(port.min..=port.max).contains(&value) {
                return Err("LV2 saved control is invalid".into());
            }
            self.controls[id as usize] = value;
        }
        let mut restoring = Restoring { values: Vec::new() };
        let mut seen = std::collections::HashSet::new();
        for property in snapshot.properties {
            if property.flags & 1 == 0
                || property.bytes.is_empty()
                || property.bytes.len() > MAX_STATE
                || !seen.insert(property.key.clone())
            {
                return Err("Malformed LV2 saved property".into());
            }
            restoring.values.push((
                self.features.id(&property.key)?,
                self.features.id(&property.kind)?,
                property.flags,
                property.bytes,
            ));
        }
        unsafe {
            if let Some(interface) = self.state_interface() {
                let restore = interface
                    .restore
                    .ok_or("LV2 state restore extension is incomplete")?;
                let status = restore(
                    (*self.raw).handle,
                    retrieve,
                    (&mut restoring as *mut Restoring).cast(),
                    0,
                    self.features.pointers.as_ptr(),
                );
                if status != 0 {
                    return Err("LV2 native state restore refused".into());
                }
            } else if !restoring.values.is_empty() {
                return Err("LV2 state extension disappeared".into());
            }
        }
        Ok(())
    }
}
unsafe extern "C" fn store(
    handle: M,
    key: u32,
    value: P,
    size: usize,
    kind: u32,
    flags: u32,
) -> u32 {
    let state = &mut *(handle as *mut Saving<'_>);
    if value.is_null()
        || size == 0
        || size > MAX_STATE
        || state.size.saturating_add(size) > MAX_STATE
        || state.properties.len() >= 4096
        || flags & 1 == 0
    {
        state.failed = true;
        return 6;
    }
    let (Ok(key), Ok(kind)) = (state.features.uri(key), state.features.uri(kind)) else {
        state.failed = true;
        return 2;
    };
    if flags & 2 == 0 && kind != "http://lv2plug.in/ns/ext/atom#Path" {
        state.failed = true;
        return 3;
    }
    let bytes = std::slice::from_raw_parts(value.cast::<u8>(), size).to_vec();
    state.size += size;
    state.properties.push(Property {
        key,
        kind,
        flags,
        bytes,
    });
    0
}
unsafe extern "C" fn retrieve(
    handle: M,
    key: u32,
    size: *mut usize,
    kind: *mut u32,
    flags: *mut u32,
) -> P {
    let state = &*(handle as *const Restoring);
    let Some((_, t, f, bytes)) = state.values.iter().find(|p| p.0 == key) else {
        return ptr::null();
    };
    if !size.is_null() {
        *size = bytes.len();
    }
    if !kind.is_null() {
        *kind = *t;
    }
    if !flags.is_null() {
        *flags = *f;
    }
    bytes.as_ptr().cast()
}
