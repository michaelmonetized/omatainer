use super::*;
use crate::engine::{fx, midi_edit::NoteId, project, Clip, Sample, TrackRt};
use std::sync::Arc;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub tracks: Vec<Id>,
    pub scenes: Vec<Id>,
    pub clips: bool,
    pub devices: bool,
    pub keep_timing: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Import {
    base_tracks: usize,
    base_scenes: usize,
    added_tracks: usize,
    added_scenes: usize,
    nodes: Vec<Box<TrackRt>>,
    racks: Vec<fx::FxChain>,
    cells: Vec<Vec<Clip>>,
    installed: bool,
    heap_bytes: usize,
    fx_bytes: usize,
    fx_ids: Vec<fx::FxId>,
}
impl Import {
    pub(super) fn bytes(&self) -> usize {
        self.heap_bytes
    }
    pub(super) fn processor_delta(&self, _rt: &RtEngine) -> i128 {
        if self.installed {
            -(self.fx_bytes as i128)
        } else {
            self.fx_bytes as i128
        }
    }
    pub(super) fn valid(&self, rt: &RtEngine) -> bool {
        let (tracks, scenes) = if self.installed {
            (
                self.base_tracks + self.added_tracks,
                self.base_scenes + self.added_scenes,
            )
        } else {
            (self.base_tracks, self.base_scenes)
        };
        rt.tracks.len() == tracks
            && rt.scene_fx.len() == scenes
            && rt.tracks.capacity() >= self.base_tracks + self.added_tracks
            && rt.scene_fx.capacity() >= self.base_scenes + self.added_scenes
            && rt
                .tracks
                .iter()
                .take(self.base_tracks)
                .all(|t| t.clips.capacity() >= self.base_scenes + self.added_scenes)
    }
    pub(super) fn swap(&mut self, rt: &mut RtEngine) {
        if self.installed {
            while rt.tracks.len() > self.base_tracks {
                self.nodes.push(rt.tracks.pop().unwrap());
            }
            self.nodes.reverse();
            while rt.scene_fx.len() > self.base_scenes {
                self.racks.push(rt.scene_fx.pop().unwrap());
            }
            self.racks.reverse();
            for (track, cells) in rt.tracks.iter_mut().zip(&mut self.cells) {
                while track.clips.len() > self.base_scenes {
                    cells.push(track.clips.pop().unwrap());
                }
                cells.reverse();
            }
        } else {
            for (track, cells) in rt.tracks.iter_mut().zip(&mut self.cells) {
                track.clips.append(cells);
            }
            rt.tracks.append(&mut self.nodes);
            rt.scene_fx.append(&mut self.racks);
        }
        self.installed = !self.installed;
    }
    pub(super) fn prepare_rate(&mut self, sr: f32) {
        for node in &mut self.nodes {
            node.fx.set_sample_rate(sr);
            node.poly.set_sample_rate(sr);
            node.eq.set_sample_rate(sr);
            node.eq_right.set_sample_rate(sr);
            node.mixer_gain = crate::engine::mixer_gain::GainPair::default();
            node.stop_clip();
            node.drum_pos.fill(None);
        }
        for rack in &mut self.racks {
            rack.set_sample_rate(sr);
        }
        self.fx_bytes = self
            .fx_ids
            .iter()
            .map(|id| fx::FxSlot::required_storage(*id, sr))
            .sum();
    }
}

fn selected(layout: &Layout, axis: Axis, ids: &[Id]) -> Result<Vec<usize>, String> {
    let mut slots = Vec::new();
    for &id in ids {
        let slot = layout
            .resolve(axis, id)
            .ok_or("A selected source identity is unavailable")?;
        if slots.contains(&slot) {
            return Err("Select each source identity once".into());
        }
        slots.push(slot);
    }
    Ok(slots)
}
fn append(layout: &mut Layout, axis: Axis, mut item: super::super::Item) -> Result<usize, String> {
    item.id = Id(layout.next_id);
    layout.next_id = layout
        .next_id
        .checked_add(1)
        .ok_or("Session identities exhausted")?;
    match axis {
        Axis::Track => {
            let slot = layout.tracks.len();
            layout.tracks.push(item);
            layout.track_order.push(slot as u8);
            Ok(slot)
        }
        Axis::Scene => {
            let slot = layout.scenes.len();
            layout.scenes.push(item);
            layout.scene_order.push(slot as u16);
            Ok(slot)
        }
    }
}

impl Request {
    /// Prepare selected source tracks and clips as one reversible session edit.
    /// Takes a coherent destination, validated source, selection and output rate.
    /// Returns an atomic edit with fresh musical IDs; neither project is changed.
    pub(crate) fn import(
        captured: project::Captured,
        source: &project::State,
        source_media: &[Arc<Sample>],
        selection: &Selection,
        rate: u32,
    ) -> Result<(Self, Ack), String> {
        Self::import_with_preflight(
            captured,
            source,
            source_media,
            selection,
            rate,
            |state, media| {
                crate::project_file::validate_metadata(
                    &crate::project_file::Bundle {
                        state,
                        media: media.to_vec(),
                    },
                    &state.import_metadata_limits(),
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .map_err(|e| e.to_string())
            },
        )
    }
    /// Prepare an import only after the complete destination passes its document preflight.
    /// Takes the same musical inputs plus a worker-side saveability check; returns one atomic edit.
    pub(crate) fn import_with_preflight(
        captured: project::Captured,
        source: &project::State,
        source_media: &[Arc<Sample>],
        selection: &Selection,
        rate: u32,
        preflight: impl FnOnce(&project::State, &[Arc<Sample>]) -> Result<(), String>,
    ) -> Result<(Self, Ack), String> {
        source.validate(source_media)?;
        let project::Captured {
            mut state,
            mut media,
            checkpoint,
            revision,
            ..
        } = captured;
        state.validate(&media)?;
        let before = state
            .session
            .clone()
            .ok_or("Destination session identities are unavailable")?;
        let source_layout = source.session.clone().unwrap_or_else(|| {
            Layout::legacy(
                source.tracks.iter().map(|t| t.name.clone()),
                source.scene_fx.len(),
            )
        });
        let tracks = selected(&source_layout, Axis::Track, &selection.tracks)?;
        let scenes = selected(&source_layout, Axis::Scene, &selection.scenes)?;
        if tracks.is_empty() || selection.clips && scenes.is_empty() {
            return Err("Select source tracks and at least one scene for clip import".into());
        }
        if !selection.keep_timing {
            return Err("Review and accept destination tempo and meter before importing".into());
        }
        let base_tracks = state.tracks.len();
        let base_scenes = state.scene_fx.len();
        if base_tracks + tracks.len() > super::super::MAX_TRACKS
            || base_scenes + scenes.len() > super::super::MAX_SCENES
        {
            return Err(
                "Import exceeds 128 stored tracks or 512 stored scenes; use fewer source objects"
                    .into(),
            );
        }
        let mut next = before.clone();
        let mut mapped_scenes = std::collections::HashMap::new();
        for &scene in &scenes {
            let slot = append(&mut next, Axis::Scene, source_layout.scenes[scene].clone())?;
            mapped_scenes.insert(scene, slot);
            state.scene_fx.push(if selection.devices {
                source.scene_fx[scene].clone()
            } else {
                Vec::new()
            });
        }
        for track in &mut state.tracks {
            track
                .clips
                .extend((0..scenes.len()).map(|_| super::structural::empty_cell()));
        }
        let mut remap = std::collections::HashMap::new();
        let mut pins = Vec::new();
        let mut index = |original: usize| -> usize {
            *remap.entry(original).or_insert_with(|| {
                let result = media.len();
                media.push(source_media[original].clone());
                pins.push(source_media[original].clone());
                result
            })
        };
        for &source_slot in &tracks {
            let mut track = source.tracks[source_slot].clone();
            track.launch = None;
            track.armed = false;
            track.solo = false;
            track.scene_bus = mapped_scenes
                .get(&track.scene_bus)
                .copied()
                .unwrap_or(state.selected_scene);
            track.drums = if selection.devices {
                track.drums.map(&mut index)
            } else {
                state.tracks[0].drums
            };
            if !selection.devices {
                track.fx.clear();
                track.kind = if track.kind == 4 { 4 } else { 2 };
                track.eq = [1.0; 3];
                track.synth = project::Synth {
                    kind: crate::engine::SynthInstrument::Keys,
                    voices: 8,
                    cutoff: crate::engine::SynthInstrument::Keys.cutoff(),
                    tuning_hz: 440.0,
                    offline: None,
                };
            }
            let source_clips = std::mem::take(&mut track.clips);
            track.clips = (0..base_scenes + scenes.len())
                .map(|_| super::structural::empty_cell())
                .collect();
            if selection.clips {
                for &scene in &scenes {
                    let mut cell = source_clips[scene].clone();
                    for note in &mut cell.notes {
                        note.id = NoteId::new();
                    }
                    cell.audio = cell.audio.map(&mut index);
                    track.clips[mapped_scenes[&scene]] = cell;
                }
            }
            append(
                &mut next,
                Axis::Track,
                source_layout.tracks[source_slot].clone(),
            )?;
            state.tracks.push(track);
        }
        state.session = Some(next.clone());
        state.capture_media_order(&mut media)?;
        state.validate_processor_storage(rate)?;
        let bytes: u64 = media.iter().map(|s| s.data.len() as u64 * 4).sum();
        if media.len() > crate::project_file::DEFAULT_MEDIA_LIMIT
            || bytes > crate::project_file::Limits::default().max_pcm_bytes
        {
            return Err("Imported dependencies exceed native project limits".into());
        }
        preflight(&state, &media)?;
        let nodes = state
            .tracks
            .into_iter()
            .skip(base_tracks)
            .map(|track| project::prepare::prepare_track(track, &media, rate))
            .collect::<Result<Vec<_>, _>>()?;
        let racks: Vec<_> = state
            .scene_fx
            .into_iter()
            .skip(base_scenes)
            .map(|rack| project::prepare::effects(rack, rate))
            .collect();
        let cells: Vec<Vec<_>> = (0..base_tracks)
            .map(|_| (0..scenes.len()).map(|_| Clip::empty()).collect())
            .collect();
        let fx_storage: Vec<_> = nodes
            .iter()
            .flat_map(|t| &t.fx.slots)
            .chain(racks.iter().flat_map(|r| &r.slots))
            .map(|slot| slot.id())
            .collect();
        let reserved_fx_bytes = fx_storage
            .iter()
            .map(|id| fx::FxSlot::required_storage(*id, rate as f32))
            .sum();
        let heap_bytes = fx_storage.len() * std::mem::size_of::<fx::FxId>()
            + nodes.capacity() * std::mem::size_of::<Box<TrackRt>>()
            + nodes.iter().map(|t| t.retained_bytes()).sum::<usize>()
            + racks.capacity() * std::mem::size_of::<fx::FxChain>()
            + racks.iter().map(fx::FxChain::retained_bytes).sum::<usize>()
            + cells.capacity() * std::mem::size_of::<Vec<Clip>>()
            + cells
                .iter()
                .map(|cells| cells.capacity() * std::mem::size_of::<Clip>())
                .sum::<usize>();
        let import = Import {
            base_tracks,
            base_scenes,
            added_tracks: tracks.len(),
            added_scenes: scenes.len(),
            nodes,
            racks,
            cells,
            installed: false,
            heap_bytes,
            fx_bytes: reserved_fx_bytes,
            fx_ids: fx_storage.clone(),
        };
        let ack = Ack::new();
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
                    content: Some(Content::Import(import)),
                    reserved_heap: heap_bytes,
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
}
