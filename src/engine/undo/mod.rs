//! Bounded, renderer-authoritative inverse transactions. Only affected musical
//! objects are swapped. A worker retires owned media/notes/processors and
//! replenishes recording scratch; it never owns or locks the live renderer.
mod capture;
mod midi_import;
mod patch;
mod recording;
#[cfg(test)]
mod tests;
use super::*;
use crossbeam_channel::{bounded, Receiver, Sender};
use patch::*;
pub(in crate::engine) use patch::sample_bytes;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const MAX_ENTRIES: usize = 256;
const MAX_PATCHES: usize = TRACKS * SCENES + 1;
const SCRATCHES: usize = 64;
const RETIRE_CAPACITY: usize = 1024;
const RESERVE_RETIRE: usize = 2 * control::MAX_COMMANDS + 2 * control::COMMANDS_PER_BLOCK + 4;
const NOTE_LIMIT: usize = 8192;
const TEXT_LIMIT: usize = 4096;
const DEFAULT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    Session,
    Tempo,
    Quantization,
    Metronome,
    Crossfader,
    Master,
    CueMix,
    MasterEffect,
    Sampler,
    Track,
    ClipGain,
    ClipNotes,
    RecordNotes,
    Deck,
    DeckSeek,
    CueStyle,
    Grid,
    LoadMedia,
    AddEffect,
    Effect,
    ComposeClip,
    Multiple,
}
impl Name {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "Edit session layout",
            Self::Tempo => "Set tempo",
            Self::Quantization => "Change quantization",
            Self::Metronome => "Toggle metronome",
            Self::Crossfader => "Move crossfader",
            Self::Master => "Set master gain",
            Self::CueMix => "Set cue mix",
            Self::MasterEffect => "Edit master effect",
            Self::Sampler => "Edit sampler",
            Self::Track => "Edit track mixer",
            Self::ClipGain => "Set clip gain",
            Self::ClipNotes => "Edit clip notes",
            Self::RecordNotes => "Record notes",
            Self::Deck => "Edit deck",
            Self::DeckSeek => "Seek deck",
            Self::CueStyle => "Rename or color cue",
            Self::Grid => "Edit beatgrid",
            Self::LoadMedia => "Replace deck media",
            Self::AddEffect => "Add effect",
            Self::Effect => "Edit effect",
            Self::ComposeClip => "Create compose clip",
            Self::Multiple => "Edit objects",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Capacity,
    Budget,
    Notes,
    Text,
    Effects,
    ProcessorBudget,
    RateHistoryPruned,
    Unavailable,
    Invalid,
}
impl Failure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Capacity => "Edit not applied: undo storage is busy. Wait for the history worker, then retry.",
            Self::Budget => "Edit not applied: its retained media or notes exceed the undo memory budget.",
            Self::Notes => "Recording write not applied: the clip exceeds 8192 notes. Live monitoring continues.",
            Self::Text => "Edit not applied: clip metadata exceeds the supported history limit.",
            Self::RateHistoryPruned => "Output rate changed. Undo history was trimmed to its memory limit; active recording inverses were preserved.",
            Self::Effects => "Edit not applied: an effect rack supports at most 128 slots.",
            Self::ProcessorBudget => "Edit not applied: the session would exceed 256 MiB of effect buffers. Remove effects or choose a lower stopped output rate, then retry.",
            Self::Unavailable => "Edit not applied: the undo retirement worker is unavailable.",
            Self::Invalid => "Undo transaction is no longer valid; no objects were changed.",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: u64,
    pub name: Name,
    pub patches: usize,
    target: TargetLabel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetLabel {
    None,
    Track(u8),
    Clip(u8, u16),
    Deck(u8),
    Effect(Rack, usize),
    Multiple,
}
impl Item {
    pub fn label(self) -> String {
        let name = self.name.label();
        match self.target {
            TargetLabel::None => name.into(),
            TargetLabel::Track(t) => format!("{name} · Track {}", t + 1),
            TargetLabel::Clip(t, s) => format!("{name} · Track {}, scene {}", t + 1, s + 1),
            TargetLabel::Deck(d) => format!("{name} · Deck {}", if d == 0 { "A" } else { "B" }),
            TargetLabel::Effect(rack, slot) => match rack {
                Rack::Track(t) => format!("{name} · Track {}, FX {}", t + 1, slot + 1),
                Rack::Scene(s) => format!("{name} · Scene {}, FX {}", s + 1, slot + 1),
            },
            TargetLabel::Multiple => format!("{name} · Multiple objects"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct View {
    pub items: [Option<Item>; MAX_ENTRIES],
    pub cursor: usize,
    pub bytes: usize,
    pub retired_bytes: usize,
    pub budget: usize,
    pub fixed_bytes: usize,
    pub failure: Option<Failure>,
    pub failures: u64,
    pub state: u64,
    pub epoch: u64,
}
impl Default for View {
    fn default() -> Self {
        Self {
            items: [None; MAX_ENTRIES],
            cursor: 0,
            bytes: 0,
            retired_bytes: 0,
            budget: DEFAULT_BYTES,
            fixed_bytes: 0,
            failure: None,
            failures: 0,
            state: 0,
            epoch: 0,
        }
    }
}
struct Shared {
    view: Mutex<View>,
    retired_bytes: AtomicUsize,
    connected: AtomicBool,
    next_gesture: AtomicU64,
    #[cfg(test)]
    worker_hold: AtomicBool,
    sequence: AtomicU64,
    state: AtomicU64,
    epoch: AtomicU64,
    untracked: AtomicU64,
}
#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
}
impl Handle {
    pub fn view(&self) -> View {
        self.shared.view.lock().clone()
    }
    /// GUI/control producer only. Pointer gesture IDs never depend on callback
    /// timing and cannot accidentally absorb another source's untagged edit.
    pub fn gesture(&self) -> u64 {
        self.shared
            .next_gesture
            .fetch_add(1, Ordering::Relaxed)
            .max(1)
    }
}

struct Entry {
    id: u64,
    name: Name,
    target: TargetLabel,
    gesture: u64,
    key: u64,
    frame: u64,
    before: u64,
    after: u64,
    patches: [Option<Patch>; MAX_PATCHES],
    len: usize,
}
impl Entry {
    fn new(id: u64, name: Name, gesture: u64, key: u64, frame: u64, before: u64) -> Self {
        Self {
            id,
            name,
            target: TargetLabel::None,
            gesture,
            key,
            frame,
            before,
            after: id,
            patches: std::array::from_fn(|_| None),
            len: 0,
        }
    }
    fn push(&mut self, patch: Patch) {
        let target = patch.target_label();
        self.target = if self.len == 0 {
            target
        } else if target != self.target {
            TargetLabel::Multiple
        } else {
            target
        };
        self.patches[self.len] = Some(patch);
        self.len += 1;
    }
}

/// Names and note storage are prepared before streaming. A recording takes a
/// snapshot into this capacity; it never aliases a mutable clip's note vector.
struct Scratch {
    name: String,
    notes: Vec<MidiNote>,
}
impl Scratch {
    fn new() -> Self {
        Self {
            name: String::with_capacity(TEXT_LIMIT),
            notes: Vec::with_capacity(NOTE_LIMIT),
        }
    }
}
enum Retired {
    Conductor(Arc<midi_data::Conductor>),
    Entry(Entry),
    Timeline(VecDeque<Option<Entry>>),
    Patch(Patch),
    Command(Command),
    Boxed(Box<Command>),
    Notes(Vec<MidiNote>),
}
struct Garbage {
    value: Retired,
    bytes: usize,
}

pub(super) struct Journal {
    enabled: bool,
    replaying: bool,
    gesture: u64,
    record_take: u64,
    protected: Option<u64>,
    // Preallocated ring: full-history eviction never shifts unrelated inline
    // transactions (each can hold all 64 clip inverses plus cue metadata).
    entries: VecDeque<Option<Entry>>,
    cursor: usize,
    next: u64,
    state: u64,
    epoch: u64,
    untracked: u64,
    failure: Option<Failure>,
    failures: u64,
    bytes: usize,
    budget: usize,
    /// Sorted pointer/byte/refcount registry over immutable media reservations.
    assets: Vec<(usize, usize, usize)>,
    retired: Option<Sender<Garbage>>,
    scratch: Option<Receiver<Scratch>>,
    /// Preserve ownership even if a worker disconnects unexpectedly. Once this
    /// is used, admission closes before any further creative payload is read.
    stranded: Vec<Garbage>,
    shared: Arc<Shared>,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            enabled: false,
            replaying: false,
            gesture: 0,
            record_take: 1 << 63,
            protected: None,
            entries: VecDeque::new(),
            cursor: 0,
            next: 1,
            state: 0,
            epoch: 0,
            untracked: 0,
            failure: None,
            failures: 0,
            bytes: 0,
            budget: DEFAULT_BYTES,
            assets: Vec::new(),
            retired: None,
            scratch: None,
            stranded: Vec::new(),
            shared: Arc::new(Shared {
                view: Mutex::new(View::default()),
                retired_bytes: AtomicUsize::new(0),
                connected: AtomicBool::new(false),
                next_gesture: AtomicU64::new(1),
                #[cfg(test)]
                worker_hold: AtomicBool::new(false),
                sequence: AtomicU64::new(0),
                state: AtomicU64::new(0),
                epoch: AtomicU64::new(0),
                untracked: AtomicU64::new(0),
            }),
        }
    }
}
impl Journal {
    /// Called only during Engine setup, before audio takes ownership.
    pub fn enable(&mut self) -> std::io::Result<Handle> {
        if self.enabled {
            return Ok(Handle {
                shared: self.shared.clone(),
            });
        }
        let (retired, garbage) = bounded::<Garbage>(RETIRE_CAPACITY);
        let (prepared, scratch) = bounded(SCRATCHES);
        for _ in 0..SCRATCHES {
            prepared.try_send(Scratch::new()).ok().unwrap();
        }
        let shared = self.shared.clone();
        shared.connected.store(true, Ordering::Release);
        std::thread::Builder::new()
            .name("omatainer-undo-retire".into())
            .spawn(move || {
                loop {
                    #[cfg(test)]
                    if shared.worker_hold.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    match garbage.recv_timeout(Duration::from_millis(10)) {
                        Ok(Garbage { value, bytes }) => {
                            drop(value);
                            shared.retired_bytes.fetch_sub(bytes, Ordering::Release);
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                    // Replenishment runs here even without a retired payload.
                    while prepared.len() < SCRATCHES {
                        if prepared.try_send(Scratch::new()).is_err() {
                            break;
                        }
                    }
                }
                shared.connected.store(false, Ordering::Release);
            })?;
        self.entries = VecDeque::with_capacity(MAX_ENTRIES);
        self.assets = Vec::with_capacity(MAX_ENTRIES * MAX_PATCHES * 2);
        self.stranded =
            Vec::with_capacity(2 * control::MAX_COMMANDS + 2 * control::COMMANDS_PER_BLOCK + 4);
        self.retired = Some(retired);
        self.scratch = Some(scratch);
        self.enabled = true;
        Ok(Handle {
            shared: self.shared.clone(),
        })
    }
    pub(super) fn reject(&mut self, reason: Failure) {
        self.failure = Some(reason);
        self.failures = self.failures.wrapping_add(1);
    }
    fn room(&self, extra: usize) -> Result<(), Failure> {
        if !self.shared.connected.load(Ordering::Acquire) {
            return Err(Failure::Unavailable);
        }
        if self
            .retired
            .as_ref()
            .is_none_or(|queue| queue.len() + extra + RESERVE_RETIRE > RETIRE_CAPACITY)
        {
            return Err(Failure::Capacity);
        }
        if self.shared.retired_bytes.load(Ordering::Acquire) > self.budget {
            return Err(Failure::Capacity);
        }
        Ok(())
    }
    fn retire(&mut self, value: Retired, bytes: usize) {
        // Budget/capacity refusal also reaches this lower-level path directly.
        // Settle pending owned requests before handing payloads to the worker;
        // already-applied acknowledgements remain applied.
        if let Retired::Command(command) = &value {
            super::beatgrid::reject_retired(command);
            super::sampler::reject(command);
            if let Some(ack) = super::session::admission_ack(command) { ack.reject(); }
        }
        self.shared
            .retired_bytes
            .fetch_add(bytes, Ordering::Release);
        let garbage = Garbage { value, bytes };
        if let Err(error) = self.retired.as_ref().unwrap().try_send(garbage) {
            self.shared.connected.store(false, Ordering::Release);
            self.stranded.push(error.into_inner());
            self.reject(Failure::Unavailable);
        }
    }
    pub fn publish(&self) {
        if let Some(mut view) = self.shared.view.try_lock() {
            view.items.fill(None);
            for (out, entry) in view.items.iter_mut().zip(&self.entries) {
                *out = entry.as_ref().map(|e| Item {
                    id: e.id,
                    name: e.name,
                    patches: e.len,
                    target: e.target,
                });
            }
            view.cursor = self.cursor;
            view.bytes = self.bytes;
            view.retired_bytes = self.shared.retired_bytes.load(Ordering::Acquire);
            view.fixed_bytes = self.fixed_bytes();
            view.budget = self.budget;
            view.failure = self.failure;
            view.failures = self.failures;
            view.state = self.state;
            view.epoch = self.epoch;
        }
    }
}

impl Entry {
    fn bytes(&self) -> usize {
        self.patches[..self.len]
            .iter()
            .flatten()
            .map(Patch::heap_bytes)
            .sum()
    }
}
impl Journal {
    fn can_group(&self, key: u64, _frame: u64) -> bool {
        self.gesture != 0
            && self.cursor == self.entries.len()
            && self
                .entries
                .back()
                .and_then(Option::as_ref)
                .is_some_and(|e| e.gesture == self.gesture && e.key == key && e.len < MAX_PATCHES)
    }
    fn preflight(&self, bytes: usize) -> Result<(), Failure> {
        if bytes > self.budget {
            return Err(Failure::Budget);
        }
        self.room(self.entries.len() + 3)?;
        if self.protected.is_some()
            && (self.bytes.saturating_add(bytes) > self.budget
                || self.entries.len() == MAX_ENTRIES
                    && self
                        .entries
                        .front()
                        .and_then(Option::as_ref)
                        .is_some_and(|e| Some(e.id) == self.protected))
        {
            return Err(Failure::Capacity);
        }
        // Retiring evictions and rejected incoming payloads has its own budget.
        // Never hide outstanding PCM behind deduplicated live-history totals.
        let pending = self.shared.retired_bytes.load(Ordering::Acquire);
        let retiring = self.bytes;
        if pending.saturating_add(retiring) > self.budget.saturating_mul(2) {
            return Err(Failure::Capacity);
        }
        Ok(())
    }
    fn begin(&mut self, name: Name, key: u64, frame: u64) {
        while self.entries.len() > self.cursor {
            let entry = self.entries.pop_back().unwrap().unwrap();
            self.retire_entry(entry);
        }
        // Evict whole oldest transactions. The conservative estimate is checked
        // before mutation; recount after adding performs exact live deduplication.
        if self.entries.len() == MAX_ENTRIES {
            let entry = self.entries.pop_front().unwrap().unwrap();
            self.retire_entry(entry);
            self.cursor -= 1;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.entries.push_back(Some(Entry::new(
            id,
            name,
            self.gesture,
            key,
            frame,
            self.state,
        )));
        self.cursor = self.entries.len();
        self.state = id;
        self.publish_checkpoint();
    }
    fn unregister_entry(&mut self, entry: &Entry) -> usize {
        let mut bytes = entry.bytes();
        for patch in entry.patches[..entry.len].iter().flatten() {
            patch.media_reservations(|pointer, _| {
                let index = self
                    .assets
                    .binary_search_by_key(&pointer, |a| a.0)
                    .expect("history media reservation");
                self.assets[index].2 -= 1;
                if self.assets[index].2 == 0 {
                    bytes += self.assets.remove(index).1;
                }
            });
        }
        bytes
    }
    fn retire_entry(&mut self, entry: Entry) {
        let bytes = self.unregister_entry(&entry);
        // FIFO retirement: earlier removed references finish before this last
        // charged holder. PCM remains charged until the worker drops this entry.
        self.retire(Retired::Entry(entry), bytes);
    }
    fn append(&mut self, patch: Patch) {
        patch.media_reservations(|pointer, bytes| {
            match self.assets.binary_search_by_key(&pointer, |a| a.0) {
                Ok(index) => self.assets[index].2 += 1,
                Err(index) => self.assets.insert(index, (pointer, bytes, 1)),
            }
        });
        self.entries[self.cursor - 1].as_mut().unwrap().push(patch);
    }
    fn recount(&mut self) {
        let bytes = self
            .entries
            .iter()
            .flatten()
            .map(Entry::bytes)
            .sum::<usize>();
        self.bytes = bytes + self.assets.iter().map(|a| a.1).sum::<usize>();
        while self.bytes > self.budget && self.entries.len() > 1 {
            let entry = self.entries.pop_front().unwrap().unwrap();
            self.retire_entry(entry);
            self.cursor = self.cursor.saturating_sub(1);
            self.recount();
        }
    }
    pub(super) fn clear(&mut self) -> bool {
        if !self.enabled {
            return true;
        }
        let protected = self.protected.take();
        if self.preflight(0).is_err() {
            self.protected = protected;
            return false;
        }
        while let Some(entry) = self.entries.pop_back() {
            let entry = entry.unwrap();
            self.retire_entry(entry);
        }
        self.cursor = 0;
        self.bytes = 0;
        self.state = 0;
        self.epoch = self.epoch.wrapping_add(1);
        self.gesture = 0;
        self.untracked = 0;
        self.protected = None;
        self.publish_checkpoint();
        true
    }
}
impl RtEngine {
    pub(crate) fn enable_undo(&mut self) -> std::io::Result<Handle> {
        for track in &mut self.tracks {
            track.midi_schedule.prepare_history(NOTE_LIMIT);
            track.recorded_playback.reserve(NOTE_LIMIT);
        }
        self.undo.enable()
    }
    pub(super) fn history_replay(&mut self, redo: bool) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        let index = if redo {
            if self.undo.cursor == self.undo.entries.len() {
                return;
            }
            self.undo.cursor
        } else {
            if self.undo.cursor == 0 {
                return;
            }
            self.undo.cursor - 1
        };
        // Validity of every target is checked before any object is changed.
        if self.undo.entries[index]
            .as_ref()
            .unwrap()
            .patches
            .iter()
            .flatten()
            .any(|p| !p.valid(self))
        {
            self.undo.reject(Failure::Invalid);
            return;
        }
        let current = self.tracks.iter().map(|track| track.fx.slots.iter().map(fx::FxSlot::storage_bytes).sum::<usize>()).sum::<usize>()
            + self.scene_fx.iter().map(|rack| rack.slots.iter().map(fx::FxSlot::storage_bytes).sum::<usize>()).sum::<usize>();
        let restored = current as i128 + self.undo.entries[index].as_ref().unwrap().patches.iter().flatten()
            .map(|patch| patch.processor_delta(self)).sum::<i128>();
        if restored < 0 || restored > session::MAX_PROCESSOR_BYTES as i128 {
            self.undo.reject(Failure::ProcessorBudget);
            return;
        }
        let mut entry = self.undo.entries[index].take().unwrap();
        self.undo.replaying = true;
        if redo {
            for patch in entry.patches[..entry.len].iter_mut().flatten() {
                patch.apply(self);
            }
        } else {
            for patch in entry.patches[..entry.len].iter_mut().rev().flatten() {
                patch.apply(self);
            }
        }
        self.undo.replaying = false;
        // Publish only after every patch (including media receipt swaps) is in
        // place, so restored preparation remains attached to its actual source.
        let mut preparation_decks = [false; DECKS];
        for patch in entry.patches[..entry.len].iter().flatten() {
            match patch {
                Patch::Deck(deck, _) | Patch::Position { deck, .. } | Patch::Media { deck, .. } => preparation_decks[*deck as usize] = true,
                _ => {}
            }
        }
        for (deck, changed) in self.decks.iter().zip(preparation_decks) {
            if changed { deck.publish_preparation(); }
        }
        self.undo.state = if redo { entry.after } else { entry.before };
        self.undo.cursor = if redo { index + 1 } else { index };
        self.undo.gesture = 0;
        self.undo.entries[index] = Some(entry);
        self.undo.recount();
        self.undo.publish_checkpoint();
        self.project.edited();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checkpoint {
    pub epoch: u64,
    pub state: u64,
    pub untracked: u64,
}
impl Handle {
    pub fn checkpoint(&self) -> Checkpoint {
        loop {
            let before = self.shared.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let value = Checkpoint {
                epoch: self.shared.epoch.load(Ordering::Relaxed),
                state: self.shared.state.load(Ordering::Relaxed),
                untracked: self.shared.untracked.load(Ordering::Relaxed),
            };
            std::sync::atomic::fence(Ordering::Acquire);
            if self.shared.sequence.load(Ordering::Relaxed) == before {
                return value;
            }
        }
    }
}
impl Journal {
    pub(super) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            epoch: self.epoch,
            state: self.state,
            untracked: self.untracked,
        }
    }
    fn publish_checkpoint(&self) {
        self.shared.sequence.fetch_add(1, Ordering::AcqRel);
        self.shared.epoch.store(self.epoch, Ordering::Relaxed);
        self.shared.state.store(self.state, Ordering::Relaxed);
        self.shared
            .untracked
            .store(self.untracked, Ordering::Relaxed);
        self.shared.sequence.fetch_add(1, Ordering::Release);
    }
    fn changed(&mut self) {
        self.state = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        if let Some(Some(entry)) = self.entries.get_mut(self.cursor.saturating_sub(1)) {
            entry.after = self.state;
        }
        self.publish_checkpoint();
    }
    pub(super) fn untracked_change(&mut self) {
        self.untracked = self.untracked.wrapping_add(1);
        self.publish_checkpoint();
    }
    fn changed_from(&mut self, index: usize) {
        for i in index..self.entries.len() {
            let before = self.entries[i].as_ref().unwrap().before;
            let after = self.next;
            self.next = self.next.wrapping_add(1).max(1);
            let entry = self.entries[i].as_mut().unwrap();
            entry.before = if i == index { before } else { self.state };
            entry.after = after;
            self.state = after;
        }
        self.state = self.entries[self.cursor - 1].as_ref().unwrap().after;
        self.publish_checkpoint();
    }
    pub(super) fn available(&self) -> bool {
        !self.enabled || self.room(0).is_ok()
    }
    pub(super) fn gesture_id(&self) -> u64 {
        self.gesture
    }
    pub(super) fn set_gesture(&mut self, id: u64) {
        self.gesture = id;
    }
    pub(super) fn retire_box(&mut self, command: Box<Command>) {
        if self.enabled {
            self.retire(Retired::Boxed(command), std::mem::size_of::<Command>());
        } else {
            drop(command);
        }
    }
}

pub(crate) fn is_gesture_edit(command: &Command) -> bool {
    matches!(
        command,
        Command::SetBpm(_)
            | Command::NudgeBpm(_)
            | Command::Quant(_)
            | Command::ToggleQuant
            | Command::Metronome
            | Command::Xfader(_)
            | Command::Master(_)
            | Command::CueMix(_)
            | Command::FxWet { .. }
            | Command::FxSelect { .. }
            | Command::SamplerBank(_)
            | Command::SamplerInst(_)
            | Command::SamplerOct(_)
            | Command::SamplerEdit(_)
            | Command::TrackGain { .. }
            | Command::TrackPan { .. }
            | Command::Mute { .. }
            | Command::Solo { .. }
            | Command::Arm { .. }
            | Command::ClipGain { .. }
            | Command::MidiImport(_)
            | Command::MidiEdit(_)
            | Command::SetNotes { .. }
            | Command::ComposeArm { .. }
            | Command::DeckSeek { .. }
            | Command::DeckPitch { .. }
            | Command::DeckGain { .. }
            | Command::DeckEq { .. }
            | Command::DeckFilter { .. }
            | Command::DeckSync { .. }
            | Command::DeckPfl { .. }
            | Command::DeckHotCue { .. }
            | Command::DeckCueStyle { .. }
            | Command::DeckGrid { .. }
            | Command::DeckCuePoint { .. }
            | Command::DeckLoop { .. }
            | Command::DeckLoopIn { .. }
            | Command::DeckLoopOut { .. }
            | Command::DeckVinyl { .. }
            | Command::DeckKeylock { .. }
            | Command::DeckLoopDouble { .. }
            | Command::DeckLoopHalf { .. }
            | Command::DeckReloop { .. }
            | Command::DeckEqCut { .. }
            | Command::DeckEqSolo { .. }
            | Command::DeckPitchRange { .. }
            | Command::DeckMatch
            | Command::FxAdd(_)
            | Command::FxToggle(_)
            | Command::FxMix { .. }
            | Command::FxParam { .. }
    )
}

impl Journal {
    pub(super) fn retire_command(&mut self, command: Command) {
        super::midi_edit::reject_retired(&command);
        super::beatgrid::reject_retired(&command);
        super::sampler::reject(&command);
        if let Some(ack) = super::session::admission_ack(&command) { ack.reject(); }
        if self.enabled
            && matches!(
                command,
                Command::SessionControl(_)
                    | Command::SessionEdit(_)
                    | Command::Gesture { .. }
                    | Command::MidiImport(_)
            | Command::MidiEdit(_)
                    | Command::SetNotes { .. }
                    | Command::DeckAudio { .. }
                    | Command::DeckDecoded { .. }
                    | Command::DeckLoadRequested { .. }
                    | Command::LibraryFence { .. }
                    | Command::DeckRestorePreparation { .. }
                    | Command::DeckCueStyle { .. }
                    | Command::DeckGrid { .. }
                    | Command::SamplerEdit(_)
                    | Command::SamplerAudition(_)
                    | Command::DeckCuePoint { .. }
                    | Command::LearnCapture { .. }
            )
        {
            let bytes = capture::command_bytes(&command);
            self.retire(Retired::Command(command), bytes);
        } else {
            drop(command);
        }
    }
}

impl Journal {
    fn fixed_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Option<Entry>>()
            + (RETIRE_CAPACITY + self.stranded.capacity()) * std::mem::size_of::<Garbage>()
            + self.assets.capacity() * std::mem::size_of::<(usize, usize, usize)>()
            + SCRATCHES * (TEXT_LIMIT + NOTE_LIMIT * std::mem::size_of::<MidiNote>())
    }
}

#[cfg(test)]
impl RtEngine {
    pub(crate) fn set_undo_budget_for_test(&mut self, budget: usize) { self.undo.budget = budget; }
    pub(crate) fn clear_undo_for_test(&mut self) {
        assert!(self.undo.clear());
    }
}

impl Journal {
    /// Called by the existing stopped-device preparation path, never audio.
    pub(super) fn prepare_sample_rate(&mut self, sr: f32, active: [(u64, usize, usize); super::recording::CAPTURES]) {
        if !self.enabled {
            return;
        }
        let prospective = self.assets.iter().map(|a| a.1).sum::<usize>()
            + self
                .entries
                .iter()
                .flatten()
                .flat_map(|e| e.patches.iter().flatten())
                .map(|p| match p {
                    Patch::Slot { id, .. } => fx::FxSlot::required_storage(*id, sr),
                    Patch::Session(value) => value.rate_bytes(sr),
                    _ => p.heap_bytes(),
                })
                .sum::<usize>();
        if prospective > self.budget {
            // This method only runs while output is stopped. Allocate the
            // replacement fixed timeline here, and send the entire discarded
            // allocation as one owned message. Never expand old processors
            // beyond the budget merely to discard them immediately afterwards.
            let mut old = std::mem::replace(&mut self.entries, VecDeque::with_capacity(MAX_ENTRIES));
            self.state = 0;
            for entry in old.iter_mut().take(self.cursor).flatten() {
                // Held captures retain this stable owner ID across pruning.
                // The epoch and before/after checkpoints still describe the new
                // timeline; entry identity is not a mutable content version.
                let mut kept = Entry::new(
                    entry.id,
                    Name::RecordNotes,
                    entry.gesture,
                    0,
                    entry.frame,
                    self.state,
                );
                for patch in &mut entry.patches[..entry.len] {
                    let keep = matches!(patch,Some(Patch::Clip {track,scene,..}) if active.iter().any(|(owner,t,s)| *owner == entry.id && *t == *track as usize && *s == *scene as usize));
                    if keep {
                        kept.push(patch.take().unwrap());
                    }
                }
                if kept.len != 0 {
                    kept.after = self.next;
                    self.next = self.next.wrapping_add(1).max(1);
                    self.state = kept.after;
                    self.entries.push_back(Some(kept));
                }
            }
            let mut bytes = old.capacity() * std::mem::size_of::<Option<Entry>>();
            for entry in old.iter().flatten() {
                bytes += self.unregister_entry(entry);
            }
            self.retire(Retired::Timeline(old), bytes);
            self.cursor = self.entries.len();
            self.epoch = self.epoch.wrapping_add(1);
            self.protected = self.entries.front().and_then(Option::as_ref).map(|e| e.id);
            self.reject(Failure::RateHistoryPruned);
            self.publish_checkpoint();
        }
        for entry in self.entries.iter_mut().flatten() {
            for patch in entry.patches.iter_mut().flatten() {
                if let Patch::Session(value) = patch { value.prepare_rate(sr); }
                if let Patch::Sampler { value: Some(bank), .. } = patch {
                    // A historical inverse retains exact embedded PCM after a
                    // device-rate change, without inferring factory identity.
                    bank.factory = None;
                }
                if let Patch::Slot {
                    slot,
                    reserved_bytes,
                    id,
                    ..
                } = patch
                {
                    if let Some(slot) = slot {
                        slot.set_sample_rate(sr);
                    }
                    *reserved_bytes = fx::FxSlot::required_storage(*id, sr);
                }
            }
        }
        self.recount();
        self.publish();
    }
}

impl Journal {
    pub(super) fn retire_midi_conductor(&mut self, value: Arc<midi_data::Conductor>, bytes: usize) {
        self.retire(Retired::Conductor(value), bytes);
    }
}
