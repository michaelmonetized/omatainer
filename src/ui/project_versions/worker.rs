use super::*;
use crate::{
    engine::{performance::WorkPermit, project::Handle},
    project_versions::{Entry, Review, Store},
};
use crossbeam_channel::{bounded, Receiver, Sender};

#[derive(Clone)]
pub(in crate::ui) struct Record {
    pub root: PathBuf,
    pub entry: Entry,
    pub fingerprint: FileFingerprint,
}
pub(super) enum Task {
    List,
    Snapshot {
        name: String,
        notes: String,
        view: project::UiState,
        identities: Vec<project::WatchIdentity>,
    },
    Compare {
        entry: Entry,
        revision: u64,
        view: project::UiState,
    },
    Branch {
        record: Record,
        path: PathBuf,
    },
    PreviewPrune {
        ids: Vec<String>,
    },
    Prune {
        review: Review,
    },
}
pub(super) struct Job {
    pub root: PathBuf,
    pub task: Task,
    pub work: WorkPermit,
}
pub(super) enum Event {
    Listed(Vec<Entry>),
    Compared(Record, u64, project::UiState, String),
    PruneReviewed(Review),
    Published(Option<Vec<Entry>>, String),
    Failed(String),
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
    #[cfg(test)]
    hooks: Arc<parking_lot::Mutex<Option<(Sender<()>, Receiver<()>)>>>,
}

/// Read the reviewed immutable revision without accepting a changed entry or file.
/// Takes a stable record and cancellation flag; returns a validated native document.
pub(in crate::ui) fn reviewed(
    record: &Record,
    cancel: &AtomicBool,
) -> Result<crate::project_file::Bundle<project::Document>, String> {
    let path = record
        .root
        .join("revisions")
        .join(format!("{}.omat", record.entry.id));
    if FileFingerprint::read(&path) != Some(record.fingerprint) {
        return Err("Named version changed after comparison; refresh and compare again".into());
    }
    let store = Store::open(&record.root, false, cancel)?;
    let bundle = store.reopen::<project::Document>(&record.entry, cancel)?;
    bundle.state.validate()?;
    bundle.state.engine.validate(&bundle.media)?;
    if FileFingerprint::read(&path) != Some(record.fingerprint) {
        return Err("Named version changed while reading".into());
    }
    Ok(bundle)
}
impl Worker {
    #[cfg(test)]
    pub fn pause_next(&self) -> (Receiver<()>, Sender<()>) {
        let (entered, observed) = bounded(1);
        let (resume, waiting) = bounded(1);
        *self.hooks.lock() = Some((entered, waiting));
        (observed, resume)
    }
    pub fn start(handle: Handle) -> Result<Self, String> {
        #[cfg(test)]
        let hooks = Arc::new(parking_lot::Mutex::new(None::<(Sender<()>, Receiver<()>)>));
        #[cfg(test)]
        let thread_hooks = hooks.clone();
        let (jobs, incoming) = bounded::<Job>(1);
        let (done, events) = bounded(2);
        std::thread::Builder::new()
            .name("project-versions".into())
            .spawn(move || {
                while let Ok(job) = incoming.recv() {
                    #[cfg(test)]
                    {
                        let gate = thread_hooks.lock().take();
                        if let Some((entered, resume)) = gate {
                            let _ = entered.send(());
                            let _ = resume.recv();
                        }
                    }
                    let cancel = job.work.cancel();
                    let result = perform(job, &handle, &cancel);
                    if cancel.load(Ordering::Acquire) {
                        let _ = handle.retire_cancelled_capture(&cancel);
                    }
                    if done.send(result.unwrap_or_else(Event::Failed)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            jobs,
            events,
            #[cfg(test)]
            hooks,
        })
    }
}
fn warning(outcome: crate::project_file::SaveOutcome) -> String {
    match outcome {
        crate::project_file::SaveOutcome::Durable => String::new(),
        crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(w) => {
            format!(" Durability warning: {w}")
        }
    }
}
fn perform(job: Job, handle: &Handle, cancel: &AtomicBool) -> Result<Event, String> {
    let Job { root, task, work } = job;
    if cancel.load(Ordering::Acquire) {
        return Err("Version operation cancelled".into());
    }
    if let Task::Branch { record, path } = task {
        let bundle = reviewed(&record, cancel)?;
        let _commit = work.commit().map_err(|e| e.to_string())?;
        let outcome = crate::project_file::save(
            &path,
            &bundle,
            crate::project_file::Overwrite::Never,
            &crate::project_file::Limits::default(),
            cancel,
        )
        .map_err(|e| e.to_string())?;
        return Ok(Event::Published(
            None,
            format!(
                "Branched {} into {}.{} Open this new project from Project → Open when ready.",
                record.entry.name,
                path.display(),
                warning(outcome)
            ),
        ));
    }
    let mut store = Store::open(&root, matches!(task, Task::Snapshot { .. }), cancel)?;
    match task {
        Task::List => Ok(Event::Listed(store.entries().to_vec())),
        Task::Snapshot {
            name,
            notes,
            mut view,
            identities,
        } => {
            let captured = handle.capture(cancel).map_err(|e| e.to_string())?;
            view.deck_identities = std::array::from_fn(|deck| {
                captured.playback_receipts[deck].as_ref().and_then(|r| {
                    identities
                        .iter()
                        .find(|i| i.receipt.same_request(r))
                        .map(|i| i.identity.clone())
                })
            });
            let document = project::Document {
                engine: captured.state,
                view,
                mapping_schema: project::FACTORY_MAPPING_SCHEMA,
            };
            document.validate()?;
            document.engine.validate(&captured.media)?;
            let bundle = crate::project_file::Bundle {
                state: document,
                media: captured.media,
            };
            let (entry, outcome) =
                store.snapshot(&bundle, name.trim().into(), notes, cancel, || {
                    work.commit().map_err(|e| e.to_string())
                })?;
            Ok(Event::Published(
                Some(store.entries().to_vec()),
                format!("Saved named version {}.{}", entry.name, warning(outcome)),
            ))
        }
        Task::Compare {
            entry,
            revision,
            view,
        } => {
            let revision_path = root.join("revisions").join(format!("{}.omat", entry.id));
            let fingerprint =
                FileFingerprint::read(&revision_path).ok_or("Cannot inspect named revision")?;
            let saved = store.reopen::<project::Document>(&entry, cancel)?;
            saved.state.validate()?;
            saved.state.engine.validate(&saved.media)?;
            let captured = handle.capture(cancel).map_err(|e| e.to_string())?;
            if captured.revision != revision {
                return Err("Current project changed before comparison; compare again".into());
            }
            let current = crate::project_file::Bundle {
                state: project::Document {
                    engine: captured.state,
                    view,
                    mapping_schema: project::FACTORY_MAPPING_SCHEMA,
                },
                media: captured.media,
            };
            let report = compare(&current, &saved, cancel)?;
            let view = current.state.view;
            if FileFingerprint::read(&revision_path) != Some(fingerprint) {
                return Err("Named version changed during comparison".into());
            }
            Ok(Event::Compared(
                Record {
                    root,
                    entry,
                    fingerprint,
                },
                revision,
                view,
                report,
            ))
        }
        Task::PreviewPrune { ids } => Ok(Event::PruneReviewed(store.review_prune(&ids, cancel)?)),
        Task::Prune { review } => {
            let result =
                store.prune(review, cancel, || work.commit().map_err(|e| e.to_string()))?;
            Ok(Event::Published(
                Some(store.entries().to_vec()),
                format!(
                    "Pruned {} versions and reclaimed {} shared audio files.{}",
                    result.removed,
                    result.reclaimed_audio,
                    result.warning.map(|w| format!(" {w}")).unwrap_or_default()
                ),
            ))
        }
        Task::Branch { .. } => unreachable!(),
    }
}

/// Compare musical categories using stable project, track and scene identities.
/// Takes live and saved native bundles; returns a bounded change preview without PCM text.
pub(super) fn compare(
    current: &crate::project_file::Bundle<project::Document>,
    saved: &crate::project_file::Bundle<project::Document>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    use serde_json::{json, Value};
    if cancel.load(Ordering::Acquire) {
        return Err("Version comparison cancelled".into());
    }
    use std::collections::BTreeMap;
    fn categories(
        bundle: &crate::project_file::Bundle<project::Document>,
        cancel: &AtomicBool,
    ) -> Result<([BTreeMap<String, Value>; 4], BTreeMap<String, String>), String> {
        let state = &bundle.state.engine;
        let legacy;
        let layout = match &state.session {
            Some(l) => l,
            None => {
                legacy = crate::engine::session::Layout::legacy(
                    state.tracks.iter().map(|t| t.name.clone()),
                    state.scene_fx.len(),
                );
                &legacy
            }
        };
        let hashes: Vec<String> = bundle
            .media
            .iter()
            .map(|s| {
                crate::project_dependencies::audio_hash(s, cancel)
                    .map(|h| h.iter().map(|b| format!("{b:02x}")).collect())
            })
            .collect::<Result<_, _>>()?;
        let scene = |slot: usize| format!("{:x?}/{}", layout.namespace, layout.scenes[slot].id.0);
        let mut labels = BTreeMap::new();
        let mut out: [BTreeMap<String, Value>; 4] = std::array::from_fn(|_| BTreeMap::new());
        for &slot in &layout.track_order {
            let slot = slot as usize;
            let track = &state.tracks[slot];
            let item = &layout.tracks[slot];
            let key = format!("Track [{:x?}/{}]", layout.namespace, item.id.0);
            labels.insert(key.clone(), format!("Track: {}", item.name));
            out[0].insert(key.clone(), json!({"name":item.name,"color":item.color,"position":layout.track_order.iter().position(|s| *s as usize == slot)}));
            out[2].insert(key.clone(), json!({"bus":scene(track.scene_bus),"gain":track.gain,"pan":track.pan,"mute":track.mute,"solo":track.solo,"armed":track.armed,"input_monitor":track.input_monitor}));
            out[3].insert(key.clone(), json!({"instrument":track.synth,"kind":track.kind,"effects":track.fx,"eq":track.eq,"drums":track.drums.map(|i| hashes.get(i))}));
            for &s in &layout.scene_order {
                let mut clip =
                    serde_json::to_value(&track.clips[s as usize]).map_err(|e| e.to_string())?;
                clip["audio"] = json!(track.clips[s as usize].audio.and_then(|i| hashes.get(i)));
                let clip_key = format!("{key} / Scene [{}]", scene(s as usize));
                labels.insert(
                    clip_key.clone(),
                    format!("{} / {}", item.name, layout.scenes[s as usize].name),
                );
                out[1].insert(clip_key, clip);
            }
        }
        let mut decks = serde_json::to_value(&state.decks).map_err(|e| e.to_string())?;
        for (i, deck) in state.decks.iter().enumerate() {
            decks[i]["audio"] = json!(deck.audio.and_then(|i| hashes.get(i)));
        }
        out[2].insert("Master and decks".into(), json!({"master":state.master,"xfader":state.xfader,"curve":state.xfader_curve,"cue":state.cue_mix,"mic_aux":state.mic_aux,"decks":decks}));
        let mut banks = serde_json::to_value(&state.banks).map_err(|e| e.to_string())?;
        for (i, bank) in state.banks.iter().enumerate() {
            banks[i]["media"] = json!(bank.media.map(|i| i.and_then(|i| hashes.get(i))));
        }
        let scene_devices: BTreeMap<_, _> = layout
            .scene_order
            .iter()
            .map(|&s| (scene(s as usize), &state.scene_fx[s as usize]))
            .collect();
        out[3].insert("Scene and master devices".into(), json!({"scenes":scene_devices,"master":state.fx_kind,"wet":state.fx_wet,"sampler":state.sampler_synth,"banks":banks}));
        out[0].insert(
            "Scene names and order".into(),
            json!({"items":layout.scenes,"order":layout.scene_order}),
        );
        Ok((out, labels))
    }
    let (live, live_labels) = categories(current, cancel)?;
    let (version, labels) = categories(saved, cancel)?;
    let mut text = format!("Current project → selected version. Restore replaces the complete project, stopped, as an unsaved copy. Save as selects a new destination.\nTiming: {} → {} BPM. Conductor changed: {}. Picture/view changed: {}.\n", current.state.engine.bpm, saved.state.engine.bpm, current.state.engine.conductor != saved.state.engine.conductor, current.state.view != saved.state.view);
    for (i, title) in [
        "Tracks and scenes",
        "Clips and controller lanes",
        "Routing and mixer",
        "Instruments and effects",
    ]
    .iter()
    .enumerate()
    {
        let added: Vec<_> = version[i]
            .keys()
            .filter(|k| !live[i].contains_key(*k))
            .collect();
        let removed: Vec<_> = live[i]
            .keys()
            .filter(|k| !version[i].contains_key(*k))
            .collect();
        let changed: Vec<_> = version[i]
            .iter()
            .filter(|(k, v)| live[i].get(*k).is_some_and(|old| old != *v))
            .map(|(k, _)| k)
            .collect();
        text.push_str(&format!(
            "\n{title}: {} added, {} removed, {} changed.\n",
            added.len(),
            removed.len(),
            changed.len()
        ));
        for (label, keys) in [("Added", added), ("Removed", removed), ("Changed", changed)] {
            for key in keys.iter().take(12) {
                text.push_str(&format!(
                    "{label}: {}\n",
                    labels
                        .get(*key)
                        .or_else(|| live_labels.get(*key))
                        .unwrap_or(*key)
                        .chars()
                        .take(180)
                        .collect::<String>()
                ));
                if label == "Changed" {
                    let mut changes = Vec::new();
                    differences("", &live[i][*key], &version[i][*key], &mut changes);
                    for change in changes {
                        text.push_str(&format!("  {change}\n"));
                    }
                }
            }
            if keys.len() > 12 {
                text.push_str(&format!("… {} more {label} items\n", keys.len() - 12));
            }
        }
    }
    Ok(text)
}

/// Describe up to eight changed fields within a musical category.
/// Takes two JSON settings values; appends bounded before/after values without audio data.
fn differences(
    path: &str,
    old: &serde_json::Value,
    new: &serde_json::Value,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    if old == new || out.len() >= 8 {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                differences(
                    &format!("{path}{key}."),
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    out,
                );
                if out.len() >= 8 {
                    break;
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!(
                    "{} count: {} → {}",
                    path.trim_end_matches('.'),
                    a.len(),
                    b.len()
                ));
            }
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}{}.", i + 1), a, b, out);
                if out.len() >= 8 {
                    break;
                }
            }
        }
        _ => {
            let compact = |v: &Value| v.to_string().chars().take(80).collect::<String>();
            out.push(format!(
                "{}: {} → {}",
                path.trim_end_matches('.'),
                compact(old),
                compact(new)
            ));
        }
    }
}
