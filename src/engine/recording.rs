//! Held input recording. Gate identity and the original destination survive
//! selection changes; elapsed beats never wrap with a clip's local cursor.

use super::midi_schedule::BEAT_EPSILON;
use super::{ClipKind, InputKey, MidiNote, RtEngine, TrackRt};

// Runtime scheduling policy only: the visible/project note is still complete.
// Its first recording pass is already monitored by the physical input voice.
#[derive(Clone, Copy, Debug)]
pub(super) struct RecordedPlayback {
    held: Option<InputKey>,
    next_loop: f64,
    released_at: f64,
}

impl RecordedPlayback {
    pub fn first_onset(&self, note_start: f64, loop_beats: f64, looping: bool) -> Option<f64> {
        if self.held.is_some() || !looping {
            return None;
        }
        let mut first = self.next_loop + note_start.rem_euclid(loop_beats);
        if first <= self.released_at + BEAT_EPSILON {
            let cycles = ((self.released_at + BEAT_EPSILON - first) / loop_beats).floor() + 1.0;
            first += cycles * loop_beats;
        }
        Some(first)
    }
}

impl TrackRt {
    fn recorded_note_started(&mut self, scene: usize, index: usize, input: InputKey, beat: f64, midi_beat: f64) {
        if let Some(playing) = self.playing.filter(|p| p.scene as usize == scene) {
            let length = self.clips[scene].bars.max(0.25) as f64 * 4.0;
            let elapsed = (if self.clips[scene].region.is_some() { midi_beat - playing.midi_start_beat } else { beat - playing.start_beat }).max(0.0);
            let next_loop = self.clips[scene].region.map_or_else(|| ((elapsed / length).floor() + 1.0) * length,
                |region| {
                    let first = region.loop_end - region.start;
                    if elapsed < first { first } else { first + ((elapsed - first) / region.period()).floor().mul_add(region.period(), region.period()) }
                });
            self.recorded_playback
                .resize(self.clips[scene].notes.len(), None);
            self.recorded_playback[index] = Some(RecordedPlayback {
                held: Some(input),
                next_loop,
                released_at: 0.0,
            });
        }
        self.clip_notes_changed(scene, beat, midi_beat);
    }

    pub(super) fn recorded_input_released(&mut self, input: InputKey, beat: f64, midi_beat: f64) {
        let Some(playing) = self.playing else { return };
        let mut changed = false;
        for policy in self.recorded_playback.iter_mut().flatten() {
            if policy.held == Some(input) {
                policy.held = None;
                policy.released_at = if self.clips[playing.scene as usize].region.is_some() { midi_beat - playing.midi_start_beat } else { beat - playing.start_beat };
                changed = true;
            }
        }
        if changed {
            self.clip_notes_changed(playing.scene as usize, beat, midi_beat);
        }
    }
}

// Command admission reserves at most 256 simultaneously held gates. Keep the
// matching capture bookkeeping fixed-size as well.
pub(super) const CAPTURES: usize = 256;

#[derive(Clone, Copy)]
struct HeldNote {
    input: InputKey,
    track: usize,
    scene: usize,
    index: usize,
    onset: f64,
    minimum: f64,
    history_owner: u64,
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
    pub(super) fn midi_recording_density_available(&self, track: usize, scene: usize) -> bool {
        let clip = &self.tracks[track].clips[scene];
        let Some(region) = clip.region.filter(|region| region.loop_enabled) else { return true; };
        let Some(start) = self.recording_position(track, scene) else { return false; };
        if (start as f64) < region.loop_start || start as f64 >= region.loop_end { return true; }
        let count = clip.notes.iter().filter(|n| !n.muted && n.len > 0.0
            && n.source_start() >= region.loop_start && (n.source_start()) < region.loop_end).count();
        (count + 1) as f64 <= super::project::MAX_NOTES_PER_CLIP as f64 * region.period()
    }
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
        if self.midi_note_count() >= super::project::MAX_TOTAL_NOTES
            || !self.midi_recording_density_available(track, scene) { return; }
        let Some(slot) = self.note_recording.held.iter().position(Option::is_none) else {
            return;
        };
        let id = crate::engine::midi_edit::NoteId::new();
        if !id.valid() { self.undo.reject(crate::engine::undo::Failure::Capacity); return; }
        let history_owner = self.recording_history_owner(track, scene);
        let clip = &mut self.tracks[track].clips[scene];
        if clip.kind != ClipKind::Midi {
            return;
        }
        // Keep the existing visible quarter-beat onset preview for compose.
        // Release replaces it with the full, unwrapped hold duration; even an
        // instantaneous press/release occupies at least one audio frame.
        let minimum = self.bpm as f64 / (60.0 * self.sr as f64);
        self.project.edited();
        let index = clip.notes.len();
        clip.notes.push(MidiNote { variation: None,
            channel:match input {InputKey::Midi {ch,..}=>ch,_=>0},release_vel:64,source_timing:None,
            id, muted: false,
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
            history_owner,
        });
        let midi_beat = self.precise_midi_beat();
        self.tracks[track].recorded_note_started(scene, index, input, self.beat, midi_beat);
        self.history_record_changed(history_owner);
    }

    pub(super) fn finish_recording_input(&mut self, input: InputKey) {
        self.finish_recording_where(|held| held.input == input);
    }

    pub(super) fn finish_recording_track(&mut self, track: usize) {
        self.finish_recording_where(|held| held.track == track);
    }

    pub(super) fn finish_recording_pads(&mut self) {
        self.finish_recording_where(|held| matches!(held.input, InputKey::Pad(_)));
    }

    pub(super) fn finish_recording_clip(&mut self, track: usize, scene: usize) {
        self.finish_recording_where(|held| held.track == track && held.scene == scene);
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
                    self.project.edited();
                    note.len = duration as f32;
                    let midi_beat = self.precise_midi_beat();
                    self.tracks[held.track].clip_notes_changed(held.scene, self.beat, midi_beat);
                    self.history_record_finished(
                        held.history_owner,
                        held.track,
                        held.scene,
                        held.index,
                        duration as f32,
                    );
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

impl RtEngine {
    pub(super) fn recording_clip_held(&self, track: usize, scene: usize) -> bool {
        self.note_recording.held.iter().flatten().any(|h| h.track == track && h.scene == scene)
    }

    pub(super) fn has_held_project_notes(&self) -> bool {
        self.note_recording.held.iter().any(Option::is_some)
    }

    pub(super) fn capture_held_durations(&self, state: &mut super::project::State) {
        for held in self.note_recording.held.iter().flatten() {
            if let Some(note) = state.tracks[held.track].clips[held.scene]
                .notes
                .get_mut(held.index)
            {
                note.len = (self.note_recording.clock - held.onset).max(held.minimum) as f32;
            }
        }
    }
}

impl Recording {
    pub(super) fn held_targets(&self) -> [u64; super::session::MAX_TRACKS * super::session::MAX_SCENES / 64] {
        let mut mask = [0; super::session::MAX_TRACKS * super::session::MAX_SCENES / 64];
        for h in self.held.iter().flatten() {
            let cell = h.track * super::session::MAX_SCENES + h.scene;
            mask[cell / 64] |= 1u64 << (cell % 64);
        }
        mask
    }
}

impl Recording {
    pub(super) fn history_owners(&self) -> ([u64; CAPTURES], usize) {
        let mut owners = [0; CAPTURES];
        let mut len = 0;
        for held in self
            .held
            .iter()
            .flatten()
            .filter(|held| held.history_owner != 0)
        {
            owners[len] = held.history_owner;
            len += 1;
        }
        (owners, len)
    }
    pub(super) fn active_history(&self) -> [(u64, usize, usize); CAPTURES] {
        std::array::from_fn(|index| {
            self.held[index].map_or((0, 0, 0), |held| {
                (held.history_owner, held.track, held.scene)
            })
        })
    }
}

impl RtEngine {
    /// An inverse captured during another note's hold contains its provisional
    /// duration. Before replay swaps that inverse into view, finalize the held
    /// notes which are actually present in it. Its own later notes are absent.
    pub(super) fn capture_held_clip_durations(
        &self,
        track: usize,
        scene: usize,
        clip: &mut super::Clip,
    ) {
        for held in self.note_recording.held.iter().flatten() {
            if held.track == track && held.scene == scene {
                if let Some(note) = clip.notes.get_mut(held.index) {
                    note.len = (self.note_recording.clock - held.onset).max(held.minimum) as f32;
                }
            }
        }
    }
}
