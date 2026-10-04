//! File copying, decoding and native project preparation stay off the GUI.
use super::*;
use crate::{
    engine::{
        performance::WorkPermit,
        project::{Handle, Prepared},
    },
    project_file::{Bundle, Limits, Overwrite, SaveOutcome},
};
use crossbeam_channel::{bounded, Receiver, Sender};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct Review {
    pub revision: u64,
    pub namespace: [u64; 2],
    pub origins: Vec<dependencies::Origin>,
    pub inventory: Arc<dependencies::Inventory>,
    pub manifest: Arc<data::Manifest>,
    pub catalog: Arc<crate::licenses::Catalog>,
}
pub(super) enum Kind {
    Inspect(project::UiState),
    Export {
        review: Review,
        selected: Vec<usize>,
        view: project::UiState,
        path: PathBuf,
    },
    Preview(PathBuf),
    Import {
        archive: PathBuf,
        destination: PathBuf,
        reviewed: Arc<data::Manifest>,
    },
}
pub(super) enum ResultData {
    Inspected(Review),
    Previewed(PathBuf, Arc<data::Manifest>, Arc<crate::licenses::Catalog>),
    Exported(PathBuf, SaveOutcome),
    Imported(PathBuf, SaveOutcome),
}
pub(super) struct Job {
    pub kind: Kind,
    pub work: WorkPermit,
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Result<ResultData, String>>,
}
impl Worker {
    /// Start one bounded portable-project worker for the active engine.
    /// `handle` captures musical state; returned channels admit one job at a time.
    pub fn start(handle: Handle) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new()
            .name("portable-project".into())
            .spawn(move || {
                while let Ok(Job { kind, work }) = incoming.recv() {
                    let cancel = work.cancel();
                    let scheduled=match &kind {
                        Kind::Inspect(_)=>crate::background::identity(&("portable-inspect",)),
                        Kind::Export{path,review,..}=>crate::background::identity(&("portable-export",path,review.revision,review.namespace)),
                        Kind::Preview(path)=>crate::background::identity(&("portable-preview",path)),
                        Kind::Import{archive,destination,..}=>crate::background::identity(&("portable-import",archive,destination)),
                    }.and_then(|key|work.background(crate::background::Kind::Prepare,key,crate::background::MEMORY_BYTES));
                    let result=match scheduled {
                        Err(error)=>Err(error),Ok(ticket)=>match ticket.enter(||work.cancelled()) {
                            Err(error)=>Err(error),Ok(_running)=>{let result=perform(kind,&handle,&cancel);ticket.progress(1,Some(1));result},
                        },
                    };
                    if work.cancelled() {
                        let _ = handle.retire_cancelled_capture(&cancel);
                    }
                    if completed.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}

/// List every saved device and its exact preset/settings identity.
/// `state` supplies native and unavailable IDs; the returned hashes cover all
/// scalar controls and opaque device state without disclosing that state in UI.
fn devices(
    state: &crate::engine::project::State,
    cancel: &AtomicBool,
) -> Result<Vec<data::Device>, String> {
    let mut result = Vec::new();
    let mut add = |placement: String,
                   value: serde_json::Value,
                   field: &str,
                   unavailable: bool|
     -> Result<(), String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Portable dependency inspection cancelled".into());
        }
        let identifier = value[field]
            .as_str()
            .ok_or("Device identifier missing")?
            .to_string();
        let settings_sha256 =
            Sha256::digest(serde_json::to_vec(&value).map_err(|e| e.to_string())?).into();
        if cancel.load(Ordering::Acquire) {
            return Err("Portable dependency inspection cancelled".into());
        }
        result.push(data::Device { placement, identifier, settings_sha256, unavailable,
            state_schema: value.get("state").and_then(|state| state.get("schema")).and_then(serde_json::Value::as_u64).map(|v| v as u32),
            rights: if unavailable { "External device license and redistribution rights are unverified; no plugin binary is included." } else { "Omatainer native implementation: MIT. Saved settings contain no external preset file." }.into() });
        Ok(())
    };
    for (track, value) in state.tracks.iter().enumerate() {
        add(
            format!("Track {} instrument", track + 1),
            serde_json::to_value(&value.synth).map_err(|e| e.to_string())?,
            "kind",
            value.synth.offline.is_some(),
        )?;
        for (slot, effect) in value.fx.iter().enumerate() {
            add(
                format!("Track {} effect {}", track + 1, slot + 1),
                serde_json::to_value(effect).map_err(|e| e.to_string())?,
                "id",
                effect.offline.is_some(),
            )?;
        }
    }
    for (scene, rack) in state.scene_fx.iter().enumerate() {
        for (slot, effect) in rack.iter().enumerate() {
            add(
                format!("Scene {} effect {}", scene + 1, slot + 1),
                serde_json::to_value(effect).map_err(|e| e.to_string())?,
                "id",
                effect.offline.is_some(),
            )?;
        }
    }
    add(
        "Sampler instrument".into(),
        serde_json::to_value(&state.sampler_synth).map_err(|e| e.to_string())?,
        "kind",
        state.sampler_synth.offline.is_some(),
    )?;
    for (slot, (kind, wet)) in state.fx_kind.iter().zip(state.fx_wet).enumerate() {
        add(
            format!("Master effect {}", slot + 1),
            serde_json::json!({"kind": kind, "wet": wet}),
            "kind",
            false,
        )?;
    }
    for (index, bank) in state.banks.iter().enumerate() {
        add(
            format!("Sampler bank {}: {}", index + 1, bank.name),
            serde_json::json!({"kind": "omatainer.sampler-bank", "settings": bank.settings, "name": bank.name}),
            "kind",
            false,
        )?;
    }
    Ok(result)
}

pub(in crate::ui) fn manifest(
    document: &project::Document,
    inventory: &dependencies::Inventory,
    cancel: &AtomicBool,
) -> Result<data::Manifest, String> {
    let mut media = Vec::new();
    for asset in &inventory.assets {
        media.push(data::Media { key: asset.key.clone(), name: asset.name.clone(), frames: asset.frames, sample_rate: asset.sample_rate, channels: asset.channels,
            source: asset.source.as_ref().map(super::super::dependencies::source_name), collected: None,
            rights: "Audio redistribution rights are unverified. Include only content you are allowed to share.".into() });
    }
    Ok(data::Manifest {
        schema: 1,
        application: format!(
            "Omatainer {} · {} {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        license_manifest: crate::licenses::MANIFEST.into(),
        license_notices: crate::licenses::NOTICES.into(),
        media,
        devices: devices(&document.engine, cancel)?,
        unresolved: inventory
            .missing
            .iter()
            .map(|missing| {
                format!(
                    "{}: {} (no embedded audio)",
                    missing.placement, missing.source
                )
            })
            .collect(),
        entries: Vec::new(),
    })
}

pub(in crate::ui) fn destination_parent(destination: &Path) -> Result<&Path, String> {
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("Choose an absolute destination path with a new name".into());
    }
    if std::fs::symlink_metadata(destination).is_ok() {
        return Err("Destination already exists; choose a new name".into());
    }
    destination
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| "Destination parent folder does not exist".into())
}
fn perform(kind: Kind, handle: &Handle, cancel: &AtomicBool) -> Result<ResultData, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Portable project operation cancelled".into());
    }
    match kind {
        kind @ (Kind::Inspect(_) | Kind::Export { .. }) => {
            let (view, export) = match kind {
                Kind::Inspect(view) => (view, None),
                Kind::Export {
                    review,
                    selected,
                    view,
                    path,
                } => (view, Some((review, selected, path))),
                _ => unreachable!(),
            };
            if view.video.is_some() { return Err("Portable archives currently contain native audio only; clear the external picture reference in a saved copy before exporting. Native projects retain it.".into()); }
            let captured = handle.capture(cancel).map_err(|e| e.to_string())?;
            let namespace = captured
                .state
                .session
                .as_ref()
                .ok_or("Project identity missing")?
                .namespace;
            let revision = captured.revision;
            let bundle = Bundle {
                state: project::Document {
                    engine: captured.state,
                    view,
                    mapping_schema: project::FACTORY_MAPPING_SCHEMA,
                },
                media: captured.media,
            };
            bundle.state.validate()?;
            let inventory = dependencies::inspect(
                &bundle.state.engine,
                &bundle.media,
                &bundle.state.view.media_origins,
                cancel,
            )?;
            let mut manifest = manifest(&bundle.state, &inventory, cancel)?;
            match export {
                None => Ok(ResultData::Inspected(Review {
                    revision,
                    namespace,
                    origins: bundle.state.view.media_origins.clone(),
                    inventory: Arc::new(inventory),
                    catalog: Arc::new(manifest.catalog()?),
                    manifest: Arc::new(manifest),
                })),
                Some((review, selected, path)) => {
                    if revision != review.revision
                        || namespace != review.namespace
                        || bundle.state.view.media_origins != review.origins
                    {
                        return Err(
                            "Project changed since inspection; inspect again before exporting"
                                .into(),
                        );
                    }
                    let parent = destination_parent(&path)?;
                    let stage = data::Stage::new(parent)?;
                    let mut remaining = data::MAX_SOURCE_BYTES;
                    let mut seen = std::collections::HashSet::new();
                    for index in selected {
                        let reviewed = review
                            .inventory
                            .assets
                            .get(index)
                            .ok_or("Invalid source selection")?;
                        if !seen.insert(&reviewed.key) {
                            return Err("Duplicate source selection".into());
                        }
                        let current = inventory
                            .assets
                            .iter()
                            .position(|asset| {
                                asset.key == reviewed.key && asset.source == reviewed.source
                            })
                            .ok_or("Source reference changed since inspection")?;
                        let asset = &inventory.assets[current];
                        let source = asset
                            .source
                            .as_ref()
                            .ok_or("Selected audio has no external original")?;
                        let (name, bytes) = data::collect(
                            &stage,
                            source,
                            &asset.key,
                            asset.pcm_bytes,
                            remaining,
                            cancel,
                        )?;
                        remaining -= bytes;
                        manifest.media[current].collected = Some(name);
                    }
                    let outcome = data::export(&path, &stage, &bundle, manifest, cancel)?;
                    Ok(ResultData::Exported(path, outcome))
                }
            }
        }
        Kind::Preview(path) => {
            let manifest = data::preview(&path, cancel)?;
            let catalog = Arc::new(manifest.catalog()?);
            Ok(ResultData::Previewed(path, Arc::new(manifest), catalog))
        }
        Kind::Import {
            archive,
            destination,
            reviewed,
        } => {
            let parent = destination_parent(&destination)?;
            let (stage, manifest, mut bundle) =
                data::extract::<project::Document>(&archive, parent, cancel)?;
            if manifest != *reviewed {
                return Err(
                    "Archive dependencies changed since review; review the archive again".into(),
                );
            }
            rebind_document(
                &stage,
                &manifest,
                &mut bundle.state,
                &bundle.media,
                &destination,
                handle.sample_rate(),
                cancel,
            )?;
            crate::project_file::save(
                &stage.path.join("session.omat"),
                &bundle,
                Overwrite::Replace,
                &Limits::default(),
                cancel,
            )
            .map_err(|e| e.to_string())?;
            let outcome = data::publish(stage, &destination, cancel)?;
            Ok(ResultData::Imported(
                destination.join("session.omat"),
                outcome,
            ))
        }
    }
}

/// Verify device identities and rebind collected audio before preparing a native document.
/// `stage` owns checked archive entries; `destination` supplies published source paths.
/// Returns an error without publication; all source audio is retained in `media`.
pub(in crate::ui) fn rebind_document(
    stage: &data::Stage,
    manifest: &data::Manifest,
    document: &mut project::Document,
    media: &[Arc<crate::engine::dsp::Sample>],
    destination: &Path,
    sample_rate: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    document.validate()?;
    document.engine.validate(media)?;
    if devices(&document.engine, cancel)? != manifest.devices {
        return Err("Device/preset manifest differs from the native session".into());
    }
    let missing: Vec<_> = dependencies::missing_sources(&document.engine)
        .iter()
        .map(|missing| {
            format!(
                "{}: {} (no embedded audio)",
                missing.placement, missing.source
            )
        })
        .collect();
    if missing != manifest.unresolved {
        return Err("Unresolved source manifest differs from the native session".into());
    }
    let mut origins = document.view.media_origins.clone();
    for media in &manifest.media {
        if let Some(name) = &media.collected {
            let temporary = LibSource::File(stage.path.join(name));
            let bytes = (media.frames as u64)
                .checked_mul(u64::from(media.channels))
                .and_then(|bytes| bytes.checked_mul(4))
                .ok_or("Invalid audio shape")?;
            dependencies::load_source(
                &temporary,
                media.key.audio_hash,
                bytes.saturating_add(4 * 1024 * 1024),
                || cancel.load(Ordering::Acquire),
            )
            .map_err(|e| e.detail)?;
            origins.retain(|origin| origin.key != media.key);
            origins.push(dependencies::Origin {
                key: media.key.clone(),
                source: LibSource::File(destination.join(name)),
            });
        }
    }
    dependencies::validate_origins(&origins)?;
    for bank in &mut document.engine.banks {
        if let Some(settings) = &mut bank.settings {
            let settings = Arc::make_mut(settings);
            for (index, slot) in settings.slots.iter_mut().enumerate() {
                if let Some(sample) = bank.media[index].and_then(|index| media.get(index)) {
                    let hash = dependencies::audio_hash(sample, cancel)?;
                    if let Some(origin) = origins.iter().find(|origin| {
                        origin.key.audio_hash == hash && origin.key.original_path == sample.path
                    }) {
                        slot.source = Some(crate::sampler_bank::Source::Project {
                            source: origin.source.clone(),
                            audio_hash: hash,
                        });
                    }
                }
            }
        }
    }
    document.view.media_origins = origins;
    document.validate()?;
    let prepared = Prepared::from_state(document.engine.clone(), media.to_vec(), sample_rate)
        .map_err(|e| e.to_string())?;
    drop(prepared);
    Ok(())
}
