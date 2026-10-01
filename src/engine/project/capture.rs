use super::{model::*, *};

pub(super) struct Frame {
    pub state: State,
    pub media: Vec<Arc<Sample>>,
    pub revision: u64,
    pub checkpoint: undo::Checkpoint,
    pub playback_receipts: [Option<load_receipt::Receipt>; DECKS],
    pub complete: bool,
    pub error: Option<&'static str>,
    shape: Shape,
}

#[derive(Default)]
struct Shape {
    names: [usize; TRACKS],
    clips: [[usize; SCENES]; TRACKS],
    notes: [[usize; SCENES]; TRACKS],
    track_fx: [usize; TRACKS],
    scene_fx: [usize; SCENES],
    titles: [usize; DECKS],
    banks: usize,
    bank_names: [usize; MAX_BANKS],
}

impl Frame {
    pub fn new() -> Self {
        Self {
            state: State::blank(),
            media: Vec::with_capacity(MAX_MEDIA_REFS),
            checkpoint: undo::Checkpoint::default(),
            revision: 0,
            playback_receipts: [None, None],
            complete: false,
            error: None,
            shape: Shape::default(),
        }
    }

    /// Worker only. Audio reports the required capacities without growing any
    /// storage. A later callback copies one coherent block-boundary state.
    pub fn prepare(&mut self) {
        for (i, track) in self.state.tracks.iter_mut().enumerate() {
            reserve_string(&mut track.name, self.shape.names[i]);
            track
                .fx
                .reserve(self.shape.track_fx[i].saturating_sub(track.fx.len()));
            for (j, clip) in track.clips.iter_mut().enumerate() {
                reserve_string(&mut clip.name, self.shape.clips[i][j]);
                clip.notes
                    .reserve(self.shape.notes[i][j].saturating_sub(clip.notes.len()));
            }
        }
        for (i, rack) in self.state.scene_fx.iter_mut().enumerate() {
            rack.reserve(self.shape.scene_fx[i].saturating_sub(rack.len()));
        }
        for (i, deck) in self.state.decks.iter_mut().enumerate() {
            reserve_string(&mut deck.title, self.shape.titles[i]);
        }
        self.state.banks.resize_with(self.shape.banks, || Bank {
            name: String::new(),
            media: [0; 16],
        });
        for (i, bank) in self.state.banks.iter_mut().enumerate() {
            reserve_string(&mut bank.name, self.shape.bank_names[i]);
        }
    }

    pub fn capture(&mut self, rt: &RtEngine) {
        debug_assert!(self.media.is_empty());
        self.error = None;
        self.complete = false;
        if rt.tracks.len() != TRACKS
            || rt.pad_banks.len() != rt.sampler_banks.len()
            || rt.pad_banks.len() > MAX_BANKS
        {
            self.error = Some("unsupported track or sample-bank count");
            return;
        }
        let shape = &mut self.shape;
        let mut fits = true;
        let mut notes = 0;
        for (i, track) in rt.tracks.iter().enumerate() {
            shape.names[i] = track.name.len();
            shape.track_fx[i] = track.fx.slots.len();
            if track.name.len() > MAX_TEXT_BYTES || track.fx.slots.len() > MAX_FX_PER_RACK {
                self.error = Some("track name or rack exceeds project format limits");
                return;
            }
            fits &= self.state.tracks[i].name.capacity() >= shape.names[i]
                && self.state.tracks[i].fx.capacity() >= shape.track_fx[i];
            for (j, clip) in track.clips.iter().enumerate() {
                shape.clips[i][j] = clip.name.len();
                shape.notes[i][j] = clip.notes.len();
                if clip.name.len() > MAX_TEXT_BYTES || clip.notes.len() > MAX_NOTES_PER_CLIP {
                    self.error = Some("clip name or note count exceeds project format limits");
                    return;
                }
                notes += clip.notes.len();
                fits &= self.state.tracks[i].clips[j].name.capacity() >= clip.name.len()
                    && self.state.tracks[i].clips[j].notes.capacity() >= clip.notes.len();
            }
        }
        if notes > MAX_TOTAL_NOTES {
            self.error = Some("project exceeds 65536 notes");
            return;
        }
        for (i, rack) in rt.scene_fx.iter().enumerate() {
            shape.scene_fx[i] = rack.slots.len();
            if rack.slots.len() > MAX_FX_PER_RACK {
                self.error = Some("scene rack exceeds project format limits");
                return;
            }
            fits &= self.state.scene_fx[i].capacity() >= rack.slots.len();
        }
        for (i, deck) in rt.decks.iter().enumerate() {
            shape.titles[i] = deck.title.len();
            if deck.title.len() > MAX_TEXT_BYTES {
                self.error = Some("deck title exceeds project format limits");
                return;
            }
            fits &= self.state.decks[i].title.capacity() >= deck.title.len();
        }
        shape.banks = rt.sampler_banks.len();
        fits &= self.state.banks.len() == shape.banks;
        for (i, name) in rt.sampler_banks.iter().enumerate() {
            shape.bank_names[i] = name.len();
            if name.len() > MAX_TEXT_BYTES {
                self.error = Some("bank name exceeds project format limits");
                return;
            }
            fits &= self
                .state
                .banks
                .get(i)
                .is_some_and(|b| b.name.capacity() >= name.len());
        }
        if !fits {
            return;
        }
        let target = &mut self.state;
        macro_rules! scalars { ($($field:ident),* $(,)?) => { $(target.$field = rt.$field;)* }; }
        scalars!(
            bpm,
            beat,
            quant,
            quantize,
            metronome,
            view,
            xfader,
            xfader_curve,
            master,
            cue_mix,
            selected_track,
            selected_scene,
            selected_deck,
            fx_view,
            fx_kind,
            fx_wet,
            sampler_bank,
            sampler_inst,
            sampler_oct
        );
        target.sampler_synth = synth(&rt.sampler_poly);
        for (i, track) in rt.tracks.iter().enumerate() {
            let out = &mut target.tracks[i];
            copy_string(&mut out.name, &track.name);
            out.scene_bus = track.scene_bus;
            out.launch = track.playing.or(track.project_resume).map(|p| Launch {
                scene: p.scene,
                start_beat: p.start_beat,
                looping: p.looping,
            });
            out.gain = track.gain;
            out.pan = track.pan;
            out.mute = track.mute;
            out.solo = track.solo;
            out.armed = track.armed;
            out.kind = track.kind;
            out.synth = synth(&track.poly);
            out.eq = eq(&track.eq);
            for (j, clip) in track.clips.iter().enumerate() {
                let saved = &mut out.clips[j];
                saved.kind = clip.kind;
                copy_string(&mut saved.name, &clip.name);
                saved.bars = clip.bars;
                saved.gain = clip.gain;
                saved.notes.clear();
                saved.notes.extend_from_slice(&clip.notes);
                saved.audio = clip.audio.as_ref().map(|s| media(&mut self.media, s));
            }
            for (j, sample) in track.drum_samples.iter().enumerate() {
                out.drums[j] = media(&mut self.media, sample);
            }
            effects(&mut out.fx, &track.fx);
        }
        for (i, deck) in rt.decks.iter().enumerate() {
            let out = &mut target.decks[i];
            macro_rules! fields { ($($field:ident),* $(,)?) => { $(out.$field = deck.$field;)* }; }
            fields!(
                pos,
                cue_pos,
                pitch,
                vinyl,
                keylock,
                sync,
                gain,
                filter_morph,
                filter_amt,
                pfl,
                loop_on,
                loop_start,
                loop_len,
                bpm,
                eq_cut,
                eq_solo,
                eq_store,
                pitch_range,
                sync_bpm
            );
            copy_string(&mut out.title, &deck.title);
            out.eq = eq(&deck.eq[0]);
            out.audio = deck.audio.as_ref().map(|s| media(&mut self.media, s));
            out.cue_styles = deck.cue_styles;
            out.grid = deck.grid;
            out.hotcues =
                std::array::from_fn(|j| deck.hotcues[j].set.then_some(deck.hotcues[j].pos));
        }
        for (out, rack) in target.scene_fx.iter_mut().zip(&rt.scene_fx) {
            effects(out, rack);
        }
        for (i, bank) in target.banks.iter_mut().enumerate() {
            copy_string(&mut bank.name, &rt.sampler_banks[i]);
            for (j, sample) in rt.pad_banks[i].iter().enumerate() {
                bank.media[j] = media(&mut self.media, sample);
            }
        }
        target.builtin =
            std::array::from_fn(|i| rt.builtin[i].as_ref().map(|s| media(&mut self.media, s)));
        rt.capture_held_durations(target);
        self.playback_receipts = std::array::from_fn(|i| rt.decks[i].load_receipt.clone());
        self.revision = rt.project.revision();
        self.checkpoint = rt.undo.checkpoint();
        self.complete = true;
    }
}

fn reserve_string(value: &mut String, needed: usize) {
    value.reserve(needed.saturating_sub(value.len()));
}
fn copy_string(value: &mut String, source: &str) {
    value.clear();
    value.push_str(source);
}
fn media(values: &mut Vec<Arc<Sample>>, sample: &Arc<Sample>) -> usize {
    let index = values.len();
    debug_assert!(index < values.capacity());
    values.push(sample.clone());
    index
}
fn synth(poly: &Poly) -> Synth {
    Synth {
        kind: poly.kind,
        voices: poly.voices.len(),
        cutoff: poly.cutoff,
        tuning_hz: poly.tuning_hz(),
    }
}
fn eq(value: &ThreeBand) -> [f32; 3] {
    [value.low_g, value.mid_g, value.high_g]
}
fn effects(out: &mut Vec<Effect>, rack: &fx::FxChain) {
    out.clear();
    out.extend(rack.slots.iter().map(|s| Effect {
        id: s.id(),
        on: s.on,
        mix: s.mix,
        p: s.p,
    }));
}
