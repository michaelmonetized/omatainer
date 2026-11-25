use super::*;
use crate::engine::{midi_edit::NoteId, mpe};
use crate::plugin_host::expression::Event;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Live { key: InputKey, origin: u64 },
    Clip { note: NoteId, track: usize },
}
#[derive(Clone, Copy)]
pub(super) struct Voice {
    owner: Owner,
    track: usize,
    note: mpe::Note,
    id: i32,
    pitch: u8,
    values: mpe::Values,
    sustain: bool,
    released: bool,
}
impl Routing {
    fn expression_push(&mut self, track: usize, event: Event) -> bool {
        if track >= MAX_TRACKS || self.refused[track] {
            return false;
        }
        if self.expressions[track].len() + self.lengths[track] == EVENTS {
            self.refused[track] = true;
            return false;
        }
        self.expressions[track].push(event);
        true
    }
    pub(crate) fn expressions(&self, track: usize) -> &[Event] {
        &self.expressions[track]
    }
    /// Start an independently controlled plugin voice.
    /// Takes stable input ownership, destination and the source's MPE state; emits a fresh native note identity or explicitly refuses exhausted storage.
    pub(crate) fn expression_on(
        &mut self,
        owner: Owner,
        track: usize,
        channel: u8,
        pitch: u8,
        velocity: u8,
        state: &mpe::State,
    ) {
        self.expression_release(|voice| voice.owner == owner);
        let origin = match owner {
            Owner::Live { origin, .. } => origin,
            Owner::Clip { .. } => 0,
        };
        let Some(note) = state.note(origin, channel) else {
            return;
        };
        if velocity == 0 || track >= MAX_TRACKS {
            return;
        }
        let slot = self
            .expression_voices
            .iter()
            .position(Option::is_none)
            .or_else(|| {
                self.expression_voices
                    .iter()
                    .position(|voice| voice.is_some_and(|voice| voice.released))
            });
        let Some(slot) = slot else {
            self.refused[track] = true;
            return;
        };
        let Some(id) = self.expression_id.checked_add(1).filter(|id| *id > 0) else {
            self.refused[track] = true;
            return;
        };
        self.expression_id = id;
        let values = state.combined(note);
        let sustain = state.sustain(note.manager);
        self.expression_voices[slot] = Some(Voice {
            owner,
            track,
            note,
            id,
            pitch,
            values,
            sustain,
            released: false,
        });
        self.expression_push(
            track,
            Event::On {
                id,
                pitch,
                velocity,
                values: values.into(),
            },
        );
    }
    fn expression_release(&mut self, matches: impl Fn(Voice) -> bool) {
        for slot in 0..self.expression_voices.len() {
            let Some(mut voice) =
                self.expression_voices[slot].filter(|voice| voice.note.held && matches(*voice))
            else {
                continue;
            };
            voice.note.held = false;
            if !voice.sustain {
                voice.released = true;
                self.expression_push(
                    voice.track,
                    Event::Off {
                        id: voice.id,
                        pitch: voice.pitch,
                    },
                );
            }
            self.expression_voices[slot] = Some(voice);
        }
    }
    pub(super) fn expression_release_input(&mut self, key: InputKey) {
        self.expression_release(
            |voice| matches!(voice.owner, Owner::Live { key: owner, .. } if owner == key),
        );
    }
    pub(crate) fn expression_off_clip(&mut self, track: usize, note: NoteId) {
        self.expression_release(|voice| voice.owner == Owner::Clip { track, note });
    }
    /// Follow one source's expression without touching another controller.
    /// Takes the native source, optional clip destination, current zone state and voice-release masks; emits only changed values to matching held notes and manager-controlled tails.
    pub(crate) fn expression_update(
        &mut self,
        source: u64,
        clip: Option<usize>,
        state: &mpe::State,
        update: mpe::Update,
    ) {
        for slot in 0..self.expression_voices.len() {
            let Some(mut voice) = self.expression_voices[slot].filter(|voice| {
                voice.note.source == source && clip.is_none_or(|track| voice.track == track)
            }) else {
                continue;
            };
            let channel = 1 << voice.note.channel;
            if (update.reset | update.silence) & channel != 0 {
                if !voice.released {
                    self.expression_push(
                        voice.track,
                        Event::Off {
                            id: voice.id,
                            pitch: voice.pitch,
                        },
                    );
                }
                self.expression_voices[slot] = None;
                continue;
            }
            if update.release & channel != 0 {
                voice.note.held = false;
            }
            if voice.note.held {
                voice.note.member = state.member(voice.note.channel);
            }
            voice.sustain = state.sustain(voice.note.manager);
            let values = state.combined(voice.note);
            if values != voice.values {
                voice.values = values;
                self.expression_push(
                    voice.track,
                    Event::Values {
                        id: voice.id,
                        values: values.into(),
                    },
                );
            }
            if !voice.note.held && !voice.sustain && !voice.released {
                voice.released = true;
                self.expression_push(
                    voice.track,
                    Event::Off {
                        id: voice.id,
                        pitch: voice.pitch,
                    },
                );
            }
            self.expression_voices[slot] = Some(voice);
        }
    }
    pub(crate) fn expression_retire(&mut self, source: u64) {
        for slot in 0..self.expression_voices.len() {
            let Some(voice) = self.expression_voices[slot].filter(|voice| matches!(voice.owner, Owner::Live { key: InputKey::Midi { source: alias, .. }, origin } if source == 0 || source == alias || source == origin)) else { continue; };
            if !voice.released {
                self.expression_push(
                    voice.track,
                    Event::Off {
                        id: voice.id,
                        pitch: voice.pitch,
                    },
                );
            }
            self.expression_voices[slot] = None;
        }
    }
    pub(super) fn expression_clear_clip(&mut self, track: usize) {
        for slot in 0..self.expression_voices.len() {
            let Some(voice) = self.expression_voices[slot].filter(
                |voice| matches!(voice.owner, Owner::Clip { track: owner, .. } if owner == track),
            ) else {
                continue;
            };
            if !voice.released {
                self.expression_push(
                    voice.track,
                    Event::Off {
                        id: voice.id,
                        pitch: voice.pitch,
                    },
                );
            }
            self.expression_voices[slot] = None;
        }
    }
}
