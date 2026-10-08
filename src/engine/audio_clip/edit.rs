use super::*;
use crate::engine::{
    midi_edit::{Ack, Outcome},
    project::{Captured, SavedClip},
    session::{Axis, Reference},
    Clip, ClipKind, RtEngine,
};
use std::sync::{atomic::AtomicBool, Arc};

/// Inspect one stable clip and its immutable source.
/// Retains producer-owned metadata and media so decoding and drawing stay outside the renderer.
#[derive(Clone, Debug)]
pub(crate) struct Document {
    pub track: u8,
    pub scene: u16,
    pub track_identity: Reference,
    pub scene_identity: Reference,
    pub epoch: u64,
    pub clip: SavedClip,
    pub source: Option<Arc<Sample>>,
}
impl Document {
    /// Inspect an empty or audio clip from a coherent capture.
    /// Takes captured state and retained slots; returns a pinned document or refuses MIDI/deleted targets.
    pub(crate) fn capture(captured: &Captured, track: u8, scene: u16) -> Result<Arc<Self>, String> {
        let layout = captured
            .state
            .session
            .as_ref()
            .ok_or("Session identity unavailable")?;
        let track_identity = layout
            .reference(Axis::Track, usize::from(track))
            .ok_or("Audio clip track is unavailable")?;
        let scene_identity = layout
            .reference(Axis::Scene, usize::from(scene))
            .ok_or("Audio clip scene is unavailable")?;
        let clip = captured
            .state
            .tracks
            .get(usize::from(track))
            .and_then(|t| t.clips.get(usize::from(scene)))
            .ok_or("Audio clip is unavailable")?;
        if clip.kind == ClipKind::Midi {
            return Err("Choose an empty or audio clip; this slot contains MIDI".into());
        }
        let source = clip
            .audio
            .map(|i| {
                captured
                    .media
                    .get(i)
                    .cloned()
                    .ok_or("Audio clip source is unavailable")
            })
            .transpose()?;
        Ok(Arc::new(Self {
            track,
            scene,
            track_identity,
            scene_identity,
            epoch: captured.checkpoint.epoch,
            clip: clip.clone(),
            source,
        }))
    }
    fn current(&self, rt: &RtEngine) -> bool {
        let Some(clip) = rt
            .tracks
            .get(usize::from(self.track))
            .and_then(|t| t.clips.get(usize::from(self.scene)))
        else {
            return false;
        };
        let source = match (&self.source, &clip.audio) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.epoch == rt.undo.checkpoint().epoch
            && rt
                .session
                .resolves(Axis::Track, usize::from(self.track), self.track_identity)
            && rt
                .session
                .resolves(Axis::Scene, usize::from(self.scene), self.scene_identity)
            && source
            && clip.kind == self.clip.kind
            && clip.name == self.clip.name
            && clip.bars == self.clip.bars
            && clip.gain == self.clip.gain
            && clip.region == self.clip.region
            && clip.audio_region.map(|p| p.region) == self.clip.audio_region
            && clip.notes == self.clip.notes
            && clip.lanes == self.clip.lanes
            && !rt.recording_clip_held(usize::from(self.track), usize::from(self.scene))
    }
}
/// Own a prepared source replacement or region edit.
/// Retains reviewed identity and both media reservations for bounded Undo and worker retirement.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(in crate::engine) baseline: Arc<Document>,
    pub(in crate::engine) replacement: Clip,
    pub(in crate::engine) spare_notes: Vec<crate::engine::MidiNote>,
    pub(in crate::engine) reserved_source: Arc<Sample>,
    pub(in crate::engine) metadata_baseline: crate::engine::undo::Checkpoint,
    pub(in crate::engine) ack: Ack,
}
impl Request {
    /// Prepare an audio edit and prove that it remains saveable.
    /// Takes coherent state, its reviewed document, shared PCM, region, name and cancellation; returns a bounded replacement and application receipt.
    pub(crate) fn prepare(
        mut captured: Captured,
        baseline: Arc<Document>,
        source: Arc<Sample>,
        region: Region,
        name: String,
        gain: f32,
        cancel: &AtomicBool,
    ) -> Result<(Box<Self>, Ack), String> {
        if name.len() > 4096
            || name.contains('\0')
            || !gain.is_finite()
            || !(0.0..=1.5).contains(&gain)
        {
            return Err("Audio clip name or gain is invalid".into());
        }
        for chunk in source.data.chunks(16384) {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Audio clip preparation cancelled".into());
            }
            if chunk.iter().any(|v| !v.is_finite()) {
                return Err("Audio source contains nonfinite PCM".into());
            }
        }
        let plan = region.prepare(&source).map_err(str::to_owned)?;
        let checked = Document::capture(&captured, baseline.track, baseline.scene)?;
        if checked.epoch != baseline.epoch
            || checked.track_identity != baseline.track_identity
            || checked.scene_identity != baseline.scene_identity
        {
            return Err("Audio clip destination changed; inspect it again".into());
        }
        let index = captured
            .media
            .iter()
            .position(|s| Arc::ptr_eq(s, &source))
            .unwrap_or_else(|| {
                captured.media.push(source.clone());
                captured.media.len() - 1
            });
        let clip = &mut captured.state.tracks[usize::from(baseline.track)].clips
            [usize::from(baseline.scene)];
        clip.kind = ClipKind::Audio;
        clip.name = name.clone();
        clip.bars = (plan.duration_beats / 4.0) as f32;
        clip.audio_region = Some(region);
        clip.region = None;
        clip.lanes = None;
        clip.audio = Some(index);
        clip.gain = gain;
        captured.state.compact_media(&mut captured.media)?;
        let limits = captured.state.import_metadata_limits();
        if captured.media.len() > limits.max_media
            || captured
                .media
                .iter()
                .map(|s| (s.data.len() as u64).saturating_mul(4))
                .sum::<u64>()
                > limits.max_pcm_bytes
        {
            return Err("Audio edit exceeds the native project's shared media budget".into());
        }
        let checkpoint = captured.checkpoint;
        crate::project_file::validate_metadata(
            &crate::project_file::Bundle {
                state: captured.state,
                media: captured.media,
            },
            &limits,
            cancel,
        )
        .map_err(|e| e.to_string())?;
        let replacement = Clip { variation: None,
            properties: baseline.clip.properties,
            audio_region: Some(plan),
            region: None,
            lanes: None,
            kind: ClipKind::Audio,
            name,
            bars: (plan.duration_beats / 4.0) as f32,
            notes: baseline.clip.notes.clone(),
            gain,
            audio: Some(source.clone()),
        };
        let ack = Ack::new();
        Ok((
            Box::new(Self {
                baseline,
                replacement,
                spare_notes: Vec::with_capacity(crate::engine::project::MAX_NOTES_PER_CLIP),
                reserved_source: source,
                metadata_baseline: checkpoint,
                ack: ack.clone(),
            }),
            ack,
        ))
    }
    /// Check source and target identity before mutation.
    /// Takes the renderer; refuses stale content, recording or an active instance of the target clip.
    pub(in crate::engine) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.baseline.current(rt)
            && self.metadata_baseline == rt.undo.checkpoint()
            && !rt.recording
            && rt
                .tracks
                .get(usize::from(self.baseline.track))
                .is_some_and(|t| {
                    t.playing
                        .or(t.project_resume)
                        .is_none_or(|p| p.scene != self.baseline.scene)
                })
    }
    /// Account for a retired payload.
    /// Takes this request; returns owned metadata and both retained sources for bounded retirement.
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.baseline.clip.name.capacity()
            + (self.baseline.clip.notes.capacity() + self.spare_notes.capacity())
                * std::mem::size_of::<crate::engine::MidiNote>()
            + self.baseline.clip.lanes.as_ref().map_or(0, |l| l.bytes())
            + self
                .baseline
                .source
                .as_ref()
                .map_or(0, |s| crate::engine::undo::sample_bytes(s))
            + self.replacement.retained_bytes()
            + crate::engine::undo::sample_bytes(&self.reserved_source)
    }
}
