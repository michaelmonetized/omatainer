//! Worker-only graph preparation. Only the affected track or scene is built.
use super::*;
use crate::engine::{fx, midi_edit::NoteId, project, Clip, Sample};
use std::sync::Arc;

fn empty_cell() -> project::SavedClip {
    project::SavedClip {
        lanes: None,
        region: None,
        kind: crate::engine::ClipKind::Empty,
        name: String::new(),
        bars: 1.0,
        notes: Vec::new(),
        gain: 1.0,
        audio: None,
    }
}
fn duplicate_cell(mut cell: project::SavedClip) -> project::SavedClip {
    for note in &mut cell.notes {
        note.id = NoteId::new();
    }
    cell
}
fn prepare_cell(cell: project::SavedClip, media: &[Arc<Sample>]) -> Result<Clip, String> {
    Ok(Clip {
        region: cell.region,
        lanes: cell.lanes.map(|l| l.prepare()).transpose()?,
        kind: cell.kind,
        name: cell.name,
        bars: cell.bars,
        notes: cell.notes,
        gain: cell.gain,
        audio: cell.audio.map(|i| media[i].clone()),
    })
}
fn pin_track(track: &project::Track, media: &[Arc<Sample>], pins: &mut Vec<Arc<Sample>>) {
    pins.extend(track.drums.iter().map(|i| media[*i].clone()));
    pins.extend(
        track
            .clips
            .iter()
            .filter_map(|c| c.audio.map(|i| media[i].clone())),
    );
}

impl Request {
    pub(crate) fn structural(
        captured: project::Captured,
        rate: u32,
        operation: Structure,
    ) -> Result<(Self, Ack), String> {
        if !(8000..=384000).contains(&rate) {
            return Err("Unsupported output rate".into());
        }
        let project::Captured {
            mut state,
            media,
            checkpoint,
            revision,
            ..
        } = captured;
        state.validate(&media)?;
        let before = state
            .session
            .clone()
            .ok_or("Session identity is unavailable")?;
        let mut next = before.clone();
        let mut pins = Vec::new();
        let mut fx_storage = Vec::new();
        let content = match operation {
            Structure::Track {
                name,
                audio,
                position,
            } => {
                let mut saved = state.tracks[usize::from(before.track_order[0])].clone();
                saved.name = name;
                saved.clips = (0..before.scenes.len()).map(|_| empty_cell()).collect();
                saved.launch = None;
                saved.scene_bus = usize::from(before.scene_order[0]);
                saved.kind = if audio { 4 } else { 2 };
                saved.gain = 0.8;
                saved.pan = 0.0;
                saved.mute = false;
                saved.solo = false;
                saved.armed = false;
                saved.fx.clear();
                saved.eq = [1.0; 3];
                saved.synth = project::Synth {
                    kind: crate::engine::SynthInstrument::Keys,
                    voices: 8,
                    cutoff: crate::engine::SynthInstrument::Keys.cutoff(),
                    tuning_hz: 440.0,
                };
                Self::prepare_track_change(
                    &mut state,
                    &media,
                    &mut next,
                    saved,
                    None,
                    position,
                    rate,
                    &mut pins,
                    &mut fx_storage,
                )?
            }
            Structure::Scene { name, position } => {
                let cells = (0..state.tracks.len()).map(|_| empty_cell()).collect();
                Self::prepare_scene_change(
                    &mut state,
                    &media,
                    &mut next,
                    name,
                    None,
                    position,
                    cells,
                    Vec::new(),
                    rate,
                    &mut pins,
                    &mut fx_storage,
                )?
            }
            Structure::Duplicate {
                axis: Axis::Track,
                id,
                name,
                position,
            } => {
                let source = before
                    .resolve(Axis::Track, id)
                    .ok_or("Track no longer exists")?;
                let mut saved = state.tracks[source].clone();
                saved.name = name;
                saved.launch = None;
                saved.armed = false;
                saved.clips = saved.clips.into_iter().map(duplicate_cell).collect();
                let color = before.tracks[source].color;
                Self::prepare_track_change(
                    &mut state,
                    &media,
                    &mut next,
                    saved,
                    color,
                    position,
                    rate,
                    &mut pins,
                    &mut fx_storage,
                )?
            }
            Structure::Duplicate {
                axis: Axis::Scene,
                id,
                name,
                position,
            } => {
                let source = before
                    .resolve(Axis::Scene, id)
                    .ok_or("Scene no longer exists")?;
                let cells = state
                    .tracks
                    .iter()
                    .map(|t| duplicate_cell(t.clips[source].clone()))
                    .collect();
                let rack = state.scene_fx[source].clone();
                let color = before.scenes[source].color;
                Self::prepare_scene_change(
                    &mut state,
                    &media,
                    &mut next,
                    name,
                    color,
                    position,
                    cells,
                    rack,
                    rate,
                    &mut pins,
                    &mut fx_storage,
                )?
            }
        };
        pins.sort_by_key(|sample| Arc::as_ptr(sample) as usize);
        pins.dedup_by_key(|sample| Arc::as_ptr(sample) as usize);
        let ack = Ack::new();
        let reserved_fx_bytes = fx_storage
            .iter()
            .map(|id| fx::FxSlot::required_storage(*id, rate as f32))
            .sum();
        let reserved_heap = match &content {
            Content::Track { node, .. } => node.as_ref().unwrap().retained_bytes(),
            Content::Scene { cells, rack, .. } => {
                cells.capacity() * std::mem::size_of::<Option<Clip>>()
                    + cells
                        .iter()
                        .flatten()
                        .map(Clip::retained_bytes)
                        .sum::<usize>()
                    + rack.as_ref().unwrap().retained_bytes()
            }
        };
        Ok((
            Self {
                namespace: before.namespace,
                generation: before.generation,
                epoch: checkpoint.epoch,
                inverse: Some(Box::new(Inverse {
                    layout: next,
                    track_name: None,
                    focus: None,
                    bus_mask: 0,
                    scene_buses: [0; super::super::MAX_TRACKS],
                    content: Some(content),
                    reserved_heap,
                    media: pins,
                    fx_storage,
                    reserved_fx_bytes,
                })),
                ack: ack.clone(),
                disruptive: true,
                receipt: Some((revision, rate)),
            },
            ack,
        ))
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_track_change(
        state: &mut project::State,
        media: &[Arc<Sample>],
        next: &mut Layout,
        saved: project::Track,
        color: Option<[u8; 3]>,
        position: usize,
        rate: u32,
        pins: &mut Vec<Arc<Sample>>,
        storage: &mut Vec<fx::FxId>,
    ) -> Result<Content, String> {
        let (_, slot) = next.create(Axis::Track, saved.name.clone(), color, position)?;
        pin_track(&saved, media, pins);
        storage.extend(saved.fx.iter().map(|e| e.id));
        if let Some(old) = state.tracks.get(slot) {
            pin_track(old, media, pins);
            storage.extend(old.fx.iter().map(|e| e.id));
        }
        if slot == state.tracks.len() {
            state.tracks.push(saved);
        } else {
            state.tracks[slot] = saved;
        }
        state.session = Some(next.clone());
        state.validate(media)?;
        state.validate_processor_storage(rate)?;
        let node = project::prepare::prepare_track(state.tracks[slot].clone(), media, rate)?;
        Ok(Content::Track {
            slot,
            node: Some(node),
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_scene_change(
        state: &mut project::State,
        media: &[Arc<Sample>],
        next: &mut Layout,
        name: String,
        color: Option<[u8; 3]>,
        position: usize,
        cells: Vec<project::SavedClip>,
        effects: Vec<project::Effect>,
        rate: u32,
        pins: &mut Vec<Arc<Sample>>,
        storage: &mut Vec<fx::FxId>,
    ) -> Result<Content, String> {
        let (_, slot) = next.create(Axis::Scene, name, color, position)?;
        storage.extend(effects.iter().map(|e| e.id));
        if let Some(old) = state.scene_fx.get(slot) {
            storage.extend(old.iter().map(|e| e.id));
        }
        for (track, cell) in state.tracks.iter_mut().zip(cells) {
            if let Some(old) = track.clips.get(slot) {
                if let Some(i) = old.audio {
                    pins.push(media[i].clone());
                }
            }
            if let Some(i) = cell.audio {
                pins.push(media[i].clone());
            }
            if slot == track.clips.len() {
                track.clips.push(cell);
            } else {
                track.clips[slot] = cell;
            }
        }
        if slot == state.scene_fx.len() {
            state.scene_fx.push(effects);
        } else {
            state.scene_fx[slot] = effects;
        }
        state.session = Some(next.clone());
        state.validate(media)?;
        state.validate_processor_storage(rate)?;
        let cells = state
            .tracks
            .iter()
            .map(|t| prepare_cell(t.clips[slot].clone(), media).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let rack = project::prepare::effects(state.scene_fx[slot].clone(), rate);
        Ok(Content::Scene {
            slot,
            cells,
            rack: Some(rack),
        })
    }
}
