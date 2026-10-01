//! Callback measurements use fixed atomics. Only the audio callback writes the
//! last-completed sample; GUI/IPC readers make one bounded coherence check.
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CallbackMeasurement {
    pub elapsed_ns: u64,
    pub budget_ns: u64,
    pub render_cpu_ns: Option<u64>,
    pub overrun_ns: u64,
}
impl CallbackMeasurement {
    pub fn render_cpu_fraction(self) -> Option<f64> {
        if self.budget_ns == 0 {
            return None;
        }
        self.render_cpu_ns
            .map(|ns| ns as f64 / self.budget_ns.max(1) as f64)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct AudioMetrics {
    /// None before the first callback or if this bounded read overlaps a write.
    pub last_callback: Option<CallbackMeasurement>,
    pub callbacks: u64,
    pub deadline_overruns: u64,
    pub max_elapsed_ns: u64,
    pub max_overrun_ns: u64,
    pub backend_errors: u64,
    pub device_lost: u64,
    /// CPAL 0.15 does not expose an exact backend dropped-buffer count.
    pub dropped_buffers: Option<u64>,
}

#[derive(Default)]
pub(super) struct Telemetry {
    sequence: AtomicU64,
    elapsed: AtomicU64,
    budget: AtomicU64,
    cpu: AtomicU64,
    overrun: AtomicU64,
    callbacks: AtomicU64,
    overruns: AtomicU64,
    max_elapsed: AtomicU64,
    max_overrun: AtomicU64,
    backend_errors: AtomicU64,
    device_lost: AtomicU64,
}

pub(super) fn nanoseconds(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

impl Telemetry {
    pub fn record(&self, elapsed: Duration, frames: usize, sr: u32, cpu: Option<u64>) {
        let elapsed = nanoseconds(elapsed);
        let budget =
            ((frames as u128 * 1_000_000_000) / sr.max(1) as u128).min(u64::MAX as u128) as u64;
        let overrun = if frames == 0 {
            0
        } else {
            elapsed.saturating_sub(budget)
        };
        // One writer, no lock or retry loop. Readers cannot mistake a partial
        // update for a completed callback; missed reads return no sample.
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.elapsed.store(elapsed, Ordering::Relaxed);
        self.budget.store(budget, Ordering::Relaxed);
        self.cpu.store(cpu.unwrap_or(u64::MAX), Ordering::Relaxed);
        self.overrun.store(overrun, Ordering::Relaxed);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        if overrun > 0 {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
        self.max_elapsed.fetch_max(elapsed, Ordering::Relaxed);
        self.max_overrun.fetch_max(overrun, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    pub fn error(&self, error: &cpal::StreamError) {
        self.backend_errors.fetch_add(1, Ordering::Relaxed);
        if matches!(error, cpal::StreamError::DeviceNotAvailable) {
            self.device_lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn read(&self) -> AudioMetrics {
        let before = self.sequence.load(Ordering::Acquire);
        let cpu = self.cpu.load(Ordering::Relaxed);
        let last = CallbackMeasurement {
            elapsed_ns: self.elapsed.load(Ordering::Relaxed),
            budget_ns: self.budget.load(Ordering::Relaxed),
            render_cpu_ns: (cpu != u64::MAX).then_some(cpu),
            overrun_ns: self.overrun.load(Ordering::Relaxed),
        };
        // Acquire fence keeps the data reads before the validation read.
        std::sync::atomic::fence(Ordering::Acquire);
        let after = self.sequence.load(Ordering::Relaxed);
        AudioMetrics {
            last_callback: (before != 0 && before == after && before & 1 == 0).then_some(last),
            callbacks: self.callbacks.load(Ordering::Relaxed),
            deadline_overruns: self.overruns.load(Ordering::Relaxed),
            max_elapsed_ns: self.max_elapsed.load(Ordering::Relaxed),
            max_overrun_ns: self.max_overrun.load(Ordering::Relaxed),
            backend_errors: self.backend_errors.load(Ordering::Relaxed),
            device_lost: self.device_lost.load(Ordering::Relaxed),
            dropped_buffers: None,
        }
    }
}

/// Actual CPU consumed by this rendering thread, including user/system CPU.
/// This is deliberately not an Instant/wall-time fallback on failure.
pub(super) fn thread_cpu_ns() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one valid timespec to the supplied pointer.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    if result != 0 || time.tv_sec < 0 || !(0..1_000_000_000).contains(&time.tv_nsec) {
        return None;
    }
    (time.tv_sec as u64)
        .checked_mul(1_000_000_000)?
        .checked_add(time.tv_nsec as u64)
}
