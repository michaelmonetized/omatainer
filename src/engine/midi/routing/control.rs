//! Shared routing admission. Only management/input workers take the routing
//! mutex. Raw MIDI and audio callbacks read atomics and copy bounded packets.
use super::{packet::Packet, Endpoint, Routing};
use crate::engine::{Command, CommandPort, TRACKS};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::*};
use std::sync::Arc;

const OUTPUT_EVENTS: usize = 2048;
const MAX_SOURCES: usize = 256;
pub(super) const MAX_PEDALS: usize = MAX_SOURCES * TRACKS * 16 + TRACKS * 16 + 16;
#[derive(Clone, Copy, Debug)]
pub(super) enum Clear {
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
            last_status: self.last_status.load(Relaxed),
            last_channel: self.last_channel.load(Relaxed),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Summary {
    pub enabled: bool,
    pub generation: u64,
    pub activity: ([Activity; TRACKS], Activity),
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
    pub(super) live: Mutex<Live>,
    pub generation: AtomicU64,
    pub explicit: AtomicBool,
    pub(super) mask: AtomicU8,
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
            live: Mutex::new(Live {
                config: Arc::new(Routing::default()),
                sources: Vec::with_capacity(MAX_SOURCES),
                actual_outputs: Vec::with_capacity(TRACKS),
            }),
            generation: AtomicU64::new(1),
            explicit: AtomicBool::new(false),
            mask: AtomicU8::new(0),
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
    pub(crate) fn output_state(&self) -> (u8, u64, u64) {
        (
            self.mask.load(Acquire),
            self.generation.load(Acquire),
            self.output_epoch.load(Acquire),
        )
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
            activity: self.activity(),
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
