use super::*;
use clap_sys::ext::gui::*;
use x11_dl::xlib;
pub(super) struct Editor {
    x: Option<xlib::Xlib>,
    display: *mut xlib::Display,
    window: libc::c_ulong,
    delete: libc::c_ulong,
    floating: bool,
}
impl Editor {
    /// Open a native CLAP editor. Args: initialized plugin. Returns a plugin-owned floating editor or an X11 parent with a window-manager close route.
    pub fn open(plugin: *const clap_plugin) -> Result<Self, String> {
        unsafe {
            let gui = gui(plugin)?;
            let supported = gui
                .is_api_supported
                .ok_or("CLAP GUI API negotiation is unavailable")?;
            let floating = supported(plugin, ptr::null(), true);
            if !floating && !supported(plugin, CLAP_WINDOW_API_X11.as_ptr(), false) {
                return Err("CLAP editor requires an unsupported window API; use the generic parameter controls".into());
            }
            if !gui.create.ok_or("Missing CLAP editor create")?(
                plugin,
                if floating {
                    ptr::null()
                } else {
                    CLAP_WINDOW_API_X11.as_ptr()
                },
                floating,
            ) {
                return Err("CLAP editor creation refused".into());
            }
            let mut result = Self {
                x: None,
                display: ptr::null_mut(),
                window: 0,
                delete: 0,
                floating,
            };
            let open = (|| {
                if !floating {
                    let x = xlib::Xlib::open().map_err(|e| e.to_string())?;
                    let display = (x.XOpenDisplay)(ptr::null());
                    if display.is_null() {
                        return Err(
                            "X11 display unavailable; use generic parameter controls".into()
                        );
                    }
                    result.display = display;
                    result.x = Some(x);
                    let x = result.x.as_ref().unwrap();
                    let (mut w, mut h) = (0, 0);
                    if !gui.get_size.ok_or("Missing CLAP editor dimensions")?(
                        plugin, &mut w, &mut h,
                    ) || w == 0
                        || h == 0
                        || w > 8192
                        || h > 8192
                    {
                        return Err("CLAP editor dimensions are invalid".into());
                    }
                    result.window = (x.XCreateSimpleWindow)(
                        display,
                        (x.XDefaultRootWindow)(display),
                        0,
                        0,
                        w,
                        h,
                        0,
                        0,
                        0,
                    );
                    if result.window == 0 {
                        return Err("X11 editor parent creation failed".into());
                    }
                    (x.XStoreName)(display, result.window, c"Omatainer plugin".as_ptr());
                    (x.XSelectInput)(display, result.window, xlib::StructureNotifyMask);
                    result.delete = (x.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
                    let mut atom = result.delete;
                    (x.XSetWMProtocols)(display, result.window, &mut atom, 1);
                    let parent = clap_window {
                        api: CLAP_WINDOW_API_X11.as_ptr(),
                        specific: clap_window_handle { x11: result.window },
                    };
                    if !gui
                        .set_parent
                        .ok_or("CLAP editor cannot accept an X11 parent")?(
                        plugin, &parent
                    ) {
                        return Err("CLAP editor refused the X11 parent".into());
                    }
                    (x.XMapWindow)(display, result.window);
                    (x.XFlush)(display);
                }
                if !gui.show.ok_or("Missing CLAP editor show")?(plugin) {
                    return Err("CLAP editor could not be shown".into());
                }
                Ok(())
            })();
            if let Err(error) = open {
                result.close(plugin);
                return Err(error);
            }
            Ok(result)
        }
    }
    /// Pump the owned editor. Args: plugin and callbacks. Returns true when the user or plugin closes it.
    pub fn service(
        &mut self,
        plugin: *const clap_plugin,
        state: &callbacks::State,
    ) -> Result<bool, String> {
        if state.closed.swap(false, Ordering::AcqRel) {
            return Ok(true);
        }
        unsafe {
            let gui = gui(plugin)?;
            let resize = state.resize.swap(0, Ordering::AcqRel);
            if resize != 0 {
                let w = (resize >> 32) as u32;
                let h = resize as u32;
                if let Some(x) = &self.x {
                    (x.XResizeWindow)(self.display, self.window, w, h);
                    (x.XFlush)(self.display);
                }
            }
            if let Some(x) = &self.x {
                for _ in 0..128 {
                    if (x.XPending)(self.display) == 0 {
                        break;
                    }
                    let mut event: xlib::XEvent = std::mem::zeroed();
                    (x.XNextEvent)(self.display, &mut event);
                    if event.get_type() == xlib::ClientMessage
                        && event.client_message.data.get_long(0) as libc::c_ulong == self.delete
                    {
                        return Ok(true);
                    }
                    if event.get_type() == xlib::ConfigureNotify {
                        let e = event.configure;
                        if let Some(size) = gui.set_size {
                            if e.width > 0 && e.height > 0 {
                                size(plugin, e.width as u32, e.height as u32);
                            }
                        }
                    }
                }
            }
        }
        Ok(false)
    }
    /// Destroy an owned editor once. Args: plugin. Returns after native resources and the parent display have been released.
    pub fn close(&mut self, plugin: *const clap_plugin) {
        unsafe {
            if let Ok(gui) = gui(plugin) {
                if let Some(hide) = gui.hide {
                    hide(plugin);
                }
                if let Some(destroy) = gui.destroy {
                    destroy(plugin);
                }
            }
            if let Some(x) = self.x.take() {
                if self.window != 0 {
                    (x.XDestroyWindow)(self.display, self.window);
                }
                if !self.display.is_null() {
                    (x.XCloseDisplay)(self.display);
                }
            }
        }
        self.window = 0;
        let _ = self.floating;
    }
}
unsafe fn gui<'a>(plugin: *const clap_plugin) -> Result<&'a clap_plugin_gui, String> {
    let get = (*plugin)
        .get_extension
        .ok_or("Missing CLAP extension accessor")?;
    get(plugin, CLAP_EXT_GUI.as_ptr())
        .cast::<clap_plugin_gui>()
        .as_ref()
        .ok_or_else(|| "CLAP native editor unavailable; use generic parameter controls".into())
}
