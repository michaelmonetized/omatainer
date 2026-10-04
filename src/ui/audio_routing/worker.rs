//! Capture, validation and native input operations stay on a bounded worker.
use super::*;
use crate::engine::{performance::WorkPermit, CommandPort};
use crossbeam_channel::{bounded, Receiver, Sender};

pub(super) enum Job {
    Inspect(Arc<AtomicBool>),
    Apply(Draft, WorkPermit),
    PreviewInput(InputConfig, Arc<AtomicBool>),
    EnableInput(InputPreview, Arc<AtomicBool>),
    DisableInput(u64, Arc<AtomicBool>),
    Record(Draft, u64, u32, std::path::PathBuf, WorkPermit),
}
pub(super) enum Event {
    Inspected(Result<Draft, String>),
    Applied(Result<Ack, String>),
    InputPreview(Result<InputPreview, String>),
    InputChanged(Result<String, String>),
    Recorded(Result<std::path::PathBuf, String>),
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
impl Worker {
    /// Start the routing worker.
    /// Takes project, command, audio and recorder handles; returns bounded job/result channels or a thread error.
    pub fn start(
        project: crate::engine::project::Handle,
        commands: CommandPort,
        output: Option<crate::engine::audio::owner::Handle>,
        input: Option<input::Handle>,
        recorder: crate::engine::audio::routing::record::Recorder,
    ) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("omatainer-routing-editor".into()).spawn(move || {
            while let Ok(job) = incoming.recv() {
                let cancel = match &job {
                    Job::Inspect(cancel) | Job::PreviewInput(_, cancel) | Job::EnableInput(_, cancel) | Job::DisableInput(_, cancel) => cancel.clone(),
                    Job::Apply(_, work) | Job::Record(_, _, _, _, work) => work.cancel(),
                };
                let event = match job {
                    Job::Inspect(cancel) => Event::Inspected((|| {
                        let captured = project.capture(&cancel).map_err(|error| error.to_string())?;
                        let layout = captured.state.session.ok_or("Session identity unavailable")?;
                        let channels = output.as_ref().and_then(|handle| handle.status().active.as_ref().map(|active| usize::from(active.plan.channels))).unwrap_or(2);
                        Ok(Draft { namespace: layout.namespace, generation: layout.generation, revision: captured.revision, rate: project.sample_rate(), layout, enabled: captured.state.routing.is_some(), model: captured.state.routing.map(|model| (*model).clone()).unwrap_or_else(|| Model::for_output_channels(channels)) })
                    })()),
                    Job::Apply(draft, work) => Event::Applied((|| {
                        if work.cancelled() { return Err("Routing edit cancelled".into()); }
                        let captured = project.capture(&work.cancel()).map_err(|error| error.to_string())?;
                        let layout = captured.state.session.as_ref().ok_or("Session identity unavailable")?;
                        if captured.revision != draft.revision || layout.namespace != draft.namespace || layout.generation != draft.generation || project.sample_rate() != draft.rate { return Err("Project changed since routing inspection. Refresh routes before applying.".into()); }
                        let model = draft.enabled.then(|| Arc::new(draft.model));
                        let (request, ack) = crate::engine::session::Request::routing(captured, draft.rate, model)?;
                        if work.cancelled() { ack.cancel(); return Err("Routing edit cancelled".into()); }
                        commands.send(Command::SessionEdit(request)).map_err(|error| error.to_string())?;
                        Ok(ack)
                    })()),
                    Job::PreviewInput(saved, cancel) => Event::InputPreview((|| {
                        if cancel.load(Ordering::Acquire) { return Err("Input preview cancelled".into()); }
                        let handle = output.as_ref().ok_or("No native output owner")?;
                        if handle.safe_mode() { return Err("Inputs are disabled in safe mode".into()); }
                        let status = handle.status();
                        let active = status.active.as_ref().ok_or("Recover an output before enabling input")?;
                        let inventory = crate::engine::audio::config::discover()?;
                        let plan = input::preview(&saved, &inventory, &active.plan)?;
                        if cancel.load(Ordering::Acquire) { return Err("Input preview cancelled".into()); }
                        Ok(InputPreview { saved, plan, output_generation: status.generation })
                    })()),
                    Job::EnableInput(preview, cancel) => Event::InputChanged(input.as_ref().ok_or_else(|| "No native input owner".into()).and_then(|handle| handle.apply(Some(preview.plan), preview.output_generation, cancel)).map(|status| status.message.clone())),
                    Job::DisableInput(generation, cancel) => Event::InputChanged(input.as_ref().ok_or_else(|| "No native input owner".into()).and_then(|handle| handle.apply(None, generation, cancel)).map(|status| status.message.clone())),
                    Job::Record(draft, alias, seconds, destination, work) => Event::Recorded((|| {
                        let epoch = recorder.epoch();
                        let captured = project.capture(&work.cancel()).map_err(|error| error.to_string())?;
                        if captured.state.session.as_ref().is_none_or(|layout| layout.namespace != draft.namespace)
                            || captured.state.routing.as_deref() != Some(&draft.model) || project.sample_rate() != draft.rate {
                            return Err("Apply routing and refresh before capturing a record source".into());
                        }
                        let port = draft.model.port(alias, Direction::Record).ok_or("Selected record source is unavailable")?;
                        recorder.write(alias, port.channels.len() as u16, draft.rate, seconds, &destination, &work.cancel(), epoch)
                    })()),
                };
                if cancel.load(Ordering::Acquire) { let _ = project.retire_cancelled_capture(&cancel); }
                if completed.send(event).is_err() { break; }
            }
        }).map_err(|error| error.to_string())?;
        Ok(Self { jobs, events })
    }
}
