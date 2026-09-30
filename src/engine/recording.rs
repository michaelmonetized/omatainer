//! Held input recording. Gate identity and the original destination survive
//! selection changes; elapsed beats never wrap with a clip's local cursor.

use super::{ClipKind, InputKey, MidiNote, RtEngine};

// Command admission reserves at most 256 simultaneously held gates. Keep the
// matching capture bookkeeping fixed-size as well.
const CAPTURES: usize = 256;

#[derive(Clone, Copy)]
struct HeldNote {
    input: InputKey,
    track: usize,
    scene: usize,
    index: usize,
    onset: f64,
    minimum: f64,
}

pub(super) struct Recording {
    pub clock: f64,
    held: [Option<HeldNote>; CAPTURES],
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            clock: 0.0,
            held: [None; CAPTURES],
        }
    }
}

impl RtEngine {
    pub(super) fn begin_recording_note(
        &mut self,
        input: InputKey,
        track: usize,
        scene: usize,
        pitch: u8,
        velocity: u8,
    ) {
        let Some(start) = self.recording_position(track, scene) else {
            return;
        };
        let Some(slot) = self.note_recording.held.iter().position(Option::is_none) else {
            return;
        };
        let clip = &mut self.tracks[track].clips[scene];
        if clip.kind != ClipKind::Midi {
            return;
        }
        // Keep the existing visible quarter-beat onset preview for compose.
        // Release replaces it with the full, unwrapped hold duration; even an
        // instantaneous press/release occupies at least one audio frame.
        let minimum = self.bpm as f64 / (60.0 * self.sr as f64);
        let index = clip.notes.len();
        clip.notes.push(MidiNote {
            pitch,
            start,
            len: 0.25,
            vel: velocity,
        });
        self.note_recording.held[slot] = Some(HeldNote {
            input,
            track,
            scene,
            index,
            onset: self.note_recording.clock,
            minimum,
        });
        self.tracks[track].clip_notes_changed(scene, self.beat);
    }

    pub(super) fn finish_recording_input(&mut self, input: InputKey) {
        self.finish_recording_where(|held| held.input == input);
    }

    pub(super) fn finish_recording_track(&mut self, track: usize) {
        self.finish_recording_where(|held| held.track == track);
    }

    pub(super) fn finish_recording_all(&mut self) {
        self.finish_recording_where(|_| true);
    }

    fn finish_recording_where(&mut self, matches: impl Fn(HeldNote) -> bool) {
        for slot in 0..CAPTURES {
            if let Some(held) = self.note_recording.held[slot].filter(|held| matches(*held)) {
                self.note_recording.held[slot] = None;
                let duration = (self.note_recording.clock - held.onset).max(held.minimum);
                if let Some(note) = self.tracks[held.track].clips[held.scene]
                    .notes
                    .get_mut(held.index)
                {
                    note.len = duration as f32;
                    self.tracks[held.track].clip_notes_changed(held.scene, self.beat);
                }
            }
        }
    }

    pub(super) fn cancel_recording_clip(&mut self, track: usize, scene: usize) {
        for held in &mut self.note_recording.held {
            if held.is_some_and(|held| held.track == track && held.scene == scene) {
                *held = None;
            }
        }
    }
}
