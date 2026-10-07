//! External clip boundaries retain wire channels and source ordering. Prepared
//! storage is fixed at the native note limit; source lanes use one heap cursor.
use super::{packet::Packet, Owner};
use crate::engine::{midi_edit::Region, midi_schedule::BEAT_EPSILON, Clip};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

const CAPACITY: usize = 3 * crate::engine::project::MAX_NOTES_PER_CLIP + 16 * 140 + 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    On(usize),
    Off(usize),
    Lane(usize, bool),
    Chase([u8; 3], u8),
}
#[derive(Clone, Copy, Debug)]
struct Event {
    beat: f64,
    order: u64,
    cycle: i64,
    weight: u32,
    kind: Kind,
    repeating: bool,
}
impl PartialEq for Event {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Event {}
impl PartialOrd for Event {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Event {
    fn cmp(&self, o: &Self) -> Ordering {
        self.beat
            .total_cmp(&o.beat)
            .then_with(|| self.cycle.cmp(&o.cycle))
            .then_with(|| self.order.cmp(&o.order))
    }
}
#[derive(Debug)]
pub(crate) struct Playback {
    events: BinaryHeap<Reverse<Event>>,
    period: f64,
    region: Region,
    looping: bool,
    lanes: (usize, usize, usize),
    pub dirty: bool,
    pub clear: bool,
    pub refused: bool,
    pub generation: u64,
    pub epoch: u64,
    #[cfg(test)]
    pub trace: Option<Vec<(f64, Packet)>>,
}
impl Clone for Playback {
    fn clone(&self) -> Self {
        let mut result = Self::default();
        result.events.extend(self.events.iter().copied());
        result.period = self.period;
        result.region = self.region;
        result.looping = self.looping;
        result.lanes = self.lanes;
        result.dirty = self.dirty;
        result.clear = self.clear;
        result.refused = self.refused;
        result.generation = self.generation;
        result.epoch = self.epoch;
        result
    }
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            events: BinaryHeap::with_capacity(CAPACITY),
            period: 1.0,
            region: Region::full(1.0),
            looping: false,
            lanes: (0, 0, 0),
            dirty: true,
            clear: false,
            refused: false,
            generation: 0,
            epoch: 0,
            #[cfg(test)]
            trace: None,
        }
    }
}
impl Playback {
    pub fn invalidate(&mut self) {
        self.events.clear();
        self.dirty = true;
        self.clear = true;
        self.refused = false;
    }
    pub fn refuse(&mut self) {
        self.events.clear();
        self.refused = true;
    }
    pub(super) fn rebuild(
        &mut self,
        clip: &Clip,
        elapsed: f64,
        looping: bool,
        recorded: &[Option<crate::engine::recording::RecordedPlayback>],
    ) {
        self.events.clear();
        self.dirty = false;
        if clip.notes.len() > crate::engine::project::MAX_NOTES_PER_CLIP || !elapsed.is_finite() {
            self.refused = true;
            return;
        }
        let explicit = clip.region;
        self.region = explicit.unwrap_or_else(|| Region::full(clip.bars.max(0.25)));
        self.looping = self.region.repeating(looping);
        self.period = if explicit.is_some() {
            self.region.period()
        } else {
            f64::from(clip.bars.max(0.25)) * 4.0
        };
        let end = if self.looping {
            self.region.loop_end
        } else {
            self.region.end
        };
        for (index, note) in clip.notes.iter().enumerate() {
            if note.muted
                || note.vel == 0
                || note.len <= 0.0
                || note.pitch > 127
                || note.channel > 15
            {
                continue;
            }
            let source_start = note.source_start();
            let source_end = source_start + note.source_duration();
            let (start, duration, repeat, phase) = if explicit.is_some() {
                if source_start >= end || source_end <= self.region.start {
                    continue;
                }
                let start = source_start.max(self.region.start);
                (
                    start - self.region.start,
                    source_end.min(end) - start,
                    self.looping && source_start >= self.region.loop_start,
                    source_start - self.region.loop_start,
                )
            } else {
                let start = source_start.rem_euclid(self.period);
                (start, note.source_duration(), self.looping, start)
            };
            let start = if let Some(policy) = recorded.get(index).and_then(Option::as_ref) {
                let Some(first) = policy.first_onset(phase, self.period, repeat) else {
                    continue;
                };
                first
            } else {
                start
            };
            if duration <= 0.0 {
                continue;
            }
            let native_order = if clip.lanes.is_some() { u64::from(u32::MAX) + 1 } else { 0 };
            let on_order = note
                .source_timing
                .map_or(native_order + 2 * index as u64 + 1, |t| u64::from(t.start_order));
            let off_order = note
                .source_timing
                .map_or(native_order + 2 * index as u64, |t| u64::from(t.end_order));
            let next = |beat: f64| {
                if beat < elapsed - BEAT_EPSILON {
                    if repeat {
                        beat + ((elapsed - BEAT_EPSILON - beat) / self.period).ceil() * self.period
                    } else {
                        f64::INFINITY
                    }
                } else {
                    beat
                }
            };
            let on = next(start);
            let off = next(start + duration);
            if elapsed > start + BEAT_EPSILON {
                let last = if repeat {
                    start + ((elapsed - BEAT_EPSILON - start) / self.period).floor() * self.period
                } else {
                    start
                };
                if elapsed < last + duration + BEAT_EPSILON {
                    let count = |first: f64| {
                        if elapsed <= first + BEAT_EPSILON {
                            0.0
                        } else if repeat {
                            ((elapsed - BEAT_EPSILON - first) / self.period).floor() + 1.0
                        } else {
                            1.0
                        }
                    };
                    let weight = (count(start) - count(start + duration)).max(0.0);
                    if weight > 0.0 && weight <= f64::from(u32::MAX) {
                        self.events.push(Reverse(Event {
                            beat: elapsed,
                            order: u64::MAX - crate::engine::project::MAX_NOTES_PER_CLIP as u64 + index as u64,
                            cycle: i64::MAX,
                            weight: weight as u32,
                            kind: Kind::On(index),
                            repeating: false,
                        }));
                    }
                }
            }
            for (beat, order, kind) in [
                (on, on_order, Kind::On(index)),
                (off, off_order, Kind::Off(index)),
            ] {
                if beat.is_finite() {
                    self.events.push(Reverse(Event {
                        beat,
                        order,
                        cycle: if repeat {
                            ((beat
                                - if matches!(kind, Kind::On(_)) {
                                    start
                                } else {
                                    start + duration
                                })
                                / self.period)
                                .round() as i64
                        } else {
                            0
                        },
                        weight: 1,
                        kind,
                        repeating: repeat,
                    }));
                }
            }
        }
        if let Some(lanes) = &clip.lanes {
            let first_pass = end - self.region.start;
            let cycling = self.looping && elapsed >= first_pass;
            let source = if cycling { self.region.loop_start + (elapsed - first_pass).rem_euclid(self.period) } else { (self.region.start + elapsed).min(end) };
            let tick = source * f64::from(lanes.ppqn);
            let previous_end = end * f64::from(lanes.ppqn);
            let loop_tick = self.region.loop_start * f64::from(lanes.ppqn);
            let point = |key: u16| {
                let lane = lanes.state.binary_search_by_key(&key, |l| l.key).ok().map(|i| &lanes.state[i])?;
                let mut value = lane.before(tick);
                if cycling && value.is_none_or(|p| (p.message.tick as f64) < loop_tick) {
                    value = lane.before(previous_end).or(value);
                }
                value
            };
            let chase = |bytes: [u8; 3], length: u8, order: u64| {
                self.events.push(Reverse(Event { beat: elapsed, order, cycle: i64::MIN, weight: 1, kind: Kind::Chase(bytes, length), repeating: false }));
            };
            crate::engine::midi_data::chase(point,chase);
            let beat = |index: usize| lanes.messages[index].tick as f64 / f64::from(lanes.ppqn);
            let first = lanes
                .messages
                .partition_point(|m| m.tick as f64 / f64::from(lanes.ppqn) < self.region.start);
            let loop_first = lanes.messages.partition_point(|m| {
                m.tick as f64 / f64::from(lanes.ppqn) < self.region.loop_start
            });
            let last = lanes.messages.partition_point(|m| {
                let beat = m.tick as f64 / f64::from(lanes.ppqn);
                beat < end || (!self.looping && beat == end)
            });
            self.lanes = (first, loop_first, last);
            if first < last {
                let first_pass = end - self.region.start;
                let cycling = self.looping && elapsed >= first_pass;
                let origin = if cycling {
                    first_pass + ((elapsed - first_pass) / self.period).floor() * self.period
                        - self.region.loop_start
                } else {
                    -self.region.start
                };
                let range_start = if cycling { loop_first } else { first };
                let offset = lanes.messages[range_start..last].partition_point(|m| {
                    m.tick as f64 / f64::from(lanes.ppqn) + origin < elapsed - BEAT_EPSILON
                });
                let index = range_start + offset;
                if index < last {
                    self.events.push(Reverse(Event {
                        beat: beat(index) + origin,
                        order: u64::from(lanes.messages[index].order),
                        cycle: if cycling {
                            1 + ((elapsed - first_pass) / self.period).floor() as i64
                        } else {
                            0
                        },
                        weight: 1,
                        kind: Kind::Lane(index, cycling),
                        repeating: false,
                    }));
                } else if self.looping && loop_first < last {
                    self.events.push(Reverse(Event {
                        beat: beat(loop_first) + origin + self.period,
                        order: u64::from(lanes.messages[loop_first].order),
                        cycle: if cycling {
                            2 + ((elapsed - first_pass) / self.period).floor() as i64
                        } else {
                            1
                        },
                        weight: 1,
                        kind: Kind::Lane(loop_first, true),
                        repeating: false,
                    }));
                }
            }
        }
        if self.events.len() > CAPACITY {
            self.events.clear();
            self.refused = true;
        }
    }
    pub fn due(&self, elapsed: f64) -> bool {
        self.events
            .peek()
            .is_some_and(|e| e.0.beat < elapsed - BEAT_EPSILON)
    }
    pub fn next(&mut self, track: u8, clip: &Clip, elapsed: f64) -> Option<(Packet, Owner, u32)> {
        if !self.due(elapsed) {
            return None;
        }
        let Reverse(event) = self.events.pop()?;
        match event.kind {
            Kind::Chase(bytes, length) => Some((Packet::new(&bytes[..usize::from(length)])?, Owner::ClipLane(track), 1)),
            Kind::On(index) | Kind::Off(index) => {
                if event.repeating {
                    let beat = event.beat + self.period;
                    if beat.is_finite() && beat > event.beat {
                        if let Some(cycle) = event.cycle.checked_add(1) {
                            self.events.push(Reverse(Event {
                                beat,
                                cycle,
                                ..event
                            }));
                        }
                    }
                }
                let note = &clip.notes[index];
                let on = matches!(event.kind, Kind::On(_));
                let bytes = [
                    (if on { 0x90 } else { 0x80 }) | note.channel,
                    note.pitch,
                    if on { note.vel } else { note.release_vel },
                ];
                Some((
                    Packet::new(&bytes)?,
                    Owner::Clip {
                        track,
                        note: note.id,
                    },
                    event.weight,
                ))
            }
            Kind::Lane(index, cycling) => {
                let lanes = clip.lanes.as_ref()?;
                let current = &lanes.messages[index];
                let beat = current.tick as f64 / f64::from(lanes.ppqn);
                let origin = event.beat - beat;
                let (_, loop_first, last) = self.lanes;
                let next = if index + 1 < last {
                    Some((index + 1, origin, cycling))
                } else if self.looping && loop_first < last {
                    Some((
                        loop_first,
                        if cycling {
                            origin + self.period
                        } else {
                            self.region.loop_end - self.region.start - self.region.loop_start
                        },
                        true,
                    ))
                } else {
                    None
                };
                if let Some((next_index, origin, cycling)) = next {
                    let m = &lanes.messages[next_index];
                    self.events.push(Reverse(Event {
                        beat: m.tick as f64 / f64::from(lanes.ppqn) + origin,
                        order: u64::from(m.order),
                        cycle: if next_index <= index {
                            event.cycle.saturating_add(1)
                        } else {
                            event.cycle
                        },
                        weight: 1,
                        kind: Kind::Lane(next_index, cycling),
                        repeating: false,
                    }));
                }
                Some((
                    Packet::new(&current.bytes[..usize::from(current.length)])?,
                    Owner::ClipLane(track),
                    1,
                ))
            }
        }
    }
}

impl crate::engine::RtEngine {
    pub(crate) fn prepare_midi_output_block(&mut self) {
        let (mask, generation, epoch) = self.midi_routing.output_state();
        self.midi_output_mask = mask;
        self.midi_output_budget = 256;
        self.arrangement.output_block((generation,epoch),self.precise_midi_beat());
        for (t, track) in self.tracks.iter_mut().enumerate() {
            let output = &mut track.midi_output;
            if output.generation != generation || output.epoch != epoch {
                let refused = output.generation == generation
                    && (output.refused || track.playing.is_some() && output.epoch != epoch);
                if self.plugin_midi.mask & (1 << t) != 0 { self.plugin_midi.clear_clip(t); }
                output.invalidate();
                output.refused = refused;
                output.generation = generation;
                output.epoch = epoch;
                // Publication/epoch invalidation already resets the output
                // owner. Do not refill an overflowing queue with clip clears.
                output.clear = false;
            }
            self.midi_routing.clip_refused(t, output.refused||self.arrangement.output_refused(t));
            if output.clear {
                if self.plugin_midi.mask & (1 << t) != 0 { self.plugin_midi.clear_clip(t); }
                if mask & (1 << t) != 0 {
                    self.midi_routing.clear_clip(t as u8);
                }
                output.clear = false;
            }
        }
    }
    pub(crate) fn render_midi_output(&mut self, track: usize) {
        if (self.midi_output_mask | self.plugin_midi.mask) & (1 << track) == 0
            || !self.playing
            || self.count_in.is_some()
            || self.tracks[track].midi_output.refused
        {
            return;
        }
        let Some(playing) = self.tracks[track].playing else {
            return;
        };
        let clip = &self.tracks[track].clips[usize::from(playing.scene)];
        if clip.kind != crate::engine::ClipKind::Midi {
            return;
        }
        let elapsed = if clip.region.is_some() {
            self.precise_midi_beat() - playing.midi_start_beat
        } else {
            self.beat - playing.start_beat
        };
        if elapsed <= BEAT_EPSILON {
            return;
        }
        if self.tracks[track].midi_output.dirty {
            let t = &mut self.tracks[track];
            t.midi_output.rebuild(
                &t.clips[usize::from(playing.scene)],
                (elapsed - self.last_midi_step).max(0.0),
                playing.looping,
                &t.recorded_playback,
            );
        }
        while self.midi_output_budget > 0 {
            let t = &mut self.tracks[track];
            let Some((packet, owner, weight)) =
                t.midi_output
                    .next(track as u8, &t.clips[usize::from(playing.scene)], elapsed)
            else {
                break;
            };
            self.midi_output_budget -= 1;
            #[cfg(test)]
            if let Some(trace) = &mut self.tracks[track].midi_output.trace {
                trace.push((elapsed, packet));
            }
            if self.plugin_midi.mask & (1 << track) != 0 {
                let b=packet.bytes();
                if b.len() <= 3 { let mut bytes=[0;3]; bytes[..b.len()].copy_from_slice(b); self.plugin_midi.clip(track, bytes, weight); }
            }
            if self.midi_output_mask & (1 << track) != 0 && !self.midi_routing.emit_weighted(
                track as u8,
                packet,
                owner,
                weight,
                (
                    self.tracks[track].midi_output.generation,
                    self.tracks[track].midi_output.epoch,
                ),
            ) {
                self.tracks[track].midi_output.refuse();
                self.midi_routing.clip_refused(track, true);
                self.midi_routing.reset_outputs();
                return;
            }
        }
        if self.tracks[track].midi_output.due(elapsed) {
            self.tracks[track].midi_output.refuse();
            self.midi_routing.clip_refused(track, true);
            self.midi_routing.overrun(track);
        }
    }
}

#[cfg(test)]
mod tests;
