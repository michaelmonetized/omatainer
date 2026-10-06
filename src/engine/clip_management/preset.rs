use super::super::{project, ClipKind, Sample};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// Retain one reusable clip with its complete native musical settings.
/// The native container embeds at most one shared audio source; importing assigns new MIDI identities and leaves source files untouched.
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preset {
    schema: u32,
    clip_schema: u32,
    pub clip: project::SavedClip,
}
impl<'de> Deserialize<'de> for Preset {
    fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
        let raw=serde_json::Value::deserialize(deserializer)?;
        if raw["clip_schema"].as_u64().unwrap_or(0)<21 && raw.get("clip").and_then(|c|c.get("properties")).is_some_and(|p|p.get("launch").is_some()){return Err(serde::de::Error::custom("Clip launch policy requires preset clip schema 21"));}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {schema:u32,clip_schema:u32,clip:project::SavedClip}
        let wire:Wire=serde_json::from_value(raw).map_err(serde::de::Error::custom)?;
        Ok(Self{schema:wire.schema,clip_schema:wire.clip_schema,clip:wire.clip})
    }
}
pub(crate) type Bundle = crate::project_file::Bundle<Preset>;
fn limits() -> crate::project_file::Limits {
    crate::project_file::Limits {
        max_media: 1,
        max_pcm_bytes: 128 * 1024 * 1024,
        ..Default::default()
    }
}
impl Preset {
    /// Capture a source clip without copying its audio.
    /// Takes a validated saved clip and project media; returns a compact native preset or refuses an empty source.
    pub(crate) fn capture(
        mut clip: project::SavedClip,
        media: &[Arc<Sample>],
        cancel: &AtomicBool,
    ) -> Result<Bundle, String> {
        let sources = clip
            .audio
            .map(|i| media.get(i).cloned().ok_or("Preset source was removed"))
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        clip.audio = clip.audio.map(|_| 0);
        let bundle = Bundle {
            state: Self {
                schema: 1,
                clip_schema: project::STATE_VERSION,
                clip,
            },
            media: sources,
        };
        bundle.state.validate(&bundle.media, cancel)?;
        Ok(bundle)
    }
    /// Validate an independent preset and its embedded source.
    /// Takes shared media and cancellation; proves strict schema, nonempty content, bounded PCM and finite samples before preparation or publication.
    pub(crate) fn validate(
        &self,
        media: &[Arc<Sample>],
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if self.schema != 1
            || !(20..=project::STATE_VERSION).contains(&self.clip_schema)
            || self.clip.kind == ClipKind::Empty
            || media.len() != usize::from(self.clip.audio.is_some())
            || self.clip.audio.is_some_and(|i| i != 0)
        {
            return Err("Unsupported or empty clip preset".into());
        }
        self.clip.validate(self.clip_schema, media)?;
        if let Some(audio) = media.first() {
            if !(8000..=384000).contains(&audio.sr)
                || !matches!(audio.ch, 1 | 2)
                || audio.frames() == 0
                || audio.data.len() % usize::from(audio.ch) != 0
                || audio.data.len() > 128 * 1024 * 1024 / 4
            {
                return Err("Clip preset source is empty, unsupported or exceeds 128 MiB".into());
            }
            for chunk in audio.data.chunks(16384) {
                active(cancel)?;
                if chunk.iter().any(|v| !v.is_finite()) {
                    return Err("Clip preset contains nonfinite PCM".into());
                }
            }
        }
        active(cancel)
    }
    /// Reuse native source media when inserting a reviewed preset.
    /// Takes the preset source and destination media on the worker; returns rebased clip content with fresh MIDI identities and shared PCM.
    pub(crate) fn insert(
        &self,
        sources: &[Arc<Sample>],
        media: &mut Vec<Arc<Sample>>,
        cancel: &AtomicBool,
    ) -> Result<project::SavedClip, String> {
        self.validate(sources, cancel)?;
        let mut clip = self.clip.clone();
        if let Some(audio) = sources.first() {
            let mut same = None;
            for (index, old) in media.iter().enumerate() {
                active(cancel)?;
                if Arc::ptr_eq(old, audio) {
                    same = Some(index);
                    break;
                }
                if old.sr != audio.sr || old.ch != audio.ch || old.data.len() != audio.data.len() {
                    continue;
                }
                let mut equal = true;
                for (a, b) in old.data.chunks(16384).zip(audio.data.chunks(16384)) {
                    active(cancel)?;
                    if a.iter().zip(b).any(|(a, b)| a.to_bits() != b.to_bits()) {
                        equal = false;
                        break;
                    }
                }
                if equal {
                    same = Some(index);
                    break;
                }
            }
            clip.audio = Some(same.unwrap_or_else(|| {
                media.push(audio.clone());
                media.len() - 1
            }));
        }
        for note in &mut clip.notes {
            note.id = super::super::midi_edit::NoteId::new();
        }
        Ok(clip)
    }
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Clip preset operation cancelled".into())
    } else {
        Ok(())
    }
}
/// Read a complete native clip preset on its worker.
/// Takes a file path and cancellation; returns validated content and bounded immutable PCM after container integrity checks.
pub(crate) fn read(path: &Path, cancel: &AtomicBool) -> Result<Bundle, String> {
    let bundle =
        crate::project_file::load::<Preset>(path, &limits(), cancel).map_err(|e| e.to_string())?;
    bundle.state.validate(&bundle.media, cancel)?;
    Ok(bundle)
}
/// Publish a new native clip preset without replacing a source file.
/// Takes reviewed content, destination and cancellation; returns the actual durable-save outcome or a refusal that preserves existing files.
pub(crate) fn write(
    bundle: &Bundle,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<crate::project_file::SaveOutcome, String> {
    bundle.state.validate(&bundle.media, cancel)?;
    crate::project_file::save(
        path,
        bundle,
        crate::project_file::Overwrite::Never,
        &limits(),
        cancel,
    )
    .map_err(|e| e.to_string())
}
