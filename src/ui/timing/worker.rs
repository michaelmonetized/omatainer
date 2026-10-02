use super::*;
use crate::engine::{
    midi_interchange::Request, performance::WorkPermit, project::Handle, CommandPort,
};
use crossbeam_channel::{bounded, Receiver, Sender};

pub(super) struct Job {
    pub draft: Draft,
    pub work: WorkPermit,
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Result<Ack, String>>,
}
impl Worker {
    pub fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("musical-timeline".into()).spawn(move || {
            while let Ok(Job { draft, work }) = incoming.recv() {
                let cancel = work.cancel();
                let result = (|| {
                    if work.cancelled() { return Err("Timing edit cancelled".into()); }
                    let map = draft.map()?;
                    let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                    if captured.state.session.as_ref().is_none_or(|layout| layout.namespace != draft.namespace)
                        || captured.state.conductor.as_deref() != draft.baseline.as_ref()
                        || draft.baseline.is_none() && captured.state.bpm != draft.bpm {
                        return Err("Project or timing changed. Draft retained; review the current timeline before applying.".into());
                    }
                    let (request, ack) = Request::prepare_timing(captured, Some(map), || work.cancelled())?;
                    if work.cancelled() { ack.cancel(); return Err("Timing edit cancelled".into()); }
                    commands.send(Command::MidiImport(request)).map_err(|e| e.to_string())?;
                    Ok(ack)
                })();
                if work.cancelled() { let _ = handle.retire_cancelled_capture(&cancel); }
                if completed.send(result).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
