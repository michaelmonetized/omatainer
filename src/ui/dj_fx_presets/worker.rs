use super::*;
use crate::engine::performance::WorkPermit;
use crossbeam_channel::{bounded, Receiver, Sender};
pub(super) enum Operation { Save(Preset), Inspect }
pub(super) struct Job { pub path: PathBuf, pub operation: Operation, pub work: WorkPermit }
pub(super) enum Event { Saved(crate::project_file::SaveOutcome), Inspected(Preset), Failed(String) }
pub(super) struct Worker { pub jobs: Sender<Job>, pub events: Receiver<Event> }
impl Worker {
    /// Start one bounded preset-file worker.
    /// Takes no device handles; returns a single-job worker or a thread-start error.
    pub(super) fn start() -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1); let (completed, events) = bounded(1);
        std::thread::Builder::new().name("omatainer-dj-fx-presets".into()).spawn(move || {
            while let Ok(job) = incoming.recv() {
                let cancel = job.work.cancel();
                let result = (|| -> Result<Event, String> {
                    let ticket = job.work.background(crate::background::Kind::Prepare, "dj-fx-presets".into(), 256 * 1024)?;
                    let _running = ticket.enter(|| cancel.load(Ordering::Acquire))?;
                    if job.work.cancelled() { return Err("Preset operation cancelled".into()); }
                    match job.operation {
                        Operation::Save(preset) => crate::engine::dj_fx_preset::save(&job.path, preset, &cancel).map(Event::Saved),
                        Operation::Inspect => crate::engine::dj_fx_preset::load(&job.path, &cancel).map(Event::Inspected),
                    }
                })();
                if completed.send(result.unwrap_or_else(Event::Failed)).is_err() { break; }
            }
        }).map_err(|error| error.to_string())?;
        Ok(Self { jobs, events })
    }
}
