use super::{model::*, *};

pub(super) struct Frame {
    pub state: State,
    pub arrangement: Option<Arc<arrangement::Plan>>,
    pub media: Vec<Arc<Sample>>,
    pub revision: u64,
    pub checkpoint: undo::Checkpoint,
    pub playback_receipts: [Option<load_receipt::Receipt>; DECKS],
    pub complete: bool,
    pub error: Option<&'static str>,
    plugins: Vec<(u64, Option<crate::plugin_host::realtime::Control>, [Option<audio::routing::plugins::Parameter>; audio::routing::plugins::MAX_PARAMETERS])>,
    shape: Shape,
}

struct Shape {
    names: Vec<usize>,
    clips: Vec<Vec<usize>>,
    notes: Vec<Vec<usize>>,
    track_fx: Vec<usize>,
    scene_fx: Vec<usize>,
    titles: [usize; DECKS],
    track_count: usize,
    scene_count: usize,
    scene_names: Vec<usize>,
    banks: usize,
    bank_names: [usize; MAX_BANKS],
}

impl Default for Shape {
    fn default() -> Self { Self { names: vec![0; session::MAX_TRACKS], clips: vec![vec![0;session::MAX_SCENES];session::MAX_TRACKS], notes: vec![vec![0;session::MAX_SCENES];session::MAX_TRACKS], track_fx: vec![0;session::MAX_TRACKS], scene_fx: vec![0;session::MAX_SCENES], titles:[0;DECKS], banks:0, bank_names:[0;MAX_BANKS], track_count:TRACKS, scene_count:SCENES, scene_names:vec![0;session::MAX_SCENES] } }
}

impl Frame {
    pub fn new() -> Self {
        Self {
            state: { let mut state = State::blank(); state.session = Some(session::Layout::legacy((0..TRACKS).map(|_| String::new()), SCENES)); state },
            arrangement: None,
            media: Vec::with_capacity(MAX_MEDIA_REFS),
            checkpoint: undo::Checkpoint::default(),
            revision: 0,
            playback_receipts: [None, None],
            complete: false,
            error: None,
            plugins: Vec::with_capacity(audio::routing::plugins::MAX_PLUGINS),
            shape: Shape::default(),
        }
    }

    /// Worker only. Audio reports the required capacities without growing any
    /// storage. A later callback copies one coherent block-boundary state.
    pub fn prepare(&mut self) {
        let blank = State::blank();
        self.state.tracks.resize_with(self.shape.track_count, || blank.tracks[0].clone());
        for track in &mut self.state.tracks { track.clips.resize_with(self.shape.scene_count, || blank.tracks[0].clips[0].clone()); }
        self.state.scene_fx.resize_with(self.shape.scene_count, Vec::new);
        self.state.session.as_mut().unwrap().prepare_storage(&self.shape.names[..self.shape.track_count], &self.shape.scene_names[..self.shape.scene_count]);
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
            media: [None; 16],
            instance: None,
            settings: None,
        });
        for (i, bank) in self.state.banks.iter_mut().enumerate() {
            reserve_string(&mut bank.name, self.shape.bank_names[i]);
        }
    }

    pub fn capture(&mut self, rt: &RtEngine) {
        debug_assert!(self.media.is_empty());
        self.error = None;
        self.complete = false;
        if rt.session.tracks.len() > session::MAX_TRACKS || rt.session.scenes.len() > session::MAX_SCENES
            || rt.sampler_banks.len() > MAX_BANKS
        {
            self.error = Some("unsupported track or sample-bank count");
            return;
        }
        let shape = &mut self.shape;
        shape.track_count = rt.session.tracks.len(); shape.scene_count = rt.session.scenes.len();
        for (out,item) in shape.scene_names.iter_mut().zip(&rt.session.scenes) { *out = item.name.len(); }
        let mut fits = self.state.tracks.len() == shape.track_count && self.state.scene_fx.len() == shape.scene_count && self.state.session.as_ref().is_some_and(|layout| layout.fits(&rt.session));
        let mut notes = 0;
        for (i, track) in rt.tracks.iter().take(shape.track_count).enumerate() {
            shape.names[i] = track.name.len().max(rt.session.tracks[i].name.len());
            shape.track_fx[i] = track.fx.slots.len();
            if track.name.len() > MAX_TEXT_BYTES || track.fx.slots.len() > MAX_FX_PER_RACK {
                self.error = Some("track name or rack exceeds project format limits");
                return;
            }
            fits &= self.state.tracks.get(i).is_some_and(|out| out.name.capacity() >= shape.names[i] && out.fx.capacity() >= shape.track_fx[i] && out.clips.len() == shape.scene_count);
            for (j, clip) in track.clips.iter().take(shape.scene_count).enumerate() {
                shape.clips[i][j] = clip.name.len();
                shape.notes[i][j] = clip.notes.len();
                if clip.name.len() > MAX_TEXT_BYTES || clip.notes.len() > MAX_NOTES_PER_CLIP {
                    self.error = Some("clip name or note count exceeds project format limits");
                    return;
                }
                notes += clip.notes.len();
                fits &= self.state.tracks.get(i).and_then(|t| t.clips.get(j)).is_some_and(|out| out.name.capacity() >= clip.name.len() && out.notes.capacity() >= clip.notes.len());
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
            fits &= self.state.scene_fx.get(i).is_some_and(|out| out.capacity() >= rack.slots.len());
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
        for (i, working) in rt.sampler_banks.iter().enumerate() {
            let name = working.name();
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
        target.session.as_mut().unwrap().copy_from_prepared(&rt.session);
        target.timeline_seconds = rt.timeline_seconds();
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
            sampler_oct,
            musical_context,
            sampler_scale
        );
        target.conductor = rt.conductor.clone();
        target.navigation = rt.navigation.saved.clone();
        target.migration = rt.migration.clone();
        target.scene_timing = rt.scenes.timing;
        target.sync_leader = rt.deck_sync.leader;
        target.routing = rt.routing.as_ref().map(|routing| routing.model.clone());
        self.plugins.clear();
        if let Some(graph) = &rt.routing {
            for (saved, plugin) in graph.model.plugins.iter().zip(&graph.plugins) { self.plugins.push((saved.id,plugin.endpoint.as_ref().map(|e| e.control.clone()),plugin.parameters)); }
        }
        target.mic_aux = rt.mic_aux.configuration();
        target.sampler_synth = synth(&rt.sampler_poly);
        for (i, track) in rt.tracks.iter().take(self.shape.track_count).enumerate() {
            let out = &mut target.tracks[i];
            copy_string(&mut out.name, &track.name);
            copy_string(&mut target.session.as_mut().unwrap().tracks[i].name, &track.name);
            out.scene_bus = track.scene_bus;
            out.launch = track.playing.or(track.project_resume).map(|p| Launch {
                scene: p.scene,
                start_beat: if track.clips[p.scene as usize].region.is_some() {
                    rt.beat - (rt.precise_midi_beat() - p.midi_start_beat)
                } else { p.start_beat },
                looping: p.looping,
            });
            out.gain = track.gain;
            out.pan = track.pan;
            out.mute = track.mute;
            out.solo = track.solo;
            out.armed = track.armed;
            out.input_monitor = track.input_monitor;
            out.kind = track.kind;
            out.synth = synth(&track.poly);
            out.eq = eq(&track.eq);
            for (j, clip) in track.clips.iter().take(self.shape.scene_count).enumerate() {
                let saved = &mut out.clips[j];
                saved.kind = clip.kind;
                saved.region = clip.region;
                saved.audio_region = clip.audio_region.map(|p|p.region);
                saved.lanes = clip.lanes.clone();
                copy_string(&mut saved.name, &clip.name);
                saved.bars = clip.bars;
                saved.gain = clip.gain;
                saved.properties = clip.properties;
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
                key_shift,
                sync,
                sync_phase,
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
            out.pos = deck.preview_position.unwrap_or(deck.pos);
            copy_string(&mut out.title, &deck.title);
            out.eq = eq(&deck.eq[0]);
            out.audio = deck.audio.as_ref().map(|s| media(&mut self.media, s));
            out.cue_styles = deck.cue_styles;
            out.saved_loops = deck.audio.as_ref().map_or_else(Default::default, |audio| deck.controls.saved_loops(audio.sr));
            out.grid = deck.grid;
            out.source_gain = deck.source_gain.policy();
            out.hotcues =
                std::array::from_fn(|j| deck.hotcues[j].set.then_some(deck.hotcues[j].pos));
        }
        for (out, rack) in target.scene_fx.iter_mut().zip(&rt.scene_fx) {
            effects(out, rack);
        }
        for (i, bank) in target.banks.iter_mut().enumerate() {
            let working = &rt.sampler_banks[i];
            copy_string(&mut bank.name, working.name());
            bank.instance = Some(working.id);
            bank.settings = Some(working.data.settings.clone());
            for (j, sample) in working.data.audio.iter().enumerate() {
                bank.media[j] = sample.as_ref().map(|sample| media(&mut self.media, sample));
            }
        }
        target.builtin =
            std::array::from_fn(|i| rt.builtin[i].as_ref().map(|s| media(&mut self.media, s)));
        rt.capture_held_durations(target);
        self.arrangement = rt.arrangement.plan.clone();
        self.playback_receipts = std::array::from_fn(|i| rt.decks[i].load_receipt.clone());
        self.revision = rt.project.revision();
        self.checkpoint = rt.undo.checkpoint();
        self.complete = true;
    }
    /// Capture live processor state after the native metadata boundary.
    /// Takes worker cancellation; retains missing or failed processor checkpoints with an explicit diagnostic.
    pub(super) fn finish_plugins(&mut self, cancel: &AtomicBool) -> Result<(), Error> {
        if self.plugins.is_empty() { return Ok(()); }
        let Some(model) = &mut self.state.routing else { return Err(Error::Invalid("Plugin state has no saved routing graph".into())); };
        let model = Arc::make_mut(model);
        for (id,control,parameters) in &self.plugins {
            if cancel.load(Ordering::Acquire) { return Err(Error::Cancelled); }
            let saved = model.plugins.iter_mut().find(|p| p.id == *id).ok_or_else(|| Error::Invalid("Captured plugin identity changed".into()))?;
            saved.parameters = parameters.iter().flatten().copied().collect();
            if let Some(control) = control {
                match control.snapshot(cancel) {
                    Ok(state) => { saved.saved = state; saved.latency = control.latency(); saved.unavailable = control.error(); }
                    Err(error) => { if cancel.load(Ordering::Acquire) { return Err(Error::Cancelled); } saved.unavailable = Some(error); }
                }
            }
        }
        Ok(())
    }
    pub(super) fn controls(&self) -> Vec<(u64,crate::plugin_host::realtime::Control)> { self.plugins.iter().filter_map(|(id,control,_)| control.as_ref().map(|control| (*id,control.clone()))).collect() }
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
        offline: poly.offline.clone(),
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
        offline: s.offline.clone(),
    }));
}
