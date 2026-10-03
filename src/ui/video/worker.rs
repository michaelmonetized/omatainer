use super::*;
use crate::engine::{performance::WorkPermit, project::Handle};
use crossbeam_channel::{bounded, Receiver, SendTimeoutError};
use std::sync::atomic::{AtomicBool, Ordering};
pub(super) enum Kind {
    Import {
        path: PathBuf,
        expected: Option<Clip>,
    },
    Decode {
        clip: Clip,
        start: u64,
    },
    Render {
        clip: Clip,
        path: PathBuf,
        revision: u64,
    },
}
pub(super) enum Reply {
    Imported(Clip),
    Decoded,
    Rendered(PathBuf, crate::project_file::SaveOutcome),
}
pub(super) struct Job {
    pub decode_start: Option<u64>,
    pub source_operation: bool,
    cancel: Arc<AtomicBool>,
    pub frames: Receiver<data::decoder::Frame>,
    result: Receiver<Result<Reply, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub intentionally_cancelled: bool,
}
impl Job {
    /// Start one bounded local picture job.
    /// Takes operation, actual project owner and performance permit; returns nonblocking result and three-frame channels.
    pub fn start(kind: Kind, handle: Handle, permit: WorkPermit) -> Result<Self, String> {
        let source_operation = !matches!(&kind, Kind::Render { .. });
        let cancel = permit.cancel();
        let stop = cancel.clone();
        let decode_start = match &kind {
            Kind::Decode { start, .. } => Some(*start),
            _ => None,
        };
        let (tx, frames) = bounded(3);
        let (done, result) = bounded(1);
        let thread=std::thread::Builder::new().name("video-picture".into()).spawn(move|| {
            let result=(|| { if stop.load(Ordering::Acquire){return Err("Video operation cancelled".into());}
                match kind {
                    Kind::Import{path,expected}=>{
                        let imported=data::decoder::probe(path,&stop)?;
                        let clip=if let Some(saved)=expected {
                            if saved.fingerprint!=imported.fingerprint || saved.info!=imported.info {return Err("Saved picture source changed; explicitly reimport it before using the new bytes".into());}saved
                        }else{imported};
                        if stop.load(Ordering::Acquire){return Err("Video import cancelled before publication".into());}Ok(Reply::Imported(clip))
                    },
                    Kind::Decode{clip,start}=>{
                        data::decoder::stream(&clip,start,&stop,|mut frame|loop {
                            if stop.load(Ordering::Acquire){return Err("Video preview cancelled".into());}
                            match tx.send_timeout(frame,Duration::from_millis(10)) {
                                Ok(())=>return Ok(()),Err(SendTimeoutError::Timeout(next))=>frame=next,
                                Err(SendTimeoutError::Disconnected(_))=>return Err("Picture viewer closed".into()),
                            }
                        })?;Ok(Reply::Decoded)
                    },
                    Kind::Render{clip,path,revision}=>{
                        let captured=handle.capture(&stop).map_err(|e|e.to_string())?;
                        if captured.revision!=revision {return Err("Project changed before render capture; review and render again".into());}
                        data::render::run(captured,&clip,&path,&permit).map(|outcome|Reply::Rendered(path,outcome))
                    },
                }
            })();
            if permit.cancelled(){let _=handle.retire_cancelled_capture(&stop);}
            let _=done.try_send(result);
        }).map_err(|e|e.to_string())?;
        Ok(Self {
            decode_start,
            source_operation,
            cancel,
            frames,
            result,
            thread: Some(thread),
            intentionally_cancelled: false,
        })
    }
    /// Cancel pending decoding or publication without blocking a native frame.
    /// Takes this job; publication that already committed retains its truthful result.
    pub fn cancel(&mut self) {
        self.intentionally_cancelled = true;
        self.cancel.store(true, Ordering::Release);
    }
    /// Consume a finished worker result.
    /// Takes this job; returns once only after its owned child has been reaped.
    pub fn poll(&mut self) -> Option<Result<Reply, String>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        let _ = self.thread.take().unwrap().join();
        let result = self
            .result
            .try_recv()
            .unwrap_or_else(|_| Err("Video worker ended without a result".into()));
        if self.cancel.load(Ordering::Acquire) && !matches!(result, Ok(Reply::Rendered(..))) {
            Some(Err("Video operation cancelled".into()))
        } else {
            Some(result)
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
