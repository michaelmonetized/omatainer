//! MIDI preference validation and one immutable, generation-qualified receipt.
//! Bookkeeping never holds its small producer/manager lock during OS work.
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{
        AtomicBool,
        Ordering::{Acquire, Release},
    },
    Arc,
};

pub const MAX_SELECTED_INPUTS: usize = 64;
pub const MAX_INPUT_NAME_BYTES: usize = 1024;
pub(super) const MAX_AVAILABLE_INPUTS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "names",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum InputPolicy {
    #[default]
    All,
    Selected(Vec<String>),
    Disabled,
}
impl InputPolicy {
    pub fn validate(&self) -> Result<(), PolicyError> {
        if let Self::Selected(names) = self {
            if names.is_empty() || names.len() > MAX_SELECTED_INPUTS {
                return Err(PolicyError::Invalid(
                    "Select between 1 and 64 exact MIDI input names, or choose Disabled.",
                ));
            }
            for (i, name) in names.iter().enumerate() {
                if name.is_empty() || name.len() > MAX_INPUT_NAME_BYTES || name.contains('\0') {
                    return Err(PolicyError::Invalid(
                        "MIDI input names must contain 1–1024 bytes and no NUL.",
                    ));
                }
                if names[..i].contains(name) {
                    return Err(PolicyError::Invalid(
                        "Selected MIDI input names must be unique.",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn allows(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Selected(names) => names.iter().any(|selected| selected == name),
            Self::Disabled => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    Invalid(&'static str),
    Unavailable,
    GenerationExhausted,
    Performance(crate::engine::performance::Error),
}
impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Performance(error) => return std::fmt::Display::fmt(error, f),
            Self::Invalid(reason) => reason,
            Self::Unavailable => {
                "MIDI input manager is unavailable; restart to apply input preferences."
            }
            Self::GenerationExhausted => {
                "MIDI preference generation exhausted; restart to apply input preferences."
            }
        })
    }
}
impl std::error::Error for PolicyError {}

/// Applied means the manager has finished installing the input filter and
/// queued source-owned releases. Audio consumes releases through its usual FIFO.
/// Missing/failed devices do not silently expand Selected to All.
#[derive(Clone, Debug)]
pub struct PolicyStatus {
    pub requested: u64,
    pub applied: Option<u64>,
    pub requested_policy: Arc<InputPolicy>,
    pub applied_policy: Option<Arc<InputPolicy>>,
    pub available_inputs: Arc<[String]>,
    pub available_truncated: bool,
    pub missing_names: Arc<[String]>,
    pub error: Option<Arc<str>>,
}
impl PolicyStatus {
    pub fn pending(&self) -> bool {
        self.applied != Some(self.requested)
    }
}

pub(super) struct Request {
    pub generation: u64,
    pub policy: Arc<InputPolicy>,
    permit: Option<Arc<crate::engine::performance::ExclusivePermit>>,
}
pub(super) struct Control {
    current: Mutex<Arc<Request>>,
    published: ArcSwap<PolicyStatus>,
    stopped: AtomicBool,
    performance: crate::engine::performance::Handle,
}
impl Control {
    pub fn new(policy: InputPolicy) -> Self { Self::with_performance(policy, crate::engine::performance::Handle::default()) }
    pub fn with_performance(policy: InputPolicy, performance: crate::engine::performance::Handle) -> Self {
        let policy = Arc::new(policy);
        // Startup from Studio is still an in-progress device operation: mode
        // entry must not overtake a delayed initial connection. A profile that
        // starts protected deliberately initializes its first inputs under that
        // already-active protection; every callback still uses central admission.
        let permit = if performance.protected() { None }
            else { performance.project_change().ok().map(Arc::new) };
        Self {
            stopped: AtomicBool::new(false),
            performance,
            current: Mutex::new(Arc::new(Request {
                generation: 1,
                permit,
                policy: policy.clone(),
            })),
            published: ArcSwap::from_pointee(PolicyStatus {
                requested: 1,
                applied: None,
                requested_policy: policy,
                applied_policy: None,
                available_inputs: Arc::from([]),
                available_truncated: false,
                missing_names: Arc::from([]),
                error: None,
            }),
        }
    }
    pub fn request(&self, policy: InputPolicy) -> Result<u64, PolicyError> {
        policy.validate()?;
        let policy = Arc::new(policy);
        let mut current = self.current.lock();
        if self.stopped() {
            return Err(PolicyError::Unavailable);
        }
        if self.performance.protected() {
            return Err(PolicyError::Performance(crate::engine::performance::Error::Protected));
        }
        let permit = match &current.permit { Some(permit) => permit.clone(), None => Arc::new(self.performance.project_change().map_err(PolicyError::Performance)?) };
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(PolicyError::GenerationExhausted)?;
        let old = self.published.load();
        self.published.store(Arc::new(PolicyStatus {
            requested: generation,
            requested_policy: policy.clone(),
            error: None,
            ..(**old).clone()
        }));
        *current = Arc::new(Request { generation, policy, permit: Some(permit) });
        Ok(generation)
    }
    // Activation, a newer policy, and shutdown have one ordering point. The
    // production closure only enables the already-prepared sink with one atomic
    // store: no OS calls, joins, channel waits or status publication under lock.
    pub fn activate(&self, name: &str, enable: impl FnOnce()) -> bool {
        let current = self.current.lock();
        if self.stopped() || !current.policy.allows(name) {
            return false;
        }
        enable();
        true
    }
    pub fn stop(&self) {
        let _current = self.current.lock();
        self.stopped.store(true, Release);
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Acquire)
    }
    pub fn requested(&self) -> Arc<Request> {
        self.current.lock().clone()
    }
    pub fn is_current(&self, request: &Request) -> bool {
        self.published.load().requested == request.generation
    }
    pub fn status(&self) -> Arc<PolicyStatus> {
        self.published.load_full()
    }
    pub fn unavailable(&self) {
        let mut current = self.current.lock();
        *current = Arc::new(Request { generation: current.generation, policy: current.policy.clone(), permit: None });
        let old = self.published.load();
        self.published.store(Arc::new(PolicyStatus {
            error: Some(Arc::from(PolicyError::Unavailable.to_string())),
            ..(**old).clone()
        }));
    }
    pub fn complete(
        &self,
        request: &Request,
        available: Option<(Vec<String>, bool)>,
        missing: Vec<String>,
        error: Option<String>,
    ) -> bool {
        let available =
            available.map(|(names, truncated)| (Arc::<[String]>::from(names), truncated));
        let missing = Arc::from(missing);
        let error =
            error.map(|error| Arc::<str>::from(error.chars().take(1024).collect::<String>()));
        let mut current = self.current.lock();
        if current.generation != request.generation {
            return false;
        }
        let old = self.published.load();
        let (available_inputs, available_truncated) =
            available.unwrap_or_else(|| (old.available_inputs.clone(), old.available_truncated));
        self.published.store(Arc::new(PolicyStatus {
            requested: request.generation,
            applied: Some(request.generation),
            requested_policy: request.policy.clone(),
            applied_policy: Some(request.policy.clone()),
            available_inputs,
            available_truncated,
            missing_names: missing,
            error,
        }));
        *current = Arc::new(Request { generation: request.generation, policy: request.policy.clone(), permit: None });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn policy_and_shutdown_cannot_publish_between_activation_check_and_enable() {
        for shutdown in [false, true] {
            let control = Control::new(InputPolicy::All);
            let enabled = AtomicBool::new(false);
            let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
            let (release_tx, release_rx) = crossbeam_channel::bounded(1);
            let (attempt_tx, attempt_rx) = crossbeam_channel::bounded(1);
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            std::thread::scope(|scope| {
                let activate = scope.spawn(|| {
                    assert!(control.activate("Keyboard", || {
                        // Test-only pause at the precise check/enable boundary;
                        // production performs only the enabled atomic store.
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                        enabled.store(true, Release);
                    }));
                });
                entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                let change = scope.spawn(|| {
                    attempt_tx.send(()).unwrap();
                    if shutdown {
                        control.stop();
                    } else {
                        control.request(InputPolicy::Disabled).unwrap();
                    }
                    done_tx.send(enabled.load(Acquire)).unwrap();
                });
                attempt_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                let premature = done_rx.recv_timeout(Duration::from_millis(20));
                // Always release before asserting, so a failing fixture cannot
                // leave the scoped worker blocked during panic unwinding.
                release_tx.send(()).unwrap();
                assert!(matches!(
                    premature,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout)
                ));
                assert!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
                activate.join().unwrap();
                change.join().unwrap();
            });
            assert!(!control.activate("Keyboard", || panic!("superseded connection enabled")));
            if shutdown {
                assert_eq!(
                    control.request(InputPolicy::All),
                    Err(PolicyError::Unavailable)
                );
            } else {
                assert_eq!(
                    control.status().requested_policy.as_ref(),
                    &InputPolicy::Disabled
                );
            }
        }
    }
    #[test]
    fn performance_policy_guard_lives_until_manager_completion_and_last_request_release() {
        let performance = crate::engine::performance::Handle::default();
        let control = Control::with_performance(InputPolicy::All, performance.clone());
        control.request(InputPolicy::Disabled).unwrap();
        let request = control.requested();
        assert_eq!(performance.set_enabled(true), Err(crate::engine::performance::Error::Changing));
        // A coalesced new policy shares this manager's existing permit.
        control.request(InputPolicy::All).unwrap();
        let newer = control.requested();
        assert!(!control.complete(&request, None, Vec::new(), None));
        assert!(control.complete(&newer, None, Vec::new(), None));
        drop(newer);
        assert_eq!(performance.set_enabled(true), Err(crate::engine::performance::Error::Changing));
        drop(request);
        performance.set_enabled(true).unwrap();
        assert_eq!(control.request(InputPolicy::Disabled), Err(PolicyError::Performance(crate::engine::performance::Error::Protected)));
        assert_eq!(control.requested().policy.as_ref(), &InputPolicy::All);
    }

}
