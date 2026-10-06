use super::super::{
    midi::routing::{packet::Packet, Owner},
    RtEngine,
};
use super::*;

#[derive(Clone, Debug)]
struct Cursor {
    audio: usize,
    activity: usize,
    notes: usize,
    gate: usize,
    output: usize,
    chase_gate: usize,
    chase_output: usize,
    chase_control: usize,
    chase: Box<[([u8; 3], u8)]>,
    chase_len: usize,
    chase_ready: bool,
    output_notes: usize,
    held: [u16; 128],
    velocity: [u8; 128],
    gain: [f32; 128],
    arp: bool,
    arp_step: i64,
    arp_note: Option<u8>,
    dirty: bool,
    output_dirty: bool,
    output_refused: bool,
}
impl Default for Cursor {
    fn default() -> Self {
        Self {
            audio: 0,
            activity: 0,
            notes: 0,
            gate: 0,
            output: 0,
            chase_gate: 0,
            chase_output: 0,
            chase_control: 0,
            chase: Vec::new().into_boxed_slice(),
            chase_len: 0,
            chase_ready: false,
            output_notes: 0,
            held: [0; 128],
            velocity: [0; 128],
            gain: [0.0; 128],
            arp: false,
            arp_step: i64::MIN,
            arp_note: None,
            dirty: true,
            output_dirty: true,
            output_refused: false,
        }
    }
}
/// Advance an immutable song with fixed renderer cursors.
/// Owns bounded gate counts and seek state; never builds a collection or releases source PCM during rendering.
#[derive(Clone, Debug)]
pub(crate) struct Playback {
    pub plan: Option<Arc<Plan>>,
    cursors: Vec<Cursor>,
    seek: f64,
    output_seek: f64,
    output_state: (u64, u64),
}
impl Playback {
    pub(crate) fn new(plan: Option<Arc<Plan>>, beat: f64) -> Box<Self> {
        let cursors = (0..session::MAX_TRACKS)
            .map(|slot| {
                let mut c = Cursor::default();
                if plan
                    .as_ref()
                    .and_then(|p| p.tracks.get(slot))
                    .and_then(Option::as_ref)
                    .is_some_and(|t| !t.controls.is_empty())
                {
                    c.chase = vec![([0; 3], 0); 2048].into_boxed_slice();
                }
                c
            })
            .collect();
        let mut playback = Box::new(Self {
            plan,
            cursors,
            seek: beat,
            output_seek: beat,
            output_state: (0, 0),
        });
        playback.reset(beat);
        playback
    }
    pub(crate) fn output_refused(&self, slot: usize) -> bool {
        self.enabled() && self.cursors[slot].output_refused
    }
    pub(crate) fn enabled(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| p.model.enabled)
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.cursors.capacity() * std::mem::size_of::<Cursor>()
            + self
                .cursors
                .iter()
                .map(|c| c.chase.len() * std::mem::size_of::<([u8; 3], u8)>())
                .sum::<usize>()
    }
    pub(crate) fn reset(&mut self, beat: f64) {
        self.seek = beat;
        self.output_seek = beat;
        for (slot, cursor) in self.cursors.iter_mut().enumerate() {
            let chase = std::mem::take(&mut cursor.chase);
            *cursor = Cursor::default();
            cursor.chase = chase;
            if let Some(track) = self
                .plan
                .as_ref()
                .and_then(|p| p.tracks.get(slot))
                .and_then(Option::as_ref)
            {
                cursor.audio = segment(&track.audio_segments, beat);
                cursor.activity = segment(&track.activity, beat);
                cursor.notes = segment(&track.note_segments, beat);
                cursor.output_notes = cursor.notes;
                cursor.gate = track.gates.partition_point(|e| e.beat < beat);
                cursor.output = track.events.partition_point(|e| e.beat < beat);
            }
        }
    }
    fn current(&self, slot: usize, layout: &session::Layout) -> bool {
        self.plan
            .as_ref()
            .and_then(|p| p.tracks.get(slot))
            .and_then(Option::as_ref)
            .is_some_and(|t| layout.resolves(session::Axis::Track, slot, t.reference))
    }
    pub(super) fn sample(&mut self, slot: usize, beat: f64) -> ([f32; 2], bool) {
        let Some(plan) = &self.plan else {
            return ([0.0; 2], false);
        };
        let Some(track) = plan.tracks.get(slot).and_then(Option::as_ref) else {
            return ([0.0; 2], false);
        };
        let cursor = &mut self.cursors[slot];
        advance(&track.audio_segments, &mut cursor.audio, beat);
        advance(&track.activity, &mut cursor.activity, beat);
        let mut output = [0.0; 2];
        for &index in selected(&track.audio_segments, cursor.audio, beat) {
            let span = track.audio[index as usize];
            let source = &plan.sources[span.source];
            let audio = source.audio.as_ref().unwrap();
            let local = span.offset + beat - span.start;
            let value = if let Some(region) = source.audio_region {
                region.sample(audio, local, span.repeating)
            } else if span.repeating || local < source.length {
                let (l, r) = audio
                    .at(local.rem_euclid(source.length) / source.length * audio.frames() as f64);
                [l, r]
            } else {
                [0.0; 2]
            };
            for channel in 0..2 {
                output[channel] += value[channel] * span.gain;
            }
        }
        (
            output,
            !selected(&track.activity, cursor.activity, beat).is_empty(),
        )
    }
    pub(super) fn gate(&mut self, slot: usize, end: f64) -> Option<(NoteSpan, bool, bool)> {
        let track = self.plan.as_ref()?.tracks.get(slot)?.as_ref()?;
        let c = &mut self.cursors[slot];
        if c.dirty {
            let notes = selected(&track.note_segments, c.notes, self.seek);
            while let Some(&index) = notes.get(c.chase_gate) {
                c.chase_gate += 1;
                let note = track.notes[index as usize];
                if note.start < self.seek {
                    c.held[note.pitch as usize] += 1;
                    c.velocity[note.pitch as usize] = note.velocity;
                    c.gain[note.pitch as usize] = note.gain;
                    return Some((note, true, true));
                }
            }
            c.dirty = false;
        }
        while let Some(event) = track
            .gates
            .get(c.gate)
            .filter(|e| e.beat < end - super::super::midi_schedule::BEAT_EPSILON)
        {
            c.gate += 1;
            match event.kind {
                Kind::On(index) => {
                    let note = track.notes[index as usize];
                    c.held[note.pitch as usize] += 1;
                    c.velocity[note.pitch as usize] = note.velocity;
                    c.gain[note.pitch as usize] = note.gain;
                    return Some((note, true, true));
                }
                Kind::Off(index) => {
                    let note = track.notes[index as usize];
                    let held = &mut c.held[note.pitch as usize];
                    *held = held.saturating_sub(1);
                    return Some((note, false, *held == 0));
                }
                Kind::Control { .. } => {}
            }
        }
        None
    }
    fn next_output(&mut self, slot: usize, end: f64) -> Option<(Packet, Owner)> {
        let track = self.plan.as_ref()?.tracks.get(slot)?.as_ref()?;
        let c = &mut self.cursors[slot];
        if c.output_refused {
            return None;
        }
        if c.output_dirty {
            if !c.chase_ready {
                c.chase_len = 0;
                midi_data::chase(
                    |key| {
                        let lane = track
                            .controls
                            .binary_search_by_key(&key, |l| l.key)
                            .ok()
                            .map(|i| &track.controls[i])?;
                        let index = lane
                            .points
                            .partition_point(|(beat, _)| *beat < self.output_seek);
                        index.checked_sub(1).map(|i| lane.points[i].1)
                    },
                    |bytes, length, _| {
                        c.chase[c.chase_len] = (bytes, length);
                        c.chase_len += 1;
                    },
                );
                c.chase_ready = true;
            }
            if c.chase_control < c.chase_len {
                let (bytes, length) = c.chase[c.chase_control];
                c.chase_control += 1;
                return Some((
                    Packet::new(&bytes[..length as usize])?,
                    Owner::ClipLane(slot as u8),
                ));
            }
            let notes = selected(&track.note_segments, c.output_notes, self.output_seek);
            while let Some(&index) = notes.get(c.chase_output) {
                c.chase_output += 1;
                let note = track.notes[index as usize];
                if note.start < self.output_seek {
                    return Some((
                        note_packet(note, true),
                        Owner::Clip {
                            track: slot as u8,
                            note: note.id,
                        },
                    ));
                }
            }
            c.output_dirty = false;
        }
        let event = *track
            .events
            .get(c.output)
            .filter(|e| e.beat < end - super::super::midi_schedule::BEAT_EPSILON)?;
        c.output += 1;
        Some(match event.kind {
            Kind::On(index) | Kind::Off(index) => {
                let note = track.notes[index as usize];
                (
                    note_packet(note, matches!(event.kind, Kind::On(_))),
                    Owner::Clip {
                        track: slot as u8,
                        note: note.id,
                    },
                )
            }
            Kind::Control { bytes, length } => (
                Packet::new(&bytes[..length as usize])?,
                Owner::ClipLane(slot as u8),
            ),
        })
    }
    fn output_due(&self, slot: usize, end: f64) -> bool {
        let c = &self.cursors[slot];
        self.plan
            .as_ref()
            .and_then(|p| p.tracks.get(slot))
            .and_then(Option::as_ref)
            .is_some_and(|t| {
                c.output_dirty
                    || t.events
                        .get(c.output)
                        .is_some_and(|e| e.beat < end - super::super::midi_schedule::BEAT_EPSILON)
            })
    }
    pub(crate) fn output_block(&mut self, state: (u64, u64), beat: f64) {
        if self.output_state != state {
            let old = self.output_state;
            self.output_state = state;
            for (slot, c) in self.cursors.iter_mut().enumerate() {
                c.output_refused = c.output_refused && old.0 == state.0;
                c.output_dirty = true;
                c.chase_output = 0;
                c.chase_control = 0;
                c.chase_ready = false;
                if let Some(track) = self
                    .plan
                    .as_ref()
                    .and_then(|p| p.tracks.get(slot))
                    .and_then(Option::as_ref)
                {
                    c.output = track.events.partition_point(|e| e.beat < beat);
                    c.output_notes = segment(&track.note_segments, beat);
                }
            }
            self.output_seek = beat;
        }
    }
}
fn note_packet(note: NoteSpan, on: bool) -> Packet {
    let velocity = if on {
        (f32::from(note.velocity) * note.gain)
            .round()
            .clamp(1.0, 127.0) as u8
    } else {
        note.release
    };
    Packet::new(&[
        (if on { 0x90 } else { 0x80 }) | note.channel,
        note.pitch,
        velocity,
    ])
    .unwrap()
}
fn segment(segments: &[Segment], beat: f64) -> usize {
    segments
        .partition_point(|s| s.beat <= beat)
        .saturating_sub(1)
}
fn advance(segments: &[Segment], index: &mut usize, beat: f64) {
    while segments.get(*index + 1).is_some_and(|s| s.beat <= beat) {
        *index += 1;
    }
}
fn selected(segments: &[Segment], index: usize, beat: f64) -> &[u32] {
    segments
        .get(index)
        .filter(|s| s.beat <= beat)
        .map_or(&[], |s| &s.active)
}

impl RtEngine {
    pub(in crate::engine) fn render_arrangement(&mut self, slot: usize) -> ([f32; 2], bool) {
        if !self.playing
            || self.count_in.is_some()
            || !self.arrangement.current(slot, &self.session)
        {
            return ([0.0; 2], false);
        }
        let end = self.precise_midi_beat();
        if end <= self.arrangement.seek + super::super::midi_schedule::BEAT_EPSILON {
            return ([0.0; 2], false);
        }
        let step = if self.conductor.is_some() {
            self.last_midi_step
        } else {
            f64::from(self.bpm) / 60.0 / f64::from(self.sr)
        };
        if self.midi_output_mask & (1 << slot) != 0
            && !self.arrangement.cursors[slot].output_refused
        {
            while self.midi_output_budget > 0 {
                let Some((packet, owner)) = self.arrangement.next_output(slot, end) else {
                    break;
                };
                self.midi_output_budget -= 1;
                if !self.midi_routing.emit_weighted(
                    slot as u8,
                    packet,
                    owner,
                    1,
                    self.arrangement.output_state,
                ) {
                    self.arrangement.cursors[slot].output_refused = true;
                    self.midi_routing.clip_refused(slot, true);
                    break;
                }
            }
            if !self.arrangement.cursors[slot].output_refused
                && self.arrangement.output_due(slot, end)
            {
                self.arrangement.cursors[slot].output_refused = true;
                self.midi_routing.clip_refused(slot, true);
                self.midi_routing.overrun(slot);
                self.midi_routing.reset_outputs();
            }
        }
        let arp = self.tracks[slot]
            .fx
            .slots
            .iter()
            .any(|s| s.id() == super::super::fx::FxId::Arp && s.on);
        if self.arrangement.cursors[slot].arp != arp {
            self.tracks[slot].poly.release_clip();
            self.arrangement.cursors[slot].arp = arp;
            self.arrangement.cursors[slot].arp_step = i64::MIN;
            self.arrangement.cursors[slot].arp_note = None;
            if !arp {
                for pitch in 0..128 {
                    let c = &self.arrangement.cursors[slot];
                    if c.held[pitch] > 0 && self.tracks[slot].kind != 0 {
                        self.tracks[slot].poly.note_on_clip_with_gain(
                            pitch as u8,
                            f32::from(c.velocity[pitch]) / 127.0,
                            c.gain[pitch],
                        );
                    }
                }
            }
        }
        while let Some((note, on, release)) = self.arrangement.gate(slot, end) {
            if arp {
                continue;
            }
            if on {
                if self.tracks[slot].kind == 0 {
                    self.trig_drum_with_gain(
                        slot,
                        note.pitch,
                        f32::from(note.velocity) / 127.0,
                        note.gain,
                    );
                } else {
                    self.tracks[slot].poly.note_on_clip_with_gain(
                        note.pitch,
                        f32::from(note.velocity) / 127.0,
                        note.gain,
                    );
                }
            } else if release && self.tracks[slot].kind != 0 {
                self.tracks[slot].poly.note_off_clip(note.pitch);
            }
        }
        if arp {
            let beat = (end - step).max(0.0);
            let step = (beat * 4.0).floor() as i64;
            let c = &mut self.arrangement.cursors[slot];
            let changed = step != c.arp_step || c.arp_note.is_some_and(|n| c.held[n as usize] == 0);
            if changed {
                if let Some(old) = c.arp_note.take() {
                    self.tracks[slot].poly.note_off_clip(old);
                }
                c.arp_step = step;
                let count = c.held.iter().filter(|n| **n > 0).count();
                let note = if count == 0 {
                    None
                } else {
                    c.held
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| **n > 0)
                        .nth(step.rem_euclid(count as i64) as usize)
                        .map(|(pitch, _)| (pitch as u8, c.velocity[pitch], c.gain[pitch]))
                };
                c.arp_note = note.map(|n| n.0);
                if let Some((pitch, velocity, gain)) = note {
                    if self.tracks[slot].kind == 0 {
                        self.trig_drum_with_gain(slot, pitch, f32::from(velocity) / 127.0, gain);
                    } else {
                        self.tracks[slot].poly.note_on_clip_with_gain(
                            pitch,
                            f32::from(velocity) / 127.0,
                            gain,
                        );
                    }
                }
            }
        }
        self.arrangement.sample(slot, (end - step).max(0.0))
    }
}
