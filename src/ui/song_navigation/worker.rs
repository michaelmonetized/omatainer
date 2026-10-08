use super::*;
use crate::engine::{performance::WorkPermit, project::Handle, CommandPort};
use crossbeam_channel::{bounded, Receiver, Sender};
pub(super) struct Job {
    pub expected: Option<Saved>,
    pub model: Model,
    pub looping: bool,
    pub jump: Option<(f64, Grid)>,
    pub work: WorkPermit,
}
pub(super) enum Event {
    Queued(Ack),
    Failed(String),
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
impl Worker {
    /// Start the bounded native section-edit worker.
    /// Takes coherent project capture and command admission; returns one-job channels or a thread-start failure without changing the song.
    pub(super) fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new()
            .name("omatainer-song-sections".into())
            .spawn(move || {
                while let Ok(job) = incoming.recv() {
                    let cancel = job.work.cancel();
                    let result = (|| -> Result<Event, String> {
                        let ticket = job.work.background(
                            crate::background::Kind::Prepare,
                            "song-sections".into(),
                            crate::background::MEMORY_BYTES,
                        )?;
                        let _running = ticket.enter(|| cancel.load(Ordering::Acquire))?;
                        let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                        let (request, ack) = crate::engine::song_navigation::Request::prepare(
                            captured,
                            job.expected,
                            job.model,
                            job.looping,
                            job.jump,
                            &cancel,
                        )?;
                        if job.work.cancelled() {
                            return Err("Song section edit cancelled".into());
                        }
                        commands
                            .send(Command::SongNavigationEdit(request))
                            .map_err(|e| e.to_string())?;
                        Ok(Event::Queued(ack))
                    })();
                    if cancel.load(Ordering::Acquire) {
                        let _ = handle.retire_cancelled_capture(&cancel);
                    }
                    if completed
                        .send(result.unwrap_or_else(Event::Failed))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
