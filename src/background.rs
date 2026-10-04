//! Shared worker admission, cancellation and measured progress stay outside audio callbacks.
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc, Condvar, Mutex, Weak,
};
use std::time::Duration;

pub(crate) const MIB: u64 = 1024 * 1024;
pub(crate) const MEMORY_BYTES: u64 = 3 * 1024 * MIB;
const RECORDS: usize = 64;
const WORKERS: usize = 3;
const OPTIONAL_WORKERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Decode,
    Analysis,
    Index,
    Render,
    Download,
    Prepare,
}
impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Decode => "Audio decode",
            Self::Analysis => "Audio analysis",
            Self::Index => "Library indexing",
            Self::Render => "Rendering",
            Self::Download => "Provider request",
            Self::Prepare => "Media preparation",
        }
    }
    fn essential(self) -> bool {
        self == Self::Decode
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Phase {
    Queued,
    Running,
    Ready,
    Retired,
    Cancelled,
    Failed,
}
impl Phase {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Queued,
            1 => Self::Running,
            2 => Self::Ready,
            3 => Self::Retired,
            4 => Self::Cancelled,
            _ => Self::Failed,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Ready => "Worker finished",
            Self::Retired => "Request retired",
            Self::Cancelled => "Cancelled",
            Self::Failed => "Worker failed",
        }
    }
}
pub(crate) enum Cancellation {
    Flag(Arc<AtomicBool>),
    Generation { current: Arc<AtomicU64>, id: u64 },
}
impl Cancellation {
    fn cancelled(&self) -> bool {
        match self {
            Self::Flag(flag) => flag.load(Ordering::Acquire),
            Self::Generation { current, id } => current.load(Ordering::Acquire) != *id,
        }
    }
    fn cancel(&self) {
        match self {
            Self::Flag(flag) => flag.store(true, Ordering::Release),
            Self::Generation { current, id } => {
                if let Some(next) = id.checked_add(1) {
                    let _ =
                        current.compare_exchange(*id, next, Ordering::AcqRel, Ordering::Acquire);
                }
            }
        }
    }
}
struct Record {
    id: u64,
    kind: Kind,
    key: String,
    bytes: u64,
    phase: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    owner: AtomicBool,
    stop: Mutex<Option<Cancellation>>,
    nice: AtomicU8,
}
struct State {
    records: Vec<Arc<Record>>,
    next: u64,
    running: usize,
    optional: usize,
    bytes: u64,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
#[derive(Clone)]
pub(crate) struct Scheduler(Arc<Shared>);
impl Default for Scheduler {
    fn default() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                records: Vec::new(),
                next: 0,
                running: 0,
                optional: 0,
                bytes: 0,
            }),
            wake: Condvar::new(),
        }))
    }
}
struct Owner {
    scheduler: Scheduler,
    record: Arc<Record>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.record.stop.lock().unwrap().take();
        self.record.owner.store(false, Ordering::Release);
        if self.record.phase.load(Ordering::Acquire) == Phase::Queued as u8 {
            self.record
                .phase
                .store(Phase::Retired as u8, Ordering::Release);
        }
        self.scheduler.0.wake.notify_all();
    }
}
#[derive(Clone, Default)]
pub(crate) struct Reporter(Weak<Record>);
impl Reporter {
    /// Publish measured worker units without retaining its cancellation owner.
    /// Takes completed units and an optional known total; updates a still-retained job record only.
    pub fn progress(&self, done: u64, total: Option<u64>) {
        if let Some(r) = self.0.upgrade() {
            r.done.store(
                total.map_or(done, |total| done.min(total)),
                Ordering::Release,
            );
            r.total.store(total.unwrap_or(u64::MAX), Ordering::Release);
        }
    }
}
#[derive(Clone)]
pub(crate) struct Ticket(Arc<Owner>);
pub(crate) struct Running {
    owner: Arc<Owner>,
}
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub id: u64,
    pub kind: Kind,
    pub phase: Phase,
    pub done: u64,
    pub total: Option<u64>,
    pub bytes: u64,
    pub nice: Option<u8>,
    pub cancellable: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub rows: Vec<Row>,
    pub running: usize,
    pub reserved: u64,
}
impl Scheduler {
    /// Queue one identified worker request.
    /// Takes its kind, bounded identity, memory reservation and cancellation owner; refuses duplicates and exhausted capacity before work starts.
    pub fn request(
        &self,
        kind: Kind,
        key: String,
        bytes: u64,
        stop: Cancellation,
    ) -> Result<Ticket, String> {
        if key.is_empty() || key.len() > 4096 || bytes == 0 || bytes > MEMORY_BYTES {
            return Err("Background request exceeds its identity or memory budget".into());
        }
        let mut state = self.0.state.lock().unwrap();
        if state.records.iter().any(|r| {
            r.kind == kind
                && r.key == key
                && r.owner.load(Ordering::Acquire)
                && matches!(
                    Phase::from(r.phase.load(Ordering::Acquire)),
                    Phase::Queued | Phase::Running
                )
                && !cancelled(r)
        }) {
            return Err("An equivalent background request is already queued or running".into());
        }
        while state.records.len() >= RECORDS {
            let Some(index) = state
                .records
                .iter()
                .position(|r| !r.owner.load(Ordering::Acquire))
            else {
                return Err(
                    "Background request capacity is full; cancel or finish an existing job".into(),
                );
            };
            state.records.remove(index);
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or("Background request identity exhausted")?;
        let record = Arc::new(Record {
            id: state.next,
            kind,
            key,
            bytes,
            phase: AtomicU8::new(Phase::Queued as u8),
            done: AtomicU64::new(0),
            total: AtomicU64::new(u64::MAX),
            owner: AtomicBool::new(true),
            stop: Mutex::new(Some(stop)),
            nice: AtomicU8::new(u8::MAX),
        });
        if kind.essential()
            && (state.running >= WORKERS || state.bytes.saturating_add(bytes) > MEMORY_BYTES)
        {
            for r in &state.records {
                if !r.kind.essential()
                    && Phase::from(r.phase.load(Ordering::Acquire)) == Phase::Running
                {
                    if let Some(stop) = r.stop.lock().unwrap().as_ref() {
                        stop.cancel();
                    }
                }
            }
        }
        state.records.push(record.clone());
        Ok(Ticket(Arc::new(Owner {
            scheduler: self.clone(),
            record,
        })))
    }
    /// Read bounded job activity on the UI or worker.
    /// Takes this scheduler; returns progress and active declared reservations without consulting the audio renderer.
    pub fn snapshot(&self) -> Snapshot {
        let state = self.0.state.lock().unwrap();
        let rows = state
            .records
            .iter()
            .map(|r| {
                let total = r.total.load(Ordering::Acquire);
                let nice = r.nice.load(Ordering::Acquire);
                let phase = Phase::from(r.phase.load(Ordering::Acquire));
                Row {
                    id: r.id,
                    kind: r.kind,
                    phase: if cancelled(r) && r.owner.load(Ordering::Acquire) {
                        Phase::Cancelled
                    } else {
                        phase
                    },
                    done: r.done.load(Ordering::Acquire),
                    total: (total != u64::MAX).then_some(total),
                    bytes: r.bytes,
                    nice: (nice != u8::MAX).then_some(nice),
                    cancellable: r.owner.load(Ordering::Acquire)
                        && matches!(phase, Phase::Queued | Phase::Running),
                }
            })
            .collect();
        Snapshot {
            rows,
            running: state.running,
            reserved: state.bytes,
        }
    }
    /// Cancel the captured request only.
    /// Takes its stable identity; returns whether its worker is still queued/running and cancellation was requested.
    pub fn cancel(&self, id: u64) -> bool {
        let state = self.0.state.lock().unwrap();
        let Some(r) = state.records.iter().find(|r| r.id == id) else {
            return false;
        };
        if !r.owner.load(Ordering::Acquire)
            || !matches!(
                Phase::from(r.phase.load(Ordering::Acquire)),
                Phase::Queued | Phase::Running
            )
        {
            return false;
        }
        let stop = r.stop.lock().unwrap();
        let Some(stop) = stop.as_ref() else {
            return false;
        };
        stop.cancel();
        self.0.wake.notify_all();
        true
    }
}
fn cancelled(record: &Record) -> bool {
    record
        .stop
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(Cancellation::cancelled)
}
impl Ticket {
    /// Enter one worker turn without blocking a native frame or callback.
    /// Takes an additional source/protection check; waits cooperatively for priority, concurrency and memory admission, then applies native worker priorities.
    pub fn enter(&self, stop: impl Fn() -> bool) -> Result<Running, String> {
        let r = &self.0.record;
        let scheduler = &self.0.scheduler.0;
        let mut state = scheduler.state.lock().unwrap();
        loop {
            if stop() || cancelled(r) {
                r.phase.store(Phase::Cancelled as u8, Ordering::Release);
                return Err("Background request cancelled before its worker turn".into());
            }
            if Phase::from(r.phase.load(Ordering::Acquire)) != Phase::Queued {
                return Err("Background worker turn was already consumed".into());
            }
            let priority = if r.kind.essential() {
                true
            } else {
                !state.records.iter().any(|job| {
                    job.kind.essential()
                        && job.owner.load(Ordering::Acquire)
                        && Phase::from(job.phase.load(Ordering::Acquire)) == Phase::Queued
                        && !cancelled(job)
                })
            };

            if priority
                && state.running < WORKERS
                && (if r.kind.essential() {
                    state.running - state.optional < 1
                } else {
                    state.optional < OPTIONAL_WORKERS
                })
                && state.bytes.saturating_add(r.bytes) <= MEMORY_BYTES
            {
                state.running += 1;
                state.optional += usize::from(!r.kind.essential());
                state.bytes += r.bytes;
                r.phase.store(Phase::Running as u8, Ordering::Release);
                break;
            }
            state = scheduler
                .wake
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        drop(state);
        let running = Running {
            owner: self.0.clone(),
        };
        match worker_priority() {
            Ok(nice) => {
                r.nice.store(nice, Ordering::Release);
                Ok(running)
            }
            Err(error) => {
                r.phase.store(Phase::Failed as u8, Ordering::Release);
                drop(running);
                Err(error)
            }
        }
    }
    /// Share a progress sink with a bounded worker algorithm.
    /// Takes this ticket; returns a weak reporter which cannot extend a cancellation lease.
    pub fn reporter(&self) -> Reporter {
        Reporter(Arc::downgrade(&self.0.record))
    }
    /// Report measured work units.
    /// Takes completed units and an optional known total; publishes bounded progress without inventing a percentage for unknown totals.
    pub fn progress(&self, done: u64, total: Option<u64>) {
        self.0.record.done.store(
            total.map_or(done, |total| done.min(total)),
            Ordering::Release,
        );
        self.0
            .record
            .total
            .store(total.unwrap_or(u64::MAX), Ordering::Release);
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let r = &self.owner.record;
        let scheduler = &self.owner.scheduler.0;
        let mut state = scheduler.state.lock().unwrap();
        state.running -= 1;
        state.optional -= usize::from(!r.kind.essential());
        state.bytes -= r.bytes;
        if Phase::from(r.phase.load(Ordering::Acquire)) == Phase::Running {
            r.phase.store(
                if cancelled(r) {
                    Phase::Cancelled as u8
                } else {
                    Phase::Ready as u8
                },
                Ordering::Release,
            );
        }
        scheduler.wake.notify_all();
    }
}
/// Identify equivalent typed requests without retaining their private contents.
/// Takes a serializable operation; returns its SHA-256 identity or refuses an oversized request.
pub(crate) fn identity(value: &impl serde::Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("Background request identity exceeds 128 KiB".into());
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
/// Lower only the current owned worker thread's CPU and I/O priority.
/// Takes no arguments; returns the observed Linux nice value or an explicit unsupported/OS failure. The audio and GUI threads are untouched.
#[cfg(target_os = "linux")]
fn worker_priority() -> Result<u8, String> {
    unsafe {
        let parameters = libc::sched_param { sched_priority: 0 };
        if libc::sched_setscheduler(0, libc::SCHED_OTHER, &parameters) != 0 {
            return Err(format!(
                "Worker scheduling policy refused: {}",
                std::io::Error::last_os_error()
            ));
        }
        *libc::__errno_location() = 0;
        let old = libc::getpriority(libc::PRIO_PROCESS, 0);
        if *libc::__errno_location() != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if libc::setpriority(libc::PRIO_PROCESS, 0, old.max(10)) != 0
            || libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13) != 0
        {
            return Err(format!(
                "Worker priority refused: {}",
                std::io::Error::last_os_error()
            ));
        }
        let observed = libc::getpriority(libc::PRIO_PROCESS, 0);
        if observed < 10
            || libc::sched_getscheduler(0) != libc::SCHED_OTHER
            || libc::syscall(libc::SYS_ioprio_get, 1, 0) != (3 << 13)
        {
            return Err("Worker CPU/I/O priority did not match its policy".into());
        }
        Ok(observed as u8)
    }
}
#[cfg(not(target_os = "linux"))]
fn worker_priority() -> Result<u8, String> {
    Err("Background CPU/I/O priorities are qualified only on Linux".into())
}

#[cfg(test)]
mod tests;
