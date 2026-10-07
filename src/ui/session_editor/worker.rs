use crate::engine::{
    midi_edit::Ack,
    performance::WorkPermit,
    project::Handle,
    session::{Action, Layout, Request, Structure},
    Command, CommandPort,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) enum Job {
    Metadata {
        layout: Layout,
        epoch: u64,
        action: Action,
        cancel: Arc<AtomicBool>,
    },
    Structure {
        namespace: [u64; 2],
        generation: u64,
        operation: Structure,
        work: WorkPermit,
    },
}
impl Job {
    fn cancel(&self) -> Arc<AtomicBool> {
        match self {
            Self::Metadata { cancel, .. } => cancel.clone(),
            Self::Structure { work, .. } => work.cancel(),
        }
    }
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Result<Ack, String>>,
}
impl Worker {
    pub fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("session-layout-editor".into()).spawn(move || {
            while let Ok(job) = incoming.recv() {
                let cancel = job.cancel();
                let result = (|| -> Result<Ack, String> {
                    if cancel.load(Ordering::Acquire) { return Err("Session edit cancelled".into()); }
                    let (request, ack) = match job {
                        Job::Metadata {layout, epoch, action,..} => Request::metadata(&layout, epoch, action)?,
                        Job::Structure {namespace, generation, operation, work} => {
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            let layout = captured.state.session.as_ref().ok_or("Session identity unavailable")?;
                            if layout.namespace != namespace || layout.generation != generation {
                                return Err("Session layout changed during preparation; review and retry".into());
                            }
                            if work.cancelled() { return Err("Session edit cancelled".into()); }
                            Request::structural(captured, handle.sample_rate(), operation)?
                        }
                    };
                    if cancel.load(Ordering::Acquire) { ack.cancel(); return Err("Session edit cancelled".into()); }
                    commands.send(Command::session_edit(request)).map_err(|e| e.to_string())?;
                    Ok(ack)
                })();
                if cancel.load(Ordering::Acquire) { let _ = handle.retire_cancelled_capture(&cancel); }
                if completed.send(result).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
