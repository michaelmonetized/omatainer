/// Create native workers without changing unrelated threads.
/// Takes a synchronous constructor; returns its result with the caller restored. Linux capacity data selects only CPUs already allowed to this thread.
pub(super) fn open<T>(build: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "linux")]
    {
        linux::open(build)
    }
    #[cfg(not(target_os = "linux"))]
    {
        build()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read;

    fn capacity(cpu: usize) -> Option<u32> {
        let mut value = String::new();
        std::fs::File::open(format!("/sys/devices/system/cpu/cpu{cpu}/cpu_capacity"))
            .ok()?
            .take(32)
            .read_to_string(&mut value)
            .ok()?;
        value.trim().parse::<u32>().ok().filter(|value| *value > 0)
    }

    fn preferred(
        allowed: &libc::cpu_set_t,
        capacity: impl Fn(usize) -> Option<u32>,
    ) -> Option<libc::cpu_set_t> {
        let mut values = [0u32; libc::CPU_SETSIZE as usize];
        let mut minimum = u32::MAX;
        let mut maximum = 0;
        for (cpu, value) in values.iter_mut().enumerate() {
            if !unsafe { libc::CPU_ISSET(cpu, allowed) } {
                continue;
            }
            *value = capacity(cpu).filter(|value| *value > 0)?;
            minimum = minimum.min(*value);
            maximum = maximum.max(*value);
        }
        if minimum >= maximum {
            return None;
        }
        let mut selected = unsafe { std::mem::zeroed() };
        for (cpu, value) in values.iter().enumerate() {
            if *value == maximum {
                unsafe {
                    libc::CPU_SET(cpu, &mut selected);
                }
            }
        }
        Some(selected)
    }

    fn current() -> Option<libc::cpu_set_t> {
        let mut mask = unsafe { std::mem::zeroed() };
        (unsafe { libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut mask) }
            == 0)
            .then_some(mask)
    }

    struct Restore(Option<libc::cpu_set_t>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(original) = &self.0 {
                if unsafe {
                    libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), original)
                } != 0
                {
                    eprintln!(
                        "Audio worker affinity restoration failed: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    }

    /// Create native audio workers on the fastest allowed CPU class.
    /// Takes a synchronous stream constructor; returns its result and restores the caller's affinity, including on failure or panic. Missing capacity data preserves the existing scheduler policy.
    pub(super) fn open<T>(build: impl FnOnce() -> T) -> T {
        let original = current();
        let selected = original.as_ref().and_then(|mask| preferred(mask, capacity));
        let changed = selected.as_ref().is_some_and(|mask|
        unsafe { libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), mask) } == 0);
        if selected.is_some() && !changed {
            eprintln!(
                "Audio worker affinity request failed: {}",
                std::io::Error::last_os_error()
            );
        }
        let _restore = Restore(if changed { original } else { None });
        build()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn mask(cpus: &[usize]) -> libc::cpu_set_t {
            let mut result = unsafe { std::mem::zeroed() };
            for cpu in cpus {
                unsafe {
                    libc::CPU_SET(*cpu, &mut result);
                }
            }
            result
        }
        fn cpus(mask: &libc::cpu_set_t) -> Vec<usize> {
            (0..libc::CPU_SETSIZE as usize)
                .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, mask) })
                .collect()
        }
        #[test]
        fn heterogeneous_capacity_respects_restricted_masks_and_unknown_data() {
            let capacity = |cpu| Some(if cpu < 2 { 485 } else { 1024 });
            assert_eq!(
                cpus(&preferred(&mask(&[0, 1, 2, 5]), capacity).unwrap()),
                [2, 5]
            );
            assert!(preferred(&mask(&[0, 1]), capacity).is_none());
            assert!(preferred(&mask(&[2, 5]), capacity).is_none());
            assert!(preferred(&mask(&[]), capacity).is_none());
            assert!(preferred(&mask(&[0, 2]), |cpu| (cpu == 0).then_some(485)).is_none());
            assert!(preferred(&mask(&[0, 2]), |_| Some(0)).is_none());
        }
        #[test]
        fn worker_inherits_selection_and_parent_restores_after_success_and_unwind() {
            let original = current().unwrap();
            let expected = preferred(&original, capacity).unwrap_or(original);
            let worker = open(|| std::thread::spawn(|| current().unwrap()));
            assert_eq!(cpus(&worker.join().unwrap()), cpus(&expected));
            assert_eq!(cpus(&current().unwrap()), cpus(&original));
            let failure =
                std::panic::catch_unwind(|| open(|| panic!("Private constructor failure")));
            assert!(failure.is_err());
            assert_eq!(cpus(&current().unwrap()), cpus(&original));
        }
    }
}
