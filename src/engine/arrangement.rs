use super::{audio_clip, midi_data, midi_edit, project, session, ClipKind, MidiNote, Sample};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(test)]
mod clock_tests;
pub(crate) mod edit;
mod fades;
mod playback;
mod prepare;
#[cfg(test)]
mod tests;

pub(crate) const MAX_SOURCES: usize = 256;
pub(crate) const MAX_INSTANCES: usize = 4096;
const MAX_EVENTS: usize = 262144;
const MAX_OVERLAP: usize = 64;
const MAX_ACTIVE_NOTES: usize = 256;
const MAX_PLAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_BEATS: f64 = 262144.0;

/// Retain shared song sources and independent timeline placements.
/// Sources are snapshots of reviewed clips; instance edits never alter their source. Playback selects this timeline explicitly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub enabled: bool,
    pub next_id: u64,
    pub sources: Vec<Source>,
    pub instances: Vec<Instance>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            enabled: false,
            next_id: 1,
            sources: Vec::new(),
            instances: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub id: u64,
    pub clip: project::SavedClip,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_clock: Option<AudioClock>,
}

/// Keep an aligned render on its original time map.
/// Stores its source start beat and immutable conductor so tempo ramps retain the rendered sample positions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioClock {
    pub origin: f64,
    pub conductor: Arc<midi_data::Conductor>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Instance {
    pub id: u64,
    pub source: u64,
    pub track: session::Reference,
    pub start: f64,
    pub offset: f64,
    pub duration: f64,
    pub repeating: bool,
    pub gain: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fades: Option<audio_clip::Fades>,
    #[serde(default, skip_serializing_if = "no_link")]
    pub fade_link: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crossfade: Option<u64>,
}
fn no_link(link: &u64) -> bool { *link == 0 }
impl Model {
    /// Allocate a stable song identity on the producer.
    /// Takes the editable model; returns a nonzero unused identity or refuses exhausted storage.
    pub(crate) fn identity(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|_| id != 0)
            .ok_or("Arrangement identities exhausted")?;
        Ok(id)
    }
    /// Validate source metadata and bounded placements.
    /// Takes shared native media and retained track identities; returns a refusal before any renderer replacement.
    pub(crate) fn validate(
        &self,
        media: &[Arc<Sample>],
        layout: &session::Layout,
    ) -> Result<(), String> {
        if self.next_id == 0
            || self.sources.len() > MAX_SOURCES
            || self.instances.len() > MAX_INSTANCES
        {
            return Err("Arrangement source or instance limit exceeded".into());
        }
        let mut ids = std::collections::HashSet::new();
        for source in &self.sources {
            if source.id == 0
                || source.id >= self.next_id
                || !ids.insert(source.id)
                || source.clip.kind == ClipKind::Empty
            {
                return Err("Arrangement source identity or kind is invalid".into());
            }
        }
        let mut notes = 0;
        let mut bytes = 0;
        for source in &self.sources {
            let (n, b) = source.clip.validate(project::STATE_VERSION, media)?;
            notes += n;
            bytes += b;
            if let Some(clock) = &source.audio_clock {
                clock.conductor.validate()?;
                if !clock.origin.is_finite()
                    || !(0.0..=MAX_BEATS).contains(&clock.origin)
                    || source.clip.kind != ClipKind::Audio
                    || source.clip.audio.is_none()
                    || source
                        .clip
                        .audio_region
                        .is_none_or(|r| r.loop_enabled || r.reverse || r.transpose != 0.)
                {
                    return Err("An aligned render needs audio, a finite source start, and an unreversed, untransposed one-shot region".into());
                }
                bytes = bytes
                    .checked_add(clock.conductor.bytes())
                    .ok_or("Aligned render clock storage overflow")?;
            }
        }
        if notes > project::MAX_TOTAL_NOTES || bytes > midi_data::MAX_LANE_BYTES {
            return Err("Arrangement shared MIDI sources exceed native project budgets".into());
        }
        for instance in &self.instances {
            if instance.id == 0
                || instance.id >= self.next_id
                || !ids.insert(instance.id)
                || !self.sources.iter().any(|s| s.id == instance.source)
                || instance.track.namespace != layout.namespace
                || instance.track.id.0 == 0
                || ![instance.start, instance.offset, instance.duration]
                    .into_iter()
                    .all(|n| n.is_finite() && (0.0..=MAX_BEATS).contains(&n))
                || instance.duration < 0.000001
                || instance.repeating
                    && self
                        .sources
                        .iter()
                        .find(|s| s.id == instance.source)
                        .is_some_and(|s| s.audio_clock.is_some())
                || instance.start + instance.duration > MAX_BEATS
                || !instance.gain.is_finite()
                || !(0.0..=1.5).contains(&instance.gain)
            {
                return Err(
                    "Arrangement instance identity, source, range or gain is invalid".into(),
                );
            }
        }
        self.validate_fades(media)?;
        Ok(())
    }
    /// Count retained MIDI content against the native project budget.
    /// Takes this shared source table; returns note count and lane bytes once per source.
    pub(crate) fn midi_storage(&self) -> (usize, usize) {
        self.sources.iter().fold((0, 0), |(n, b), s| {
            (
                n + s.clip.notes.len(),
                b + s.clip.lanes.as_ref().map_or(0, |l| l.bytes()),
            )
        })
    }
}

#[derive(Debug)]
struct PreparedSource {
    audio: Option<Arc<Sample>>,
    audio_region: Option<audio_clip::Plan>,
    length: f64,
    audio_clock: Option<AudioClock>,
}
#[derive(Clone, Copy, Debug)]
struct AudioSpan {
    source: usize,
    start: f64,
    end: f64,
    offset: f64,
    repeating: bool,
    gain: f32,
    fades: Option<audio_clip::Fades>,
}
#[derive(Clone, Copy, Debug)]
struct NoteSpan {
    start: f64,
    end: f64,
    id: midi_edit::NoteId,
    pitch: u8,
    velocity: u8,
    channel: u8,
    release: u8,
    gain: f32,
}
#[derive(Clone, Copy, Debug)]
enum Kind {
    Off(u32),
    On(u32),
    Control { bytes: [u8; 3], length: u8 },
}
#[derive(Clone, Copy, Debug)]
struct Event {
    beat: f64,
    order: u128,
    kind: Kind,
}
#[derive(Debug)]
struct Segment {
    beat: f64,
    active: Box<[u32]>,
}
#[derive(Debug)]
struct TrackPlan {
    reference: session::Reference,
    activity: Vec<Segment>,
    audio: Vec<AudioSpan>,
    audio_segments: Vec<Segment>,
    notes: Vec<NoteSpan>,
    note_segments: Vec<Segment>,
    events: Vec<Event>,
    gates: Vec<Event>,
    controls: Vec<ControlLane>,
}
#[derive(Debug)]
struct ControlLane {
    key: u16,
    points: Vec<(f64, midi_data::StatePoint)>,
}
/// Own an immutable, worker-prepared song schedule.
/// Holds the source PCM, bounded overlap segments and note/controller seek indexes shared by listening and export.
#[derive(Debug)]
pub(crate) struct Plan {
    pub model: Arc<Model>,
    sources: Vec<PreparedSource>,
    tracks: Vec<Option<TrackPlan>>,
    pub end: f64,
    bytes: usize,
}
impl Plan {
    /// Rebind song source media for a coherent native save.
    /// Takes the capture's media table on its worker; returns independent metadata with shared PCM references.
    pub(crate) fn capture(&self, media: &mut Vec<Arc<Sample>>) -> Arc<Model> {
        let mut model = (*self.model).clone();
        for (source, prepared) in model.sources.iter_mut().zip(&self.sources) {
            source.clip.audio = prepared.audio.as_ref().map(|audio| {
                media
                    .iter()
                    .position(|a| Arc::ptr_eq(a, audio))
                    .unwrap_or_else(|| {
                        media.push(audio.clone());
                        media.len() - 1
                    })
            });
        }
        Arc::new(model)
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn media(&self) -> impl Iterator<Item = &Arc<Sample>> {
        self.sources.iter().filter_map(|s| s.audio.as_ref())
    }
}
pub(crate) use playback::Playback;
