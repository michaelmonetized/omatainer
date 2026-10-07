use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering::*},
        Arc,
    },
    time::Instant,
};

const EVENTS: usize = 256;
const MIN_PERIOD: f64 = 6_250_000.0;
const MAX_PERIOD: f64 = 125_000_000.0;
const MIN_SONG_PERIOD: f64 = 60_000_000_000.0 / 240.0 / 24.0;
const MAX_SONG_PERIOD: f64 = 60_000_000_000.0 / 40.0 / 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LossPolicy {
    #[default]
    Freewheel,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Config {
    pub source: Option<u64>,
    pub follow_transport: bool,
    pub loss: LossPolicy,
    pub timeout_ms: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            source: None,
            follow_transport: true,
            loss: LossPolicy::Freewheel,
            timeout_ms: 500,
        }
    }
}
impl Config {
    /// Validate an explicit clock source and silence deadline.
    /// Takes transient settings; returns whether identity and 250–2,000 ms timeout are usable.
    pub(crate) fn valid(self) -> bool {
        self.source != Some(0) && (250..=2000).contains(&self.timeout_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Loss {
    Silence,
    Retired,
    Overflow,
    Feedback,
    TempoRange,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Message {
    Tick { packet_ticks: u16 },
    Start,
    Continue,
    Stop,
    Position(u16),
}

#[derive(Clone, Copy)]
struct Event {
    generation: u64,
    safety: u64,
    source: u64,
    at: u64,
    message: Message,
}

pub(crate) struct Shared {
    origin: Instant,
    selected: AtomicU64,
    generation: AtomicU64,
    lost: AtomicU64,
    ignored: AtomicU64,
    overflow: AtomicU64,
    transport_stop: AtomicU64,
    sender: crossbeam_channel::Sender<Event>,
    receiver: crossbeam_channel::Receiver<Event>,
}
impl Default for Shared {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(EVENTS);
        Self {
            origin: Instant::now(),
            selected: AtomicU64::new(0),
            generation: AtomicU64::new(2),
            lost: AtomicU64::new(0),
            ignored: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            transport_stop: AtomicU64::new(0),
            sender,
            receiver,
        }
    }
}
impl Shared {
    /// Capture the clock selection fence without locks.
    /// Takes this input policy; returns the generation retained by the native input packet.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Acquire)
    }
    /// Check whether explicit external following owns realtime transport.
    /// Takes this shared policy; returns true while any source is selected.
    pub(crate) fn enabled(&self) -> bool {
        self.selected.load(Acquire) != 0
    }
    /// Retire only the selected source independently of ordinary queue space.
    /// Takes original source and loss reason; publishes a generation-scoped loss without blocking.
    pub(crate) fn retire(&self, source: u64, reason: Loss) {
        let generation = self.generation();
        if source != 0 && self.selected.load(Acquire) == source {
            if reason == Loss::Overflow {
                self.overflow.fetch_add(1, Relaxed);
            }
            let code = match reason {
                Loss::Silence => 1,
                Loss::Retired => 2,
                Loss::Overflow => 3,
                Loss::Feedback => 4,
                Loss::TempoRange => 5,
            };
            self.lost
                .fetch_max(generation.wrapping_shl(3) | code, AcqRel);
        }
    }
    /// Preserve a selected-source Stop even when a queue or input is retired.
    /// Takes original source and captured clock generation; publishes a bounded transport edge without borrowing ordinary command capacity.
    pub(crate) fn request_stop(&self, source: u64, generation: u64) {
        if source != 0 && generation == self.generation() && self.selected.load(Acquire) == source {
            self.transport_stop.fetch_max(generation, AcqRel);
        }
    }
    /// Admit timestamped clock bytes from the ordinary private input worker.
    /// Takes original source/fences, callback time, decoded message and output-port conflict; returns whether explicit clock policy consumed the message.
    pub(crate) fn input(
        &self,
        source: u64,
        generation: u64,
        safety: u64,
        at: Instant,
        message: Message,
        conflict: bool,
    ) -> bool {
        if generation & 1 != 0 || generation != self.generation() {
            self.ignored.fetch_add(1, Relaxed);
            return true;
        }
        let selected = self.selected.load(Acquire);
        if selected == 0 {
            return matches!(message, Message::Position(_))
                || conflict && !matches!(message, Message::Tick { .. });
        }
        if selected != 0 && (source != selected || self.lost.load(Acquire) >> 3 == generation) {
            self.ignored.fetch_add(1, Relaxed);
            return true;
        }
        if conflict && selected != 0 {
            self.retire(source, Loss::Feedback);
            return true;
        }
        let Some(at) = at.checked_duration_since(self.origin) else {
            self.ignored.fetch_add(1, Relaxed);
            return true;
        };
        let event = Event {
            generation,
            safety,
            source,
            at: at.as_nanos().min(u128::from(u64::MAX)) as u64,
            message,
        };
        if self.sender.try_send(event).is_err() {
            if matches!(message, Message::Stop) {
                self.request_stop(source, generation);
            }
            self.retire(source, Loss::Overflow);
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Status {
    pub config: Config,
    pub locked: bool,
    pub awaiting_tick: bool,
    pub lost: Option<Loss>,
    pub estimated_bpm: Option<f64>,
    pub unsupported_bpm: Option<f64>,
    pub jitter_ms: f64,
    pub phase_error_beats: f64,
    pub accepted_ticks: u64,
    pub batched_ticks: u64,
    pub inferred_missing_ticks: u64,
    pub stale: u64,
    pub ignored: u64,
    pub overflow: u64,
    pub reacquisitions: u64,
    pub backend_timing: bool,
    pub transport_running: bool,
    pub needs_transport: bool,
}
impl Status {
    #[cfg(test)]
    pub(crate) fn maximum_for_test() -> Self {
        Self {
            config: Config {
                source: Some(u64::MAX),
                follow_transport: true,
                loss: LossPolicy::Stop,
                timeout_ms: 2000,
            },
            locked: true,
            awaiting_tick: true,
            lost: Some(Loss::TempoRange),
            estimated_bpm: Some(240.0),
            unsupported_bpm: Some(320.0),
            jitter_ms: f64::MAX,
            phase_error_beats: f64::MAX,
            accepted_ticks: u64::MAX,
            batched_ticks: u64::MAX,
            inferred_missing_ticks: u64::MAX,
            stale: u64::MAX,
            ignored: u64::MAX,
            overflow: u64::MAX,
            reacquisitions: u64::MAX,
            backend_timing: true,
            transport_running: true,
            needs_transport: true,
        }
    }
}

#[derive(Default)]
pub(crate) struct Update {
    pub stop: bool,
    pub start: Option<f64>,
    pub ticks: u64,
    pub tick_source: Option<u64>,
    pub step: Option<f64>,
}

pub(crate) struct Runtime {
    shared: Arc<Shared>,
    status: Status,
    generation: u64,
    safety: u64,
    base_ns: u64,
    rate: u32,
    anchored: bool,
    next: Option<Event>,
    last_tick: Option<u64>,
    last_event: Option<u64>,
    last_interval: f64,
    last_missing: u64,
    period: f64,
    intervals: [f64; 16],
    count: usize,
    cursor: usize,
    pulse_beat: f64,
    pending: Option<f64>,
    pending_at: Option<u64>,
    position: Option<f64>,
    running: bool,
    stop_required: bool,
}
impl Runtime {
    /// Prepare the audio-owned follower without opening a MIDI device.
    /// Takes shared bounded input; returns an internal-clock runtime with fixed estimator storage.
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self {
            generation: shared.generation(),
            shared,
            status: Status::default(),
            safety: 0,
            base_ns: 0,
            rate: 0,
            anchored: false,
            next: None,
            last_tick: None,
            last_event: None,
            last_interval: 0.0,
            last_missing: 0,
            period: 0.0,
            intervals: [0.0; 16],
            count: 0,
            cursor: 0,
            pulse_beat: 0.0,
            pending: None,
            pending_at: None,
            position: None,
            running: false,
            stop_required: false,
        }
    }
    /// Identify an explicit external clock selection.
    /// Takes this renderer; returns whether the selected source owns song timing.
    pub(crate) fn enabled(&self) -> bool {
        self.status.config.source.is_some()
    }
    /// Read bounded following status.
    /// Takes this renderer; returns scalars suitable for the snapshot and IPC surfaces.
    pub(crate) fn status(&self) -> Status {
        Status {
            transport_running: self.running,
            needs_transport: self.enabled()
                && self.status.config.follow_transport
                && !self.running
                && self.pending.is_none(),
            ignored: self.shared.ignored.load(Relaxed),
            overflow: self.shared.overflow.load(Relaxed),
            ..self.status
        }
    }
    /// Keep the selected clock phase through an automatic musical loop wrap.
    /// Takes the song's beat displacement; shifts the local phase anchor while retaining source, estimator and explicit incoming transport.
    pub(crate) fn rebase(&mut self, displacement: f64) {
        if self.enabled() && displacement.is_finite() {
            self.pulse_beat += displacement;
        }
    }
    /// Apply a reviewed source selection at the audio boundary.
    /// Takes configuration, current beat/running state and safety epoch; clears prior source events and estimator state.
    pub(crate) fn configure(&mut self, config: Config, beat: f64, running: bool, safety: u64) {
        if !config.valid() {
            return;
        }
        self.shared.generation.fetch_add(1, AcqRel);
        self.shared.selected.store(0, Release);
        self.shared.lost.store(0, Release);
        for _ in 0..EVENTS {
            if self.shared.receiver.try_recv().is_err() {
                break;
            }
        }
        self.shared
            .selected
            .store(config.source.unwrap_or(0), Release);
        self.generation = self.shared.generation.fetch_add(1, AcqRel).wrapping_add(1);
        self.status = Status {
            config,
            ..Default::default()
        };
        self.safety = safety;
        self.next = None;
        self.last_tick = None;
        self.last_event = None;
        self.last_interval = 0.0;
        self.last_missing = 0;
        self.count = 0;
        self.cursor = 0;
        self.period = 0.0;
        self.pulse_beat = beat;
        self.pending = None;
        self.pending_at = None;
        self.position = None;
        self.running = running;
        self.stop_required = false;
    }
    /// Anchor following to the first scheduled audio sample.
    /// Takes sample rate, callback frames and optional playback timestamp; uses the same one-buffer estimate as clock output when timing is unavailable.
    pub(crate) fn begin(&mut self, rate: u32, frames: usize, first: Option<Instant>) {
        let known = first.and_then(|at| at.checked_duration_since(self.shared.origin));
        let now = self
            .shared
            .origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let base = known.map_or_else(
            || now.saturating_add(frames as u64 * 1_000_000_000 / u64::from(rate.max(1))),
            |time| time.as_nanos().min(u128::from(u64::MAX)) as u64,
        );
        self.begin_at(rate, base, known.is_some());
    }
    /// Set one monotonic audio-frame anchor without heap work.
    /// Takes rate, first-frame nanoseconds and backend timing quality; returns after retaining the anchor for one block.
    pub(crate) fn begin_at(&mut self, rate: u32, base: u64, known: bool) {
        self.rate = rate;
        self.base_ns = base;
        self.anchored = true;
        self.status.backend_timing = known;
    }
    /// Provide a timestamp for offline/direct renderer calls.
    /// Takes rate and frame count; preserves a callback's supplied anchor or estimates a new one, then consumes it once.
    pub(crate) fn ensure_begin(&mut self, rate: u32, frames: usize) {
        if !self.anchored {
            self.begin(rate, frames, None);
        }
        self.anchored = false;
    }
    fn lose(&mut self, reason: Loss) {
        self.shared.lost.store(0, Release);
        self.generation = self.shared.generation.fetch_add(2, AcqRel).wrapping_add(2);
        self.next = None;
        self.last_tick = None;
        self.last_event = None;
        self.last_interval = 0.0;
        self.last_missing = 0;
        self.pending = None;
        self.pending_at = None;
        if reason != Loss::Silence {
            self.position = None;
        }
        self.count = 0;
        self.cursor = 0;
        self.status.locked = false;
        self.status.awaiting_tick = false;
        self.status.lost = Some(reason);
        if self.status.config.loss == LossPolicy::Stop {
            self.running = false;
            self.stop_required = true;
        }
    }
    fn tick(&mut self, at: u64, packet_ticks: u16, beat: f64, update: &mut Update) {
        if !(1..=256).contains(&packet_ticks) {
            self.status.stale += 1;
            return;
        }
        if self.last_tick.is_some_and(|last| {
            at.saturating_sub(last) > u64::from(self.status.config.timeout_ms) * 1_000_000
        }) {
            self.last_tick = None;
            self.count = 0;
            self.cursor = 0;
            self.last_missing = 0;
            self.last_interval = 0.0;
            self.status.locked = false;
            self.status.lost = Some(Loss::Silence);
            if self.status.config.loss == LossPolicy::Stop && self.pending.is_none() {
                self.running = false;
                update.stop = true;
            }
        }
        let mut missing = 0_u64;
        if let Some(previous) = self.last_tick {
            if at < previous {
                self.status.stale += 1;
                return;
            }
            if at > previous {
                let interval = (at - previous) as f64;
                let observed = interval / f64::from(packet_ticks);
                if self.last_missing > 0
                    && (observed - self.last_interval).abs() <= self.last_interval * 0.1
                {
                    if self.running {
                        self.pulse_beat -= self.last_missing as f64 / 24.0;
                    }
                    self.status.inferred_missing_ticks = self
                        .status
                        .inferred_missing_ticks
                        .saturating_sub(self.last_missing);
                    self.period = observed.clamp(MIN_SONG_PERIOD, MAX_SONG_PERIOD);
                    self.count = 0;
                    self.cursor = 0;
                } else if self.count >= 8 && self.period > 0.0 && observed > self.period * 1.6 {
                    let candidate = (interval / self.period)
                        .round()
                        .clamp(f64::from(packet_ticks), f64::from(packet_ticks) + 12.0);
                    if (interval / candidate - self.period).abs() < self.period * 0.2 {
                        missing = candidate as u64 - u64::from(packet_ticks);
                    }
                }
                let normalized = interval / (u64::from(packet_ticks) + missing) as f64;
                if !(MIN_PERIOD..=MAX_PERIOD).contains(&normalized) {
                    self.status.stale += 1;
                    return;
                }
                self.intervals[self.cursor] = normalized;
                self.cursor = (self.cursor + 1) % self.intervals.len();
                self.count = (self.count + 1).min(self.intervals.len());
                let estimate_count = if self.count == 1 { 1 } else { self.count & !1 };
                let mut sorted = self.intervals;
                for index in 1..estimate_count {
                    let value = sorted[index];
                    let mut j = index;
                    while j > 0 && sorted[j - 1] > value {
                        sorted[j] = sorted[j - 1];
                        j -= 1;
                    }
                    sorted[j] = value;
                }
                let trim = estimate_count / 4;
                let estimate = sorted[trim..estimate_count - trim].iter().sum::<f64>()
                    / (estimate_count - 2 * trim) as f64;
                let candidate = 60_000_000_000.0 / (estimate * 24.0);
                if self.count >= 8 && !(39.5..=243.0).contains(&candidate) {
                    self.lose(Loss::TempoRange);
                    self.status.unsupported_bpm = Some(candidate);
                    return;
                }
                let estimate = 60_000_000_000.0 / (candidate.clamp(40.0, 240.0) * 24.0);
                self.period = if self.period == 0.0 {
                    estimate
                } else {
                    self.period + (estimate - self.period) * 0.25
                };
                self.status.jitter_ms +=
                    ((normalized - estimate).abs() / 1_000_000.0 - self.status.jitter_ms) * 0.125;
                self.status.estimated_bpm = Some(60_000_000_000.0 / (self.period * 24.0));
                if self.count >= 8 {
                    self.status.unsupported_bpm = None;
                    if self.status.lost == Some(Loss::TempoRange) {
                        self.status.lost = None;
                        self.status.reacquisitions += 1;
                    }
                }
                self.status.locked = self.count >= 8;
                self.status.inferred_missing_ticks += missing;
                self.last_interval = observed;
                self.last_missing = missing;
            }
            if self.running {
                self.pulse_beat += (1 + missing) as f64 / 24.0;
            }
        } else {
            self.pulse_beat = beat;
            if self.status.lost != Some(Loss::TempoRange) && self.status.lost.take().is_some() {
                self.status.reacquisitions += 1;
            }
        }
        if let Some(position) = self.pending.take() {
            self.pending_at = None;
            self.pulse_beat = position;
            self.running = true;
            update.start = Some(position);
            self.status.awaiting_tick = false;
        }
        self.last_tick = Some(at);
        self.status.accepted_ticks += 1;
        if packet_ticks > 1 {
            self.status.batched_ticks += 1;
        }
        update.ticks += 1;
    }
    /// Advance timestamped transport and estimate one coherent musical sample step.
    /// Takes sample offset, original beat/running state and safety fence; returns bounded transport edges, accepted clocks and a gently corrected forward step.
    pub(crate) fn frame(&mut self, offset: usize, beat: f64, playing: bool, safety: u64) -> Update {
        let mut update = Update::default();
        if safety != self.safety {
            self.configure(Config::default(), beat, false, safety);
            return update;
        }
        if !self.enabled() {
            return update;
        }
        if self.shared.transport_stop.swap(0, AcqRel) == self.generation
            && self.status.config.follow_transport
        {
            update.stop = true;
            self.running = false;
            self.pending = None;
            self.pending_at = None;
            self.status.awaiting_tick = false;
        }
        let lost = self.shared.lost.load(Acquire);
        if self.enabled() && lost >> 3 == self.generation {
            self.lose(match lost & 7 {
                2 => Loss::Retired,
                3 => Loss::Overflow,
                4 => Loss::Feedback,
                5 => Loss::TempoRange,
                _ => Loss::Silence,
            });
        }
        let now = self
            .base_ns
            .saturating_add(offset as u64 * 1_000_000_000 / u64::from(self.rate.max(1)));
        if !update.stop && !self.stop_required && self.pending.is_none() && self.running != playing
        {
            self.running = playing;
            self.pulse_beat = beat
                - if self.period > 0.0 {
                    self.last_tick
                        .map_or(0.0, |at| now.saturating_sub(at) as f64 / self.period / 24.0)
                } else {
                    0.0
                };
        }
        for _ in 0..EVENTS {
            let Some(event) = self
                .next
                .take()
                .or_else(|| self.shared.receiver.try_recv().ok())
            else {
                break;
            };
            if event.generation != self.generation
                || self
                    .status
                    .config
                    .source
                    .is_some_and(|source| event.source != source)
                || event.safety != safety
            {
                self.status.stale += 1;
                continue;
            }
            if event.at > now {
                self.next = Some(event);
                break;
            }
            if !matches!(event.message, Message::Stop)
                && now.saturating_sub(event.at)
                    > u64::from(self.status.config.timeout_ms) * 1_000_000
            {
                self.status.stale += 1;
                continue;
            }
            if self.last_event.is_some_and(|previous| event.at < previous) {
                self.status.stale += 1;
                continue;
            }
            self.last_event = Some(event.at);

            match event.message {
                Message::Tick { packet_ticks } => {
                    self.tick(event.at, packet_ticks, beat, &mut update);
                    update.tick_source = Some(event.source);
                }
                Message::Start if self.status.config.follow_transport => {
                    self.pending = Some(0.0);
                    self.pending_at = Some(event.at);
                    self.position = None;
                    self.status.awaiting_tick = true;
                }
                Message::Continue if self.status.config.follow_transport => {
                    self.pending = Some(self.position.take().unwrap_or(beat));
                    self.pending_at = Some(event.at);
                    self.status.awaiting_tick = true;
                }
                Message::Stop if self.status.config.follow_transport => {
                    self.running = false;
                    self.pending = None;
                    self.pending_at = None;
                    self.status.awaiting_tick = false;
                    update.stop = true;
                    update.start = None;
                }
                Message::Position(position) if self.status.config.follow_transport => {
                    let position = f64::from(position) / 4.0;
                    if self.running || self.pending.is_some() {
                        self.pending = Some(position);
                        self.pending_at = Some(event.at);
                        self.status.awaiting_tick = true;
                    } else {
                        self.position = Some(position);
                    }
                }
                _ => {}
            }
        }
        if self.pending_at.or(self.last_tick).is_some_and(|last| {
            now.saturating_sub(last) > u64::from(self.status.config.timeout_ms) * 1_000_000
        }) {
            self.lose(Loss::Silence);
        }
        update.stop |= std::mem::take(&mut self.stop_required);
        if self.period > 0.0 {
            let nominal = 1_000_000_000.0 / self.period / 24.0 / f64::from(self.rate.max(1));
            let active = playing || update.start.is_some();
            let correction = if active && !update.stop && self.status.locked {
                let target = self.pulse_beat
                    + self
                        .last_tick
                        .map_or(0.0, |at| now.saturating_sub(at) as f64 / self.period / 24.0);
                self.status.phase_error_beats = target - update.start.unwrap_or(beat);
                (self.status.phase_error_beats * 2.0 / f64::from(self.rate.max(1)))
                    .clamp(-nominal * 0.05, nominal * 0.05)
            } else {
                0.0
            };
            update.step = Some(nominal + correction);
        }
        update
    }
}

impl crate::engine::RtEngine {
    /// Return deliberately to the native song clock.
    /// Takes this renderer; fences queued external transport while preserving the imported conductor and current song position.
    pub(in crate::engine) fn internal_clock(&mut self) {
        if !self.clock_input.enabled() {
            return;
        }
        let beat = self.precise_midi_beat();
        self.clock_input.configure(
            Config::default(),
            beat,
            self.playing,
            self.performance.input_epoch(),
        );
        self.mapped_clock = None;
    }
    /// Select one transient external clock policy.
    /// Takes validated settings; retains current transport and stored tempo maps while resetting source-owned estimation.
    pub(in crate::engine) fn configure_clock_input(&mut self, config: Config) {
        if !config.valid() {
            return;
        }
        let beat = self.precise_midi_beat();
        self.clock_input
            .configure(config, beat, self.playing, self.performance.input_epoch());
        self.mapped_clock = None;
        if config.source.is_some() {
            self.count_in = None;
        }
    }
    fn external_pause(&mut self) {
        self.history_finish_take();
        self.finish_recording_all();
        self.playing = false;
        self.recording = false;
        self.compose_target = None;
        self.metro.reset();
        self.count_in = None;
        self.scenes.cancel();
        self.navigation.cancel();
        self.transport_epoch = self.transport_epoch.wrapping_add(1);
        self.arrangement.reset(self.precise_midi_beat());
        for (slot, track) in self.tracks.iter_mut().enumerate() {
            if let Some(playing) = track.playing.take() {
                track.project_resume = Some(playing);
            }
            track.launch.clear();
            track.release_clip_notes();
            track.drum_pos.fill(None);
            self.midi_routing.clear_clip(slot as u8);
        }
    }
    fn external_resume(&mut self, beat: f64) {
        self.scenes.cancel();
        self.navigation.cancel();
        self.finish_recording_all();
        self.recording = false;
        self.compose_target = None;
        self.count_in = None;
        self.metro.reset();
        self.transport_epoch = self.transport_epoch.wrapping_add(1);
        let changed = (beat - self.precise_midi_beat()).abs() > 1e-10;
        if changed {
            self.timeline_anchor = beat * 60.0 / f64::from(self.bpm);
            self.timeline_frames = 0;
        }
        self.beat = beat;
        self.midi_beat = beat;
        self.midi_beat_reference = beat;
        self.beat_roundoff = 0.0;
        self.mapped_clock = None;
        self.arrangement.reset(beat);
        for (slot, track) in self.tracks.iter_mut().enumerate() {
            track.launch.seek();
            track.release_clip_notes();
            track.drum_pos.fill(None);
            self.midi_routing.clear_clip(slot as u8);
            if let Some(mut launch) = track.playing.or(track.project_resume.take()) {
                launch.last_beat = 0.0;
                track.playing = Some(launch);
                track.rebuild_midi_schedule(beat, beat);
            }
        }
        self.playing = true;
    }
    /// Apply only selected timestamped clock transport at this sample.
    /// Takes audio sample offset; returns the follower's coherent forward beat step and preserves independent DJ decks and live-note owners.
    pub(in crate::engine) fn external_clock_frame(&mut self, offset: usize) -> Option<f64> {
        if !self.clock_input.enabled() {
            return None;
        }
        let beat = self.precise_midi_beat();
        let update =
            self.clock_input
                .frame(offset, beat, self.playing, self.performance.input_epoch());
        if let Some(source) = update.tick_source {
            for _ in 0..update.ticks {
                self.midi_clock.receive_tick(source);
            }
        }
        if let Some(bpm) = self.clock_input.status().estimated_bpm {
            self.bpm = bpm as f32;
        }
        if update.stop {
            self.external_pause();
        }
        if let Some(beat) = update.start {
            self.external_resume(beat);
        }
        update.step
    }
}

#[cfg(test)]
mod tests;
