use super::{preset, Properties};
use crate::engine::{
    midi_edit::{Ack, NoteId, Outcome},
    project,
    session::{Axis, Reference},
    undo::Checkpoint,
    Clip, ClipKind, RtEngine, Sample,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Address a clip through its retained track and scene identities.
/// Slot positions are resolved during preparation and checked again before admission and every Undo direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub track: Reference,
    pub scene: Reference,
}
impl Slot {
    /// Capture an active Session slot.
    /// Takes retained layout and storage indices; returns stable identities or refuses unavailable targets.
    pub(crate) fn at(
        layout: &crate::engine::session::Layout,
        track: usize,
        scene: usize,
    ) -> Result<Self, String> {
        Ok(Self {
            track: layout
                .reference(Axis::Track, track)
                .ok_or("Clip track was removed")?,
            scene: layout
                .reference(Axis::Scene, scene)
                .ok_or("Clip scene was removed")?,
        })
    }
    /// Resolve a retained slot.
    /// Takes the current layout; returns storage indices only while both namespace and identities still agree.
    pub(crate) fn resolve(
        self,
        layout: &crate::engine::session::Layout,
    ) -> Result<(usize, usize), String> {
        let track = layout
            .resolve(Axis::Track, self.track.id)
            .filter(|slot| layout.resolves(Axis::Track, *slot, self.track))
            .ok_or("Clip track identity changed")?;
        let scene = layout
            .resolve(Axis::Scene, self.scene.id)
            .filter(|slot| layout.resolves(Axis::Scene, *slot, self.scene))
            .ok_or("Clip scene identity changed")?;
        Ok((track, scene))
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Metadata {
        target: Slot,
        name: String,
        properties: Properties,
    },
    CreateMidi {
        target: Slot,
        name: String,
        bars: f32,
    },
    Copy {
        source: Slot,
        destination: Slot,
    },
    Move {
        source: Slot,
        destination: Slot,
    },
    Delete {
        target: Slot,
    },
    Insert {
        target: Slot,
        preset: preset::Bundle,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(in crate::engine) inverse: Option<Box<Inverse>>,
    pub(in crate::engine) ack: Ack,
    checkpoint: Checkpoint,
}
#[derive(Clone, Debug)]
struct Cell {
    slot: Slot,
    track: usize,
    scene: usize,
    value: Clip,
}
#[derive(Clone, Debug)]
pub(crate) struct Inverse {
    cells: Vec<Cell>,
    media: Vec<Arc<Sample>>,
    reserved: usize,
}
impl Request {
    /// Prepare one complete clip operation on its worker.
    /// Takes coherent native state, stable targets and cancellation; returns saveable renderer-ready replacements and an acknowledged transaction without copying source PCM.
    pub(crate) fn prepare(
        mut captured: project::Captured,
        action: Action,
        cancel: &AtomicBool,
    ) -> Result<(Box<Self>, Ack), String> {
        active(cancel)?;
        captured.state.validate(&captured.media)?;
        let layout = captured
            .state
            .session
            .as_ref()
            .ok_or("Clip identities are unavailable")?
            .clone();
        let mut affected = Vec::with_capacity(2);
        let mut media = Vec::with_capacity(4);
        let mut replacements = match action {
            Action::Metadata {
                target,
                name,
                properties,
            } => {
                let mut clip = original(&captured.state, target, &layout)?;
                if clip.kind == ClipKind::Empty {
                    return Err("Create a clip before editing its properties".into());
                }
                clip.name = name;
                clip.properties = properties;
                vec![(target, clip)]
            }
            Action::CreateMidi { target, name, bars } => {
                let mut clip = original(&captured.state, target, &layout)?;
                empty(&clip)?;
                clip.kind = ClipKind::Midi;
                clip.name = name;
                clip.bars = bars;
                clip.properties.disabled = false;
                vec![(target, clip)]
            }
            Action::Copy {
                source,
                destination,
            } => {
                if source == destination {
                    return Err("Choose another destination slot".into());
                }
                let mut clip = original(&captured.state, source, &layout)?;
                if clip.kind == ClipKind::Empty {
                    return Err("Choose a nonempty source clip".into());
                }
                empty(&original(&captured.state, destination, &layout)?)?;
                for note in &mut clip.notes {
                    note.id = NoteId::new();
                }
                vec![(destination, clip)]
            }
            Action::Move {
                source,
                destination,
            } => {
                if source == destination {
                    return Err("Choose another destination slot".into());
                }
                let clip = original(&captured.state, source, &layout)?;
                if clip.kind == ClipKind::Empty {
                    return Err("Choose a nonempty source clip".into());
                }
                empty(&original(&captured.state, destination, &layout)?)?;
                vec![(source, blank()), (destination, clip)]
            }
            Action::Delete { target } => {
                let clip = original(&captured.state, target, &layout)?;
                if clip.kind == ClipKind::Empty {
                    return Err("The selected clip is empty".into());
                }
                vec![(target, blank())]
            }
            Action::Insert { target, preset } => {
                empty(&original(&captured.state, target, &layout)?)?;
                let clip = preset
                    .state
                    .insert(&preset.media, &mut captured.media, cancel)?;
                vec![(target, clip)]
            }
        };
        for (slot, _) in &replacements {
            if !affected.contains(slot) {
                affected.push(*slot);
            }
        }
        for slot in &affected {
            let (t, s) = slot.resolve(&layout)?;
            if let Some(index) = captured.state.tracks[t].clips[s].audio {
                pin(&mut media, captured.media[index].clone());
            }
        }
        for (slot, clip) in replacements.drain(..) {
            let (t, s) = slot.resolve(&layout)?;
            captured.state.tracks[t].clips[s] = clip;
        }
        captured.state.version = project::STATE_VERSION;
        captured.state.compact_media(&mut captured.media)?;
        captured.state.validate(&captured.media)?;
        let limits = crate::project_file::Limits::default();
        if captured.media.len() > limits.max_media
            || captured
                .media
                .iter()
                .map(|s| s.data.len() as u64 * 4)
                .sum::<u64>()
                > limits.max_pcm_bytes
        {
            return Err("Clip operation exceeds native media limits".into());
        }
        crate::project_file::validate_metadata(
            &crate::project_file::Bundle {
                state: captured.state.clone(),
                media: captured.media.clone(),
            },
            &limits,
            cancel,
        )
        .map_err(|e| e.to_string())?;
        let mut cells = Vec::with_capacity(affected.len());
        for slot in affected {
            active(cancel)?;
            let (track, scene) = slot.resolve(&layout)?;
            let value = project::prepare::prepare_clip(
                captured.state.tracks[track].clips[scene].clone(),
                &captured.media,
            )?;
            if let Some(audio) = &value.audio {
                pin(&mut media, audio.clone());
            }
            cells.push(Cell {
                slot,
                track,
                scene,
                value,
            });
        }
        let ack = Ack::new();
        Ok((
            Box::new(Self {
                inverse: Some(Box::new(Inverse {
                    cells,
                    media,
                    reserved: 0,
                })),
                ack: ack.clone(),
                checkpoint: captured.checkpoint,
            }),
            ack,
        ))
    }
    pub(in crate::engine) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.checkpoint == rt.undo.checkpoint()
            && self.inverse.as_ref().is_some_and(|p| p.valid(rt))
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.inverse.as_ref().map_or(0, |p| {
                p.bytes().saturating_add(
                    p.media()
                        .map(|sample|crate::engine::undo::sample_bytes(sample))
                        .sum::<usize>(),
                )
            })
    }
}
impl Inverse {
    pub(in crate::engine) fn valid(&self, rt: &RtEngine) -> bool {
        !rt.recording
            && self.cells.iter().all(|c| {
                rt.session.resolves(Axis::Track, c.track, c.slot.track)
                    && rt.session.resolves(Axis::Scene, c.scene, c.slot.scene)
                    && rt.tracks.get(c.track).is_some_and(|t| {
                        t.playing
                            .or(t.project_resume)
                            .is_none_or(|p| usize::from(p.scene) != c.scene)
                    })
                    && !rt.recording_clip_held(c.track, c.scene)
            })
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        if self.reserved > 0 {
            self.reserved
        } else {
            std::mem::size_of::<Self>()
                + self.cells.capacity() * std::mem::size_of::<Cell>()
                + self
                    .cells
                    .iter()
                    .map(|c| c.value.retained_bytes())
                    .sum::<usize>()
                + self.media.capacity() * std::mem::size_of::<Arc<Sample>>()
        }
    }
    pub(in crate::engine) fn reserve(&mut self, rt: &RtEngine) {
        self.reserved = self.bytes()
            + self
                .cells
                .iter()
                .map(|c| rt.tracks[c.track].clips[c.scene].retained_bytes())
                .sum::<usize>();
    }
    pub(in crate::engine) fn swap(&mut self, rt: &mut RtEngine) {
        let beat = rt.beat;
        let midi = rt.precise_midi_beat();
        for cell in &mut self.cells {
            std::mem::swap(
                &mut cell.value,
                &mut rt.tracks[cell.track].clips[cell.scene],
            );
            rt.tracks[cell.track].clip_notes_changed(cell.scene, beat, midi);
        }
    }
    pub(in crate::engine) fn media(&self) -> impl Iterator<Item = &Arc<Sample>> {
        self.media.iter()
    }
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Clip operation cancelled".into())
    } else {
        Ok(())
    }
}
fn original(
    state: &project::State,
    slot: Slot,
    layout: &crate::engine::session::Layout,
) -> Result<project::SavedClip, String> {
    let (t, s) = slot.resolve(layout)?;
    Ok(state.tracks[t].clips[s].clone())
}
fn empty(clip: &project::SavedClip) -> Result<(), String> {
    if clip.kind == ClipKind::Empty {
        Ok(())
    } else {
        Err("Destination contains a clip; choose an empty slot".into())
    }
}
fn blank() -> project::SavedClip {
    project::SavedClip {
        properties: Default::default(),
        audio_region: None,
        lanes: None,
        region: None,
        kind: ClipKind::Empty,
        name: String::new(),
        bars: 1.0,
        notes: Vec::new(),
        gain: 1.0,
        audio: None,
    }
}
fn pin(media: &mut Vec<Arc<Sample>>, audio: Arc<Sample>) {
    if !media.iter().any(|s| Arc::ptr_eq(s, &audio)) {
        media.push(audio);
    }
}
