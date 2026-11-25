//! MIDI 1.0 MPE zones and source-owned voice expression.
use std::sync::atomic::{
    AtomicBool, AtomicU64,
    Ordering::{AcqRel, Acquire, Release},
};

mod index;
mod drum;
pub(crate) use drum::Drum;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) use index::Index;
pub(crate) mod envelope;

const SOURCES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Values {
    pub pitch: f32,
    pub pressure: u8,
    pub timbre: u8,
}
impl Default for Values {
    fn default() -> Self {
        Self {
            pitch: 0.0,
            pressure: 0,
            timbre: 64,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Note {
    pub source: u64,
    pub channel: u8,
    pub manager: u8,
    pub held: bool,
    pub member: Values,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Channel {
    bend: u16,
    range: u8,
    pressure: u8,
    timbre: u8,
    sustain: bool,
    rpn: [u8; 2],
}
impl Default for Channel {
    fn default() -> Self {
        Self {
            bend: 8192,
            range: 2,
            pressure: 0,
            timbre: 64,
            sustain: false,
            rpn: [127; 2],
        }
    }
}
impl Channel {
    fn values(self) -> Values {
        Values {
            pitch: (f32::from(self.bend) - 8192.0) / 8192.0 * f32::from(self.range),
            pressure: self.pressure,
            timbre: self.timbre,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Update {
    pub reset: u16,
    pub silence: u16,
    pub release: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct State {
    lower: u8,
    upper: u8,
    channels: [Channel; 16],
}
impl State {
    /// Find a channel's zone.
    /// Takes a wire channel; returns its manager and whether it is a member.
    pub(crate) fn role(&self, channel: u8) -> Option<(u8, bool)> {
        if self.lower > 0 && channel <= self.lower {
            Some((0, channel != 0))
        } else if self.upper > 0 && channel >= 15 - self.upper {
            Some((15, channel != 15))
        } else {
            None
        }
    }
    fn zone(&self, manager: u8) -> u16 {
        (0..16).fold(0, |mask, channel| {
            mask | if self.role(channel).is_some_and(|role| role.0 == manager) {
                1 << channel
            } else {
                0
            }
        })
    }
    pub(crate) fn note(&self, source: u64, channel: u8) -> Option<Note> {
        let (manager, member) = self.role(channel)?;
        member.then(|| Note {
            source,
            channel,
            manager,
            held: true,
            member: self.channels[usize::from(channel)].values(),
        })
    }
    pub(crate) fn member(&self, channel: u8) -> Values {
        self.channels[usize::from(channel)].values()
    }
    pub(crate) fn range(&self, channel: u8) -> u8 {
        self.channels[usize::from(channel)].range
    }
    pub(crate) fn member_packets(&self, channel: u8) -> [[u8; 3]; 3] {
        let control = self.channels[usize::from(channel)];
        [
            [
                0xe0 | channel,
                (control.bend & 127) as u8,
                (control.bend >> 7) as u8,
            ],
            [0xd0 | channel, control.pressure, 0],
            [0xb0 | channel, 74, control.timbre],
        ]
    }
    pub(crate) fn sustain(&self, manager: u8) -> bool {
        self.channels[usize::from(manager)].sustain
    }
    pub(crate) fn combined(&self, note: Note) -> Values {
        let manager = self.channels[usize::from(note.manager)].values();
        Values {
            pitch: manager.pitch + note.member.pitch,
            pressure: manager.pressure.max(note.member.pressure),
            timbre: (i16::from(manager.timbre) + i16::from(note.member.timbre) - 64).clamp(0, 127)
                as u8,
        }
    }
    fn configure(&mut self, manager: u8, members: u8) -> Update {
        let old = *self;
        if manager == 0 {
            self.lower = members;
            if members > 0 {
                self.upper = self.upper.min(14u8.saturating_sub(members));
            }
        } else {
            self.upper = members;
            if members > 0 {
                self.lower = self.lower.min(14u8.saturating_sub(members));
            }
        }
        let mut reset = 0;
        for channel in 0..16 {
            if old.role(channel) != self.role(channel) {
                reset |= 1 << channel;
                self.channels[usize::from(channel)] = Channel::default();
            }
            if self.role(channel).is_some_and(|role| role.0 == manager) {
                self.channels[usize::from(channel)].range = if channel == manager { 2 } else { 48 };
            }
        }
        Update {
            reset,
            ..Update::default()
        }
    }
    /// Consume one complete channel message.
    /// Takes up to three validated MIDI bytes; updates zones and controls and returns channels whose voices must end.
    pub(crate) fn input(&mut self, bytes: &[u8]) -> Update {
        if !valid(bytes) {
            return Update::default();
        }
        let channel = bytes[0] & 15;
        let role = self.role(channel);
        let control = &mut self.channels[usize::from(channel)];
        match bytes[0] & 0xf0 {
            0xe0 => control.bend = u16::from(bytes[1]) | (u16::from(bytes[2]) << 7),
            0xd0 => control.pressure = bytes[1],
            0xb0 => match bytes[1] {
                101 => control.rpn[0] = bytes[2],
                100 => control.rpn[1] = bytes[2],
                98 | 99 => control.rpn = [127; 2],
                6 if control.rpn == [0, 6] && matches!(channel, 0 | 15) && bytes[2] <= 15 => {
                    return self.configure(channel, bytes[2])
                }
                6 if control.rpn == [0, 0] && bytes[2] <= 96 => {
                    control.range = bytes[2];
                    if let Some((manager, true)) = role {
                        for member in 0..16 {
                            if self.role(member) == Some((manager, true)) {
                                self.channels[usize::from(member)].range = bytes[2];
                            }
                        }
                    }
                }
                74 => control.timbre = bytes[2],
                64 if role.is_some_and(|role| !role.1) => control.sustain = bytes[2] >= 64,
                120 | 123 => {
                    let mask = role
                        .filter(|role| !role.1)
                        .map_or(1 << channel, |role| self.zone(role.0));
                    return if bytes[1] == 120 {
                        Update {
                            silence: mask,
                            ..Update::default()
                        }
                    } else {
                        Update {
                            release: mask,
                            ..Update::default()
                        }
                    };
                }
                121 => {
                    let mask = role
                        .filter(|role| !role.1)
                        .map_or(1 << channel, |role| self.zone(role.0));
                    for index in 0..16 {
                        if mask & (1 << index) != 0 {
                            let range = self.channels[index].range;
                            self.channels[index] = Channel {
                                range,
                                ..Channel::default()
                            };
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Update::default()
    }
}

/// Check one expression packet.
/// Takes wire bytes; returns whether a complete CC, bend or channel-pressure message has legal data bytes.
pub(crate) fn valid(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|status| match status & 0xf0 {
        0xb0 | 0xe0 => bytes.len() == 3,
        0xd0 => bytes.len() == 2,
        _ => false,
    }) && bytes[1..].iter().all(|value| *value <= 127)
}

pub(crate) struct Sources {
    entries: Box<[(u64, State)]>,
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            entries: vec![(0, State::default()); SOURCES].into_boxed_slice(),
        }
    }
}
impl Sources {
    pub(crate) fn get(&self, source: u64) -> Option<&State> {
        self.entries
            .iter()
            .find(|entry| source != 0 && entry.0 == source)
            .map(|entry| &entry.1)
    }
    pub(crate) fn input(&mut self, source: u64, bytes: &[u8]) -> Option<(State, Update)> {
        if source == 0 || !valid(bytes) {
            return None;
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.0 == source)
            .or_else(|| self.entries.iter().position(|entry| entry.0 == 0))?;
        self.entries[index].0 = source;
        let update = self.entries[index].1.input(bytes);
        Some((self.entries[index].1, update))
    }
    pub(crate) fn retire(&mut self, source: u64) {
        for entry in &mut self.entries {
            if source == 0 || entry.0 == source {
                *entry = (0, State::default());
            }
        }
    }
}

pub(crate) struct Resets {
    sources: Box<[AtomicU64]>,
    forgotten: Box<[AtomicU64]>,
    all: AtomicBool,
    forget_all: AtomicBool,
}
impl Default for Resets {
    fn default() -> Self {
        Self {
            sources: (0..SOURCES).map(|_| AtomicU64::new(0)).collect(),
            forgotten: (0..SOURCES).map(|_| AtomicU64::new(0)).collect(),
            all: AtomicBool::new(false),
            forget_all: AtomicBool::new(false),
        }
    }
}
impl Resets {
    pub(crate) fn forget(&self, source: u64) {
        self.request(source);
        if source == 0 {
            return;
        }
        for pending in &self.forgotten {
            if pending.load(Acquire) == source
                || pending.compare_exchange(0, source, AcqRel, Acquire).is_ok()
            {
                return;
            }
        }
        self.forget_all.store(true, Release);
    }
    fn take_forgotten(&self, index: usize) -> u64 {
        self.forgotten[index].swap(0, AcqRel)
    }
    /// Retire input expression through a separate release path.
    /// Takes a native or routed source; publishes a bounded reset even when the musical command queue is full.
    pub(crate) fn request(&self, source: u64) {
        if source == 0 {
            return;
        }
        for pending in &self.sources {
            if pending.load(Acquire) == source
                || pending.compare_exchange(0, source, AcqRel, Acquire).is_ok()
            {
                return;
            }
        }
        self.all.store(true, Release);
    }
    pub(crate) fn take_all(&self) -> bool {
        self.all.swap(false, AcqRel)
    }
    pub(crate) fn take(&self, index: usize) -> u64 {
        self.sources[index].swap(0, AcqRel)
    }
    pub(crate) fn len(&self) -> usize {
        self.sources.len()
    }
}

impl super::RtEngine {
    pub(super) fn expression_clip_packet(
        &mut self,
        track: usize,
        packet: super::midi::routing::packet::Packet,
        owner: super::midi::routing::Owner,
    ) {
        let bytes = packet.bytes();
        if valid(bytes) {
            let update = self.tracks[track].expression_clip.input(bytes);
            self.plugin_midi.expression_update(0, Some(track), &self.tracks[track].expression_clip, update);
            let track = &mut self.tracks[track];
            track
                .poly
                .expression_input(0, &track.expression_clip, update);
            for voice in &mut track.drum_pos {
                if voice.as_mut().and_then(|voice|voice.expression.as_mut()).is_some_and(|voice|voice.update(0,&track.expression_clip,update,self.sr)) { *voice=None; }
            }
        } else if let super::midi::routing::Owner::Clip { note, .. } = owner {
            if let Some((pitch, velocity, on)) = packet.note() {
                if on {
                    if self.plugin_midi.mask & (1 << track) != 0 {
                        self.plugin_midi.expression_on(super::plugin_midi::ExpressionOwner::Clip { note, track }, track, packet.channel().unwrap(), pitch, velocity, &self.tracks[track].expression_clip);
                    }
                    let gain = self.tracks[track].playing.map_or(1.0, |playing| {
                        self.tracks[track].clips[usize::from(playing.scene)].gain
                    });
                    if self.tracks[track].kind == 0 {
                        let state=self.tracks[track].expression_clip;
                        self.trig_drum_expression(track,pitch,f32::from(velocity)/127.0,gain,Drum::new(&state,0,packet.channel().unwrap(),None,Some(note),self.sr));
                    } else {
                        let track = &mut self.tracks[track];
                        track.poly.note_on_clip_expressive(
                            pitch,
                            f32::from(velocity) / 127.0,
                            gain,
                            note,
                            packet.channel().unwrap(),
                            &track.expression_clip,
                        );
                    }
                } else {
                    self.tracks[track].poly.note_off_clip_id(note);
                    self.plugin_midi.expression_off_clip(track, note);
                    for voice in self.tracks[track].drum_pos.iter_mut().flatten() {if let Some(expression)=voice.expression.as_mut().filter(|expression|expression.clip==Some(note)) {expression.release();}}
                }
            }
        }
    }
    pub(super) fn expression_input(&mut self, source: u64, bytes: &[u8]) {
        let Some((state, update)) = self.expression.input(source, bytes) else {
            return;
        };
        self.plugin_midi.expression_update(source, None, &state, update);
        for track in &mut self.tracks {
            track.poly.expression_input(source, &state, update);
            for voice in &mut track.drum_pos {
                if voice.as_mut().and_then(|voice|voice.expression.as_mut()).is_some_and(|voice|voice.update(source,&state,update,self.sr)) { *voice=None; }
            }
        }
        self.sampler_poly.expression_input(source, &state, update);
    }
    pub(super) fn expression_releases(&mut self) {
        if self.expression_resets.forget_all.swap(false, AcqRel) {
            self.expression.retire(0);
        }
        if self.expression_resets.take_all() {
            self.expression_retire(0);
        }
        for index in 0..self.expression_resets.len() {
            let forgotten = self.expression_resets.take_forgotten(index);
            if forgotten != 0 {
                self.expression.retire(forgotten);
            }
            let source = self.expression_resets.take(index);
            if source != 0 {
                self.expression_retire(source);
            }
        }
    }
    fn expression_retire(&mut self, source: u64) {
        self.plugin_midi.expression_retire(source);
        for track in &mut self.tracks {
            track.poly.expression_retire(source);
            for voice in &mut track.drum_pos {if voice.as_ref().and_then(|voice|voice.expression.as_ref()).is_some_and(|voice|voice.owned(source)) {*voice=None;}}
        }
        self.sampler_poly.expression_retire(source);
    }
}
