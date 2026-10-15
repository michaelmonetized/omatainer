use std::ffi::{c_char, c_void};
pub(super) type P = *const c_void;
pub(super) type M = *mut c_void;
#[repr(C)]
pub(super) struct Feature {
    pub uri: *const c_char,
    pub data: *mut c_void,
}
#[repr(C)]
pub(super) struct Descriptor {
    pub uri: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(*const Descriptor, f64, *const c_char, *const *const Feature) -> M,
    >,
    pub connect: Option<unsafe extern "C" fn(M, u32, M)>,
    pub activate: Option<unsafe extern "C" fn(M)>,
    pub run: Option<unsafe extern "C" fn(M, u32)>,
    pub deactivate: Option<unsafe extern "C" fn(M)>,
    pub cleanup: Option<unsafe extern "C" fn(M)>,
    pub extension: Option<unsafe extern "C" fn(*const c_char) -> P>,
}
#[repr(C)]
pub(super) struct RawInstance {
    pub descriptor: *const Descriptor,
    pub handle: M,
    pub private: M,
}
macro_rules! api {($($name:ident : $ty:ty),* $(,)?)=>{pub(super) struct Api {$(pub $name:$ty,)*_library:libloading::Library}impl Api {pub fn open()->Result<Self,String> {unsafe {let library=libloading::Library::new("liblilv-0.so.0").map_err(|e|format!("Install the native lilv LV2 runtime: {e}"))?;Ok(Self {$($name:*library.get::<$ty>(concat!("lilv_",stringify!($name),"\0").as_bytes()).map_err(|e|e.to_string())?,)*_library:library})}}}};}
api! {
 world_new:unsafe extern "C" fn()->M,
 world_free:unsafe extern "C" fn(M),
 free:unsafe extern "C" fn(M),
 world_load_resource:unsafe extern "C" fn(M,P)->i32,
 world_find_nodes:unsafe extern "C" fn(M,P,P,P)->M,
 world_load_bundle:unsafe extern "C" fn(M,P),
 world_get_all_plugins:unsafe extern "C" fn(P)->P,
 new_file_uri:unsafe extern "C" fn(M,*const c_char,*const c_char)->M,
 new_uri:unsafe extern "C" fn(M,*const c_char)->M,
 node_free:unsafe extern "C" fn(M),
 node_get_path:unsafe extern "C" fn(P,*mut *mut c_char)->*mut c_char,
 node_as_string:unsafe extern "C" fn(P)->*const c_char,
 node_as_uri:unsafe extern "C" fn(P)->*const c_char,
 node_is_uri:unsafe extern "C" fn(P)->bool,
 nodes_free:unsafe extern "C" fn(M),
 nodes_begin:unsafe extern "C" fn(P)->P,
 nodes_get:unsafe extern "C" fn(P,P)->P,
 nodes_next:unsafe extern "C" fn(P,P)->P,
 nodes_is_end:unsafe extern "C" fn(P,P)->bool,
 nodes_get_first:unsafe extern "C" fn(P)->P,
 nodes_contains:unsafe extern "C" fn(P,P)->bool,
 plugins_size:unsafe extern "C" fn(P)->u32,
 plugins_begin:unsafe extern "C" fn(P)->P,
 plugins_get:unsafe extern "C" fn(P,P)->P,
 plugins_next:unsafe extern "C" fn(P,P)->P,
 plugins_is_end:unsafe extern "C" fn(P,P)->bool,
 plugins_get_by_uri:unsafe extern "C" fn(P,P)->P,
 plugin_get_uis:unsafe extern "C" fn(P)->M,
 uis_free:unsafe extern "C" fn(M),
 uis_begin:unsafe extern "C" fn(P)->P,
 uis_get:unsafe extern "C" fn(P,P)->P,
 uis_next:unsafe extern "C" fn(P,P)->P,
 uis_is_end:unsafe extern "C" fn(P,P)->bool,
 ui_get_uri:unsafe extern "C" fn(P)->P,
 ui_get_binary_uri:unsafe extern "C" fn(P)->P,
 ui_get_bundle_uri:unsafe extern "C" fn(P)->P,
 plugin_get_uri:unsafe extern "C" fn(P)->P,
 plugin_get_name:unsafe extern "C" fn(P)->M,
 plugin_get_author_name:unsafe extern "C" fn(P)->M,
 plugin_get_value:unsafe extern "C" fn(P,P)->M,
 plugin_get_required_features:unsafe extern "C" fn(P)->M,
 plugin_get_optional_features:unsafe extern "C" fn(P)->M,
 plugin_get_num_ports:unsafe extern "C" fn(P)->u32,
 plugin_get_port_ranges_float:unsafe extern "C" fn(P,*mut f32,*mut f32,*mut f32),
 plugin_get_port_by_index:unsafe extern "C" fn(P,u32)->P,
 port_is_a:unsafe extern "C" fn(P,P,P)->bool,
 port_has_property:unsafe extern "C" fn(P,P,P)->bool,
 port_supports_event:unsafe extern "C" fn(P,P,P)->bool,
 port_get_name:unsafe extern "C" fn(P,P)->M,
 port_get_symbol:unsafe extern "C" fn(P,P)->P,
 port_get_value:unsafe extern "C" fn(P,P,P)->M,
 plugin_instantiate:unsafe extern "C" fn(P,f64,*const *const Feature)->*mut RawInstance,
 instance_free:unsafe extern "C" fn(*mut RawInstance),
}
