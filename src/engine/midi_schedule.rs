//! Repeating note boundaries. Between events rendering only checks the heap
//! head; edits rebuild the schedule once, without copying notes in the renderer.

use super::MidiNote;
use super::recording::RecordedPlayback;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// Transport accumulates f64 beat increments. Ignore sub-sample rounding at an
// exact boundary, rather than moving that event one sample earlier by accident.
pub(super) const BEAT_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gate {
    Off(u8),
    On(u8, u8),
}

#[derive(Clone, Copy, Debug)]
struct Event {
    beat: f64,
    gate: Gate,
    note: usize,
    repeating: bool,
    boundary: bool,
}

impl Event {
    fn off(&self) -> bool {
        matches!(self.gate, Gate::Off(_))
    }
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Event {}
impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Event {
    fn cmp(&self, other: &Self) -> Ordering {
        self.beat
            .total_cmp(&other.beat)
            // Old gates close before new gates open at the same beat.
            .then_with(|| other.off().cmp(&self.off()))
            .then_with(|| self.note.cmp(&other.note))
    }
}

#[derive(Clone, Debug)]
pub(super) struct MidiSchedule {
    events: BinaryHeap<Reverse<Event>>,
    held: [usize; 256],
    loop_beats: f64,
    pub paused: bool,
    #[cfg(test)]
    pub rebuilds: usize,
    #[cfg(test)]
    pub events_visited: usize,
    #[cfg(test)]
    pub trace: Option<Vec<(f64, Gate)>>,
    #[cfg(test)]
    pub sample_trace: Option<Vec<(u64, Gate)>>,
}

impl Default for MidiSchedule {
    fn default() -> Self {
        Self {
            events: BinaryHeap::new(),
            held: [0; 256],
            loop_beats: 1.0,
            paused: false,
            #[cfg(test)]
            rebuilds: 0,
            #[cfg(test)]
            events_visited: 0,
            #[cfg(test)]
            trace: None,
            #[cfg(test)]
            sample_trace: None,
        }
    }
}

impl MidiSchedule {
    pub(super) fn retained_bytes(&self) -> usize {
        self.events.capacity() * std::mem::size_of::<Reverse<Event>>()
    }

    pub fn has_length(&self, loop_beats: f64) -> bool {
        self.loop_beats == loop_beats
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.held.fill(0);
        self.paused = false;
    }

    pub fn rebuild(
        &mut self,
        notes: &[MidiNote],
        loop_beats: f64,
        elapsed: Option<f64>,
        looping: bool,
        recorded: &[Option<RecordedPlayback>],
    ) {
        self.rebuild_region(notes, loop_beats, elapsed, looping, recorded, None);
    }
    pub fn rebuild_region(
        &mut self,
        notes: &[MidiNote],
        loop_beats: f64,
        elapsed: Option<f64>,
        looping: bool,
        recorded: &[Option<RecordedPlayback>],
        region: Option<super::midi_edit::Region>,
    ) {
        let looping = region.map_or(looping, |region| region.repeating(looping));
        let loop_beats = region.map_or(loop_beats, |region| region.period());
        // Edits arrive between output samples. Keep boundary gates and edit
        // reconciliations which have been scheduled but not rendered yet.
        let mut pending_on = [false; 256];
        let mut pending_release = [false; 256];
        let mut pending_offs = [0usize; 256];
        if let Some(now) = elapsed {
            for Reverse(event) in &self.events {
                if event.beat <= now {
                    match event.gate {
                        Gate::On(pitch, _) if !event.boundary => pending_on[pitch as usize] = true,
                        Gate::Off(pitch) if !event.boundary => {
                            pending_release[pitch as usize] = true
                        }
                        Gate::Off(pitch) => pending_offs[pitch as usize] += 1,
                        _ => {}
                    }
                }
            }
        }
        self.events.clear();
        self.paused = false;
        if !loop_beats.is_finite() || loop_beats <= 0.0 {
            self.held.fill(0);
            return;
        }
        self.loop_beats = loop_beats;
        // Enough room for two recurring boundaries per note plus an edit
        // off/on pair per pitch. Pop/reinsert never grows this heap.
        self.events
            .reserve(notes.len().saturating_mul(2).saturating_add(512));
        let mut held = [0usize; 256];
        let mut latest: [Option<(f64, usize, u8)>; 256] = [None; 256];
        let elapsed = elapsed.filter(|beat| beat.is_finite() && *beat >= 0.0);
        for (index, note) in notes.iter().enumerate() {
            if note.muted || !note.start.is_finite() || !note.len.is_finite() || note.len <= 0.0 {
                continue;
            }
            let (start, duration, repeating, phase) = if let Some(region) = region {
                if !region.valid() { continue; }
                let end = if looping { region.loop_end } else { region.end };
                let source_start = note.source_start();
                let source_end = (source_start + note.source_duration()).min(end);
                if source_start >= end || source_end <= region.start { continue; }
                let clipped_start = source_start.max(region.start);
                (clipped_start - region.start, source_end - clipped_start,
                    looping && source_start >= region.loop_start,
                    source_start - region.loop_start)
            } else {
                let start = (note.source_start()).rem_euclid(loop_beats);
                (start, note.source_duration(), looping, start)
            };
            let start = if let Some(policy) = recorded.get(index).and_then(Option::as_ref) {
                let Some(first) = policy.first_onset(phase, loop_beats, repeating) else {
                    continue;
                };
                first
            } else {
                start
            };
            let end = start + duration;
            let mut on = start;
            let mut off = end;
            if let Some(now) = elapsed {
                let count = |first: f64| -> usize {
                    if now < first {
                        0
                    } else if repeating {
                        (((now - first) / loop_beats).floor() as usize).saturating_add(1)
                    } else {
                        1
                    }
                };
                let ons = count(start);
                let offs = count(end);
                let active = ons.saturating_sub(offs);
                held[note.pitch as usize] = held[note.pitch as usize].saturating_add(active);
                if active > 0 {
                    let last_on = start + ons.saturating_sub(1) as f64 * loop_beats;
                    let candidate = (last_on, index, note.vel);
                    if latest[note.pitch as usize].is_none_or(|old| candidate > old) {
                        latest[note.pitch as usize] = Some(candidate);
                    }
                }
                on += ons as f64 * loop_beats;
                off += offs as f64 * loop_beats;
                if !repeating {
                    if ons > 0 {
                        on = f64::INFINITY;
                    }
                    if offs > 0 {
                        off = f64::INFINITY;
                    }
                }
            }
            for (beat, gate) in [
                (on, Gate::On(note.pitch, note.vel)),
                (off, Gate::Off(note.pitch)),
            ] {
                if beat.is_finite() {
                    self.events.push(Reverse(Event {
                        beat,
                        gate,
                        note: index,
                        repeating,
                        boundary: true,
                    }));
                }
            }
        }
        if let Some(beat) = elapsed {
            for pitch in 0..256 {
                let release = self.held[pitch] > 0
                    && (held[pitch] == 0
                        || pending_release[pitch]
                        || pending_offs[pitch] >= self.held[pitch]);
                let onset = held[pitch] > 0
                    && (held[pitch] > self.held[pitch]
                        || pending_on[pitch]
                        || release
                        || latest[pitch].is_some_and(|(start, _, _)| start >= beat - BEAT_EPSILON));
                for gate in [
                    release.then_some(Gate::Off(pitch as u8)),
                    onset.then(|| Gate::On(pitch as u8, latest[pitch].unwrap().2)),
                ]
                .into_iter()
                .flatten()
                {
                    self.events.push(Reverse(Event {
                        beat,
                        gate,
                        note: pitch,
                        repeating: false,
                        boundary: false,
                    }));
                }
            }
        }
        self.held = held;
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
    }

    pub fn next_due(&mut self, elapsed: f64, looping: bool) -> Option<Gate> {
        if !elapsed.is_finite() {
            return None;
        }
        while self
            .events
            .peek()
            .is_some_and(|event| event.0.beat < elapsed - BEAT_EPSILON)
        {
            let Reverse(event) = self.events.pop().unwrap();
            #[cfg(test)]
            {
                self.events_visited += 1;
            }
            if event.repeating && looping {
                let beat = event.beat + self.loop_beats;
                // Also guards precision exhaustion after extreme timeline
                // values: reinserting an unchanged time could otherwise spin.
                if beat.is_finite() && beat > event.beat {
                    self.events.push(Reverse(Event { beat, ..event }));
                }
            }
            if event.boundary {
                match event.gate {
                    Gate::On(pitch, _) => {
                        self.held[pitch as usize] = self.held[pitch as usize].saturating_add(1);
                    }
                    Gate::Off(pitch) => {
                        let count = &mut self.held[pitch as usize];
                        if *count == 0 {
                            continue;
                        }
                        *count -= 1;
                        if *count > 0 {
                            continue;
                        }
                    }
                }
            }
            #[cfg(test)]
            if let Some(trace) = &mut self.trace {
                trace.push((elapsed, event.gate));
            }
            return Some(event.gate);
        }
        None
    }
}

impl MidiSchedule {
    /// Worker/startup preparation for bounded, allocation-free note undo.
    pub(super) fn prepare_history(&mut self,notes:usize) {self.events.reserve(notes.saturating_mul(2).saturating_add(512));}
}
