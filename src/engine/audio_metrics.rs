//! Callback measurements use fixed atomics. Only the audio callback writes the
//! last-completed sample; GUI/IPC readers make one bounded coherence check.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallbackMeasurement {
    pub elapsed_ns: u64,
    pub budget_ns: u64,
    pub render_cpu_ns: Option<u64>,
    pub overrun_ns: u64,
    pub output_latency_ns: Option<u64>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioMetrics {
    /// None before the first callback or if this bounded read overlaps a write.
    pub last_callback: Option<CallbackMeasurement>,
    #[serde(default)]
    pub max_callback: Option<CallbackMeasurement>,
    pub callbacks: u64,
    pub deadline_overruns: u64,
    pub max_elapsed_ns: u64,
    pub max_overrun_ns: u64,
    pub backend_errors: u64,
    pub device_lost: u64,
    #[serde(default)]
    pub xruns: u64,
    #[serde(default)]
    pub realtime_denied: u64,
    #[serde(default)]
    pub cpu_budget_exhaustions: u64,
    /// Backend underrun events do not reveal an exact dropped-buffer count.
    pub dropped_buffers: Option<u64>,
}

#[derive(Default)]
pub(super) struct Telemetry {
    pub profiler: super::diagnostics::Profiler,
    last: Measurement,
    longest: Measurement,
    sequence: AtomicU64,
    callbacks: AtomicU64,
    overruns: AtomicU64,
    max_elapsed: AtomicU64,
    max_overrun: AtomicU64,
    backend_errors: AtomicU64,
    device_lost: AtomicU64,
    xruns: AtomicU64,
    realtime_denied: AtomicU64,
    cpu_budget_exhaustions: AtomicU64,
}

#[derive(Default)]
struct Measurement {
    latency: AtomicU64,
    sample_rate: AtomicU64,
    channels: AtomicU64,
    frames: AtomicU64,
    elapsed: AtomicU64,
    budget: AtomicU64,
    cpu: AtomicU64,
    overrun: AtomicU64,
}
impl Measurement {
    fn store(&self, sample: CallbackMeasurement) {
        self.latency.store(sample.output_latency_ns.unwrap_or(u64::MAX), Ordering::Relaxed);
        self.sample_rate.store(u64::from(sample.sample_rate), Ordering::Relaxed);
        self.channels.store(u64::from(sample.channels), Ordering::Relaxed);
        self.frames.store(sample.frames as u64, Ordering::Relaxed);
        self.elapsed.store(sample.elapsed_ns, Ordering::Relaxed);
        self.budget.store(sample.budget_ns, Ordering::Relaxed);
        self.cpu.store(sample.render_cpu_ns.unwrap_or(u64::MAX), Ordering::Relaxed);
        self.overrun.store(sample.overrun_ns, Ordering::Relaxed);
    }
    fn read(&self) -> CallbackMeasurement {
        let cpu = self.cpu.load(Ordering::Relaxed);
        let latency = self.latency.load(Ordering::Relaxed);
        CallbackMeasurement {
            output_latency_ns: (latency != u64::MAX).then_some(latency),
            sample_rate: self.sample_rate.load(Ordering::Relaxed) as u32,
            channels: self.channels.load(Ordering::Relaxed) as u16,
            frames: self.frames.load(Ordering::Relaxed) as usize,
            elapsed_ns: self.elapsed.load(Ordering::Relaxed),
            budget_ns: self.budget.load(Ordering::Relaxed),
            render_cpu_ns: (cpu != u64::MAX).then_some(cpu),
            overrun_ns: self.overrun.load(Ordering::Relaxed),
        }
    }
}

pub(super) fn nanoseconds(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

impl Telemetry {
    pub fn record(&self, elapsed: Duration, frames: usize, sr: u32, cpu: Option<u64>) {
        self.record_output(elapsed, frames, sr, cpu, 0, None);
    }

    pub fn record_output(
        &self,
        elapsed: Duration,
        frames: usize,
        sr: u32,
        cpu: Option<u64>,
        channels: u16,
        latency: Option<Duration>,
    ) {
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
        let sample = CallbackMeasurement { elapsed_ns: elapsed, budget_ns: budget, render_cpu_ns: cpu,
            overrun_ns: overrun, output_latency_ns: latency.map(nanoseconds), sample_rate: sr, channels, frames };
        self.last.store(sample);
        let first = self.callbacks.fetch_add(1, Ordering::Relaxed) == 0;
        if overrun > 0 {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
        if first || elapsed > self.max_elapsed.load(Ordering::Relaxed) {
            self.longest.store(sample);
            self.max_elapsed.store(elapsed, Ordering::Relaxed);
        }
        self.max_overrun.fetch_max(overrun, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Count a backend event without allocation.
    /// Takes its classified error; returns whether the output must be retired.
    pub fn error(&self, error: &cpal::Error) -> bool {
        match error.kind() {
            cpal::ErrorKind::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            cpal::ErrorKind::RealtimeDenied => {
                self.realtime_denied.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            _ => {}
        }
        self.backend_errors.fetch_add(1, Ordering::Relaxed);
        if error.kind() == cpal::ErrorKind::DeviceNotAvailable {
            self.device_lost.fetch_add(1, Ordering::Relaxed);
        }
        true
    }

    /// Record a kernel CPU budget breach on the output callback.
    /// Takes this telemetry; increments the retained breach counter without allocation or logging.
    pub fn cpu_budget_exhausted(&self) { self.cpu_budget_exhaustions.fetch_add(1, Ordering::Relaxed); }

    pub fn read(&self) -> AudioMetrics {
        let before = self.sequence.load(Ordering::Acquire);
        let last = self.last.read();
        let longest = self.longest.read();
        // Acquire fence keeps the data reads before the validation read.
        std::sync::atomic::fence(Ordering::Acquire);
        let after = self.sequence.load(Ordering::Relaxed);
        AudioMetrics {
            last_callback: (before != 0 && before == after && before & 1 == 0).then_some(last),
            max_callback: (before != 0 && before == after && before & 1 == 0).then_some(longest),
            callbacks: self.callbacks.load(Ordering::Relaxed),
            deadline_overruns: self.overruns.load(Ordering::Relaxed),
            max_elapsed_ns: self.max_elapsed.load(Ordering::Relaxed),
            max_overrun_ns: self.max_overrun.load(Ordering::Relaxed),
            backend_errors: self.backend_errors.load(Ordering::Relaxed),
            device_lost: self.device_lost.load(Ordering::Relaxed),
            xruns: self.xruns.load(Ordering::Relaxed),
            realtime_denied: self.realtime_denied.load(Ordering::Relaxed),
            cpu_budget_exhaustions: self.cpu_budget_exhaustions.load(Ordering::Relaxed),
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
