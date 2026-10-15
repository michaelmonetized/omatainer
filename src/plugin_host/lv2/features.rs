use super::*;
use std::{collections::BTreeMap, sync::Mutex};
#[repr(C)]
pub(super) struct Map {
    pub(super) handle: M,
    map: Option<unsafe extern "C" fn(M, *const c_char) -> u32>,
}
#[repr(C)]
struct Unmap {
    handle: M,
    unmap: Option<unsafe extern "C" fn(M, u32) -> *const c_char>,
}
#[repr(C)]
struct OptionValue {
    context: u32,
    subject: u32,
    key: u32,
    size: u32,
    kind: u32,
    value: P,
}
#[repr(C)]
struct MapPath {
    handle: M,
    abstract_path: Option<unsafe extern "C" fn(M, *const c_char) -> *mut c_char>,
    absolute_path: Option<unsafe extern "C" fn(M, *const c_char) -> *mut c_char>,
}
#[repr(C)]
struct MakePath {
    handle: M,
    path: Option<unsafe extern "C" fn(M, *const c_char) -> *mut c_char>,
}
#[repr(C)]
struct FreePath {
    handle: M,
    free: Option<unsafe extern "C" fn(M, *mut c_char)>,
}
#[derive(Default)]
struct State {
    uris: Mutex<Vec<CString>>,
    paths: Mutex<BTreeMap<String, PathBuf>>,
    root: PathBuf,
}
pub(super) struct Features {
    state: Box<State>,
    pub(super) _map: Box<Map>,
    _unmap: Box<Unmap>,
    _options: Box<[OptionValue; 4]>,
    _block: Box<i32>,
    _rate: Box<f32>,
    _map_path: Box<MapPath>,
    _make_path: Box<MakePath>,
    _free_path: Box<FreePath>,
    _features: Vec<Feature>,
    pub pointers: Vec<*const Feature>,
}
pub(super) const SUPPORTED: &[&str] = &[
    "http://lv2plug.in/ns/ext/urid#map",
    "http://lv2plug.in/ns/ext/urid#unmap",
    "http://lv2plug.in/ns/ext/options#options",
    "http://lv2plug.in/ns/ext/buf-size#boundedBlockLength",
    "http://lv2plug.in/ns/ext/state#mapPath",
    "http://lv2plug.in/ns/ext/state#makePath",
    "http://lv2plug.in/ns/ext/state#freePath",
];
impl Features {
    /// Allocate a native feature table. Args: sample rate and private work directory. Returns stable callback pointers retained through cleanup.
    pub fn new(rate: u32, root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let mut state = Box::new(State {
            root,
            ..Default::default()
        });
        let handle = (&mut *state as *mut State).cast();
        let map = Box::new(Map {
            handle,
            map: Some(map_uri),
        });
        let unmap = Box::new(Unmap {
            handle,
            unmap: Some(unmap_uri),
        });
        let block = Box::new(BLOCK as i32);
        let rate = Box::new(rate as f32);
        let id = |s: &CStr| unsafe { map_uri(handle, s.as_ptr()) };
        let int = id(c"http://lv2plug.in/ns/ext/atom#Int");
        let float = id(c"http://lv2plug.in/ns/ext/atom#Float");
        let options = Box::new([
            OptionValue {
                context: 0,
                subject: 0,
                key: id(c"http://lv2plug.in/ns/ext/buf-size#maxBlockLength"),
                size: 4,
                kind: int,
                value: (&*block as *const i32).cast(),
            },
            OptionValue {
                context: 0,
                subject: 0,
                key: id(c"http://lv2plug.in/ns/ext/buf-size#nominalBlockLength"),
                size: 4,
                kind: int,
                value: (&*block as *const i32).cast(),
            },
            OptionValue {
                context: 0,
                subject: 0,
                key: id(c"http://lv2plug.in/ns/ext/parameters#sampleRate"),
                size: 4,
                kind: float,
                value: (&*rate as *const f32).cast(),
            },
            OptionValue {
                context: 0,
                subject: 0,
                key: 0,
                size: 0,
                kind: 0,
                value: ptr::null(),
            },
        ]);
        let map_path = Box::new(MapPath {
            handle,
            abstract_path: Some(abstract_path),
            absolute_path: Some(absolute_path),
        });
        let make_path = Box::new(MakePath {
            handle,
            path: Some(make_path),
        });
        let free_path = Box::new(FreePath {
            handle,
            free: Some(free_path),
        });
        let mut features = vec![
            Feature {
                uri: c"http://lv2plug.in/ns/ext/urid#map".as_ptr(),
                data: (&*map as *const Map).cast_mut().cast(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/urid#unmap".as_ptr(),
                data: (&*unmap as *const Unmap).cast_mut().cast(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/options#options".as_ptr(),
                data: options.as_ptr().cast_mut().cast(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength".as_ptr(),
                data: ptr::null_mut(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/state#mapPath".as_ptr(),
                data: (&*map_path as *const MapPath).cast_mut().cast(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/state#makePath".as_ptr(),
                data: (&*make_path as *const MakePath).cast_mut().cast(),
            },
            Feature {
                uri: c"http://lv2plug.in/ns/ext/state#freePath".as_ptr(),
                data: (&*free_path as *const FreePath).cast_mut().cast(),
            },
        ];
        let mut pointers = features
            .iter_mut()
            .map(|f| f as *const Feature)
            .collect::<Vec<_>>();
        pointers.push(ptr::null());
        Ok(Self {
            state,
            _map: map,
            _unmap: unmap,
            _options: options,
            _block: block,
            _rate: rate,
            _map_path: map_path,
            _make_path: make_path,
            _free_path: free_path,
            _features: features,
            pointers,
        })
    }
    pub fn id(&self, uri: &str) -> Result<u32, String> {
        let uri = CString::new(uri).map_err(|e| e.to_string())?;
        let id = unsafe { map_uri(self._map.handle, uri.as_ptr()) };
        if id == 0 {
            Err("LV2 URI map capacity exceeded".into())
        } else {
            Ok(id)
        }
    }
    pub fn uri(&self, id: u32) -> Result<String, String> {
        let uris = self.state.uris.lock().map_err(|e| e.to_string())?;
        uris.get(id.wrapping_sub(1) as usize)
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| "Unknown LV2 state URI".into())
    }
    pub fn assets(&self) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let paths = self.state.paths.lock().map_err(|e| e.to_string())?;
        let mut size = 0usize;
        paths
            .iter()
            .map(|(id, path)| {
                let mut file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(path)
                    .map_err(|e| e.to_string())?;
                if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                    return Err("LV2 state asset is not a regular file".into());
                }
                let mut bytes = Vec::new();
                file.by_ref()
                    .take(MAX_STATE as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                size = size.saturating_add(bytes.len());
                if size > MAX_STATE {
                    return Err("LV2 state assets exceed 8 MiB".into());
                }
                Ok((id.clone(), bytes))
            })
            .collect()
    }
    pub fn restore_assets(&self, assets: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        let mut paths = self.state.paths.lock().map_err(|e| e.to_string())?;
        let mut size = 0;
        for (id, bytes) in assets {
            if !safe(id) {
                return Err("LV2 state asset path escapes its namespace".into());
            }
            size += bytes.len();
            if size > MAX_STATE {
                return Err("LV2 state assets exceed 8 MiB".into());
            }
            let path = self.state.root.join(id);
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            paths.insert(id.clone(), path);
        }
        Ok(())
    }
}
impl Drop for Features {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.state.root);
    }
}
unsafe extern "C" fn map_uri(h: M, u: *const c_char) -> u32 {
    if u.is_null() {
        return 0;
    }
    let state = &*(h as *const State);
    let Ok(mut uris) = state.uris.lock() else {
        return 0;
    };
    let uri = CStr::from_ptr(u);
    if uri.to_bytes().len() > 4096 {
        return 0;
    }
    if let Some(i) = uris.iter().position(|v| v.as_c_str() == uri) {
        return i as u32 + 1;
    }
    if uris.len() >= 8192 {
        return 0;
    }
    uris.push(uri.into());
    uris.len() as u32
}
unsafe extern "C" fn unmap_uri(h: M, id: u32) -> *const c_char {
    let state = &*(h as *const State);
    let Ok(uris) = state.uris.lock() else {
        return ptr::null();
    };
    uris.get(id.wrapping_sub(1) as usize)
        .map_or(ptr::null(), |u| u.as_ptr())
}
fn safe(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && Path::new(path)
            .components()
            .all(|p| matches!(p, std::path::Component::Normal(_)))
}
pub(super) unsafe extern "C" fn abstract_path(h: M, p: *const c_char) -> *mut c_char {
    if p.is_null() {
        return ptr::null_mut();
    }
    let state = &*(h as *const State);
    let path = PathBuf::from(CStr::from_ptr(p).to_string_lossy().as_ref());
    if !path.is_absolute() {
        return ptr::null_mut();
    }
    let Ok(mut paths) = state.paths.lock() else {
        return ptr::null_mut();
    };
    if paths.len() >= 256 {
        return ptr::null_mut();
    }
    let id = format!("assets/{}", paths.len());
    paths.insert(id.clone(), path);
    CString::new(id).map_or(ptr::null_mut(), |s| libc::strdup(s.as_ptr()))
}
unsafe extern "C" fn absolute_path(h: M, p: *const c_char) -> *mut c_char {
    if p.is_null() {
        return ptr::null_mut();
    }
    let state = &*(h as *const State);
    let Ok(paths) = state.paths.lock() else {
        return ptr::null_mut();
    };
    let id = CStr::from_ptr(p).to_string_lossy();
    paths
        .get(id.as_ref())
        .and_then(|p| CString::new(p.as_os_str().as_encoded_bytes()).ok())
        .map_or(ptr::null_mut(), |s| libc::strdup(s.as_ptr()))
}
unsafe extern "C" fn make_path(h: M, p: *const c_char) -> *mut c_char {
    if p.is_null() {
        return ptr::null_mut();
    }
    let state = &*(h as *const State);
    let path = CStr::from_ptr(p).to_string_lossy();
    if !safe(&path) {
        return ptr::null_mut();
    }
    let path = state.root.join(path.as_ref());
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return ptr::null_mut();
        }
    }
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_or(ptr::null_mut(), |s| libc::strdup(s.as_ptr()))
}
pub(super) unsafe extern "C" fn free_path(_h: M, p: *mut c_char) {
    libc::free(p.cast());
}
