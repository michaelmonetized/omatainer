use super::*;
use crate::engine::{
    clip_management::{edit::Request, preset},
    performance::WorkPermit,
    project::{Captured, Handle},
    CommandPort,
};
use crossbeam_channel::{bounded, Receiver, Sender};
pub(super) struct Preview {
    pub captured: Arc<Captured>,
    pub slots: Vec<(Slot, String)>,
    pub preset: Option<Arc<preset::Bundle>>,
}
impl Preview {
    pub fn indices(&self, slot: Slot) -> Option<(usize, usize)> {
        slot.resolve(self.captured.state.session.as_ref()?).ok()
    }
    pub fn clip(&self, slot: Slot) -> Option<&crate::engine::project::SavedClip> {
        let (t, s) = self.indices(slot)?;
        self.captured.state.tracks.get(t)?.clips.get(s)
    }
}
pub(super) enum Job {
    Inspect(WorkPermit),
    Apply {
        preview: Arc<Preview>,
        action: Action,
        work: WorkPermit,
    },
    Read {
        preview: Arc<Preview>,
        path: PathBuf,
        work: WorkPermit,
    },
    Write {
        preview: Arc<Preview>,
        source: Slot,
        path: PathBuf,
        work: WorkPermit,
    },
}
impl Job {
    fn work(&self) -> &WorkPermit {
        match self {
            Self::Inspect(w)
            | Self::Apply { work: w, .. }
            | Self::Read { work: w, .. }
            | Self::Write { work: w, .. } => w,
        }
    }
}
pub(super) enum Event {
    Preview(Arc<Preview>),
    Queued(Ack),
    Saved(String),
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
        std::thread::Builder::new().name("omatainer-clip-manager".into()).spawn(move||{
            let mut pins=Vec::<Arc<Preview>>::with_capacity(4);
            loop{
                pins.retain(|p|Arc::strong_count(p)!=1);
                let job=match incoming.recv_timeout(std::time::Duration::from_millis(20)){Ok(j)=>j,Err(crossbeam_channel::RecvTimeoutError::Timeout)=>continue,Err(_)=>break};
                let cancel=job.work().cancel();
                let result=(||->Result<Event,String>{
                    let ticket=job.work().background(crate::background::Kind::Prepare,"session-clip-management".into(),crate::background::MEMORY_BYTES)?;
                    let _running=ticket.enter(||cancel.load(Ordering::Acquire))?;
                    match job{
                        Job::Inspect(work)=>{
                            if pins.len()>=4{return Err("Previous clip previews are retiring; retry shortly".into());}
                            let captured=handle.capture(&cancel).map_err(|e|e.to_string())?;
                            let layout=captured.state.session.as_ref().ok_or("Clip identities are unavailable")?;
                            let mut slots=Vec::new();for &t in &layout.track_order{for &s in &layout.scene_order{let(t,s)=(usize::from(t),usize::from(s));let clip=&captured.state.tracks[t].clips[s];slots.push((Slot::at(layout,t,s)?,format!("{} / {} · {:?} · {}",layout.tracks[t].name,layout.scenes[s].name,clip.kind,clip.name)));}}
                            if work.cancelled(){return Err("Clip review cancelled".into());}
                            let preview=Arc::new(Preview{captured:Arc::new(captured),slots,preset:None});pins.push(preview.clone());Ok(Event::Preview(preview))
                        },
                        Job::Apply{preview,action,work}=>{
                            let captured=handle.capture(&cancel).map_err(|e|e.to_string())?;
                            if captured.checkpoint!=preview.captured.checkpoint{return Err("Project changed since clip review; refresh before applying".into());}
                            let(request,ack)=Request::prepare(captured,action,&cancel)?;
                            if work.cancelled(){return Err("Clip edit cancelled".into());}
                            commands.send(Command::ClipManage(request)).map_err(|e|e.to_string())?;Ok(Event::Queued(ack))
                        },
                        Job::Read{preview,path,work}=>{
                            if pins.len()>=4{return Err("Previous clip previews are retiring; retry shortly".into());}
                            let preset=Arc::new(preset::read(&path,&cancel)?);if work.cancelled(){return Err("Preset inspection cancelled".into());}
                            let next=Arc::new(Preview{captured:preview.captured.clone(),slots:preview.slots.clone(),preset:Some(preset)});pins.push(next.clone());Ok(Event::Preview(next))
                        },
                        Job::Write{preview,source,path,work}=>{
                            let clip=preview.clip(source).ok_or("Preset source identity changed")?.clone();
                            let bundle=preset::Preset::capture(clip,&preview.captured.media,&cancel)?;
                            if work.cancelled(){return Err("Preset save cancelled".into());}
                            let outcome=preset::write(&bundle,&path,&cancel)?;
                            Ok(Event::Saved(format!("Clip preset saved: {}{}",path.display(),if matches!(outcome,crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(_)){" (directory sync warning)"}else{""})))
                        },
                    }
                })();
                if cancel.load(Ordering::Acquire){let _=handle.retire_cancelled_capture(&cancel);}
                if completed.send(result.unwrap_or_else(Event::Failed)).is_err(){break;}
            }
            while !pins.is_empty(){pins.retain(|p|Arc::strong_count(p)!=1);if !pins.is_empty(){std::thread::sleep(std::time::Duration::from_millis(20));}}
        }).map_err(|e|e.to_string())?;
        Ok(Self { jobs, events })
    }
}
