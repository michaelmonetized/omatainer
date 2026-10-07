use super::{DeckRt, DeckTransition, RtEngine};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    #[default]
    None,
    Beat,
    Bar,
}
impl Phase {
    /// Omit independent tempo intent from older-compatible storage.
    /// Takes this phase request; returns whether no phase alignment is requested.
    pub(crate) fn is_none(&self) -> bool {
        *self == Self::None
    }
    /// Read the alignment period in quarter notes.
    /// Takes this request; returns a beat or a four-quarter-note DJ bar.
    fn period(self) -> Option<f64> {
        match self {
            Self::None => None,
            Self::Beat => Some(1.0),
            Self::Bar => Some(4.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    Tempo,
    Beat,
    Bar,
}
impl Mode {
    pub(crate) const ALL: [Self; 4] = [Self::Off, Self::Tempo, Self::Beat, Self::Bar];
    /// Name a declared sync behavior.
    /// Takes this mode; returns its native and portable label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Tempo => "Tempo",
            Self::Beat => "Beat",
            Self::Bar => "Bar (4 beats)",
        }
    }
    /// Decode a fixed learned mode ID.
    /// Takes a zero-based ID; returns a declared mode or no target.
    pub(crate) fn from_id(id: u16) -> Option<Self> {
        Self::ALL.get(usize::from(id)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leader {
    Transport,
    DeckA,
    DeckB,
}
impl Leader {
    /// Identify a physical deck leader.
    /// Takes this selection; returns no deck for the native transport clock.
    pub(crate) fn deck(self) -> Option<usize> {
        match self {
            Self::Transport => None,
            Self::DeckA => Some(0),
            Self::DeckB => Some(1),
        }
    }
    /// Name a deliberate clock leader.
    /// Takes this selection; returns its native label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Transport => "Transport",
            Self::DeckA => "Deck A",
            Self::DeckB => "Deck B",
        }
    }
    /// Decode an explicit learned leader selection.
    /// Takes a fixed ID; returns a declared shared leader.
    pub(crate) fn from_id(id: u16) -> Option<Self> {
        match id {
            0 => Some(Self::Transport),
            1 => Some(Self::DeckA),
            2 => Some(Self::DeckB),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    pub leader: Option<Leader>,
    identity: [u64; 3],
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Intent {
    pub enabled: bool,
    pub phase: Phase,
    pub bpm: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Saved {
    leader: Option<Leader>,
    decks: [Intent; 2],
}
impl Saved {
    /// Capture sync intent without an advancing playhead.
    /// Takes the renderer; returns fixed undo storage for leader and modes.
    pub(super) fn get(rt: &RtEngine) -> Self {
        Self {
            leader: rt.deck_sync.leader,
            decks: std::array::from_fn(|i| Intent {
                enabled: rt.decks[i].sync,
                phase: rt.decks[i].sync_phase,
                bpm: rt.decks[i].sync_bpm,
            }),
        }
    }
    /// Exchange sync intent and re-arm phase alignment.
    /// Takes this inverse and the renderer; swaps only sync scalars without moving playheads.
    pub(super) fn swap(&mut self, rt: &mut RtEngine) {
        let current = Self::get(rt);
        rt.deck_sync = State {
            leader: self.leader,
            ..Default::default()
        };
        for (deck, intent) in rt.decks.iter_mut().zip(self.decks) {
            deck.sync = intent.enabled;
            deck.sync_phase = intent.phase;
            deck.sync_bpm = intent.bpm;
            deck.sync_phase_locked = false;
            deck.sync_step = None;
        }
        *self = current;
    }
}

#[derive(Clone, Copy)]
pub(super) struct Pair {
    pub audio: [[f32; 2]; 2],
    pub taps: [[[f32; 2]; 2]; 2],
}

impl DeckRt {
    /// Derive the public mode from tempo enable and phase intent.
    /// Takes this deck; returns one consistent sync state.
    pub(crate) fn sync_mode(&self) -> Mode {
        if !self.sync {
            Mode::Off
        } else {
            match self.sync_phase {
                Phase::None => Mode::Tempo,
                Phase::Beat => Mode::Beat,
                Phase::Bar => Mode::Bar,
            }
        }
    }
    /// Retain tempo while releasing phase ownership.
    /// Takes this deck; clears alignment and the pending per-sample step without changing source position.
    pub(super) fn sync_disengage_phase(&mut self) {
        self.sync_phase = Phase::None;
        self.sync_phase_locked = false;
        self.sync_step = None;
    }
    /// Refuse phase correction during hand or temporary transport manipulation.
    /// Takes this deck; returns whether ordinary forward playback owns its source clock.
    fn sync_forward(&self) -> bool {
        self.playing
            && self.audio.is_some()
            && !self.touching
            && self.preview_position.is_none()
            && !self.controls.braking
            && !self.controls.held(super::deck_controls::Button::Reverse)
            && !self.controls.held(super::deck_controls::Button::Bleep)
            && self.controls.multiplier() == 1.0
            && (!self.follows_spindle() || self.spindle.as_ref().is_some_and(|s| !s.scratching()))
    }
}

impl RtEngine {
    /// Read the selected leader's intended tempo while stopped.
    /// Takes the renderer; returns the native transport tempo or a loaded deck's current mapped pitch tempo.
    fn deck_sync_leader_bpm(&self) -> Option<f32> {
        match self.deck_sync.leader {
            None => None,
            Some(Leader::Transport) => Some(self.bpm),
            Some(leader) => {
                let deck = &self.decks[leader.deck().unwrap()];
                deck.audio.as_ref().map(|_| {
                    if deck.sync {
                        deck.sync_bpm
                    } else {
                        deck.musical_bpm() * deck.pitch_rate()
                    }
                })
            }
        }
        .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
    }
    /// Select independent targets or one deliberate leader.
    /// Takes a selection; clears the new deck leader's follower state and re-arms other phase requests without starting audio.
    pub(super) fn deck_sync_leader(&mut self, leader: Option<Leader>) {
        self.deck_sync = State {
            leader,
            ..Default::default()
        };
        if let Some(index) = leader.and_then(Leader::deck) {
            let deck = &mut self.decks[index];
            deck.sync = false;
            deck.sync_disengage_phase();
        }
        let tempo = self.deck_sync_leader_bpm();
        for deck in &mut self.decks {
            if deck.sync {
                if let Some(bpm) = tempo {
                    deck.sync_bpm = bpm;
                }
            }
            deck.sync_phase_locked = false;
            deck.sync_step = None;
            if leader.is_none() {
                deck.sync_phase = Phase::None;
            }
        }
    }
    /// Set or re-arm a deck's sync behavior.
    /// Takes a validated deck and mode; changes intent without starting a stopped deck.
    pub(super) fn deck_sync_mode(&mut self, index: u8, mode: Mode) {
        let index = usize::from(index);
        if index >= 2
            || mode != Mode::Off && self.deck_sync.leader.and_then(Leader::deck) == Some(index)
        {
            return;
        }
        if mode != Mode::Off && self.deck_sync.leader.is_none() {
            self.deck_sync_leader(Some(Leader::Transport));
        }
        let tempo = self.deck_sync_leader_bpm();
        let deck = &mut self.decks[index];
        deck.sync = mode != Mode::Off;
        if deck.sync {
            if let Some(bpm) = tempo {
                deck.sync_bpm = bpm;
            }
        }
        deck.sync_phase = match mode {
            Mode::Beat => Phase::Beat,
            Mode::Bar => Phase::Bar,
            _ => Phase::None,
        };
        deck.sync_phase_locked = false;
        deck.sync_step = None;
    }
    /// Preserve a performed offset before a source movement.
    /// Takes its deck; keeps the shared tempo and disengages the affected phase followers.
    pub(super) fn deck_sync_manipulation(&mut self, index: usize) {
        if index >= 2 {
            return;
        }
        if self.deck_sync.leader.and_then(Leader::deck) == Some(index) {
            for deck in &mut self.decks {
                deck.sync_disengage_phase();
            }
        } else {
            self.decks[index].sync_disengage_phase();
        }
    }
    /// Read whether the selected clock can lead ordinary forward playback.
    /// Takes the renderer; returns false for independent targets, stopped transport or manipulated/unprepared decks.
    pub(crate) fn deck_sync_ready(&self) -> bool {
        match self.deck_sync.leader {
            None => false,
            Some(Leader::Transport) => self.playing && self.count_in.is_none(),
            Some(leader) => self.decks[leader.deck().unwrap()].sync_forward(),
        }
    }
    /// Render each deck once against the same emitted leader frame.
    /// Takes the renderer; returns fixed stereo outputs and taps for both native routing and the default mixer.
    pub(super) fn render_deck_pair(&mut self) -> Pair {
        let mut pair = Pair {
            audio: [[0.0; 2]; 2],
            taps: [[[0.0; 2]; 2]; 2],
        };
        let leader = self.deck_sync.leader;
        let first = leader.and_then(Leader::deck).unwrap_or(0);
        let ready = self.deck_sync_ready();
        let origin = self
            .conductor
            .as_ref()
            .and_then(|map| map.native)
            .map_or_else(
                || self.scenes.timing.map_or(0.0, |timing| timing.anchor),
                |settings| settings.pickup,
            );
        let identity = match leader {
            Some(Leader::Transport) => [0, self.transport_epoch, origin.to_bits()],
            Some(_) => [
                self.decks[first]
                    .audio
                    .as_ref()
                    .map_or(0, |audio| std::sync::Arc::as_ptr(audio) as usize as u64),
                self.decks[first].history_key,
                0,
            ],
            None => [0; 3],
        };
        if self.deck_sync.active && (!ready || identity != self.deck_sync.identity) {
            for deck in &mut self.decks {
                if deck.sync_phase_locked {
                    deck.sync_disengage_phase();
                }
            }
        }
        self.deck_sync.active = ready;
        self.deck_sync.identity = identity;
        let before = self.decks[first].grid_beat_at(self.decks[first].pos, self.sr, self.bpm);
        let mut reference = (ready && leader == Some(Leader::Transport)).then_some((
            self.precise_midi_beat() - self.last_midi_step - origin,
            self.last_midi_step,
        ));
        for index in [first, 1 - first] {
            if let Some((beat, step)) = reference {
                if leader.and_then(Leader::deck) != Some(index) {
                    self.deck_sync_step(index, beat, step);
                }
            } else if !self.decks[index].sync_forward() && self.decks[index].sync_phase_locked {
                self.decks[index].sync_disengage_phase();
            }
            let timer = self.load_profile.start();
            let (left, right) = self.render_deck(index);
            self.load_profile.deck(index, timer);
            pair.audio[index] = [left, right];
            pair.taps[index] = self.routing_deck_taps;
            if ready && leader.and_then(Leader::deck) == Some(index) {
                let deck = &self.decks[index];
                let after = deck.grid_beat_at(deck.pos, self.sr, self.bpm);
                let mut step = after - before;
                if step < 0.0 && deck.loop_on && deck.loop_len > 1.0 && deck.rate > 0.0 {
                    step += deck.grid_beats_between(
                        deck.loop_start,
                        deck.loop_start + deck.loop_len,
                        self.sr,
                        self.bpm,
                    );
                }
                if step.is_finite() && step > 0.0 && step <= 1000.0 / (60.0 * f64::from(self.sr)) {
                    let wrapped = after < before && deck.loop_on;
                    let span = if wrapped {
                        Some(deck.grid_beats_between(
                            deck.loop_start,
                            deck.loop_start + deck.loop_len,
                            self.sr,
                            self.bpm,
                        ))
                    } else {
                        None
                    };
                    if let Some(span) = span {
                        for follower in &mut self.decks {
                            if follower.sync_phase.period().is_some_and(|period| {
                                (span / period - (span / period).round()).abs() > 1e-8
                            }) {
                                follower.sync_disengage_phase();
                            }
                        }
                    }
                    reference = Some((before, step));
                } else {
                    for deck in &mut self.decks {
                        if deck.sync_phase_locked {
                            deck.sync_disengage_phase();
                        }
                    }
                }
            }
        }
        pair
    }
    /// Advance one follower through its own immutable source beat map.
    /// Takes deck, emitted leader start beat and exact beat interval; prepares a heap-free renderer step and arms phase at a bounded source position.
    fn deck_sync_step(&mut self, index: usize, reference: f64, step: f64) {
        if !step.is_finite() || step <= 0.0 {
            return;
        }
        let deck = &mut self.decks[index];
        if !deck.sync {
            return;
        }
        deck.sync_bpm = (step * 60.0 * f64::from(self.sr)).clamp(1.0, 1000.0) as f32;
        if !deck.sync_forward() {
            if deck.sync_phase_locked {
                deck.sync_disengage_phase();
            }
            return;
        }
        if let Some(period) = deck.sync_phase.period() {
            if deck.loop_on
                && (deck.grid_beats_between(
                    deck.loop_start,
                    deck.loop_start + deck.loop_len,
                    self.sr,
                    self.bpm,
                ) / period
                    - (deck.grid_beats_between(
                        deck.loop_start,
                        deck.loop_start + deck.loop_len,
                        self.sr,
                        self.bpm,
                    ) / period)
                        .round())
                .abs()
                    > 1e-8
            {
                deck.sync_disengage_phase();
            } else if !deck.sync_phase_locked {
                let audio = deck.audio.as_ref().unwrap();
                let start = if deck.loop_on { deck.loop_start } else { 0.0 };
                let end = if deck.loop_on {
                    deck.loop_start + deck.loop_len
                } else {
                    audio.frames().saturating_sub(1) as f64
                };
                let low =
                    ((deck.grid_beat_at(start, self.sr, self.bpm) - reference) / period).ceil();
                let high =
                    ((deck.grid_beat_at(end, self.sr, self.bpm) - reference) / period).floor();
                if low <= high {
                    let current = deck.grid_beat_at(deck.pos, self.sr, self.bpm);
                    let beat = reference
                        + ((current - reference) / period).round().clamp(low, high) * period;
                    deck.transition_to(
                        deck.grid_position_at(beat, self.sr, self.bpm),
                        self.sr,
                        DeckTransition::Jump,
                    );
                    deck.sync_phase_locked = true;
                } else {
                    deck.sync_disengage_phase();
                }
            }
        }
        if deck.follows_spindle() && deck.sync_phase.is_none() {
            return;
        }
        let beat = deck.grid_beat_at(deck.pos, self.sr, self.bpm);
        let position = deck.grid_position_at(beat + step, self.sr, self.bpm);
        let source_rate = f64::from(deck.audio.as_ref().unwrap().sr);
        let rate = ((position - deck.pos) / source_rate * f64::from(self.sr)) as f32;
        if position.is_finite() && rate.is_finite() && rate > 0.0 {
            deck.sync_step = Some((position, rate));
        }
    }
}

#[cfg(test)]
mod tests;
