use super::*;
use clap_sys::ext::{gui::*, log::*, posix_fd_support::*, thread_check::*, timer_support::*};
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Mutex,
};
use std::time::{Duration, Instant};
pub(super) struct State {
    pub restart: AtomicBool,
    pub callback: AtomicBool,
    pub audio: AtomicBool,
    pub closed: AtomicBool,
    pub destroyed: AtomicBool,
    pub resize: AtomicU64,
    pub unsupported: Mutex<Vec<String>>,
    main: std::thread::ThreadId,
    timers: Mutex<Vec<(u32, Duration, Instant)>>,
    fds: Mutex<Vec<(i32, u32)>>,
    next: std::sync::atomic::AtomicU32,
}
impl Default for State {
    fn default() -> Self {
        Self {
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            audio: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            destroyed: AtomicBool::new(false),
            resize: AtomicU64::new(0),
            unsupported: Mutex::new(Vec::new()),
            main: std::thread::current().id(),
            timers: Mutex::new(Vec::new()),
            fds: Mutex::new(Vec::new()),
            next: std::sync::atomic::AtomicU32::new(1),
        }
    }
}
unsafe fn state<'a>(h: *const clap_host) -> &'a State {
    &*((*h).host_data as *const State)
}
pub(super) unsafe extern "C" fn restart(h: *const clap_host) {
    state(h).restart.store(true, Ordering::Release);
}
pub(super) unsafe extern "C" fn callback(h: *const clap_host) {
    state(h).callback.store(true, Ordering::Release);
}
pub(super) unsafe extern "C" fn process(_h: *const clap_host) {}
unsafe extern "C" fn audio(h: *const clap_host) -> bool {
    state(h).audio.load(Ordering::Acquire) && state(h).main == std::thread::current().id()
}
unsafe extern "C" fn main(h: *const clap_host) -> bool {
    state(h).main == std::thread::current().id()
}
unsafe extern "C" fn log(_h: *const clap_host, _severity: i32, _message: *const c_char) {}
unsafe extern "C" fn rescan(h: *const clap_host, _flags: u32) {
    restart(h);
}
unsafe extern "C" fn clear(h: *const clap_host, _id: u32, _flags: u32) {
    restart(h);
}
unsafe extern "C" fn flush(h: *const clap_host) {
    callback(h);
}
unsafe extern "C" fn dirty(_h: *const clap_host) {}
unsafe extern "C" fn resize(h: *const clap_host, w: u32, t: u32) -> bool {
    if w == 0 || t == 0 || w > 8192 || t > 8192 {
        return false;
    }
    state(h)
        .resize
        .store(((w as u64) << 32) | t as u64, Ordering::Release);
    true
}
unsafe extern "C" fn closed(h: *const clap_host, destroyed: bool) {
    state(h).destroyed.store(destroyed, Ordering::Release);
    state(h).closed.store(true, Ordering::Release);
}
unsafe extern "C" fn hints(_h: *const clap_host) {}
unsafe extern "C" fn show(_h: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn timer(h: *const clap_host, ms: u32, id: *mut u32) -> bool {
    if id.is_null() || ms == 0 {
        return false;
    }
    let Ok(mut v) = state(h).timers.lock() else {
        return false;
    };
    if v.len() >= 64 {
        return false;
    }
    let n = state(h).next.fetch_add(1, Ordering::Relaxed);
    if n == u32::MAX {
        return false;
    }
    let period = Duration::from_millis(ms as u64);
    v.push((n, period, Instant::now() + period));
    *id = n;
    true
}
unsafe extern "C" fn untimer(h: *const clap_host, id: u32) -> bool {
    let Ok(mut v) = state(h).timers.lock() else {
        return false;
    };
    let n = v.len();
    v.retain(|t| t.0 != id);
    v.len() != n
}
unsafe extern "C" fn fd(h: *const clap_host, n: i32, flags: u32) -> bool {
    if n < 0 || flags == 0 || flags & !7 != 0 {
        return false;
    }
    let Ok(mut v) = state(h).fds.lock() else {
        return false;
    };
    if v.len() >= 64 || v.iter().any(|v| v.0 == n) {
        return false;
    }
    v.push((n, flags));
    true
}
unsafe extern "C" fn modify(h: *const clap_host, n: i32, flags: u32) -> bool {
    if flags == 0 || flags & !7 != 0 {
        return false;
    }
    let Ok(mut v) = state(h).fds.lock() else {
        return false;
    };
    if let Some(v) = v.iter_mut().find(|v| v.0 == n) {
        v.1 = flags;
        true
    } else {
        false
    }
}
unsafe extern "C" fn unfd(h: *const clap_host, n: i32) -> bool {
    let Ok(mut v) = state(h).fds.lock() else {
        return false;
    };
    let before = v.len();
    v.retain(|v| v.0 != n);
    before != v.len()
}
static THREAD: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(main),
    is_audio_thread: Some(audio),
};
static LOG: clap_host_log = clap_host_log { log: Some(log) };
static PARAMS: clap_host_params = clap_host_params {
    rescan: Some(rescan),
    clear: Some(clear),
    request_flush: Some(flush),
};
static LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(dirty),
};
static TAIL: clap_host_tail = clap_host_tail {
    changed: Some(dirty),
};
static STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(dirty),
};
static GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(hints),
    request_resize: Some(resize),
    request_show: Some(show),
    request_hide: Some(show),
    closed: Some(closed),
};
static TIMER: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(timer),
    unregister_timer: Some(untimer),
};
static FD: clap_host_posix_fd_support = clap_host_posix_fd_support {
    register_fd: Some(fd),
    modify_fd: Some(modify),
    unregister_fd: Some(unfd),
};
/// Negotiate host extensions. Args: live host and identifier. Returns only implemented vtables and records unsupported requests for the browser.
pub(super) unsafe extern "C" fn extension(h: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_THREAD_CHECK {
        return (&THREAD as *const clap_host_thread_check).cast();
    }
    if id == CLAP_EXT_LOG {
        return (&LOG as *const clap_host_log).cast();
    }
    if id == CLAP_EXT_PARAMS {
        return (&PARAMS as *const clap_host_params).cast();
    }
    if id == CLAP_EXT_LATENCY {
        return (&LATENCY as *const clap_host_latency).cast();
    }
    if id == CLAP_EXT_TAIL {
        return (&TAIL as *const clap_host_tail).cast();
    }
    if id == CLAP_EXT_STATE {
        return (&STATE as *const clap_host_state).cast();
    }
    if id == CLAP_EXT_GUI {
        return (&GUI as *const clap_host_gui).cast();
    }
    if id == CLAP_EXT_TIMER_SUPPORT {
        return (&TIMER as *const clap_host_timer_support).cast();
    }
    if id == CLAP_EXT_POSIX_FD_SUPPORT {
        return (&FD as *const clap_host_posix_fd_support).cast();
    }
    if let Ok(mut v) = state(h).unsupported.lock() {
        let id = id.to_string_lossy();
        if id.len() <= 256 && v.len() < 64 && !v.iter().any(|v| v == id.as_ref()) {
            v.push(id.into());
        }
    }
    ptr::null()
}
/// Service bounded main-thread callbacks. Args: host state and live plugin. Returns after timers and ready descriptors have been delivered without retaining locks across plugin calls.
pub(super) fn service(state: &State, plugin: *const clap_plugin) -> Result<(), String> {
    let due = {
        let mut timers = state.timers.lock().map_err(|e| e.to_string())?;
        let now = Instant::now();
        timers
            .iter_mut()
            .filter_map(|(id, period, next)| {
                if now >= *next {
                    *next = now + *period;
                    Some(*id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    unsafe {
        let extension = (*plugin)
            .get_extension
            .ok_or("Missing CLAP extension accessor")?;
        if let Some(t) = extension(plugin, CLAP_EXT_TIMER_SUPPORT.as_ptr())
            .cast::<clap_plugin_timer_support>()
            .as_ref()
        {
            if let Some(f) = t.on_timer {
                for id in due {
                    f(plugin, id);
                }
            }
        }
        let fds = state.fds.lock().map_err(|e| e.to_string())?.clone();
        if let Some(p) = extension(plugin, CLAP_EXT_POSIX_FD_SUPPORT.as_ptr())
            .cast::<clap_plugin_posix_fd_support>()
            .as_ref()
        {
            if let Some(f) = p.on_fd {
                for (fd, flags) in fds {
                    let mut poll = libc::pollfd {
                        fd,
                        events: (if flags & 1 != 0 { libc::POLLIN } else { 0 })
                            | (if flags & 2 != 0 { libc::POLLOUT } else { 0 }),
                        revents: 0,
                    };
                    if libc::poll(&mut poll, 1, 0) > 0 {
                        let flags = (if poll.revents & libc::POLLIN != 0 {
                            1
                        } else {
                            0
                        }) | (if poll.revents & libc::POLLOUT != 0 {
                            2
                        } else {
                            0
                        }) | (if poll.revents
                            & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)
                            != 0
                        {
                            4
                        } else {
                            0
                        });
                        if flags != 0 {
                            f(plugin, fd, flags);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
