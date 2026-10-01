//! Bounded producer-to-GUI theme requests; no audio receiver owns this port.
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering::*},
    Arc,
};
use std::time::{Duration, Instant};

pub(crate) const CAPACITY: usize = 8;
pub(crate) const DEADLINE: Duration = Duration::from_secs(3);
const WAITING: u8 = 0;
const APPLYING: u8 = 1;
const COMPLETE: u8 = 2;
const CANCELLED: u8 = 3;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Applied {
    pub generation: u64,
    pub follow_theme: bool,
    pub font: String,
    pub font_size: f32,
    pub scale: f32,
}
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub message: String,
}
impl Failure {
    pub fn new(code: &'static str, message: impl AsRef<str>) -> Self {
        Self {
            code,
            message: message.as_ref().chars().take(1024).collect(),
        }
    }
}
pub(crate) type Outcome = Result<Applied, Failure>;
struct State {
    attached: bool,
    closed: bool,
}
struct Shared {
    state: Mutex<State>,
    send: Sender<Request>,
    receive: Receiver<Request>,
    next: AtomicU64,
}
#[derive(Clone)]
pub(crate) struct Port(Arc<Shared>);
impl Default for Port {
    fn default() -> Self {
        let (send, receive) = bounded(CAPACITY);
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                attached: false,
                closed: false,
            }),
            send,
            receive,
            next: AtomicU64::new(1),
        }))
    }
}
impl Port {
    pub fn attach(&self) -> Option<Endpoint> {
        let mut state = self.0.state.lock();
        if state.attached || state.closed {
            return None;
        }
        state.attached = true;
        Some(Endpoint(self.clone()))
    }
    pub fn request(&self, budget: Duration) -> Result<Waiter, Failure> {
        let (send, receive) = bounded(1);
        let status = Arc::new(AtomicU8::new(WAITING));
        let deadline = Instant::now() + budget;
        let state = self.0.state.lock();
        if !state.attached || state.closed {
            return Err(Failure::new(
                "theme_unavailable",
                "Theme GUI is unavailable",
            ));
        }
        let generation = self
            .0
            .next
            .fetch_update(Relaxed, Relaxed, |n| n.checked_add(1))
            .map_err(|_| {
                Failure::new(
                    "theme_unavailable",
                    "Theme request generation exhausted; restart",
                )
            })?;
        self.0
            .send
            .try_send(Request {
                generation,
                deadline,
                status: status.clone(),
                send,
            })
            .map_err(|_| {
                Failure::new(
                    "theme_busy",
                    "Theme request queue is full; retry after a request completes",
                )
            })?;
        Ok(Waiter {
            receive,
            status,
            deadline,
        })
    }
}
pub(crate) struct Endpoint(Port);
impl Endpoint {
    pub fn next(&self) -> Option<Request> {
        self.0 .0.receive.try_recv().ok()
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let mut state = self.0 .0.state.lock();
        state.closed = true;
        // At most CAPACITY small requests. No worker join or filesystem work.
        while self.0 .0.receive.try_recv().is_ok() {}
    }
}
pub(crate) struct Request {
    pub generation: u64,
    deadline: Instant,
    status: Arc<AtomicU8>,
    send: Sender<Outcome>,
}
impl Request {
    pub fn cancelled(&self) -> bool {
        if Instant::now() >= self.deadline {
            let _ = self
                .status
                .compare_exchange(WAITING, CANCELLED, AcqRel, Acquire);
        }
        self.status.load(Acquire) == CANCELLED
    }
    pub fn begin_apply(&self) -> bool {
        !self.cancelled()
            && self
                .status
                .compare_exchange(WAITING, APPLYING, AcqRel, Acquire)
                .is_ok()
    }
    pub fn finish(&self, result: Outcome) {
        let mut previous = self.status.load(Acquire);
        while matches!(previous, WAITING | APPLYING) {
            match self
                .status
                .compare_exchange_weak(previous, COMPLETE, AcqRel, Acquire)
            {
                Ok(_) => {
                    let _ = self.send.try_send(result);
                    return;
                }
                Err(actual) => previous = actual,
            }
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.finish(Err(Failure::new(
            "theme_unavailable",
            if self.status.load(Acquire) == APPLYING {
                "GUI closed while applying theme; completion was not acknowledged"
            } else {
                "GUI closed or abandoned the theme request before application"
            },
        )));
    }
}
pub(crate) struct Waiter {
    receive: Receiver<Outcome>,
    status: Arc<AtomicU8>,
    deadline: Instant,
}
impl Waiter {
    pub fn wait(&self, stopped: impl Fn() -> bool) -> Outcome {
        loop {
            if let Ok(result) = self.receive.try_recv() {
                return result;
            }
            if stopped() {
                return Err(self.cancel(
                    "theme_cancelled",
                    "Theme request cancelled by server shutdown",
                ));
            }
            let now = Instant::now();
            if now >= self.deadline {
                return Err(self.cancel(
                    "theme_timeout",
                    "Theme reload timed out before a completed application was acknowledged",
                ));
            }
            match self
                .receive
                .recv_timeout((self.deadline - now).min(Duration::from_millis(20)))
            {
                Ok(result) => return result,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err(Failure::new("theme_unavailable", "Theme GUI disconnected"))
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn cancel(&self, code: &'static str, message: &str) -> Failure {
        let previous = self
            .status
            .compare_exchange(WAITING, CANCELLED, AcqRel, Acquire);
        Failure::new(
            code,
            if matches!(previous, Err(APPLYING | COMPLETE)) {
                format!("{message}; application may have begun, so its outcome is unknown")
            } else {
                format!("{message}; application was cancelled before it began")
            },
        )
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        let _ = self
            .status
            .compare_exchange(WAITING, CANCELLED, AcqRel, Acquire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_admission_detach_and_fifo_never_claim_an_unapplied_success() {
        let port = Port::default();
        assert!(matches!(port.request(DEADLINE), Err(e) if e.code == "theme_unavailable"));
        let endpoint = port.attach().unwrap();
        assert!(port.attach().is_none());
        let requests: Vec<_> = (0..CAPACITY)
            .map(|_| port.request(DEADLINE).unwrap())
            .collect();
        assert!(matches!(port.request(DEADLINE), Err(e) if e.code == "theme_busy"));
        let first = endpoint.next().unwrap();
        assert_eq!(first.generation, 1);
        drop(first);
        assert_eq!(
            requests[0].wait(|| false).unwrap_err().code,
            "theme_unavailable"
        );
        drop(endpoint);
        for request in requests.iter().skip(1) {
            assert_eq!(
                request.wait(|| false).unwrap_err().code,
                "theme_unavailable"
            );
        }
        assert!(matches!(port.request(DEADLINE), Err(e) if e.code == "theme_unavailable"));
    }

    #[test]
    fn cancellation_before_apply_is_final_and_after_apply_is_explicitly_unknown() {
        let port = Port::default();
        let endpoint = port.attach().unwrap();
        let waiter = port.request(DEADLINE).unwrap();
        let request = endpoint.next().unwrap();
        let error = waiter.wait(|| true).unwrap_err();
        assert_eq!(error.code, "theme_cancelled");
        assert!(error.message.contains("before it began"));
        assert!(!request.begin_apply());
        let waiter = port.request(DEADLINE).unwrap();
        let request = endpoint.next().unwrap();
        assert!(request.begin_apply());
        assert!(!request.begin_apply());
        let error = waiter.wait(|| true).unwrap_err();
        assert!(error.message.contains("outcome is unknown"));
        let waiter = port.request(Duration::ZERO).unwrap();
        let request = endpoint.next().unwrap();
        assert_eq!(waiter.wait(|| false).unwrap_err().code, "theme_timeout");
        assert!(!request.begin_apply());
    }
}
