//! One bounded off-callback owner for resident sampler assets.
//!
//! Renderers retain a client. Banks, settings, and PCM each have an independent
//! worker pin: a voice, Undo entry, or project capture may outlive its bank.
//! The callback only clones/drops nonfinal Arcs; it never visits this registry.
//! A process-local weak registry lets a newly prepared graph join the same
//! owner while an older capture survives, avoiding mutually pinning pools.
use super::resident::{Data, Settings};
use crate::engine::dsp::Sample;
use std::{
    ops::Deref,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak,
    },
    time::Duration,
};

const INTERVAL: Duration = Duration::from_millis(20);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Budget {
    pub banks: usize,
    pub settings: usize,
    pub samples: usize,
    pub pcm_bytes: u64,
    pub metadata_bytes: u64,
}
impl Budget {
    pub fn limits() -> Self {
        Self {
            banks: 1024,
            settings: 1024,
            samples: 512,
            pcm_bytes: crate::project_file::DEFAULT_PCM_LIMIT,
            metadata_bytes: crate::project_file::DEFAULT_METADATA_LIMIT as u64,
        }
    }
    fn add(self, other: Self) -> Self {
        Self {
            banks: self.banks.saturating_add(other.banks),
            settings: self.settings.saturating_add(other.settings),
            samples: self.samples.saturating_add(other.samples),
            pcm_bytes: self.pcm_bytes.saturating_add(other.pcm_bytes),
            metadata_bytes: self.metadata_bytes.saturating_add(other.metadata_bytes),
        }
    }
    fn subtract(self, other: Self) -> Self {
        Self {
            banks: self.banks - other.banks,
            settings: self.settings - other.settings,
            samples: self.samples - other.samples,
            pcm_bytes: self.pcm_bytes - other.pcm_bytes,
            metadata_bytes: self.metadata_bytes - other.metadata_bytes,
        }
    }
    fn fits(self, limit: Self) -> bool {
        self.banks <= limit.banks
            && self.settings <= limit.settings
            && self.samples <= limit.samples
            && self.pcm_bytes <= limit.pcm_bytes
            && self.metadata_bytes <= limit.metadata_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Full,
    Poisoned,
    Closed,
    Reservation,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "sampler asset capacity is full; release unused banks/history or retry after retirement",
            Self::Poisoned => "sampler asset worker failed; new preparation is disabled until the application restarts",
            Self::Closed => "sampler asset owner is closed",
            Self::Reservation => "sampler preparation exceeded its reserved asset budget",
        })
    }
}

#[derive(Default)]
struct Health {
    poisoned: AtomicBool,
    alive: AtomicBool,
}
#[derive(Default)]
struct Registry {
    owner: Weak<Shared>,
    health: Option<Arc<Health>>,
}
impl Registry {
    fn acquire(&mut self, limits: Budget) -> Result<Owner, Error> {
        // Latch poison even after all assets drain. Restart, not a silent new
        // owner, is the recovery policy for an unexpected worker failure.
        if self
            .health
            .as_ref()
            .is_some_and(|h| h.poisoned.load(Ordering::Acquire))
        {
            return Err(Error::Poisoned);
        }
        if let Some(shared) = self.owner.upgrade() {
            let state = shared.state();
            if !state.sealed {
                shared.check()?;
                shared.clients.fetch_add(1, Ordering::Relaxed);
                drop(state);
                return Ok(Owner(Arc::new(Client { shared })));
            }
        }
        let shared = Arc::new(Shared {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            limits,
            health: Arc::new(Health::default()),
            clients: AtomicUsize::new(1),
            state: Mutex::new(State {
                sealed: false,
                reserved: Budget::default(),
                banks: Vec::with_capacity(limits.banks),
                settings: Vec::with_capacity(limits.settings),
                samples: Vec::with_capacity(limits.samples),
                factories: Vec::with_capacity(96),
            }),
            wake: Condvar::new(),
            #[cfg(test)]
            panic_next: AtomicBool::new(false),
        });
        shared.health.alive.store(true, Ordering::Release);
        let worker = shared.clone();
        if std::thread::Builder::new()
            .name("omat-sampler-assets".into())
            .spawn(move || worker.run())
            .is_err()
        {
            shared.health.alive.store(false, Ordering::Release);
            return Err(Error::Closed);
        }
        self.owner = Arc::downgrade(&shared);
        self.health = Some(shared.health.clone());
        Ok(Owner(Arc::new(Client { shared })))
    }
}

struct State {
    sealed: bool,
    reserved: Budget,
    banks: Vec<(Arc<Data>, u64)>,
    settings: Vec<(Arc<Settings>, u64)>,
    samples: Vec<(Arc<Sample>, u64, u64)>,
    factories: Vec<(super::Factory, u32, Weak<Data>)>,
}
impl State {
    fn used(&self) -> Budget {
        Budget {
            banks: self.banks.len(),
            settings: self.settings.len(),
            samples: self.samples.len(),
            pcm_bytes: self.samples.iter().map(|(_, pcm, _)| *pcm).sum(),
            metadata_bytes: self.banks.iter().map(|(_, bytes)| *bytes).sum::<u64>()
                + self.settings.iter().map(|(_, bytes)| *bytes).sum::<u64>()
                + self.samples.iter().map(|(_, _, bytes)| *bytes).sum::<u64>(),
        }
    }
    fn extra(&self, bank: &Arc<Data>) -> Budget {
        if self.banks.iter().any(|(old, _)| Arc::ptr_eq(old, bank)) {
            return Budget::default();
        }
        let mut result = Budget {
            banks: 1,
            metadata_bytes: bank_bytes(bank),
            ..Budget::default()
        };
        if !self
            .settings
            .iter()
            .any(|(old, _)| Arc::ptr_eq(old, &bank.settings))
        {
            result.settings = 1;
            result.metadata_bytes += settings_bytes(&bank.settings);
        }
        for (index, sample) in bank
            .audio
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
        {
            if self
                .samples
                .iter()
                .any(|(old, _, _)| Arc::ptr_eq(old, sample))
                || bank.audio[..index]
                    .iter()
                    .flatten()
                    .any(|old| Arc::ptr_eq(old, sample))
            {
                continue;
            }
            let (pcm, metadata) = sample_bytes(sample);
            result.samples += 1;
            result.pcm_bytes += pcm;
            result.metadata_bytes += metadata;
        }
        result
    }
    fn pin(&mut self, bank: &Arc<Data>) {
        if self.banks.iter().any(|(old, _)| Arc::ptr_eq(old, bank)) {
            return;
        }
        if !self
            .settings
            .iter()
            .any(|(old, _)| Arc::ptr_eq(old, &bank.settings))
        {
            self.settings
                .push((bank.settings.clone(), settings_bytes(&bank.settings)));
        }
        for sample in bank.audio.iter().flatten() {
            if !self
                .samples
                .iter()
                .any(|(old, _, _)| Arc::ptr_eq(old, sample))
            {
                let (pcm, metadata) = sample_bytes(sample);
                self.samples.push((sample.clone(), pcm, metadata));
            }
        }
        self.banks.push((bank.clone(), bank_bytes(bank)));
    }
    fn reap(&mut self) {
        // Removing a bank may make its settings and samples retireable in the
        // same pass. These are the only normal final drops of registered data.
        self.banks.retain(|(bank, _)| Arc::strong_count(bank) != 1);
        self.settings
            .retain(|(settings, _)| Arc::strong_count(settings) != 1);
        self.samples
            .retain(|(sample, _, _)| Arc::strong_count(sample) != 1);
        self.factories
            .retain(|(_, _, bank)| bank.strong_count() != 0);
    }
    fn empty(&self) -> bool {
        self.banks.is_empty()
            && self.settings.is_empty()
            && self.samples.is_empty()
            && self.reserved == Budget::default()
    }
}
fn sample_bytes(sample: &Sample) -> (u64, u64) {
    (
        sample.data.capacity() as u64 * 4,
        (std::mem::size_of::<Sample>()
            + 4 * std::mem::size_of::<usize>()
            + std::mem::size_of::<Vec<[f32; 3]>>()
            + sample.name.capacity()
            + sample.path.capacity()
            + sample.peaks.capacity() * std::mem::size_of::<[f32; 3]>()) as u64,
    )
}
fn bank_bytes(bank: &Data) -> u64 {
    (bank.storage_bytes() + 2 * std::mem::size_of::<usize>()) as u64
}
fn settings_bytes(settings: &Settings) -> u64 {
    (settings.storage_bytes() + 2 * std::mem::size_of::<usize>()) as u64
}

struct Shared {
    id: u64,
    limits: Budget,
    health: Arc<Health>,
    clients: AtomicUsize,
    state: Mutex<State>,
    wake: Condvar,
    #[cfg(test)]
    panic_next: AtomicBool,
}
impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| {
            self.health.poisoned.store(true, Ordering::Release);
            // Admission remains poisoned, but the drain loop must retain its
            // finite sleep instead of repeatedly failing Condvar::wait.
            self.state.clear_poison();
            poisoned.into_inner()
        })
    }
    fn check(&self) -> Result<(), Error> {
        if self.health.poisoned.load(Ordering::Acquire) {
            Err(Error::Poisoned)
        } else if !self.health.alive.load(Ordering::Acquire) {
            Err(Error::Closed)
        } else {
            Ok(())
        }
    }
    fn cycle(&self) -> bool {
        let mut state = self.state();
        #[cfg(test)]
        if self.panic_next.swap(false, Ordering::AcqRel) {
            panic!("injected sampler reaper failure");
        }
        state.reap();
        if self.clients.load(Ordering::Acquire) == 0 && state.empty() {
            state.sealed = true;
            return false;
        }
        // The registry stays owned by this thread through an unwind. In an
        // unwind-enabled build, failure poisons admission but the drain loop
        // continues to keep final releases away from active callbacks. Release
        // builds use panic=abort; they never claim recovery from a panic.
        drop(
            self.wake
                .wait_timeout(state, INTERVAL)
                .unwrap_or_else(|p| p.into_inner()),
        );
        true
    }
    fn run(self: Arc<Self>) {
        loop {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.cycle())) {
                Ok(false) => break,
                Ok(true) => (),
                Err(_) => {
                    self.health.poisoned.store(true, Ordering::Release);
                }
            }
        }
        self.health.alive.store(false, Ordering::Release);
    }
}
struct Client {
    shared: Arc<Shared>,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.shared.clients.fetch_sub(1, Ordering::AcqRel);
        self.shared.wake.notify_one();
    }
}

#[derive(Clone)]
pub(crate) struct Owner(Arc<Client>);
impl Owner {
    #[cfg(test)]
    pub(crate) fn isolated_for_test(limits: Budget) -> Self { Registry::default().acquire(limits).unwrap() }
    /// Preparation/worker only. Never called by rendering or command apply.
    pub fn acquire() -> Result<Self, Error> {
        REGISTRY
            .get_or_init(|| Mutex::new(Registry::default()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .acquire(Budget::limits())
    }
    pub fn available(&self) -> Result<Budget, Error> {
        let state = self.0.shared.state();
        self.0.shared.check()?;
        Ok(self
            .0
            .shared
            .limits
            .subtract(state.used().add(state.reserved)))
    }
    /// Reserve before decoding, so pending/prepared work shares the resident
    /// aggregate cap. The reservation lives only on the preparation worker.
    pub fn reserve(&self, budget: Budget) -> Result<Reservation, Error> {
        let shared = &self.0.shared;
        let mut state = shared.state();
        shared.check()?;
        if !state
            .used()
            .add(state.reserved)
            .add(budget)
            .fits(shared.limits)
        {
            return Err(Error::Full);
        }
        state.reserved = state.reserved.add(budget);
        Ok(Reservation {
            owner: self.clone(),
            budget,
            active: true,
        })
    }
    /// Already prepared shared data (factory generation/project preparation).
    /// New decoding instead uses reserve before allocating its PCM buffers.
    pub fn pin(&self, bank: Data) -> Result<Bank, Error> {
        let data = Arc::new(bank);
        let budget = self.0.shared.state().extra(&data);
        self.reserve(budget)?.publish(data)
    }
    /// Constant-time render-side ownership proof; no registry lock or lookup.
    pub fn owns(&self, bank: &Bank) -> bool {
        self.0.shared.id == bank.owner
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        self.0.shared.id == other.0.shared.id
    }
    /// Factory PCM is immutable and reusable across simultaneously prepared
    /// graphs at the same output rate. Cache entries are weak, never extra pins.
    pub fn factory(&self, factory: super::Factory, rate: u32) -> Result<Option<Bank>, Error> {
        let state = self.0.shared.state();
        self.0.shared.check()?;
        Ok(state
            .factories
            .iter()
            .find(|(f, r, _)| *f == factory && *r == rate)
            .and_then(|(_, _, data)| data.upgrade())
            .map(|data| Bank {
                data,
                owner: self.0.shared.id,
            }))
    }
    pub fn pin_factory(
        &self,
        factory: super::Factory,
        rate: u32,
        data: Data,
    ) -> Result<Bank, Error> {
        if let Some(bank) = self.factory(factory, rate)? {
            return Ok(bank);
        }
        let bank = self.pin(data)?;
        let mut state = self.0.shared.state();
        state
            .factories
            .retain(|(_, _, bank)| bank.strong_count() != 0);
        if state.factories.len() < 96
            && !state
                .factories
                .iter()
                .any(|(f, r, _)| *f == factory && *r == rate)
        {
            state
                .factories
                .push((factory, rate, Arc::downgrade(&bank.data)));
        }
        Ok(bank)
    }
}

pub(crate) struct Reservation {
    owner: Owner,
    budget: Budget,
    active: bool,
}
impl Reservation {
    pub fn budget(&self) -> Budget {
        self.budget
    }
    pub fn publish(mut self, data: Arc<Data>) -> Result<Bank, Error> {
        let shared = &self.owner.0.shared;
        let mut state = shared.state();
        shared.check()?;
        let extra = state.extra(&data);
        if !extra.fits(self.budget) {
            return Err(Error::Reservation);
        }
        // The reservation was accounted before decoding. Commit merely moves
        // those credits to pins, returning unused capacity atomically.
        state.pin(&data);
        state.reserved = state.reserved.subtract(self.budget);
        self.active = false;
        Ok(Bank {
            data,
            owner: shared.id,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active {
            let shared = &self.owner.0.shared;
            let mut state = shared.state();
            state.reserved = state.reserved.subtract(self.budget);
            shared.wake.notify_one();
        }
    }
}

/// Only Owner/Reservation can produce this certificate. A command may retain
/// it without owning or querying the registry; a different live owner rejects
/// it before installing any unpinned asset.
#[derive(Clone, Debug)]
pub(crate) struct Bank {
    data: Arc<Data>,
    owner: u64,
}
impl Deref for Bank {
    type Target = Data;
    fn deref(&self) -> &Data {
        &self.data
    }
}
impl Bank {
    pub fn metadata_bytes(&self) -> usize {
        (bank_bytes(&self.data) + settings_bytes(&self.settings)) as usize
    }
    pub fn same_data(&self, other: &Self) -> bool {
        self.owner == other.owner && Arc::ptr_eq(&self.data, &other.data)
    }
}

#[cfg(test)]
mod tests;
