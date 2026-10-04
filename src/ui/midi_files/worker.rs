use super::*;
use crate::engine::media_source::FileFingerprint;
use crate::engine::{performance::WorkPermit, project::Handle, CommandPort};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::time::Duration;

pub(super) struct Preview {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub file: crate::midi_file::File,
}
pub(super) enum Job {
    Inspect {
        path: PathBuf,
        work: WorkPermit,
    },
    Import {
        preview: Arc<Preview>,
        mappings: Vec<Mapping>,
        targets: Vec<Option<(crate::engine::session::Reference,crate::engine::session::Reference)>>,
        split: bool,
        merge: bool,
        tempo: TempoChoice,
        reviewed: bool,
        rounding: bool,
        work: WorkPermit,
    },
    Export {
        path: PathBuf,
        cells: Vec<(u8, u16)>,
        targets: Vec<Option<(crate::engine::session::Reference,crate::engine::session::Reference)>>,
        options: ExportOptions,
        work: WorkPermit,
    },
}
impl Job {
    fn work(&self) -> &WorkPermit {
        match self {
            Self::Inspect { work, .. } | Self::Import { work, .. } | Self::Export { work, .. } => {
                work
            }
        }
    }
}
pub(super) enum Event {
    Preview(Arc<Preview>),
    Queued(Ack),
    Written(PathBuf),
    Failed(String),
}
#[cfg(test)]
type Hook = Arc<parking_lot::Mutex<Option<(Sender<()>, Receiver<()>)>>>;
pub(super) struct Worker {
    #[cfg(test)]
    hook: Hook,
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
impl Worker {
    #[cfg(test)]
    pub fn pause_import(&self) -> (Receiver<()>, Sender<()>) {
        let (entered, entered_rx) = bounded(1);
        let (resume_tx, resume) = bounded(1);
        *self.hook.lock() = Some((entered, resume));
        (entered_rx, resume_tx)
    }
    pub fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        #[cfg(test)]
        let hook = Hook::default();
        #[cfg(test)]
        let worker_hook = hook.clone();
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("midi-file-interchange".into()).spawn(move || {
            // Preview pins are retired only on this worker after every GUI/job
            // reference leaves. Closing a large file never frees it on the GUI.
            let mut pins: Vec<Arc<Preview>> = Vec::with_capacity(3);
            loop {
                pins.retain(|pin| Arc::strong_count(pin) != 1);
                let job = match incoming.recv_timeout(Duration::from_millis(20)) {
                    Ok(job) => job,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let cancel = job.work().cancel();
                let result = (|| -> Result<Event, String> {
                    if job.work().cancelled() { return Err("MIDI operation cancelled".into()); }
                    let (kind,key)=match &job {
                        Job::Inspect{path,..}=>(crate::background::Kind::Prepare,crate::background::identity(&("midi-inspect",path))?),
                        Job::Import{preview,..}=>(crate::background::Kind::Prepare,crate::background::identity(&("midi-import",&preview.path,preview.fingerprint))?),
                        Job::Export{path,cells,..}=>(crate::background::Kind::Render,crate::background::identity(&("midi-export",path,cells))?),
                    };
                    let ticket=job.work().background(kind,key,crate::background::MEMORY_BYTES)?;
                    let _running=ticket.enter(||job.work().cancelled())?;
                    match job {
                        Job::Inspect { path, work: _work } => {
                            if pins.len() >= 3 { return Err("Previous MIDI previews are still retiring; retry shortly".into()); }
                            let (file, fingerprint) = crate::midi_file_io::read(&path, &cancel)?;
                            let preview = Arc::new(Preview { path, fingerprint, file });
                            pins.push(preview.clone()); Ok(Event::Preview(preview))
                        }
                        Job::Import { preview, mappings, targets, split, merge, tempo, reviewed, rounding, work } => {
                            if FileFingerprint::read(&preview.path) != Some(preview.fingerprint) {
                                return Err("Inspected MIDI file changed; inspect it again before importing".into());
                            }
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            for (mapping,expected) in mappings.iter().zip(targets) {
                                if let (Some((t,s)),Some(expected))=(mapping.destination,expected) {
                                    if super::cell_reference(captured.state.session.as_ref(),t,s)!=Some(expected) {return Err("MIDI destination identity changed; review its replacement explicitly".into());}
                                }
                            }
                            let (request, ack) = Request::prepare_with_cancel(captured, &preview.file, &mappings, split, merge, tempo, reviewed, rounding, || work.cancelled())?;
                            #[cfg(test)] if let Some((entered, resume)) = worker_hook.lock().take() { let _ = entered.send(()); let _ = resume.recv(); }
                            if work.cancelled() || FileFingerprint::read(&preview.path) != Some(preview.fingerprint) {
                                return Err("MIDI import cancelled or source changed during preparation".into());
                            }
                            commands.send(Command::MidiImport(request)).map_err(|e| e.to_string())?;
                            Ok(Event::Queued(ack))
                        }
                        Job::Export { path, cells, targets, options, work } => {
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            if work.cancelled() { return Err("MIDI export cancelled".into()); }
                            for (&(t,s),expected) in cells.iter().zip(targets) {
                                if let Some(expected)=expected {
                                    if super::cell_reference(captured.state.session.as_ref(),t,s)!=Some(expected) {return Err("MIDI export target changed; uncheck it and explicitly select its replacement".into());}
                                }
                            }
                            let file = crate::engine::midi_interchange::export_with_cancel(&captured.state, &cells, options, || work.cancelled())?;
                            let bytes = crate::midi_file::encode_with_cancel(&file, false, || work.cancelled()).map_err(|e| e.to_string())?;
                            crate::midi_file_io::write_new(&path, &bytes, &work)?;
                            Ok(Event::Written(path))
                        }
                    }
                })();
                if cancel.load(Ordering::Acquire) {
                    let _ = handle.retire_cancelled_capture(&cancel);
                }
                if completed.send(result.unwrap_or_else(Event::Failed)).is_err() { break; }
            }
            // On shutdown keep outstanding GUI previews pinned until they
            // leave; the detached recycler never holds a live audio graph.
            while !pins.is_empty() {
                pins.retain(|pin| Arc::strong_count(pin) != 1);
                if !pins.is_empty() { std::thread::sleep(Duration::from_millis(20)); }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self {
            jobs,
            events,
            #[cfg(test)]
            hook,
        })
    }
}
