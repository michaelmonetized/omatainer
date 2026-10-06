use super::*;
use crate::engine::{
    performance::WorkPermit,
    project::{Captured, Handle},
    CommandPort,
};
use crossbeam_channel::{bounded, Receiver, Sender};

pub(super) struct Preview {
    pub captured: Captured,
    pub choices: Vec<(usize, usize, String)>,
}
pub(super) enum Job {
    Inspect(WorkPermit),
    Apply {
        preview: Arc<Preview>,
        model: Model,
        work: WorkPermit,
    },
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
    pub(super) fn start(handle: Handle, commands: CommandPort) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("omatainer-arrangement".into()).spawn(move||{
            let mut pins=Vec::<Arc<Preview>>::with_capacity(4);
            loop {
                pins.retain(|p|Arc::strong_count(p)!=1);
                let job=match incoming.recv_timeout(std::time::Duration::from_millis(20)){Ok(j)=>j,Err(crossbeam_channel::RecvTimeoutError::Timeout)=>continue,Err(_)=>break};
                let work=match &job{Job::Inspect(w)=>w,Job::Apply{work,..}=>work};let cancel=work.cancel();
                let ticket=work.background(crate::background::Kind::Prepare,"arrangement-edit".into(),crate::background::MEMORY_BYTES);
                let result=(||->Result<Event,String>{
                    let ticket=ticket?;
                    let _running=ticket.enter(||cancel.load(Ordering::Acquire))?;
                    match job {
                        Job::Inspect(work)=>{
                            if pins.len()>=4{return Err("Previous song previews are retiring; retry shortly".into());}
                            let captured=handle.capture(&cancel).map_err(|e|e.to_string())?;if work.cancelled(){return Err("Song inspection cancelled".into());}
                            let layout=captured.state.session.as_ref().ok_or("Song track identities unavailable")?;
                            let mut choices=Vec::new();
                            for &track in &layout.track_order{for &scene in &layout.scene_order{let clip=&captured.state.tracks[track as usize].clips[scene as usize];if clip.kind!=crate::engine::ClipKind::Empty{choices.push((track as usize,scene as usize,format!("{} / {} · {}",layout.tracks[track as usize].name,layout.scenes[scene as usize].name,clip.name)));}}}
                            let preview=Arc::new(Preview{captured,choices});pins.push(preview.clone());Ok(Event::Preview(preview))
                        }
                        Job::Apply{preview,mut model,work}=>{
                            let mut captured=handle.capture(&cancel).map_err(|e|e.to_string())?;
                            if captured.checkpoint!=preview.captured.checkpoint{return Err("Project changed since song review; refresh before applying".into());}
                            for source in &mut model.sources{if let Some(index)=source.clip.audio{let audio=preview.captured.media.get(index).ok_or("Reviewed song source disappeared")?;source.clip.audio=Some(captured.media.iter().position(|a|Arc::ptr_eq(a,audio)).unwrap_or_else(||{captured.media.push(audio.clone());captured.media.len()-1}));}}
                            let(request,ack)=crate::engine::arrangement::edit::Request::prepare(captured,model,&cancel)?;
                            if work.cancelled(){return Err("Song edit cancelled".into());}
                            commands.send(Command::ArrangementEdit(request)).map_err(|e|e.to_string())?;Ok(Event::Queued(ack))
                        }
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
