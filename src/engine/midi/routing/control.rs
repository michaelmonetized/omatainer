//! Shared routing admission. Only management/input workers take the routing
//! mutex. Raw MIDI and audio callbacks read atomics and copy bounded packets.
use super::{packet::Packet, Endpoint, Routing};
use crate::engine::{Command, CommandPort, session::MAX_TRACKS as TRACKS};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::*};
use std::sync::Arc;

const OUTPUT_EVENTS: usize = 2048;
const MAX_SOURCES: usize = 256;
pub(super) const MAX_PEDALS: usize = 65536;
#[derive(Clone, Copy, Debug)]
pub(super) enum Clear {
    Track,
    Clip,
    Source(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Owner {
    Raw,
    Live {
        source: u64,
        channel: u8,
    },
    Clip {
        track: u8,
        note: crate::engine::midi_edit::NoteId,
    },
    ClipLane(u8),
}
#[derive(Clone, Copy, Debug)]
pub(super) struct OutputEvent {
    pub generation: u64,
    pub epoch: u64,
    pub track: u8,
    pub packet: Packet,
    pub owner: Owner,
    pub clear: Option<Clear>,
    pub weight: u32,
}
#[derive(Default)]
pub(super) struct Counts {
    pub received: AtomicU64,
    pub routed: AtomicU64,
    pub filtered: AtomicU64,
    pub sent: AtomicU64,
    pub failed: AtomicU64,
    pub overruns: AtomicU64,
    pub clip_refused: AtomicBool,
    pub identity_refused: AtomicBool,
    pub malformed: AtomicU64,
    pub last_status: AtomicU8,
    pub last_channel: AtomicU8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Activity {
    pub received: u64,
    pub routed: u64,
    pub filtered: u64,
    pub sent: u64,
    pub failed: u64,
    pub malformed: u64,
    pub overruns: u64,
    pub clip_refused: bool,
    pub identity_refused: bool,
    pub last_status: u8,
    pub last_channel: u8,
}
impl Counts {
    fn snapshot(&self) -> Activity {
        Activity {
            received: self.received.load(Relaxed),
            routed: self.routed.load(Relaxed),
            filtered: self.filtered.load(Relaxed),
            sent: self.sent.load(Relaxed),
            failed: self.failed.load(Relaxed),
            malformed: self.malformed.load(Relaxed),
            overruns: self.overruns.load(Relaxed),
            clip_refused: self.clip_refused.load(Relaxed),
            identity_refused: self.identity_refused.load(Relaxed),
            last_status: self.last_status.load(Relaxed),
            last_channel: self.last_channel.load(Relaxed),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Summary {
    pub enabled: bool,
    pub generation: u64,
    pub received: u64,
    pub sent: u64,
    pub failed: u64,
    pub overruns: u64,
    pub refused_tracks: u8,
    pub refused_tracks_extended: [u64; 2],
}
#[derive(Clone, Copy)]
pub(crate) struct Sources {
    pub native: u64,
    pub tracks: [u64; TRACKS],
}
pub(super) struct SourceEntry {
    pub sources: Sources,
    pub endpoint: Endpoint,
}
pub(super) struct Live {
    pub config: Arc<Routing>,
    pub sources: Vec<SourceEntry>,
    pub actual_outputs: Vec<Endpoint>,
}
pub(crate) struct Shared {
    #[cfg(test)]
    pub(super) input_after_target: Mutex<Option<(crossbeam_channel::Sender<()>, crossbeam_channel::Receiver<()>)>>,
    pub(crate) identity: crate::engine::session::Registry,
    bound: [[AtomicU64; 3]; TRACKS],
    pub(super) live: Mutex<Live>,
    pub generation: AtomicU64,
    pub explicit: AtomicBool,
    pub(super) mask: super::mask::AtomicMask,
    pub(super) output_epoch: AtomicU64,
    pub(super) alive: AtomicBool,
    pub(super) counts: [Counts; TRACKS],
    pub(super) global: Counts,
    pub(super) output: crossbeam_channel::Sender<OutputEvent>,
    pub(super) receiver: Mutex<Option<crossbeam_channel::Receiver<OutputEvent>>>,
}
impl Default for Shared {
    fn default() -> Self {
        let (output, receiver) = crossbeam_channel::bounded(OUTPUT_EVENTS);
        Self {
            #[cfg(test)]
            input_after_target: Mutex::new(None),
            identity: crate::engine::session::Registry::default(),
            bound: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            live: Mutex::new(Live {
                config: Arc::new(Routing::default()),
                sources: Vec::with_capacity(MAX_SOURCES),
                actual_outputs: Vec::with_capacity(TRACKS),
            }),
            generation: AtomicU64::new(1),
            explicit: AtomicBool::new(false),
            mask: super::mask::AtomicMask::new(0),
            output_epoch: AtomicU64::new(1),
            alive: AtomicBool::new(false),
            counts: std::array::from_fn(|_| Counts::default()),
            global: Counts::default(),
            output,
            receiver: Mutex::new(Some(receiver)),
        }
    }
}
impl Shared {
    pub(super) fn bind_identity(&self, config: &Routing) {
        for route in &config.routes {
            let slot = usize::from(route.track);
            let reference = self.identity.reference(crate::engine::session::Axis::Track, slot);
            let words = reference.map_or([0;3], |r| [r.namespace[0], r.namespace[1], r.id.0]);
            for (word, value) in self.bound[slot].iter().zip(words) { word.store(value, Release); }
            self.counts[slot].identity_refused.store(self.identity.known() && reference.is_none(), Relaxed);
        }
    }
    pub(super) fn current_target(&self, track: u8) -> Result<Option<crate::engine::session::Reference>, ()> {
        if !self.identity.known() { return Ok(None); }
        let slot = usize::from(track);
        let current = self.identity.reference(crate::engine::session::Axis::Track, slot);
        let bound = crate::engine::session::Reference {namespace:[self.bound[slot][0].load(Acquire),self.bound[slot][1].load(Acquire)],id:crate::engine::session::Id(self.bound[slot][2].load(Acquire))};
        let valid = current == Some(bound);
        self.counts[slot].identity_refused.store(!valid, Relaxed);
        if valid {Ok(Some(bound))} else {Err(())}
    }
    pub(super) fn target_current(&self, track: u8) -> bool {
        self.current_target(track).is_ok()
    }
    pub(crate) fn output_state(&self) -> (u128, u64, u64) {
        let before = self.generation.load(Acquire);
        let mask = self.mask.load(Acquire);
        let epoch = self.output_epoch.load(Acquire);
        let after = self.generation.load(Acquire);
        (if before == after && after != 0 {mask} else {0}, after, epoch)
    }
    pub(crate) fn overrun(&self, track: usize) {
        self.counts[track].overruns.fetch_add(1, Relaxed);
        self.counts[track].failed.fetch_add(1, Relaxed);
        self.reset_outputs();
    }
    pub fn activity(&self) -> ([Activity; TRACKS], Activity) {
        (
            std::array::from_fn(|i| self.counts[i].snapshot()),
            self.global.snapshot(),
        )
    }
    pub(crate) fn summary(&self) -> Summary {
        Summary {
            enabled: self.explicit.load(Acquire),
            generation: self.generation.load(Acquire),
            received: self.global.received.load(Relaxed),
            sent: self
                .counts
                .iter()
                .fold(0u64, |total, c| total.saturating_add(c.sent.load(Relaxed))),
            failed: self.counts.iter().fold(0u64, |total, c| {
                total.saturating_add(c.failed.load(Relaxed))
            }),
            overruns: self.counts.iter().fold(0u64, |total, c| {
                total.saturating_add(c.overruns.load(Relaxed))
            }),
            refused_tracks_extended: std::array::from_fn(|word| self.counts[word*64..(word+1)*64].iter().enumerate().fold(0, |mask, (bit, count)| mask | (u64::from(count.clip_refused.load(Relaxed)) << bit))),
            refused_tracks: self.counts.iter().take(8).enumerate().fold(0, |mask, (track, c)| {
                if c.clip_refused.load(Relaxed) {
                    mask | (1 << track)
                } else {
                    mask
                }
            }),
        }
    }
    #[cfg(test)]
    pub(crate) fn maximum_activity_for_test(&self) {
        self.explicit.store(true, Release);
        self.generation.store(u64::MAX, Release);
        self.global.received.store(u64::MAX, Relaxed);
        for counts in &self.counts {
            counts.sent.store(u64::MAX, Relaxed);
            counts.failed.store(u64::MAX, Relaxed);
            counts.overruns.store(u64::MAX, Relaxed);
            counts.clip_refused.store(true, Relaxed);
        }
    }
    pub fn config(&self) -> Arc<Routing> {
        self.live.lock().config.clone()
    }
    pub(crate) fn register(
        &self,
        source: u64,
        name: &str,
        id: &str,
    ) -> Result<Sources, std::io::Error> {
        let mut live = self.live.lock();
        if live.sources.len() >= MAX_SOURCES {
            return Err(std::io::Error::other(
                "MIDI routing supports at most256 simultaneous input sources",
            ));
        }
        let endpoint = Endpoint {
            name: name.into(),
            id: Some(id.into()),
        };
        endpoint.validate().map_err(std::io::Error::other)?;
        if live
            .actual_outputs
            .iter()
            .any(|output| output.conflicts(&endpoint))
        {
            return Err(std::io::Error::other(
                "Feedback guard: this input device/port is an active output destination",
            ));
        }
        let sources = Sources {
            native: source,
            tracks: std::array::from_fn(|_| super::super::next_source_id()),
        };
        if source == 0 || sources.tracks.contains(&0) {
            return Err(std::io::Error::other(
                "MIDI source identity space exhausted",
            ));
        }
        live.sources.push(SourceEntry { sources, endpoint });
        Ok(sources)
    }
    pub(crate) fn unregister(&self, native: u64) {
        self.live
            .lock()
            .sources
            .retain(|s| s.sources.native != native);
    }
    pub(crate) fn release(&self, sources: Sources, cmd: &CommandPort) {
        cmd.release_midi_source(sources.native);
        for (track, source) in sources.tracks.into_iter().enumerate() {
            cmd.release_midi_source(source);
            self.emit_event(
                track as u8,
                Packet::new(&[0xb0, 123, 0]).unwrap(),
                Owner::Raw,
                Some(Clear::Source(source)),
                0,
            );
        }
    }
    /// Audio/input worker producer. Overflow invalidates every earlier queued
    /// gate before the output owner sends all-notes-off/reset; never overwrite.
    pub(crate) fn emit(&self, track: u8, packet: Packet) -> bool {
        self.emit_event(track, packet, Owner::Raw, None, 1)
    }
    pub(crate) fn emit_owned(&self, track: u8, packet: Packet, owner: Owner) -> bool {
        self.emit_event(track, packet, owner, None, 1)
    }
    pub(crate) fn clip_refused(&self, track: usize, refused: bool) {
        self.counts[track].clip_refused.store(refused, Relaxed);
    }
    pub(crate) fn emit_weighted(
        &self,
        track: u8,
        packet: Packet,
        owner: Owner,
        weight: u32,
        state: (u64, u64),
    ) -> bool {
        self.enqueue(OutputEvent {
            generation: state.0,
            epoch: state.1,
            track,
            packet,
            owner,
            clear: None,
            weight,
        })
    }
    pub(crate) fn clear_track(&self, track: u8) -> bool {
        self.emit_event(track, Packet::new(&[0xb0, 123, 0]).unwrap(), Owner::Raw, Some(Clear::Track), 0)
    }
    pub(crate) fn clear_clip(&self, track: u8) -> bool {
        self.emit_event(
            track,
            Packet::new(&[0xb0, 123, 0]).unwrap(),
            Owner::Raw,
            Some(Clear::Clip),
            0,
        )
    }
    fn emit_event(
        &self,
        track: u8,
        packet: Packet,
        owner: Owner,
        clear: Option<Clear>,
        weight: u32,
    ) -> bool {
        self.enqueue(OutputEvent {
            generation: self.generation.load(Acquire),
            epoch: self.output_epoch.load(Acquire),
            track,
            packet,
            owner,
            clear,
            weight,
        })
    }
    fn enqueue(&self, event: OutputEvent) -> bool {
        if usize::from(event.track) >= TRACKS
            || self.mask.load(Acquire) & (1 << event.track) == 0
            || event.clear.is_none() && !self.target_current(event.track)
            || !self.alive.load(Acquire)
            || event.generation == 0
            || event.generation != self.generation.load(Acquire)
            || event.epoch != self.output_epoch.load(Acquire)
        {
            return false;
        }
        if self.output.try_send(event).is_err() {
            self.overrun(usize::from(event.track));
            false
        } else {
            true
        }
    }
    pub(crate) fn reset_outputs(&self) {
        self.output_epoch.fetch_add(1, AcqRel);
    }
    pub(crate) fn input(
        &self,
        sources: Sources,
        name: &str,
        id: &str,
        generation: u64,
        packet: Packet,
        cmd: &CommandPort,
        controller: impl FnOnce(bool),
    ) {
        // Serialize dispatch with route publication. A worker cannot enqueue an
        // old onset after that publication's guaranteed source releases.
        let live = self.live.lock();
        self.global.received.fetch_add(1, Relaxed);
        if generation == 0 || generation != self.generation.load(Acquire) {
            self.global.filtered.fetch_add(1, Relaxed);
            return;
        }
        if cmd.performance().status().recovery && !packet.note().is_some_and(|(_, _, on)| !on) {
            self.global.filtered.fetch_add(1, Relaxed);
            return;
        }
        controller(!live.config.enabled);
        if !live.config.enabled {
            return;
        }
        for route in &live.config.routes {
            if !route.inputs.iter().any(|i| {
                i.port.matches(name, id)
                    && packet
                        .channel()
                        .is_none_or(|ch| i.channels & (1 << ch) != 0)
            }) {
                continue;
            }
            let counts = &self.counts[usize::from(route.track)];
            counts.received.fetch_add(1, Relaxed);
            counts.last_status.store(packet.bytes()[0], Relaxed);
            counts
                .last_channel
                .store(packet.channel().map_or(0, |ch| ch + 1), Relaxed);
            if !route.filter.accepts(packet.bytes()) {
                counts.filtered.fetch_add(1, Relaxed);
                continue;
            }
            let target = match self.current_target(route.track) {
                Ok(target) => target,
                Err(()) => {
                if route.monitor { if let Some((note, _, false)) = packet.note() { let _ = cmd.send(Command::LiveNoteOff { source: sources.tracks[usize::from(route.track)], ch: packet.channel().unwrap(), note }); } }
                counts.filtered.fetch_add(1, Relaxed); continue;
                }
            };
            // Keep this exact configured reference through enqueue. A second
            // registry lookup can observe a replacement and retarget an onset.
            #[cfg(test)]
            if let Some((entered, resume)) = self.input_after_target.lock().take() {
                let _ = entered.send(()); let _ = resume.recv_timeout(std::time::Duration::from_secs(10));
            }
            // Controller maps remain separate; only an explicit route supplies
            // an instrument destination. Each track has its own physical key.
            if route.monitor {
                if let Some((note, vel, on)) = packet.note() {
                    let source = sources.tracks[usize::from(route.track)];
                    let ch = packet.channel().unwrap();
                    let command = if on {
                        Command::RoutedNoteOn {
                            source,
                            ch,
                            note,
                            vel,
                            track: route.track,
                            target,
                        }
                    } else {
                        Command::LiveNoteOff { source, ch, note }
                    };
                    if cmd.send(command).is_err() {
                        counts.failed.fetch_add(1, Relaxed);
                        continue;
                    }
                }
            }
            counts.routed.fetch_add(1, Relaxed);
            if route.thru {
                self.emit_owned(
                    route.track,
                    packet,
                    Owner::Live {
                        source: sources.tracks[usize::from(route.track)],
                        channel: packet.channel().unwrap_or(0),
                    },
                );
            }
        }
    }
    pub(crate) fn malformed(&self) {
        self.global.malformed.fetch_add(1, Relaxed);
    }
}
