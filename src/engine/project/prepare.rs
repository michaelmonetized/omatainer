use super::{model::*, *};

/// Owns a fully allocated renderer graph. Only its musical fields are swapped;
/// the current stream, command receiver, snapshots and device ownership remain.
pub struct Prepared {
    pub(super) rt: Box<RtEngine>,
}

impl Prepared {
    pub fn from_state(
        state: State,
        media: Vec<Arc<Sample>>,
        output_sr: u32,
    ) -> Result<Self, Error> {
        state.validate(&media).map_err(Error::Invalid)?;
        if !(8000..=384000).contains(&output_sr) {
            return Err(Error::Invalid("unsupported output sample rate".into()));
        }
        let (_, rx) = crossbeam_channel::bounded(1);
        let mut rt = Box::new(RtEngine::new(
            output_sr as f32,
            rx,
            Arc::new(Mutex::new(Snapshot::default())),
        ));
        macro_rules! scalars { ($($field:ident),* $(,)?) => { $(rt.$field = state.$field;)* }; }
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
        rt.playing = false;
        rt.recording = false;
        rt.compose_target = None;
        rt.sampler_poly = synth(state.sampler_synth, output_sr);
        for (i, saved) in state.tracks.into_iter().enumerate() {
            let track = &mut rt.tracks[i];
            track.name = saved.name;
            track.scene_bus = saved.scene_bus;
            track.gain = saved.gain;
            track.pan = saved.pan;
            track.mute = saved.mute;
            track.solo = saved.solo;
            track.armed = saved.armed;
            track.kind = saved.kind;
            track.poly = synth(saved.synth, output_sr);
            track.eq = eq(saved.eq, output_sr);
            track.eq_right = track.eq;
            track.drum_samples = saved.drums.map(|index| media[index].clone());
            track.fx = effects(saved.fx, output_sr);
            track.clips = saved.clips.map(|c| Clip {
                kind: c.kind,
                name: c.name,
                bars: c.bars,
                notes: c.notes,
                gain: c.gain,
                audio: c.audio.map(|index| media[index].clone()),
            });
            // Prepare the note heap off audio, then hold the launch until Play.
            track.project_resume = saved.launch.map(|p| PlayingClip {
                scene: p.scene,
                start_beat: p.start_beat,
                last_beat: -0.0001,
                looping: p.looping,
            });
            track.rebuild_midi_schedule(state.beat);
        }
        for (i, saved) in state.decks.into_iter().enumerate() {
            let deck = &mut rt.decks[i];
            *deck = DeckRt::new(output_sr as f32);
            macro_rules! fields { ($($field:ident),* $(,)?) => { $(deck.$field = saved.$field;)* }; }
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
            deck.title = saved.title;
            deck.audio = saved.audio.map(|index| media[index].clone());
            deck.eq = [eq(saved.eq, output_sr); 2];
            deck.filter_position = saved.filter_amt;
            deck.hotcues = saved.hotcues.map(|pos| HotCue {
                set: pos.is_some(),
                pos: pos.unwrap_or(0.0),
            });
            deck.rate = if deck.sync {
                deck.sync_bpm / deck.bpm.max(1.0)
            } else {
                deck.pitch_rate()
            };
            deck.target_rate = deck.rate;
            deck.transition_to(deck.pos, output_sr as f32, DeckTransition::Jump);
            // No previous output exists in an opened project. Start its canonical
            // render directly; warm DSP tails/physical scratch motion are transient.
            deck.transition_remaining = 0;
            if deck.audio.is_some() {
                let receipt = load_receipt::Receipt::new();
                receipt.claim();
                receipt.finish(load_receipt::State::Current);
                deck.load_receipt = Some(receipt);
                deck.publish_preparation();
            }
        }
        rt.scene_fx = state.scene_fx.map(|rack| effects(rack, output_sr));
        rt.sampler_banks = state.banks.iter().map(|b| b.name.clone()).collect();
        rt.pad_banks = state
            .banks
            .into_iter()
            .map(|b| b.media.map(|index| media[index].clone()))
            .collect();
        rt.builtin = state.builtin.map(|index| index.map(|i| media[i].clone()));
        for track in &mut rt.tracks {
            track.midi_schedule.prepare_history(8192);
            track.recorded_playback.reserve(8192);
        }
        Ok(Self { rt })
    }

    pub fn empty(output_sr: u32) -> Result<Self, Error> {
        if !(8000..=384000).contains(&output_sr) {
            return Err(Error::Invalid("unsupported output sample rate".into()));
        }
        let (_, rx) = crossbeam_channel::bounded(1);
        let mut rt = Box::new(RtEngine::new(
            output_sr as f32,
            rx,
            Arc::new(Mutex::new(Snapshot::default())),
        ));
        for track in &mut rt.tracks {
            track.clips = std::array::from_fn(|_| Clip::empty());
        }
        for deck in &mut rt.decks {
            *deck = DeckRt::new(output_sr as f32);
        }
        for track in &mut rt.tracks {
            track.midi_schedule.prepare_history(8192);
            track.recorded_playback.reserve(8192);
        }
        Ok(Self { rt })
    }

    pub(super) fn swap_into(&mut self, rt: &mut RtEngine) {
        // Supersede old identities, while keeping their receipt/media ownership
        // in the retired graph until this Prepared is dropped on the worker.
        for deck in &rt.decks {
            if let Some(receipt) = &deck.load_receipt {
                receipt.supersede();
            }
        }
        macro_rules! swap { ($($field:ident),* $(,)?) => { $(std::mem::swap(&mut rt.$field, &mut self.rt.$field);)* }; }
        swap!(
            playing,
            recording,
            bpm,
            beat,
            quant,
            view,
            xfader,
            xfader_curve,
            xfader_gain,
            master,
            cue_mix,
            tracks,
            decks,
            master_fx,
            fx_kind,
            fx_wet,
            tap,
            selected_track,
            selected_scene,
            selected_deck,
            note_recording,
            metronome,
            metro,
            quantize,
            sampler_bank,
            sampler_inst,
            sampler_oct,
            sampler_poly,
            sampler_banks,
            pad_banks,
            pad_voices,
            pad_destinations,
            pad_output,
            pad_targets,
            builtin,
            fx_view,
            scene_fx,
            compose_target
        );
    }
}

fn synth(s: Synth, sr: u32) -> Poly {
    let mut value = Poly::new(sr as f32, s.kind, s.voices);
    value.cutoff = s.cutoff;
    value.set_tuning_hz(s.tuning_hz);
    value
}
fn eq(gains: [f32; 3], sr: u32) -> ThreeBand {
    let mut value = ThreeBand::new(sr as f32);
    [value.low_g, value.mid_g, value.high_g] = gains;
    value
}
fn effects(values: Vec<Effect>, sr: u32) -> fx::FxChain {
    fx::FxChain {
        slots: values
            .into_iter()
            .map(|s| {
                let mut slot = fx::FxSlot::new(s.id, sr as f32);
                slot.on = s.on;
                slot.mix = s.mix;
                slot.p = s.p;
                slot
            })
            .collect(),
    }
}
