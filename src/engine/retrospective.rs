use super::{midi_tools::Content, MidiNote};
use crate::midi_file::{self as smf, Message};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const PPQN: u16 = 960;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub enabled: bool,
    pub seconds: u16,
    pub events: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: 120,
            events: 8192,
        }
    }
}
impl Config {
    /// Validate recent-input resource limits.
    /// Takes saved capture preferences; returns refusal outside one to 600 seconds or 256 to 65536 events.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=600).contains(&self.seconds) || !(256..=65536).contains(&self.events) {
            return Err("Recent MIDI history accepts 1–600 seconds and 256–65536 events".into());
        }
        Ok(())
    }
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub track: u8,
    pub source: u64,
    pub at: Instant,
    pub order: u64,
    pub bytes: [u8; 3],
    pub length: u8,
}
struct State {
    config: Config,
    origin: Instant,
    watermark: Instant,
    epoch: u64,
    order: u64,
    events: VecDeque<Event>,
    dropped: u64,
    lost_before: Option<Instant>,
}
pub(crate) struct Shared {
    enabled: AtomicBool,
    state: Mutex<State>,
}
impl Default for Shared {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            enabled: AtomicBool::new(false),
            state: Mutex::new(State {
                config: Config::default(),
                origin: now,
                watermark: now,
                epoch: 1,
                order: 0,
                events: VecDeque::new(),
                dropped: 0,
                lost_before: None,
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub config: Config,
    pub epoch: u64,
    pub captured_at: Instant,
    pub origin: Instant,
    pub events: Vec<Event>,
    pub dropped: u64,
    pub lost_before: Option<Instant>,
}
impl Shared {
    /// Change opt-in MIDI history settings and clear private input.
    /// Takes validated preferences and the monotonic boundary; returns the new history generation.
    pub(crate) fn configure(&self, config: Config, now: Instant) -> Result<u64, String> {
        config.validate()?;
        let events = if config.enabled {
            VecDeque::with_capacity(config.events as usize)
        } else {
            VecDeque::new()
        };
        let mut state = self.state.lock();
        let epoch = state.epoch.wrapping_add(1).max(1);
        for event in &mut state.events {
            event.bytes.fill(0);
            event.source = 0;
            event.length = 0;
        }
        *state = State {
            config,
            origin: now,
            watermark: now,
            epoch,
            order: 0,
            events,
            dropped: 0,
            lost_before: None,
        };
        self.enabled.store(config.enabled, Ordering::Release);
        Ok(epoch)
    }

    /// Discard all recent MIDI input explicitly.
    /// Takes the clear boundary; returns a generation that invalidates every old review.
    pub(crate) fn clear(&self, now: Instant) -> u64 {
        let mut state = self.state.lock();
        for event in &mut state.events {
            event.bytes.fill(0);
            event.source = 0;
            event.length = 0;
        }
        state.events.clear();
        state.origin = now;
        state.watermark = now;
        state.lost_before = None;
        state.dropped = 0;
        state.epoch = state.epoch.wrapping_add(1).max(1);
        state.epoch
    }

    /// Read the current history generation.
    /// Takes this shared owner; returns the token used to refuse cleared or reconfigured reviews.
    pub(crate) fn epoch(&self) -> u64 {
        self.state.lock().epoch
    }

    /// Retain monitored channel data outside the audio renderer.
    /// Takes a stable source, destination track, original timestamp and bounded wire frame; returns whether it entered the opt-in ring without allocating.
    pub(crate) fn observe(&self, track: u8, source: u64, at: Instant, bytes: &[u8]) -> bool {
        if !self.enabled.load(Ordering::Acquire)
            || source == 0
            || usize::from(track) >= super::session::MAX_TRACKS
        {
            return false;
        }
        let Some(&status) = bytes.first() else {
            return false;
        };
        let expected = match status & 0xf0 {
            0x80..=0xb0 | 0xe0 => 3,
            0xc0 | 0xd0 => 2,
            _ => return false,
        };
        if bytes.len() != expected || bytes[1..].iter().any(|&v| v > 127) {
            return false;
        }
        let mut state = self.state.lock();
        if !state.config.enabled || at < state.origin {
            return false;
        }
        state.watermark = state.watermark.max(at);
        let cutoff = state
            .watermark
            .checked_sub(Duration::from_secs(u64::from(state.config.seconds)))
            .unwrap_or(state.origin)
            .max(state.origin);
        if at < cutoff {
            state.dropped = state.dropped.saturating_add(1);
            return false;
        }
        while state.events.front().is_some_and(|event| event.at < cutoff) {
            state.events.pop_front();
        }
        if state.events.len() == state.config.events as usize {
            let old = state.events.pop_front().unwrap();
            state.lost_before = Some(state.lost_before.map_or(old.at, |at| at.max(old.at)));
            state.dropped = state.dropped.saturating_add(1);
        }
        state.order = state.order.wrapping_add(1);
        let mut frame = [0; 3];
        frame[..bytes.len()].copy_from_slice(bytes);
        let event = Event {
            track,
            source,
            at,
            order: state.order,
            bytes: frame,
            length: bytes.len() as u8,
        };
        state.events.push_back(event);
        true
    }

    /// Mark a source disconnect without retargeting held input.
    /// Takes the source and original disconnect timestamp; preserves a visible boundary for every affected monitored track.
    pub(crate) fn disconnect(&self, source: u64, at: Instant) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.state.lock();
        if !state.config.enabled || at < state.origin {
            return;
        }
        let mut tracks = [false; super::session::MAX_TRACKS];
        for event in &state.events {
            if event.source == source {
                tracks[event.track as usize] = true;
            }
        }
        if !tracks.iter().any(|present| *present) {
            return;
        }
        state.watermark = state.watermark.max(at);
        for (track, present) in tracks.into_iter().enumerate() {
            if !present {
                continue;
            }
            if state.events.len() == state.config.events as usize {
                let old = state.events.pop_front().unwrap();
                state.lost_before = Some(state.lost_before.map_or(old.at, |at| at.max(old.at)));
                state.dropped = state.dropped.saturating_add(1);
            }
            state.order = state.order.wrapping_add(1);
            let event = Event {
                track: track as u8,
                source,
                at,
                order: state.order,
                bytes: [0; 3],
                length: 0,
            };
            state.events.push_back(event);
        }
        state.epoch = state.epoch.wrapping_add(1).max(1);
    }

    /// Copy one immutable review outside the audio renderer.
    /// Takes the review time; returns retained original timestamps and explicit loss information without saving input to disk.
    pub(crate) fn snapshot(&self, now: Instant) -> Snapshot {
        let state = self.state.lock();
        let cutoff = now
            .checked_sub(Duration::from_secs(u64::from(state.config.seconds)))
            .unwrap_or(state.origin)
            .max(state.origin);
        let mut events: Vec<_> = state
            .events
            .iter()
            .copied()
            .filter(|e| e.at >= cutoff && e.at <= now)
            .collect();
        events.sort_by_key(|e| (e.at, e.order));
        Snapshot {
            config: state.config,
            epoch: state.epoch,
            captured_at: now,
            origin: state.origin.max(cutoff),
            events,
            dropped: state.dropped,
            lost_before: state.lost_before,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Choice {
    pub track: u8,
    pub source: u64,
    pub from_seconds_ago: f64,
    pub to_seconds_ago: f64,
    pub tempo: f64,
    pub loop_beats: f64,
}
#[derive(Debug)]
pub(crate) struct Prepared {
    pub epoch: u64,
    pub content: Content,
    pub clipped_at_start: usize,
    pub held_at_end: usize,
    pub short_gates_extended: usize,
}
#[derive(Clone, Copy)]
struct Held {
    tick: u64,
    velocity: u8,
    order: u32,
    clipped: bool,
}

/// Convert a reviewed recent performance to ordinary editable MIDI.
/// Takes a frozen input range, explicit source/tempo/loop and cancellation; returns exact note/control ordering or a complete refusal.
pub(crate) fn prepare(
    snapshot: &Snapshot,
    choice: Choice,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    if !snapshot.config.enabled
        || choice.source == 0
        || usize::from(choice.track) >= super::session::MAX_TRACKS
        || !choice.from_seconds_ago.is_finite()
        || !choice.to_seconds_ago.is_finite()
        || choice.to_seconds_ago < 0.0
        || choice.from_seconds_ago <= choice.to_seconds_ago
        || choice.from_seconds_ago > f64::from(snapshot.config.seconds)
        || !choice.tempo.is_finite()
        || !(30.0..=300.0).contains(&choice.tempo)
        || !choice.loop_beats.is_finite()
        || !(0.25..=4096.0).contains(&choice.loop_beats)
    {
        return Err(
            "Choose an enabled retained range, one source, tempo 30–300 and a finite capture loop"
                .into(),
        );
    }
    let start = snapshot
        .captured_at
        .checked_sub(Duration::from_secs_f64(choice.from_seconds_ago))
        .ok_or("Capture start precedes the monotonic clock")?;
    let end = snapshot
        .captured_at
        .checked_sub(Duration::from_secs_f64(choice.to_seconds_ago))
        .ok_or("Capture end precedes the monotonic clock")?;
    if start < snapshot.origin || snapshot.lost_before.is_some_and(|lost| start <= lost) {
        return Err(
            "This range crosses discarded MIDI history; choose a later complete range".into(),
        );
    }
    let duration_beats = end.duration_since(start).as_secs_f64() * choice.tempo / 60.0;
    if duration_beats > choice.loop_beats + 1e-9 {
        return Err(
            "The capture loop is shorter than the chosen range; review tempo, range or loop length"
                .into(),
        );
    }
    let end_tick = (choice.loop_beats * f64::from(PPQN)).round() as u64;
    let capture_end = ((duration_beats * f64::from(PPQN)).round() as u64).clamp(1, end_tick);
    let mut held = BTreeMap::<(u8, u8), Held>::new();
    let mut controls = Vec::<Event>::new();
    let events: Vec<_> = snapshot
        .events
        .iter()
        .filter(|e| e.track == choice.track && e.source == choice.source && e.at <= end)
        .collect();
    let mut serial = 0u32;
    for (index, event) in events.iter().enumerate().take_while(|(_, e)| e.at < start) {
        if index & 63 == 0 && cancel.load(Ordering::Acquire) {
            return Err("Recent MIDI capture cancelled".into());
        }
        if event.length == 0 {
            held.clear();
            controls.clear();
            continue;
        }
        let status = event.bytes[0] & 0xf0;
        let key = (event.bytes[0] & 15, event.bytes[1]);
        match status {
            0x90 if event.bytes[2] != 0 => {
                if held
                    .insert(
                        key,
                        Held {
                            tick: 0,
                            velocity: event.bytes[2],
                            order: 0,
                            clipped: true,
                        },
                    )
                    .is_some()
                {
                    return Err(
                        "Overlapping same-channel same-pitch input cannot be paired safely".into(),
                    );
                }
            }
            0x80 | 0x90 => {
                held.remove(&key);
            }
            _ => {
                controls.push(**event);
            }
        }
    }
    let mut messages = Vec::new();
    for event in &controls {
        serial += 1;
        messages.push(Message {
            tick: 0,
            order: serial,
            bytes: event.bytes,
            length: event.length,
        });
    }
    for gate in held.values_mut() {
        serial += 1;
        gate.order = serial;
    }
    let mut notes = Vec::new();
    let mut clipped_at_start = 0;
    let mut short_gates_extended = 0;
    let finish = |key: (u8, u8),
                  gate: Held,
                  tick: u64,
                  release: u8,
                  order: u32,
                  notes: &mut Vec<MidiNote>,
                  clipped: &mut usize,
                  short: &mut usize|
     -> Result<(), String> {
        if notes.len() >= super::project::MAX_NOTES_PER_CLIP {
            return Err("Recent MIDI capture exceeds the editable note limit".into());
        }
        if gate.clipped {
            *clipped += 1;
        }
        if tick <= gate.tick {
            *short += 1;
        }
        let duration = tick.max(gate.tick + 1) - gate.tick;
        notes.push(MidiNote::from_smf(
            &smf::Note {
                channel: key.0,
                pitch: key.1,
                velocity: gate.velocity,
                release_velocity: release,
                start_tick: gate.tick,
                duration_ticks: duration,
                start_order: gate.order,
                end_order: order,
            },
            PPQN,
        )?);
        Ok(())
    };
    for (index, event) in events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.at >= start && e.at < end)
    {
        if index & 63 == 0 && cancel.load(Ordering::Acquire) {
            return Err("Recent MIDI capture cancelled".into());
        }
        if event.length == 0 {
            return Err(
                "The selected performance crosses an input disconnect; choose one connected range"
                    .into(),
            );
        }
        let tick = ((event.at.duration_since(start).as_secs_f64() * choice.tempo / 60.0
            * f64::from(PPQN))
        .round() as u64)
            .min(capture_end - 1);
        serial += 1;
        let status = event.bytes[0] & 0xf0;
        let key = (event.bytes[0] & 15, event.bytes[1]);
        match status {
            0x90 if event.bytes[2] != 0 => {
                if held
                    .insert(
                        key,
                        Held {
                            tick,
                            velocity: event.bytes[2],
                            order: serial,
                            clipped: false,
                        },
                    )
                    .is_some()
                {
                    return Err(
                        "Overlapping same-channel same-pitch input cannot be paired safely".into(),
                    );
                }
            }
            0x80 | 0x90 => {
                let gate = held
                    .remove(&key)
                    .ok_or("A note release has no retained onset; choose a later complete range")?;
                finish(
                    key,
                    gate,
                    tick,
                    event.bytes[2],
                    serial,
                    &mut notes,
                    &mut clipped_at_start,
                    &mut short_gates_extended,
                )?;
            }
            _ => messages.push(Message {
                tick,
                order: serial,
                bytes: event.bytes,
                length: event.length,
            }),
        }
    }
    let held_at_end = held.len();
    for (key, gate) in held {
        serial += 1;
        finish(
            key,
            gate,
            capture_end,
            0,
            serial,
            &mut notes,
            &mut clipped_at_start,
            &mut short_gates_extended,
        )?;
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Recent MIDI capture cancelled".into());
    }
    if notes.is_empty() {
        return Err("The selected range contains no retained note performance".into());
    }
    notes.sort_by_key(|n| {
        (
            n.source_timing.unwrap().start,
            n.source_timing.unwrap().start_order,
        )
    });
    let wire = smf::File {
        format: smf::Format::Single,
        ppqn: PPQN,
        tracks: vec![smf::Track {
            end_tick,
            notes: notes
                .iter()
                .map(|n| {
                    let t = n.source_timing.unwrap();
                    smf::Note {
                        channel: n.channel,
                        pitch: n.pitch,
                        velocity: n.vel,
                        release_velocity: n.release_vel,
                        start_tick: t.start,
                        duration_ticks: t.duration,
                        start_order: t.start_order,
                        end_order: t.end_order,
                    }
                })
                .collect(),
            messages: messages.clone(),
            meta: vec![],
        }],
        warnings: vec![],
    };
    smf::encode(&wire, false)
        .map_err(|e| format!("Recent MIDI capture cannot be exported: {e}"))?;
    Ok(Prepared {
        epoch: snapshot.epoch,
        content: Content {
            notes,
            ppqn: PPQN,
            end_tick,
            messages,
            meta: vec![],
            labels: vec![],
        },
        clipped_at_start,
        held_at_end,
        short_gates_extended,
    })
}

#[cfg(test)]
mod tests;
