use super::{InputKey, session::MAX_TRACKS};

const EVENTS: usize = 512;
#[derive(Clone, Copy)]
struct Held { key: InputKey, track: usize, channel: u8, note: u8 }

pub(crate) struct Routing {
    pub mask: u128,
    frames: [[[u8; 3]; EVENTS]; MAX_TRACKS],
    lengths: [usize; MAX_TRACKS],
    held: [Option<Held>; 256],
    refused: [bool; MAX_TRACKS],
    clips: Vec<[[u32;128];16]>,
}
impl Default for Routing {
    fn default() -> Self { Self { mask: 0, frames: [[[0;3];EVENTS];MAX_TRACKS], lengths:[0;MAX_TRACKS],held:[None;256],refused:[false;MAX_TRACKS], clips: (0..MAX_TRACKS).map(|_| [[0;128];16]).collect() } }
}
impl Routing {
    /// Retain one channel message for every processor using this track.
    /// Takes track and complete MIDI bytes; returns fixed storage admission without waiting or allocating.
    pub(crate) fn push(&mut self, track: usize, bytes: [u8;3]) -> bool {
        if track >= MAX_TRACKS || self.refused[track] { return false; }
        let length = self.lengths[track];
        if length == EVENTS { self.refused[track] = true; return false; }
        self.frames[track][length] = bytes; self.lengths[track] += 1; true
    }
    pub(crate) fn events(&self, track: usize) -> &[[u8;3]] { &self.frames[track][..self.lengths[track]] }
    pub(crate) fn refused(&self, track: usize) -> bool { self.refused[track] }
    /// Route a physical note release to its original destination.
    /// Takes retained input ownership; queues its note off even after track selection changes.
    pub(crate) fn release(&mut self, key: InputKey) {
        for index in 0..self.held.len() {
            if let Some(held) = self.held[index].filter(|held| held.key == key) {
                self.held[index] = None;
                if self.clips[held.track][held.channel as usize][held.note as usize] == 0 && !self.held.iter().flatten().any(|h| h.track == held.track && h.channel == held.channel && h.note == held.note) { self.push(held.track, [0x80 | held.channel, held.note, 0]); }
            }
        }
    }
    /// Route a physical note with bounded ownership.
    /// Takes its exact input, destination, channel, pitch and velocity; refuses sounding tracks when storage is exhausted.
    pub(crate) fn on(&mut self, key: InputKey, track: usize, channel: u8, note: u8, velocity: u8) {
        self.release(key);
        if velocity == 0 || track >= MAX_TRACKS { return; }
        if let Some(slot) = self.held.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(Held { key, track, channel, note }); self.push(track,[0x90 | channel,note,velocity]);
        } else { self.refused.fill(true); }
    }
    /// End one shared graph sample after all MIDI consumers read it.
    /// Takes mutable event storage; clears counters without dropping note ownership.
    pub(crate) fn next(&mut self) { self.lengths.fill(0); }
    /// Preserve overlapping clip and live gates on a shared plugin channel.
    /// Takes a complete clip packet and overlap weight; emits a release only when every owner of that pitch has ended.
    pub(crate) fn clip(&mut self, track: usize, bytes: [u8;3], weight: u32) {
        let channel = usize::from(bytes[0] & 15);
        let note = usize::from(bytes[1]);
        if matches!(bytes[0] & 0xf0, 0x80 | 0x90) {
            if bytes[0] & 0xf0 == 0x90 && bytes[2] > 0 {
                let Some(count) = self.clips[track][channel][note].checked_add(weight) else { self.refused[track] = true; return; };
                self.clips[track][channel][note] = count;
            } else {
                self.clips[track][channel][note] = self.clips[track][channel][note].saturating_sub(weight);
                if self.clips[track][channel][note] > 0 || self.held.iter().flatten().any(|h| h.track == track && usize::from(h.channel) == channel && usize::from(h.note) == note) { return; }
            }
        }
        self.push(track, bytes);
    }
    /// Release clip gates while preserving physical keys still held on that track.
    /// Takes the original track slot; emits bounded note offs or refuses the overflowing processor.
    pub(crate) fn clear_clip(&mut self, track: usize) {
        for channel in 0..16 { for note in 0..128 {
            if self.clips[track][channel][note] > 0 {
                self.clips[track][channel][note] = 0;
                if !self.held.iter().flatten().any(|h| h.track == track && usize::from(h.channel) == channel && usize::from(h.note) == note) { self.push(track, [0x80 | channel as u8, note as u8, 0]); }
            }
        } }
    }
    pub(crate) fn reset(&mut self) { self.lengths.fill(0); self.held.fill(None); self.refused.fill(false); self.clips.fill([[0;128];16]); }
}
