use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    OnceLock,
};

struct Slot {
    tid: AtomicI32,
    exceeded: AtomicBool,
}

const SLOTS: usize = 16;
static CALLBACKS: [Slot; SLOTS] = [const {
    Slot {
        tid: AtomicI32::new(0),
        exceeded: AtomicBool::new(false),
    }
}; SLOTS];
static PREVIOUS_HANDLER: AtomicUsize = AtomicUsize::new(libc::SIG_DFL);
static PREVIOUS_FLAGS: AtomicI32 = AtomicI32::new(0);
static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

/// Retain a project when Linux exhausts an owned audio thread's CPU budget.
/// Takes no arguments; installs one process handler before opening native streams, or returns the OS refusal.
pub(super) fn install() -> Result<(), String> {
    INSTALLED
        .get_or_init(|| {
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            if unsafe { libc::sigaction(libc::SIGXCPU, std::ptr::null(), &mut previous) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            PREVIOUS_HANDLER.store(previous.sa_sigaction, Ordering::Release);
            PREVIOUS_FLAGS.store(previous.sa_flags, Ordering::Release);
            previous.sa_sigaction = exhausted as *const () as usize;
            previous.sa_flags = (previous.sa_flags | libc::SA_SIGINFO) & !libc::SA_RESETHAND;
            if unsafe { libc::sigaction(libc::SIGXCPU, &previous, std::ptr::null_mut()) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(())
        })
        .clone()
}

/// Stop owned real-time callbacks after Linux's process-wide watchdog signal.
/// Takes the kernel signal information; demotes registered FIFO/RR callbacks or forwards unrelated signals to their original handler.
extern "C" fn exhausted(signal: i32, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
    let errno = Errno::save();
    #[cfg(test)]
    tests::SIGNAL_TID.store(
        unsafe { libc::syscall(libc::SYS_gettid) } as i32,
        Ordering::Release,
    );
    if !info.is_null() && unsafe { (*info).si_code } == libc::SI_KERNEL && realtime_limit_raised() {
        let mut recovered = false;
        for slot in &CALLBACKS {
            let tid = slot.tid.load(Ordering::Acquire);
            if tid <= 0 {
                continue;
            }
            let policy = unsafe { libc::syscall(libc::SYS_sched_getscheduler, tid) } as i32;
            if !matches!(
                policy & !libc::SCHED_RESET_ON_FORK,
                libc::SCHED_FIFO | libc::SCHED_RR
            ) {
                continue;
            }
            let priority = libc::sched_param { sched_priority: 0 };
            if slot.tid.load(Ordering::Acquire) == tid
                && unsafe {
                    libc::syscall(
                        libc::SYS_sched_setscheduler,
                        tid,
                        libc::SCHED_OTHER | (policy & libc::SCHED_RESET_ON_FORK),
                        &priority,
                    )
                } == 0
            {
                slot.exceeded.store(true, Ordering::Release);
                recovered = true;
            }
        }
        if recovered {
            return;
        }
    }
    let handler = PREVIOUS_HANDLER.load(Ordering::Acquire);
    if handler == libc::SIG_IGN {
        return;
    }
    if handler == libc::SIG_DFL {
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        return;
    }
    let flags = PREVIOUS_FLAGS.load(Ordering::Acquire);
    errno.restore();
    if flags & libc::SA_RESETHAND != 0 {
        PREVIOUS_HANDLER.store(libc::SIG_DFL, Ordering::Release);
    }
    if flags & libc::SA_SIGINFO != 0 {
        let previous: extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void) =
            unsafe { std::mem::transmute(handler) };
        previous(signal, info, context);
    } else {
        let previous: extern "C" fn(i32) = unsafe { std::mem::transmute(handler) };
        previous(signal);
    }
}

/// Recognize the process-wide real-time watchdog notification.
/// Takes no arguments; returns true only with unlimited ordinary CPU time and the kernel-raised finite RT limit.
fn realtime_limit_raised() -> bool {
    let mut cpu: libc::rlimit = unsafe { std::mem::zeroed() };
    let mut realtime: libc::rlimit = unsafe { std::mem::zeroed() };
    for (resource, limit) in [
        (libc::RLIMIT_CPU, &mut cpu),
        (libc::RLIMIT_RTTIME, &mut realtime),
    ] {
        if unsafe {
            libc::syscall(
                libc::SYS_prlimit64,
                0,
                resource,
                std::ptr::null::<libc::rlimit>(),
                limit,
            )
        } != 0
        {
            return false;
        }
    }
    cpu.rlim_cur == libc::RLIM_INFINITY
        && realtime.rlim_max != libc::RLIM_INFINITY
        && realtime.rlim_cur > realtime.rlim_max
}

struct Errno {
    address: *mut i32,
    value: i32,
}
impl Errno {
    /// Preserve the interrupted thread's errno.
    /// Takes no arguments; returns its thread-local errno address and original value without allocation.
    fn save() -> Self {
        let address = unsafe { libc::__errno_location() };
        Self {
            address,
            value: unsafe { *address },
        }
    }
    /// Restore the interrupted thread's errno.
    /// Takes this marker; writes the original value before forwarding a foreign signal or returning.
    fn restore(&self) {
        unsafe {
            *self.address = self.value;
        }
    }
}
impl Drop for Errno {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(super) struct Guard {
    slot: Option<usize>,
    registered: bool,
}
impl Guard {
    /// Prepare bounded callback registration.
    /// Takes no arguments; returns an unregistered guard without allocation or OS changes.
    pub(super) fn new() -> Self {
        Self {
            slot: None,
            registered: false,
        }
    }

    /// Check the current callback's CPU recovery state.
    /// Takes this callback-owned guard; registers its native thread once and returns true after exhaustion or capacity refusal.
    pub(super) fn exceeded(&mut self) -> bool {
        if !self.registered {
            self.registered = true;
            for (index, slot) in CALLBACKS.iter().enumerate() {
                if slot
                    .tid
                    .compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    continue;
                }
                slot.exceeded.store(false, Ordering::Relaxed);
                slot.tid.store(
                    unsafe { libc::syscall(libc::SYS_gettid) } as i32,
                    Ordering::Release,
                );
                self.slot = Some(index);
                break;
            }
        }
        self.slot
            .is_none_or(|index| CALLBACKS[index].exceeded.load(Ordering::Acquire))
    }

    /// Distinguish kernel CPU exhaustion from registration capacity refusal.
    /// Takes this guard; returns whether a registered native callback exhausted its CPU limit.
    pub(super) fn cpu_exhausted(&self) -> bool {
        self.slot
            .is_some_and(|index| CALLBACKS[index].exceeded.load(Ordering::Acquire))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(index) = self.slot {
            CALLBACKS[index].tid.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
