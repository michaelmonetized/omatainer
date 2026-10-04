use super::*;
use std::sync::mpsc;

pub(crate) enum Operation {
    Search { query: String, offset: u32 },
    Preview(TrackId),
}
pub(crate) enum Reply {
    Page(Page),
    Preview(Preview),
}
pub(crate) struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Reply, Failure>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Job {
    /// Run one explicit request away from the UI and audio callback.
    /// Takes provider, operation, license and optional performance permit; returns a cancellable bounded job.
    pub fn start(
        provider: Arc<dyn MusicProvider>,
        operation: Operation,
        license: License,
        permit: Option<crate::engine::performance::WorkPermit>,
    ) -> Result<Self, Failure> {
        let cancel = permit
            .as_ref()
            .map_or_else(|| Arc::new(AtomicBool::new(false)), |p| p.cancel());
        let request = Request::new(license, cancel.clone());
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("music-provider".into())
            .spawn(move || {
                let mut request=request;
                let scheduled=permit.as_ref().map(|permit| {
                    let key=match &operation {Operation::Search{query,offset}=>crate::background::identity(&("search",query,offset)),Operation::Preview(id)=>crate::background::identity(&("preview",&id.item))}?;
                    permit.background(crate::background::Kind::Download,key,128*crate::background::MIB)
                }).transpose();
                let ticket=match scheduled {Ok(ticket)=>ticket,Err(_)=>{let _=sender.send(Err(Failure::Unavailable));return;}};
                let running=match ticket.as_ref().map(|ticket|ticket.enter(|| request.check().is_err() || permit.as_ref().is_some_and(|permit|permit.cancelled()))).transpose(){Ok(running)=>running,Err(_)=>{let _=sender.send(Err(Failure::Cancelled));return;}};
                if let Some(ticket)=&ticket {request.progress=ticket.reporter();}
                let result = match operation {
                    Operation::Search { query, offset } => {
                        provider.search(&query, offset, &request).map(Reply::Page)
                    }
                    Operation::Preview(id) => provider.preview(&id, &request).map(Reply::Preview),
                };
                let result = request.check().and(result);
                if let Some(ticket)=&ticket {ticket.progress(1,Some(1));}
                drop(running);
                let _ = sender.send(result);
            })
            .map_err(|_| Failure::Unavailable)?;
        Ok(Self {
            cancel,
            receiver,
            thread: Some(thread),
        })
    }
    /// Cancel without waiting for the network on a frame.
    /// Takes this job; invalidates worker publication and any resulting preview.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    /// Poll one worker result without blocking the native frame.
    /// Takes this job; returns a completed result once, or no result while it runs.
    pub fn poll(&mut self) -> Option<Result<Reply, Failure>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        let _ = self.thread.take().unwrap().join();
        if self.cancel.load(Ordering::Acquire) {
            return Some(Err(Failure::Cancelled));
        }
        Some(
            self.receiver
                .try_recv()
                .unwrap_or(Err(Failure::Unavailable)),
        )
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.cancel();
        }
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_worker_completion_still_blocks_publication() {
        let provider = Arc::new(crate::music_provider::tests::ContractProvider::default());
        let mut job = Job::start(
            provider,
            Operation::Search {
                query: String::new(),
                offset: 0,
            },
            License::NonCommercialAttribution,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !job.thread.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        job.cancel();
        assert!(matches!(job.poll(), Some(Err(Failure::Cancelled))));
    }
}
