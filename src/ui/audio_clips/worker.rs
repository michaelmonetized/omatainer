use super::*;
use crate::engine::{
    media_source::FileFingerprint, performance::WorkPermit, project::Handle, CommandPort,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::time::Duration;

pub(super) struct Preview {
    pub document: Arc<Document>,
    pub source: Option<Arc<Sample>>,
    pub sources: Vec<Arc<Sample>>,
    pub tempo: f32,
    pub warning: Option<String>,
}
pub(super) enum Job {
    Inspect {
        track: u8,
        scene: u16,
        target: (Reference, Reference),
        work: WorkPermit,
    },
    Load {
        preview: Arc<Preview>,
        path: PathBuf,
        work: WorkPermit,
    },
    Apply {
        preview: Arc<Preview>,
        source: Arc<Sample>,
        region: Region,
        name: String,
        gain: f32,
        work: WorkPermit,
    },
}
impl Job {
    fn work(&self) -> &WorkPermit {
        match self {
            Self::Inspect { work, .. } | Self::Load { work, .. } | Self::Apply { work, .. } => work,
        }
    }
}
pub(super) enum Event {
    Preview(Arc<Preview>),
    Queued(Ack),
    Failed(String),
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
impl Worker {
    pub fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("omatainer-audio-clips".into()).spawn(move || {
            let mut pins: Vec<Arc<Preview>> = Vec::with_capacity(4);
            loop {
                pins.retain(|pin| Arc::strong_count(pin) != 1);
                let job = match incoming.recv_timeout(Duration::from_millis(20)) {
                    Ok(job) => job, Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue, Err(_) => break,
                };
                let cancel = job.work().cancel();
                let result = (|| -> Result<Event, String> {
                    if job.work().cancelled() { return Err("Audio clip operation cancelled".into()); }
                    let key = match &job {
                        Job::Inspect { track, scene, .. } => format!("audio-clip-review-{track}-{scene}"),
                        Job::Load { path, .. } => crate::background::identity(&("audio-clip-decode", path))?,
                        Job::Apply { preview, .. } => format!("audio-clip-edit-{}-{}", preview.document.track, preview.document.scene),
                    };
                    let ticket = job.work().background(crate::background::Kind::Prepare, key, crate::background::MEMORY_BYTES)?;
                    let _running = ticket.enter(|| job.work().cancelled())?;
                    match job {
                        Job::Inspect { track, scene, target, work } => {
                            if pins.len() >= 4 { return Err("Previous audio previews are still retiring; retry shortly".into()); }
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            let document = Document::capture(&captured, track, scene)?;
                            if (document.track_identity, document.scene_identity) != target || work.cancelled() { return Err("Audio target changed or inspection was cancelled".into()); }
                            let preview = Arc::new(Preview { source: document.source.clone(), document, sources: captured.media, tempo: captured.state.bpm, warning: None });
                            pins.push(preview.clone()); Ok(Event::Preview(preview))
                        }
                        Job::Load { preview, path, work } => {
                            if pins.len() >= 4 { return Err("Previous audio previews are still retiring; retry shortly".into()); }
                            use std::os::unix::fs::OpenOptionsExt;
                            let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&path).map_err(|e| e.to_string())?;
                            let metadata = file.metadata().map_err(|e| e.to_string())?;
                            if !metadata.is_file() { return Err("Audio import requires a regular file".into()); }
                            let fingerprint = FileFingerprint::from_metadata(&metadata);
                            let identity = file.try_clone().map_err(|e| e.to_string())?;
                            let decoded = crate::engine::decode::decode_sampler_file(&path, file, 128 * 1024 * 1024, || work.cancelled()).map_err(|e| e.to_string())?;
                            if work.cancelled() || FileFingerprint::read(&path) != Some(fingerprint) || identity.metadata().ok().map(|m| FileFingerprint::from_metadata(&m)) != Some(fingerprint) {
                                return Err("Audio file changed or import was cancelled; inspect it again".into());
                            }
                            Region::full(&decoded.sample, preview.tempo).map_err(str::to_owned)?;
                            let source = preview.sources.iter().find(|s| s.sr == decoded.sample.sr && s.ch == decoded.sample.ch && s.data == decoded.sample.data).cloned().unwrap_or_else(|| Arc::new(decoded.sample));
                            let mut sources = preview.sources.clone();
                            if !sources.iter().any(|s| Arc::ptr_eq(s, &source)) { sources.push(source.clone()); }
                            let next = Arc::new(Preview { document: preview.document.clone(), source: Some(source), sources, tempo: preview.tempo, warning: decoded.diagnostics.warning().map(str::to_owned) });
                            pins.push(next.clone()); Ok(Event::Preview(next))
                        }
                        Job::Apply { preview, source, region, name, gain, work } => {
                            let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                            let (request, ack) = Request::prepare(captured, preview.document.clone(), source, region, name, gain, &cancel)?;
                            if work.cancelled() { return Err("Audio clip edit cancelled".into()); }
                            commands.send(Command::AudioClipEdit(request)).map_err(|e| e.to_string())?;
                            Ok(Event::Queued(ack))
                        }
                    }
                })();
                if cancel.load(Ordering::Acquire) { let _ = handle.retire_cancelled_capture(&cancel); }
                if completed.send(result.unwrap_or_else(Event::Failed)).is_err() { break; }
            }
            while !pins.is_empty() {
                pins.retain(|pin| Arc::strong_count(pin) != 1);
                if !pins.is_empty() { std::thread::sleep(Duration::from_millis(20)); }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
