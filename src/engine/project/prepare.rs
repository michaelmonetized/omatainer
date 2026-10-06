use super::{model::*, *};

/// Owns a fully allocated renderer graph. Only its musical fields are swapped;
/// the current stream, command receiver, snapshots and device ownership remain.
pub struct Prepared {
    pub(in crate::engine) rt: Box<RtEngine>,
}

impl Prepared {
    pub fn from_state(
        mut state: State,
        media: Vec<Arc<Sample>>,
        output_sr: u32,
    ) -> Result<Self, Error> {
        state.validate(&media).map_err(Error::Invalid)?;
        state.validate_processor_storage(output_sr).map_err(Error::Invalid)?;
        state.migrate_notes();
        if !(8000..=384000).contains(&output_sr) {
            return Err(Error::Invalid("unsupported output sample rate".into()));
        }
        let rx = crate::engine::control::CommandReceiver::disconnected();
        let mut rt = Box::new(RtEngine::try_new(
            output_sr as f32,
            rx,
            Arc::new(Mutex::new(Snapshot::default())),
        ).map_err(Error::Invalid)?);
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
        rt.timeline_anchor = state.timeline_seconds;
        rt.timeline_frames = 0;
        rt.midi_beat = rt.beat;
        rt.midi_beat_reference = rt.beat;
        rt.session = if let Some(layout) = state.session.take() {layout} else {
            use sha2::{Digest, Sha256};
            let mut layout = session::Layout::legacy(state.tracks.iter().map(|t| t.name.clone()), state.scene_fx.len());
            let bytes = serde_json::to_vec(&state).map_err(|e| Error::Invalid(e.to_string()))?;
            let mut digest = Sha256::new();
            digest.update(b"omatainer-legacy-session-v7\0");
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(&bytes);
            digest.update((media.len() as u64).to_le_bytes());
            // Legacy files have no project ID. Include their bounded immutable
            // PCM so equal metadata with different embedded audio cannot alias.
            // This runs only on the preparing worker, never on the renderer.
            for sample in &media {
                digest.update(sample.sr.to_le_bytes());
                digest.update(sample.ch.to_le_bytes());
                digest.update((sample.data.len() as u64).to_le_bytes());
                for chunk in sample.data.chunks(1024) {
                    let mut pcm = [0u8; 4096];
                    for (frame, bytes) in chunk.iter().zip(pcm.chunks_exact_mut(4)) {
                        bytes.copy_from_slice(&frame.to_bits().to_le_bytes());
                    }
                    digest.update(&pcm[..chunk.len() * 4]);
                }
            }
            let hash = digest.finalize();
            layout.namespace = [u64::from_le_bytes(hash[0..8].try_into().unwrap()) | (1 << 63), u64::from_le_bytes(hash[8..16].try_into().unwrap())];
            layout
        };
        if state.version < 7 && rt.fx_view >= 100 { rt.fx_view += session::SCENE_FX_BASE - 100; }
        rt.routing = state.routing.take().map(|model| audio::routing::prepared::Prepared::new(model, &rt.session).map(Box::new)).transpose().map_err(Error::Invalid)?;
        rt.tracks.clear();
        rt.tracks.reserve(session::MAX_TRACKS);
        rt.conductor = state.conductor.as_ref().map(|c| c.prepare()).transpose().map_err(Error::Invalid)?;
        rt.sync_midi_clock();
        rt.playing = false;
        rt.recording = false;
        rt.compose_target = None;
        rt.sampler_poly = synth(state.sampler_synth, output_sr);
        for (i, saved) in state.tracks.into_iter().enumerate() {
            rt.session.tracks[i].name = saved.name.clone();
            let mut track = prepare_track(saved, &media, output_sr).map_err(Error::Invalid)?;
            track.rebuild_midi_schedule(state.beat, state.beat);
            rt.tracks.push(track);
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
            let level = deck.audio.as_ref().map(|sample| crate::track_gain::measure_channels(&sample.data, sample.ch, || false)).transpose().map_err(|error| Error::Invalid(error.into()))?;
            deck.source_gain = crate::track_gain::Resolved::prepare(saved.source_gain, level).map_err(|error| Error::Invalid(error.into()))?;
            deck.history_key = 0;
            deck.eq = [eq(saved.eq, output_sr); 2];
            deck.filter_position = saved.filter_amt;
            deck.cue_styles = saved.cue_styles;
            deck.grid = saved.grid;
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
                let receipt = load_receipt::Receipt::with_preparation(deck.preparation()).with_source_level(level).map_err(|error| Error::Invalid(error.into()))?;
                receipt.claim();
                receipt.finish(load_receipt::State::Current);
                deck.history_key = receipt.history_key();
                deck.load_receipt = Some(receipt);
                deck.publish_preparation();
            }
        }
        rt.scene_fx = state.scene_fx.into_iter().map(|rack| effects(rack, output_sr)).collect();
        rt.scene_fx.reserve(session::MAX_SCENES - rt.scene_fx.len());
        rt.sampler_banks.clear();
        for saved in state.banks {
            let settings = match saved.settings {
                Some(settings) => settings,
                None => Arc::new(crate::sampler_bank::resident::Settings::empty(saved.name).map_err(Error::Invalid)?),
            };
            let issues = std::array::from_fn(|slot| (settings.slots[slot].source.is_some() && saved.media[slot].is_none()).then(|| "Source unavailable in this embedded project; assign or retry the original source".into()));
            let audio = saved.media.map(|index| index.map(|i| media[i].clone()));
            let data = crate::sampler_bank::resident::Data::prepare(settings, audio, issues).map_err(Error::Invalid)?;
            let data = rt.sampler_assets.pin(data).map_err(|e| Error::Invalid(e.to_string()))?;
            rt.sampler_banks.push(sampler::Bank { id: match saved.instance { Some(id) => id, None => crate::sampler_bank::BankId::new().map_err(Error::Invalid)? }, revision: 1, factory: None, data });
        }
        rt.builtin = state.builtin.map(|index| index.map(|i| media[i].clone()));
        rt.builtin_levels = std::array::from_fn(|stem| rt.builtin[stem].as_ref().and_then(|sample| crate::track_gain::measure_channels(&sample.data, sample.ch, || false).ok()));
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
        let rx = crate::engine::control::CommandReceiver::disconnected();
        let mut rt = Box::new(RtEngine::try_new(
            output_sr as f32,
            rx,
            Arc::new(Mutex::new(Snapshot::default())),
        ).map_err(Error::Invalid)?);
        for track in &mut rt.tracks {
            track.clips = (0..SCENES).map(|_| Clip::empty()).collect();
            track.clips.reserve(session::MAX_SCENES - track.clips.len());
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

    /// Give a preparing worker sole ownership of an offline graph.
    /// Takes this prepared graph; returns its renderer without device or GUI ownership.
    pub(crate) fn into_offline(self) -> Box<RtEngine> { self.rt }

    pub(in crate::engine) fn swap_into(&mut self, rt: &mut RtEngine) {
        for deck in 0..DECKS { rt.performance.deck_media_changed(deck); }
        rt.routing_pipe.recorder.invalidate();
        rt.transport_epoch = rt.transport_epoch.wrapping_add(1);
        rt.midi_routing.reset_outputs();
        if let Some(active) = &rt.sampler_audition { active.ended(); }
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
            beat_roundoff,
            timeline_anchor,
            timeline_frames,
            midi_beat,
            midi_beat_reference,
            conductor,
            routing,
            last_midi_step,
            quant,
            view,
            xfader,
            xfader_curve,
            xfader_gain,
            master,
            cue_mix,
            tracks,
            session,
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
            count_in,
            quantize,
            sampler_bank,
            sampler_inst,
            sampler_oct,
            sampler_poly,
            sampler_banks,
            sampler_revision,
            pad_voices,
            sampler_audition,
            pad_destinations,
            pad_output,
            pad_targets,
            builtin,
            builtin_levels,
            fx_view,
            scene_fx,
            compose_target
        );
        rt.midi_routing.identity.publish(&rt.session);
        if let Some(history) = &mut rt.history_measurement { history.reset_dsp(); }
    }
}

fn synth(s: Synth, sr: u32) -> Poly {
    let mut value = Poly::new(sr as f32, s.kind, s.voices);
    value.offline = s.offline;
    value.cutoff = s.cutoff;
    value.set_tuning_hz(s.tuning_hz);
    value
}
fn eq(gains: [f32; 3], sr: u32) -> ThreeBand {
    let mut value = ThreeBand::new(sr as f32);
    [value.low_g, value.mid_g, value.high_g] = gains;
    value
}
pub(in crate::engine) fn effects(values: Vec<Effect>, sr: u32) -> fx::FxChain {
    fx::FxChain {
        slots: values
            .into_iter()
            .map(|s| {
                let mut slot = fx::FxSlot::new(s.id, sr as f32);
                slot.on = s.on;
                slot.mix = s.mix;
                slot.p = s.p;
                slot.offline = s.offline;
                slot
            })
            .collect(),
    }
}

/// Builds only the edited node; existing playing nodes and DSP histories stay live.
pub(in crate::engine) fn prepare_track(mut saved: Track, media: &[Arc<Sample>], sr: u32) -> Result<Box<TrackRt>, String> {
    let drums = saved.drums.map(|index| media[index].clone());
    let mut track = Box::new(TrackRt::empty(sr as f32, saved.name, saved.kind, drums, 0));
    for c in &mut saved.clips { c.lanes = c.lanes.as_ref().map(|l| l.prepare()).transpose()?; }
    track.scene_bus = saved.scene_bus;
    track.gain = saved.gain; track.pan = saved.pan;
    track.mute = saved.mute; track.solo = saved.solo; track.armed = saved.armed;
    track.poly = synth(saved.synth, sr); track.eq = eq(saved.eq, sr); track.eq_right = track.eq;
    track.fx = effects(saved.fx, sr);
    track.clips = saved.clips.into_iter().map(|c| Clip {
        region: c.region, lanes: c.lanes, kind: c.kind, name: c.name,
        bars: c.bars, notes: c.notes, gain: c.gain, audio: c.audio.map(|i| media[i].clone()),
    }).collect();
    track.clips.reserve(session::MAX_SCENES - track.clips.len());
    track.project_resume = saved.launch.map(|p| PlayingClip { scene: p.scene,
        start_beat: p.start_beat, midi_start_beat: p.start_beat, last_beat: -0.0001, looping: p.looping });
    track.midi_schedule.prepare_history(8192); track.recorded_playback.reserve(8192);
    Ok(track)
}
