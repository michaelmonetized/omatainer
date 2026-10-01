//! Typed inverse values. Applying one swaps it with the corresponding current
//! value; that same owned patch then becomes redo. No whole-engine replacement.
use super::super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rack {
    Track(u8),
    Scene(u8),
}
impl Rack {
    pub fn selected(rt: &RtEngine) -> Option<Self> {
        // Follow active_chain exactly, including commands received while the
        // panel is closed. History must target the rack the renderer mutates.
        Some(if rt.fx_view >= 100 {
            Self::Scene(((rt.fx_view as usize - 100).min(SCENES - 1)) as u8)
        } else if rt.fx_view >= 0 {
            Self::Track((rt.fx_view as usize % TRACKS) as u8)
        } else {
            Self::Scene(0)
        })
    }
    pub fn get(self, rt: &RtEngine) -> &fx::FxChain {
        match self {
            Self::Track(t) => &rt.tracks[t as usize].fx,
            Self::Scene(s) => &rt.scene_fx[s as usize],
        }
    }
    pub fn get_mut(self, rt: &mut RtEngine) -> &mut fx::FxChain {
        match self {
            Self::Track(t) => &mut rt.tracks[t as usize].fx,
            Self::Scene(s) => &mut rt.scene_fx[s as usize],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Global {
    bpm: f32,
    quant: f32,
    quantize: bool,
    metronome: bool,
    xfader: f32,
    master: f32,
    cue_mix: f32,
    fx_kind: [FxKind; 3],
    fx_wet: [f32; 3],
    bank: usize,
    instrument: SamplerInstrument,
    octave: i8,
    synth: SynthInstrument,
    cutoff: f32,
}
impl Global {
    pub fn get(rt: &RtEngine) -> Self {
        Self {
            bpm: rt.bpm,
            quant: rt.quant,
            quantize: rt.quantize,
            metronome: rt.metronome,
            xfader: rt.xfader,
            master: rt.master,
            cue_mix: rt.cue_mix,
            fx_kind: rt.fx_kind,
            fx_wet: rt.fx_wet,
            bank: rt.sampler_bank,
            instrument: rt.sampler_inst,
            octave: rt.sampler_oct,
            synth: rt.sampler_poly.kind,
            cutoff: rt.sampler_poly.cutoff,
        }
    }
    fn swap(&mut self, rt: &mut RtEngine) {
        let current = Self::get(rt);
        rt.bpm = self.bpm;
        rt.quant = self.quant;
        rt.quantize = self.quantize;
        if rt.metronome != self.metronome {
            rt.metro.reset();
        }
        rt.metronome = self.metronome;
        rt.xfader = self.xfader;
        rt.master = self.master;
        rt.cue_mix = self.cue_mix;
        for slot in 0..3 {
            if rt.fx_kind[slot] != self.fx_kind[slot] {
                rt.master_fx[slot].reset(self.fx_kind[slot]);
            }
        }
        rt.fx_kind = self.fx_kind;
        rt.fx_wet = self.fx_wet;
        rt.sampler_bank = self.bank;
        if rt.sampler_inst != self.instrument {
            rt.apply(Command::SamplerInst(self.instrument));
        }
        rt.sampler_poly.kind = self.synth;
        rt.sampler_poly.cutoff = self.cutoff;
        if rt.sampler_oct != self.octave {
            rt.apply(Command::SamplerOct(self.octave - rt.sampler_oct));
        }
        *self = current;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TrackControls {
    gain: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    armed: bool,
}
impl TrackControls {
    pub fn get(track: &TrackRt) -> Self {
        Self {
            gain: track.gain,
            pan: track.pan,
            mute: track.mute,
            solo: track.solo,
            armed: track.armed,
        }
    }
    fn swap(&mut self, track: &mut TrackRt) {
        let current = Self::get(track);
        track.gain = self.gain;
        track.pan = self.pan;
        track.mute = self.mute;
        track.solo = self.solo;
        track.armed = self.armed;
        *self = current;
    }
}

/// Persistent deck controls, excluding the advancing playhead and physical
/// scratch owners. A knob undo must not rewind another performed movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DeckControls {
    pitch: f32,
    sync: bool,
    sync_bpm: f32,
    gain: f32,
    eq: [f32; 3],
    eq_cut: [bool; 4],
    eq_solo: i8,
    eq_store: [f32; 4],
    filter: f32,
    pfl: bool,
    vinyl: bool,
    keylock: bool,
    pitch_range: u8,
    cue: f64,
    hotcues: [Option<f64>; HOTCUES],
    cue_styles: [super::super::cue_metadata::Style; HOTCUES],
    grid: Option<super::super::beatgrid::Grid>,
    loop_on: bool,
    loop_start: f64,
    loop_len: f64,
}
impl DeckControls {
    pub fn get(deck: &DeckRt) -> Self {
        Self {
            pitch: deck.pitch,
            sync: deck.sync,
            sync_bpm: deck.sync_bpm,
            gain: deck.gain,
            eq: [deck.eq[0].low_g, deck.eq[0].mid_g, deck.eq[0].high_g],
            eq_cut: deck.eq_cut,
            eq_solo: deck.eq_solo,
            eq_store: deck.eq_store,
            filter: deck.filter_amt,
            pfl: deck.pfl,
            vinyl: deck.vinyl,
            keylock: deck.keylock,
            pitch_range: deck.pitch_range,
            cue: deck.cue_pos,
            cue_styles: deck.cue_styles,
            grid: deck.grid,
            hotcues: std::array::from_fn(|i| deck.hotcues[i].set.then_some(deck.hotcues[i].pos)),
            loop_on: deck.loop_on,
            loop_start: deck.loop_start,
            loop_len: deck.loop_len,
        }
    }
    fn swap(&mut self, deck: &mut DeckRt, sr: f32) {
        let current = Self::get(deck);
        let jump = self.keylock != deck.keylock
            || self.loop_on != deck.loop_on
            || self.loop_start != deck.loop_start
            || self.loop_len != deck.loop_len;
        deck.pitch = self.pitch;
        deck.sync = self.sync;
        deck.sync_bpm = self.sync_bpm;
        deck.gain = self.gain;
        deck.eq_cut = self.eq_cut;
        deck.eq_solo = self.eq_solo;
        deck.eq_store = self.eq_store;
        for eq in &mut deck.eq {
            [eq.low_g, eq.mid_g, eq.high_g] = self.eq;
        }
        deck.filter_amt = self.filter;
        deck.pfl = self.pfl;
        deck.vinyl = self.vinyl;
        deck.keylock = self.keylock;
        deck.pitch_range = self.pitch_range;
        deck.cue_pos = self.cue;
        deck.cue_styles = self.cue_styles;
        deck.grid = self.grid;
        deck.hotcues = self.hotcues.map(|p| HotCue {
            set: p.is_some(),
            pos: p.unwrap_or(0.0),
        });
        deck.loop_on = self.loop_on;
        deck.loop_start = self.loop_start;
        deck.loop_len = self.loop_len;
        if jump {
            deck.transition_to(deck.pos, sr, DeckTransition::Jump);
        }
        *self = current;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Effect {
    on: bool,
    mix: f32,
    p: [f32; 4],
}
impl Effect {
    pub fn get(slot: &fx::FxSlot) -> Self {
        Self {
            on: slot.on,
            mix: slot.mix,
            p: slot.p,
        }
    }
    fn swap(&mut self, slot: &mut fx::FxSlot) {
        let current = Self::get(slot);
        slot.on = self.on;
        slot.mix = self.mix;
        slot.p = self.p;
        *self = current;
    }
}

pub(super) enum Patch {
    Global(Global),
    Track(u8, TrackControls),
    ClipGain {
        track: u8,
        scene: u8,
        gain: f32,
    },
    Deck(u8, DeckControls),
    Position {
        deck: u8,
        position: f64,
    },
    Effect {
        rack: Rack,
        index: usize,
        value: Effect,
    },
    /// None removes the slot into this patch; Some reinserts the owned DSP.
    Slot {
        rack: Rack,
        index: usize,
        slot: Option<fx::FxSlot>,
        reserved_bytes: usize,
        id: fx::FxId,
    },
    Clip {
        track: u8,
        scene: u8,
        value: Clip,
        spare_notes: Vec<MidiNote>,
    },
    Media {
        deck: u8,
        audio: Option<Arc<Sample>>,
        title: String,
        bpm: f32,
        position: f64,
        controls: DeckControls,
        receipt: PinnedReceipt,
        reserved_media: Option<Arc<Sample>>,
        reserved_original: Option<Arc<Sample>>,
    },
}

pub(super) struct PinnedReceipt(Option<load_receipt::Receipt>);
impl PinnedReceipt {
    pub fn new(receipt: Option<load_receipt::Receipt>) -> Self {
        if let Some(receipt) = &receipt {
            receipt.pin_history();
        }
        Self(receipt)
    }
    fn swap(&mut self, current: &mut Option<load_receipt::Receipt>) {
        // Publish Current before removing the restored identity's last pin;
        // the GUI must never observe a terminal, unpinned gap and retire it.
        if let Some(receipt) = &self.0 {
            receipt.restore_from_history();
        }
        if let Some(receipt) = current.as_ref() {
            receipt.pin_history();
            receipt.supersede();
        }
        std::mem::swap(&mut self.0, current);
        if let Some(receipt) = current.as_ref() {
            receipt.unpin_history();
        }
    }
}
impl Drop for PinnedReceipt {
    fn drop(&mut self) {
        if let Some(receipt) = &self.0 {
            receipt.unpin_history();
        }
    }
}

impl Patch {
    pub fn target_label(&self) -> super::TargetLabel {
        use super::TargetLabel as T;
        match self {
            Self::Global(_) => T::None,
            Self::Track(t, _) => T::Track(*t),
            Self::ClipGain { track, scene, .. } | Self::Clip { track, scene, .. } => {
                T::Clip(*track, *scene)
            }
            Self::Deck(d, _) | Self::Position { deck: d, .. } | Self::Media { deck: d, .. } => {
                T::Deck(*d)
            }
            Self::Effect { rack, index, .. } | Self::Slot { rack, index, .. } => {
                T::Effect(*rack, *index)
            }
        }
    }
    pub fn valid(&self, rt: &RtEngine) -> bool {
        match self {
            Self::Effect { rack, index, .. } => *index < rack.get(rt).slots.len(),
            Self::Slot {
                rack, index, slot, ..
            } => {
                if slot.is_some() {
                    *index <= rack.get(rt).slots.len()
                        && rack.get(rt).slots.len() < rack.get(rt).slots.capacity()
                } else {
                    *index < rack.get(rt).slots.len()
                }
            }
            _ => true,
        }
    }
    pub fn apply(&mut self, rt: &mut RtEngine) {
        match self {
            Self::Global(value) => value.swap(rt),
            Self::Track(track, value) => value.swap(&mut rt.tracks[*track as usize]),
            Self::ClipGain { track, scene, gain } => std::mem::swap(
                gain,
                &mut rt.tracks[*track as usize].clips[*scene as usize].gain,
            ),
            Self::Deck(deck, value) => value.swap(&mut rt.decks[*deck as usize], rt.sr),
            Self::Position { deck, position } => {
                let deck = &mut rt.decks[*deck as usize];
                let current = deck.pos;
                deck.transition_to(*position, rt.sr, DeckTransition::Jump);
                *position = current;
            }
            Self::Effect { rack, index, value } => value.swap(&mut rack.get_mut(rt).slots[*index]),
            Self::Slot {
                rack, index, slot, ..
            } => {
                let slots = &mut rack.get_mut(rt).slots;
                if let Some(value) = slot.take() {
                    slots.insert(*index, value);
                } else {
                    *slot = Some(slots.remove(*index));
                }
            }
            Self::Clip {
                track,
                scene,
                value,
                ..
            } => {
                let t = *track as usize;
                let s = *scene as usize;
                // Redo retains the actual duration captured up to this undo,
                // not the temporary quarter-beat onset preview. Only this
                // target is finalized; its live input voice stays owned.
                rt.capture_held_clip_durations(t, s, value);
                rt.finish_recording_clip(t, s);
                rt.cancel_recording_clip(t, s);
                let track = &mut rt.tracks[t];
                if track
                    .playing
                    .or(track.project_resume)
                    .is_some_and(|p| p.scene as usize == s)
                {
                    track.release_clip_notes();
                }
                std::mem::swap(value, &mut track.clips[s]);
                track.clip_notes_changed(s, rt.beat);
            }
            Self::Media {
                deck,
                audio,
                title,
                bpm,
                position,
                controls,
                receipt,
                ..
            } => {
                let d = &mut rt.decks[*deck as usize];
                std::mem::swap(audio, &mut d.audio);
                std::mem::swap(title, &mut d.title);
                std::mem::swap(bpm, &mut d.bpm);
                controls.swap(d, rt.sr);
                let current = d.pos;
                d.transition_to(*position, rt.sr, DeckTransition::Jump);
                *position = current;
                d.playing = false;
                d.playback_active = false;
                // A media swap cannot retain a physical scratch session on the
                // replaced buffer; unrelated deck and live MIDI gates survive.
                d.touch_sources.fill(None);
                d.touching = false;
                d.scratch = 0.0;
                receipt.swap(&mut d.load_receipt);
            }
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::Slot { reserved_bytes, .. } => *reserved_bytes,
            Self::Clip {
                value, spare_notes, ..
            } => {
                value.name.capacity()
                    + (value.notes.capacity() + spare_notes.capacity()).max(2 * super::NOTE_LIMIT)
                        * std::mem::size_of::<MidiNote>()
            }
            Self::Media { title, .. } => title.capacity(),
            _ => 0,
        }
    }
    /// Immutable reservations keep registry membership stable across swaps.
    /// Current track commands do not replace clip audio; deck patches explicitly
    /// retain both original and replacement samples, including while undone.
    pub fn media_reservations(&self, mut add: impl FnMut(usize, usize)) {
        let mut visit = |audio: &Option<Arc<Sample>>| {
            if let Some(audio) = audio {
                add(Arc::as_ptr(audio) as usize, sample_bytes(audio));
            }
        };
        match self {
            Self::Clip { value, .. } => visit(&value.audio),
            Self::Media {
                reserved_original,
                reserved_media,
                ..
            } => {
                visit(reserved_original);
                visit(reserved_media);
            }
            _ => {}
        }
    }
}

pub(super) fn sample_bytes(sample: &Sample) -> usize {
    std::mem::size_of::<Sample>()
        + 4 * std::mem::size_of::<usize>() // Sample and peaks Arc counters.
        + std::mem::size_of::<Vec<[f32; 3]>>()
        + sample.name.capacity()
        + sample.path.capacity()
        + sample.data.capacity() * std::mem::size_of::<f32>()
        + sample.peaks.capacity() * std::mem::size_of::<[f32; 3]>()
}
