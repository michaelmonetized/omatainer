//! A lesson observes the same published state the producer sees. It does not
//! enqueue musical commands or bypass project/overwrite decisions.
use super::*;
use crate::engine::{undo::View, ComposeTarget};

pub(super) struct Observation<'a> {
    pub snap: &'a Snapshot,
    pub history: &'a View,
    pub receipts: [Option<&'a Receipt>; DECKS],
    pub edit: Option<ClipGainEdit>,
    pub diagnostics: bool,
    pub midi: bool,
    pub midi_dispatched: u64,
    pub audio_running: bool,
    pub audio_pending_restart: bool,
    pub project: &'a (Option<PathBuf>, bool, bool),
}
pub(super) struct Lesson {
    pub topic: Topic,
    pub step: usize,
    pub ready: bool,
    pub physical_ack: bool,
    pub invalidated: bool,
    initial: Snapshot,
    epoch: u64,
    cursor: usize,
    target: Option<ComposeTarget>,
    deck: usize,
    receipts: [Option<Receipt>; DECKS],
    active_receipt: Option<Receipt>,
    changed_gain: Option<f32>,
    cue_baseline: [bool; 8],
    midi_before: u64,
    path: Option<PathBuf>,
    opened_epoch: Option<u64>,
}
impl Lesson {
    pub fn new(topic: Topic, observation: &Observation<'_>) -> Self {
        let snap = observation.snap;
        Self {
            topic,
            step: 0,
            ready: false,
            physical_ack: false,
            invalidated: false,
            initial: snap.clone(),
            epoch: observation.history.epoch,
            cursor: observation.history.cursor,
            target: (topic == Topic::Editing).then_some(ComposeTarget {
                track: snap.selected_track,
                scene: snap.selected_scene,
            }),
            deck: snap.selected_deck.min(DECKS - 1),
            receipts: observation.receipts.map(|r| r.cloned()),
            active_receipt: None,
            changed_gain: None,
            cue_baseline: [false; 8],
            midi_before: observation.midi_dispatched,
            path: observation.project.0.clone(),
            opened_epoch: None,
        }
    }
    pub fn steps(&self) -> &'static [&'static str] {
        steps(self.topic)
    }
    pub fn complete(&self) -> bool {
        self.step >= self.steps().len()
    }
    pub fn needs_physical_ack(&self) -> bool {
        matches!(
            (self.topic, self.step),
            (Topic::Setup, 2) | (Topic::Controllers, 2) | (Topic::Emergency, 2)
        )
    }
    pub fn observe(&mut self, o: &Observation<'_>) {
        if self.complete() || self.invalidated {
            return;
        }
        if self.topic != Topic::Projects && o.history.epoch != self.epoch {
            self.invalidated = true;
            self.ready = false;
            return;
        }
        let snap = o.snap;
        let clip = |s: &Snapshot, target: ComposeTarget| {
            s.tracks
                .get(target.track)
                .and_then(|track| track.clips.get(target.scene))
                .cloned()
        };
        let current_receipt = |deck: usize| {
            o.receipts[deck]
                .filter(|receipt| receipt.state() == crate::engine::load_receipt::State::Current)
        };
        if self.topic == Topic::Dj
            && self.step > 0
            && !current_receipt(self.deck)
                .zip(self.active_receipt.as_ref())
                .is_some_and(|(now, loaded)| now.same_request(loaded))
        {
            self.invalidated = true;
            self.ready = false;
            return;
        }
        let fresh_load = |deck: usize| {
            current_receipt(deck).filter(|receipt| {
                self.receipts[deck]
                    .as_ref()
                    .is_none_or(|old| !old.same_request(receipt))
            })
        };
        let condition = match (self.topic, self.step) {
            (Topic::Setup, 0) => {
                o.audio_running
                    && !o.audio_pending_restart
                    && o.diagnostics
                    && snap.audio.last_callback.is_some()
            }
            (Topic::Setup, 1) => (0..DECKS).any(|d| {
                fresh_load(d).is_some_and(|receipt| receipt.last_play().is_some())
                    && snap.decks[d].playing
            }),
            (Topic::Setup, 2) => o.audio_running && self.physical_ack,
            (Topic::Recording, 0) => snap.compose_target.is_some_and(|target| {
                let empty = clip(&self.initial, target).is_some_and(|clip| clip.note_count == 0);
                if empty {
                    self.target = Some(target);
                }
                empty
            }),
            (Topic::Recording, 1) => self.target.is_some_and(|target| {
                snap.compose_target == Some(target)
                    && snap.sampler_inst.synth().is_some()
                    && clip(snap, target).is_some_and(|c| c.note_count > 0 && c.recording_held)
            }),
            (Topic::Recording, 2) => self.target.is_some_and(|target| {
                clip(snap, target).is_some_and(|c| c.note_count > 0 && !c.recording_held)
            }),
            (Topic::Recording, 3) => {
                snap.compose_target.is_none()
                    && self
                        .target
                        .is_some_and(|target| clip(snap, target).is_some_and(|c| c.note_count > 0))
            }
            (Topic::Recording, 4) => self.target.is_some_and(|target| {
                snap.tracks
                    .get(target.track)
                    .is_some_and(|t| t.playing_scene == target.scene as i8 && !t.clip_pending)
            }),
            (Topic::Editing, 0) => self.target.is_some_and(|target| {
                clip(snap, target).is_some_and(|c| c.kind != 0)
                    && o.edit.is_some_and(|edit| {
                        edit.track as usize == target.track && edit.scene as usize == target.scene
                    })
            }),
            (Topic::Editing, 1) => self.target.is_some_and(|target| {
                let changed = clip(snap, target)
                    .zip(clip(&self.initial, target))
                    .is_some_and(|(now, before)| {
                        if now.gain != before.gain {
                            self.changed_gain = Some(now.gain);
                            true
                        } else {
                            false
                        }
                    });
                if changed && o.history.cursor > self.cursor {
                    self.cursor = o.history.cursor;
                    true
                } else {
                    false
                }
            }),
            (Topic::Editing, 2) => {
                self.target.is_some_and(|target| {
                    clip(snap, target)
                        .zip(clip(&self.initial, target))
                        .is_some_and(|(now, before)| now.gain == before.gain)
                }) && o.history.cursor + 1 == self.cursor
            }
            (Topic::Editing, 3) => {
                self.target.is_some_and(|target| {
                    clip(snap, target).is_some_and(|c| Some(c.gain) == self.changed_gain)
                }) && o.history.cursor == self.cursor
            }
            (Topic::Mixing, 0) => {
                snap.playing
                    && snap
                        .tracks
                        .iter()
                        .any(|t| t.playing_scene >= 0 && !t.clip_pending)
            }
            (Topic::Mixing, 1) => snap
                .tracks
                .get(self.initial.selected_track)
                .zip(self.initial.tracks.get(self.initial.selected_track))
                .is_some_and(|(now, before)| now.gain != before.gain),
            (Topic::Mixing, 2) => snap
                .tracks
                .get(self.initial.selected_track)
                .zip(self.initial.tracks.get(self.initial.selected_track))
                .is_some_and(|(now, before)| now.mute != before.mute),
            (Topic::Mixing, 3) => snap
                .tracks
                .get(self.initial.selected_track)
                .zip(self.initial.tracks.get(self.initial.selected_track))
                .is_some_and(|(now, before)| now.mute == before.mute),
            (Topic::Dj, 0) => fresh_load(self.deck).is_some_and(|receipt| {
                // Current can precede GUI Snapshot publication. Read the
                // renderer-qualified preparation from this same load receipt.
                let Some((_, preparation)) = receipt.preparation() else {
                    return false;
                };
                self.active_receipt = Some(receipt.clone());
                self.cue_baseline = preparation.hotcues.map(|cue| cue.is_some());
                true
            }),
            (Topic::Dj, 1) => {
                current_receipt(self.deck)
                    .zip(self.active_receipt.as_ref())
                    .is_some_and(|(now, loaded)| {
                        now.same_request(loaded) && now.last_play().is_some()
                    })
                    && snap.decks[self.deck].playing
            }
            (Topic::Dj, 2) => current_receipt(self.deck)
                .zip(self.active_receipt.as_ref())
                .is_some_and(|(now, loaded)| {
                    now.same_request(loaded)
                        && now.preparation().is_some_and(|(_, preparation)| {
                            preparation.hotcues.map(|cue| cue.is_some()) != self.cue_baseline
                        })
                }),
            (Topic::Dj, 3) => {
                current_receipt(self.deck)
                    .zip(self.active_receipt.as_ref())
                    .is_some_and(|(now, loaded)| now.same_request(loaded))
                    && !snap.decks[self.deck].playing
            }
            (Topic::Controllers, 0) => o.midi,
            (Topic::Controllers, 1) => o.midi_dispatched > self.midi_before,
            (Topic::Controllers, 2) => self.physical_ack,
            (Topic::Emergency, 0) => snap.playing || snap.decks.iter().any(|deck| deck.playing),
            (Topic::Emergency, 1) => {
                !snap.playing
                    && snap.decks.iter().all(|deck| !deck.playing)
                    && snap.compose_target.is_none()
            }
            (Topic::Emergency, 2) => o.diagnostics && self.physical_ack,
            (Topic::Projects, 0) => {
                o.history.epoch == self.epoch
                    && !o.project.1
                    && !o.project.2
                    && o.project.0.is_some()
                    && o.project.0 != self.path
                    && {
                        self.path = o.project.0.clone();
                        true
                    }
            }
            (Topic::Projects, 1) => {
                !o.project.2 && o.project.0.is_none() && o.history.epoch != self.epoch && {
                    self.opened_epoch = Some(o.history.epoch);
                    true
                }
            }
            (Topic::Projects, 2) => {
                !o.project.1
                    && !o.project.2
                    && o.project.0 == self.path
                    && self
                        .opened_epoch
                        .is_some_and(|epoch| o.history.epoch != epoch)
                    && !snap.playing
                    && snap.decks.iter().all(|deck| !deck.playing)
            }
            _ => false,
        };
        // Evidence is latched, not inferred from a button press. A short release
        // before clicking Next must not lose the observed held-note step.
        self.ready |= condition;
    }
    pub fn next(&mut self) {
        if self.ready && !self.invalidated {
            self.step += 1;
            self.ready = false;
            self.physical_ack = false;
        }
    }
}

pub(super) fn steps(topic: Topic) -> &'static [&'static str] {
    match topic {
        Topic::Setup => &[
            "Inspect the running output in Preferences, then open Diagnostics and wait for a completed audio callback. If saved audio changes are pending, preview and explicitly confirm them in Audio devices and latency, or restart. A saved profile is not the running output.",
            "Load Drums (session) or Harmony (session) onto a deck and play it. Wait for renderer-confirmed playback of that accepted load.",
            "At a quiet level, verify the sound at your physical speakers/headphones. Mark only your own listening observation; the app cannot measure it.",
        ],
        Topic::Recording => &[
            "Choose an empty clip and explicitly Arm compose (Shift-click or alternate action). Watch the armed track/scene label; browsing alone does not arm.",
            "Choose Keys, Analog or Pad. Hold a sampler pad long enough to observe the held capture. Wait for a new note in the original armed cell.",
            "Release the same pad. Wait for its recorded duration to finalize; changing selection cannot redirect the release.",
            "Disarm compose. The recorded note remains in its original cell.",
            "Launch that original clip. Wait until it is playing, not merely queued. Stopped note entry used beat zero.",
        ],
        Topic::Editing => &[
            "Select a filled clip before starting this lesson. Open Edit clip gain from that cell's alternate actions (or Alt-click). Empty cells cannot demonstrate gain editing.",
            "Change the target clip's gain and release the gesture. Wait for the renderer-confirmed value and its history entry.",
            "Use Edit → Undo. Wait for the original target gain and the matching history position to return.",
            "Use Edit → Redo. Wait for the changed gain to return. Unrelated held inputs and transport remain owned by their sources.",
        ],
        Topic::Mixing => &[
            "Launch a populated scene or clip. Wait for session playback and a clip past its queued boundary.",
            "Adjust the gain of the track selected when this lesson started. Its output meter and effect chain remain on that track.",
            "Toggle that track's mute using its header or alternate action. Observe the renderer-confirmed mute state.",
            "Restore that track's original mute state. Playback kept advancing while muted; no elapsed hits are replayed.",
        ],
        Topic::Dj => &[
            "Load a built-in stem or your file onto the deck selected when this lesson started. Wait for Loaded; decode/queue errors do not count.",
            "Start that deck and wait for actual source playback. The accepted media receipt must remain current.",
            "Set a previously empty hot cue, or remove and re-set one. The deck's cue indicator must change from the initial state.",
            "Pause the same deck. Its preparation stays attached to this media identity; next try loop in/out and pitch lock using the reference.",
        ],
        Topic::Controllers => &[
            "Open MIDI. Inspect connections and errors; use Retry / rescan MIDI after connecting your controller. Missing hardware is not a passed check.",
            "Operate a supported control and wait for new handled MIDI input. Input traffic only proves dispatch, not the meaning of a physical control.",
            "Verify the intended on-screen/audio action and matching release on your actual device. Mark only your own observation. Unsupported controls and motorized NS7II behavior remain unverified.",
        ],
        Topic::Emergency => &[
            "With an audition already playing, identify session and deck playback separately. This lesson never starts audio for you; Cancel is safe at any time.",
            "Stop the session, pause every playing deck, and disarm compose. Release physical keys/pads/touches separately; session Stop preserves unrelated held live gates.",
            "Open Diagnostics and read callback/queue errors. Then verify that your physical inputs are released and the output is safe; tails may finish naturally.",
        ],
        Topic::Projects => &[
            "Use Project → Save project as… and a private new .omat path. Wait for a successful clean save; Save copy intentionally does not satisfy this step.",
            "Use Project → New. Respond to any unsaved-work prompt deliberately. Wait for the empty replacement; cancellation or failed replacement does not count.",
            "Open the exact saved file through Project → Open or Recent projects. Wait for the clean document with playback stopped; its stored media needs no original files.",
        ],
    }
}
