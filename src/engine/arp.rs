//! The active chord changes at note boundaries, not at audio sample rate.

use super::MidiNote;

#[derive(Clone, Debug)]
pub(super) struct ChordCache {
    // A pitch is a u8 in the project format. Fixed storage also handles
    // imported values above MIDI's usual 127 without panicking or allocating.
    pitches: [u8; 256],
    velocities: [u8; 256],
    len: usize,
    next_change: f64,
    clip_beats: f64,
    visibility: u8,
    dirty: bool,
    #[cfg(test)]
    pub rebuilds: usize,
}

impl Default for ChordCache {
    fn default() -> Self {
        Self {
            pitches: [0; 256],
            velocities: [0; 256],
            len: 0,
            next_change: 0.0,
            clip_beats: 0.0,
            visibility: 0,
            dirty: true,
            #[cfg(test)]
            rebuilds: 0,
        }
    }
}

impl ChordCache {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn refresh(&mut self, notes: &[MidiNote], local: f64, prev: f64, clip_beats: f64) {
        self.refresh_visible(notes, local, prev, clip_beats, |_| true);
    }

    pub fn refresh_visible(
        &mut self,
        notes: &[MidiNote],
        local: f64,
        prev: f64,
        clip_beats: f64,
        visible: impl Fn(usize) -> bool,
    ) {
        self.refresh_region_visible(notes, local, prev, clip_beats, 0, visible);
    }

    pub fn refresh_region_visible(
        &mut self, notes: &[MidiNote], local: f64, prev: f64, clip_beats: f64,
        visibility: u8, visible: impl Fn(usize) -> bool,
    ) {
        if !self.dirty
            && prev >= 0.0
            && local >= prev
            && local < self.next_change
            && clip_beats == self.clip_beats
            && visibility == self.visibility
        {
            return;
        }

        let mut present = [false; 256];
        self.velocities.fill(0);
        self.next_change = clip_beats;
        for (index, note) in notes.iter().enumerate() {
            if note.muted || !visible(index) {
                continue;
            }
            let start = note.start as f64;
            let end = (note.start + note.len) as f64;
            if local >= start && local < end {
                present[note.pitch as usize] = true;
                // One arp step represents a deduplicated pitch. The loudest
                // currently active visible note supplies its drum velocity.
                let velocity = &mut self.velocities[note.pitch as usize];
                *velocity = (*velocity).max(note.vel.min(127));
            }
            if start > local {
                self.next_change = self.next_change.min(start);
            }
            if end > local {
                self.next_change = self.next_change.min(end);
            }
        }
        self.len = 0;
        for (pitch, exists) in present.into_iter().enumerate() {
            if exists {
                self.pitches[self.len] = pitch as u8;
                self.len += 1;
            }
        }
        self.clip_beats = clip_beats;
        self.visibility = visibility;
        self.dirty = false;
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
    }

    pub fn velocity(&self, pitch: u8) -> u8 {
        self.velocities[pitch as usize]
    }

    pub fn contains(&self, pitch: u8) -> bool {
        self.pitches[..self.len].binary_search(&pitch).is_ok()
    }

    pub fn pitch(&self, step: i64) -> Option<u8> {
        (self.len > 0).then(|| self.pitches[step.rem_euclid(self.len as i64) as usize])
    }
}
