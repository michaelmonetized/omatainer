//! Optional, cancellable local inspection/export. No upload or network API.
use super::*;
use crate::engine::performance::{Handle, WorkPermit};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::Arc;
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub routes: bool,
    pub events: bool,
    pub performance: bool,
    pub recovery: bool,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            routes: true,
            events: true,
            performance: true,
            recovery: true,
        }
    }
}
pub struct Preview {
    pub report: Arc<Report>,
    pub json: Arc<str>,
    pub lines: Vec<std::ops::Range<usize>>,
}
impl Preview {
    fn new(report: Report) -> Result<Self, Error> {
        let bytes = storage::encode(&report)?;
        let json = String::from_utf8(bytes).map_err(|_| Error::Invalid("invalid report text"))?;
        let mut lines = Vec::new();
        let mut start = 0;
        for line in json.split_inclusive('\n') {
            lines.push(start..start + line.len());
            start += line.len();
        }
        Ok(Self {
            report: Arc::new(report),
            json: json.into(),
            lines,
        })
    }
}
pub enum Job {
    Preview(Arc<Report>, Selection),
    Previous(PathBuf),
    Reopen(PathBuf),
    Export(PathBuf, Arc<Report>),
    Recovery(PathBuf, RecoveryRef),
}
pub enum ResultValue {
    Preview(Preview),
    Previous(storage::Inventory),
    Exported(storage::Published),
    Recovery(Option<crate::recovery::Candidate>),
}
pub struct Completed {
    pub result: Result<ResultValue, Error>,
    /// Held through GUI publication so protection can cancel queued results.
    pub work: Option<WorkPermit>,
}
pub struct Worker {
    send: Sender<(Job, WorkPermit)>,
    receive: Receiver<Completed>,
    cancel: Option<Arc<AtomicBool>>,
    performance: Handle,
}
impl Worker {
    pub fn new(performance: Handle) -> std::io::Result<Self> {
        let (send, receive) = bounded::<(Job, WorkPermit)>(1);
        let (output, result) = bounded(1);
        std::thread::Builder::new().name("omatainer-support-files".into()).spawn(move||{
            while let Ok((job,work))=receive.recv() {
                let cancel=work.cancel();
                let exported=matches!(&job,Job::Export(..));
                let result=(||{
                    check(&cancel)?;
                    match job {
                        Job::Preview(report,selection)=>{
                            let mut report=(*report).clone();
                            if !selection.routes {report.requested_audio=None;report.active_audio=None;}
                            if !selection.events {report.events.clear();report.dropped_events=0;report.dropped_observations=0;}
                            if !selection.performance {report.samples.clear();report.dropped_samples=0;}
                            if !selection.recovery {report.recovery.clear();}
                            Preview::new(report).map(ResultValue::Preview)
                        },
                        Job::Previous(root)=>storage::discover(&root,&cancel).map(ResultValue::Previous),
                        Job::Reopen(path)=>Preview::new(storage::reopen(&path,&cancel)?).map(ResultValue::Preview),
                        Job::Export(path,report)=>{
                            let _commit=work.commit().map_err(|_|Error::Cancelled)?;
                            storage::export(&path,&report,&cancel).map(ResultValue::Exported)
                        },
                        Job::Recovery(root,reference)=>{
                            let candidate=crate::recovery::lookup_exact(&root,reference.session,reference.epoch,reference.sequence,&cancel)
                                .map_err(|e|match e {crate::recovery::Error::Cancelled=>Error::Cancelled,_=>Error::Invalid("referenced recovery is corrupt, active or unavailable; original files were preserved")})?;
                            if candidate.as_ref().is_some_and(|candidate|candidate.metadata.revision!=reference.revision || candidate.metadata.view_revision!=reference.view_revision || candidate.metadata.captured_unix_ms!=reference.captured_unix_ms){return Err(Error::Invalid("recovery reference metadata differs; no project selected"));}
                            Ok(ResultValue::Recovery(candidate))
                        },
                    }
                })();
                // Once export commits, preserve its truthful result through a
                // later cancellation/protection request. Other results remain
                // guarded until the GUI observes them.
                let completed=Completed{result,work:if exported{None}else{Some(work)}};
                if output.send(completed).is_err(){return;}
            }
        })?;
        Ok(Self {
            send,
            receive: result,
            cancel: None,
            performance,
        })
    }
    pub fn request(&mut self, job: Job) -> Result<(), String> {
        if self.cancel.is_some() {
            return Err("Support work is already pending".into());
        }
        let work = self
            .performance
            .optional_work()
            .map_err(|e| e.to_string())?;
        let cancel = work.cancel();
        self.send
            .try_send((job, work))
            .map_err(|_| "Support worker unavailable")?;
        self.cancel = Some(cancel);
        Ok(())
    }
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn busy(&self) -> bool {
        self.cancel.is_some()
    }
    pub fn poll(&mut self) -> Option<Completed> {
        match self.receive.try_recv() {
            Ok(mut completed) => {
                self.cancel = None;
                if completed.work.as_ref().is_some_and(|work| work.cancelled()) {
                    completed.result = Err(Error::Cancelled);
                }
                Some(completed)
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) if self.cancel.take().is_some() => {
                Some(Completed {
                    result: Err(Error::Invalid("support worker closed")),
                    work: None,
                })
            }
            Err(_) => None,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
