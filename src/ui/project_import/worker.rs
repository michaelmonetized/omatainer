use super::*;
use crate::engine::{performance::WorkPermit, project::Handle, session::Request, CommandPort};
use crate::project_file::{self, Bundle, Limits};
use crossbeam_channel::{bounded, Receiver, Sender};

#[derive(Clone)]
pub(super) struct Catalog {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub tracks: Vec<(session::Id, String)>,
    pub scenes: Vec<(session::Id, String)>,
    pub bpm: f32,
}
pub(super) enum Job {
    Browse {
        path: PathBuf,
        work: WorkPermit,
    },
    Review {
        catalog: Catalog,
        selection: session::ImportSelection,
        revision: u64,
        decision: Receiver<()>,
        work: WorkPermit,
    },
}
pub(super) enum Event {
    Browsed(Catalog),
    Reviewed(String, Ack),
    Finished(Ack),
    Failed(String),
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
fn load(
    path: &std::path::Path,
    expected: Option<FileFingerprint>,
    cancel: &AtomicBool,
) -> Result<(Bundle<project::Document>, FileFingerprint), String> {
    let fingerprint = FileFingerprint::read(path).ok_or("Cannot inspect source project file")?;
    if expected.is_some_and(|expected| expected != fingerprint) {
        return Err("Source project changed; browse it again".into());
    }
    let bundle = project_file::load::<project::Document>(path, &Limits::default(), cancel)
        .map_err(|e| e.to_string())?;
    bundle.state.validate()?;
    bundle.state.engine.validate(&bundle.media)?;
    if FileFingerprint::read(path) != Some(fingerprint) {
        return Err("Source project changed while reading; browse it again".into());
    }
    Ok((bundle, fingerprint))
}
impl Worker {
    pub fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (done, events) = bounded(2);
        std::thread::Builder::new().name("project-material-import".into()).spawn(move || {
            while let Ok(job) = incoming.recv() {
                let work = match &job { Job::Browse { work, .. } | Job::Review { work, .. } => work };
                let cancel = work.cancel();
                let result = (|| -> Result<Event, String> {
                    if cancel.load(Ordering::Acquire) { return Err("Project import cancelled".into()); }
                    match job {
                        Job::Browse { path, work: _work } => {
                            let (bundle, fingerprint) = load(&path, None, &cancel)?;
                            let state = bundle.state.engine;
                            let layout = state.session.unwrap_or_else(|| session::Layout::legacy(state.tracks.iter().map(|t| t.name.clone()), state.scene_fx.len()));
                            Ok(Event::Browsed(Catalog { path, fingerprint, bpm: state.bpm,
                                tracks: layout.track_order.iter().map(|&slot| { let item = &layout.tracks[slot as usize]; (item.id, item.name.clone()) }).collect(),
                                scenes: layout.scene_order.iter().map(|&slot| { let item = &layout.scenes[slot as usize]; (item.id, item.name.clone()) }).collect() }))
                        }
                        Job::Review { catalog, selection, revision, decision, work: _work } => {
                            let (source, _) = load(&catalog.path, Some(catalog.fingerprint), &cancel)?;
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            if captured.revision != revision { return Err("Destination changed before review; review the import again".into()); }
                            let rate = handle.sample_rate();
                            let source_rates: std::collections::BTreeSet<_> = source.media.iter().map(|sample| sample.sr).collect();
                            let source_layout = source.state.engine.session.clone().unwrap_or_else(|| session::Layout::legacy(source.state.engine.tracks.iter().map(|t| t.name.clone()), source.state.engine.scene_fx.len()));
                            let mut unavailable = Vec::new();
                            if selection.devices {
                                for id in &selection.tracks {
                                    if let Some(slot) = source_layout.resolve(session::Axis::Track, *id) {
                                        let track = &source.state.engine.tracks[slot];
                                        if let Some(device) = &track.synth.offline { unavailable.push(device.identifier.clone()); }
                                        unavailable.extend(track.fx.iter().filter_map(|effect| effect.offline.as_ref().map(|device| device.identifier.clone())));
                                    }
                                }
                                for id in &selection.scenes {
                                    if let Some(slot) = source_layout.resolve(session::Axis::Scene, *id) {
                                        unavailable.extend(source.state.engine.scene_fx[slot].iter().filter_map(|effect| effect.offline.as_ref().map(|device| device.identifier.clone())));
                                    }
                                }
                            }
                            unavailable.sort(); unavailable.dedup();
                            let mut text = format!("Import {} tracks and {} new scenes. Existing tracks, deck audio, tempo, meter and output routing remain. Source tempo: {} BPM; destination: {} BPM. Clips retain beat/tick positions and play at destination timing. Source audio rates: {:?}; native playback converts to {} Hz. Imported tracks are disarmed, not soloed and not auto-launched. Scene buses use imported source scenes when selected; other buses use the current selected destination scene. Available native devices are restored; unavailable devices retain serialized state and remain bypassed. Picture, decks, global sampler banks, hardware profiles and project-wide conductor automation are outside this selection. Embedded clip/drum audio and clip MIDI controller lanes accompany their tracks.", selection.tracks.len(), selection.scenes.len(), source.state.engine.bpm, captured.state.bpm, source_rates, rate);
                            if !unavailable.is_empty() { text.push_str(&format!("\nUnavailable device identifiers ({}): {}", unavailable.len(), unavailable.join(", "))); }
                            let (request, ack) = Request::import(captured, &source.state.engine, &source.media, &selection, rate)?;
                            if done.send(Event::Reviewed(text, ack.clone())).is_err() { ack.cancel(); return Err("Project import viewer closed".into()); }
                            loop {
                                if cancel.load(Ordering::Acquire) { ack.cancel(); return Err("Project import cancelled before application".into()); }
                                match decision.recv_timeout(Duration::from_millis(20)) {
                                    Ok(()) => break,
                                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                                    Err(_) => { ack.cancel(); return Err("Project import review closed".into()); }
                                }
                            }
                            if FileFingerprint::read(&catalog.path) != Some(catalog.fingerprint) { ack.cancel(); return Err("Source project changed after review; browse it again".into()); }
                            if cancel.load(Ordering::Acquire) { ack.cancel(); return Err("Project import cancelled".into()); }
                            commands.send(Command::SessionEdit(request)).map_err(|e| e.to_string())?;
                            Ok(Event::Finished(ack))
                        }
                    }
                })();
                if cancel.load(Ordering::Acquire) { let _ = handle.retire_cancelled_capture(&cancel); }
                if done.send(result.unwrap_or_else(Event::Failed)).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
