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
                let _permit = permit;
                let result = match operation {
                    Operation::Search { query, offset } => {
                        provider.search(&query, offset, &request).map(Reply::Page)
                    }
                    Operation::Preview(id) => provider.preview(&id, &request).map(Reply::Preview),
                };
                let result = request.check().and(result);
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
