//! Fixed typed observations enter a bounded worker lane. Only this worker builds
//! JSON or persists reports. It never receives media, names, paths or raw errors.
use super::*;
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{atomic::AtomicU64, Arc};
use std::time::{Duration, Instant};

const QUEUE: usize = 256;
#[derive(Clone)]
pub struct Port {
    send: Sender<Observation>,
    dropped: Arc<AtomicU64>,
    start: Instant,
}
enum Observation {
    Event(Event),
    Sample(Sample),
    Routes(Option<Route>, Option<Route>),
    Recovery(RecoveryRef),
}
impl Port {
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    pub fn event(&self, code: Code, failure: Option<FailureClass>) {
        self.send(Observation::Event(Event {
            elapsed_ms: self.elapsed_ms(),
            code,
            failure,
            repeats: 0,
        }));
    }
    pub fn sample(&self, mut sample: Sample) {
        sample.elapsed_ms = self.elapsed_ms();
        self.send(Observation::Sample(sample));
    }
    pub fn routes(&self, requested: Option<Route>, active: Option<Route>) {
        self.send(Observation::Routes(requested, active));
    }
    pub fn recovery(&self, reference: RecoveryRef) {
        self.send(Observation::Recovery(reference));
    }
    fn send(&self, observation: Observation) {
        if self.send.try_send(observation).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[derive(Clone)]
pub struct View {
    pub report: Arc<Report>,
    pub durable_unix_ms: Option<u64>,
    pub storage_failure: Option<FailureClass>,
    pub committed_warning: bool,
    /// Bounded queued observations rejected before the worker could collect.
    pub dropped_observations: u64,
}
#[derive(Clone)]
pub struct Client {
    pub port: Port,
    view: Arc<ArcSwap<View>>,
}
impl Client {
    pub fn view(&self) -> Arc<View> {
        self.view.load_full()
    }
}
pub struct Session {
    pub port: Port,
    view: Arc<ArcSwap<View>>,
    quit: Sender<(Exit, Sender<bool>)>,
    stop: Arc<AtomicBool>,
    run: Option<Arc<storage::Run>>,
}
impl Session {
    /// Startup only, before opening devices. A storage failure still produces a
    /// usable in-memory diagnostic session; its missing durability is explicit.
    pub fn start(root: &Path, safe_mode: bool) -> Result<Self, Error> {
        let (run, failure) = match storage::Run::begin(root, safe_mode) {
            Ok(run) => (Some(Arc::new(run)), None),
            Err(error) => (None, Some(error.class())),
        };
        let now = storage::unix_ms();
        let mut report = run.as_ref().map_or_else(
            || {
                let id = Id::digest(
                    format!("{}-{}-{:?}", std::process::id(), now, Instant::now()).as_bytes(),
                );
                Report::new(id, safe_mode, now)
            },
            |run| run.report(),
        );
        report.record(Event {
            elapsed_ms: 0,
            code: if safe_mode {
                Code::SafeModeStartup
            } else {
                Code::Startup
            },
            failure: None,
            repeats: 0,
        });
        if let Some(failure) = failure {
            report.record(Event {
                elapsed_ms: 0,
                code: Code::SupportStorageFailed,
                failure: Some(failure),
                repeats: 0,
            });
        }
        let view = Arc::new(ArcSwap::from_pointee(View {
            report: Arc::new(report.clone()),
            durable_unix_ms: None,
            storage_failure: failure,
            committed_warning: false,
            dropped_observations: 0,
        }));
        let (send, receive) = bounded(QUEUE);
        let (quit, shutdown) = bounded(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_run = run.clone();
        let worker_view = view.clone();
        let worker_stop = stop.clone();
        let worker_dropped = dropped.clone();
        std::thread::Builder::new()
            .name("omatainer-support".into())
            .spawn(move || {
                collect(
                    worker_run,
                    report,
                    worker_view,
                    receive,
                    shutdown,
                    worker_stop,
                    worker_dropped,
                );
            })
            .map_err(|e| Error::io("start collector", e))?;
        Ok(Self {
            port: Port {
                send,
                dropped,
                start: Instant::now(),
            },
            view,
            quit,
            stop,
            run,
        })
    }
    pub fn client(&self) -> Client {
        Client {
            port: self.port.clone(),
            view: self.view.clone(),
        }
    }
    pub fn view(&self) -> Arc<View> {
        self.view.load_full()
    }
    pub fn install_panic_hook(&self) {
        if let Some(run) = &self.run {
            run.install_panic_hook();
        }
    }
    /// Main-thread shutdown after the GUI/device owners close. File I/O still
    /// happens on the worker. A timeout leaves the marker unclean; it is never
    /// reported as a confirmed clean exit. No caller waits on the audio thread.
    pub fn finish(&self, exit: Exit, timeout: Duration) -> bool {
        let (send, receive) = bounded(1);
        if self.quit.try_send((exit, send)).is_err() {
            return false;
        }
        receive.recv_timeout(timeout).unwrap_or(false)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn apply(report: &mut Report, observation: Observation) {
    match observation {
        Observation::Event(event) => report.record(event),
        Observation::Sample(sample) => report.sample(sample),
        Observation::Routes(requested, active) => {
            report.requested_audio = requested;
            report.active_audio = active;
        }
        Observation::Recovery(reference) => {
            if report.recovery.last().is_none_or(|old| old != &reference) {
                if report.recovery.len() == MAX_RECOVERIES {
                    report.recovery.remove(0);
                }
                report.recovery.push(reference);
            }
        }
    }
}
fn collect(
    run: Option<Arc<storage::Run>>,
    mut report: Report,
    view: Arc<ArcSwap<View>>,
    receive: Receiver<Observation>,
    shutdown: Receiver<(Exit, Sender<bool>)>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
) {
    let mut next_persist = Instant::now();
    let mut next_publish = Instant::now();
    let mut dirty = true;
    let mut last_durable = None;
    let mut failure = view.load().storage_failure;
    let mut warning = false;
    loop {
        let finish = shutdown.try_recv().ok();
        for observation in receive.try_iter().take(QUEUE) {
            apply(&mut report, observation);
            dirty = true;
        }
        if let Some((exit, _)) = &finish {
            report.exit = *exit;
            if *exit == Exit::Clean {
                report.record(Event {
                    elapsed_ms: report
                        .samples
                        .last()
                        .map_or(0, |s| s.elapsed_ms)
                        .max(report.events.last().map_or(0, |s| s.elapsed_ms)),
                    code: Code::CleanExit,
                    failure: None,
                    repeats: 0,
                });
            }
            dirty = true;
        }
        if dirty && (Instant::now() >= next_persist || finish.is_some()) {
            report.collected_unix_ms = storage::unix_ms();
            report.dropped_observations = dropped.load(Ordering::Relaxed);
            if let Some(run) = &run {
                match run.persist(&report, &AtomicBool::new(false)) {
                    Ok(storage::Published::Durable) => {
                        last_durable = Some(report.collected_unix_ms);
                        failure = None;
                        warning = false;
                    }
                    Ok(storage::Published::CommittedWarning) => {
                        failure = None;
                        warning = true;
                    }
                    Err(error) => {
                        failure = Some(error.class());
                        warning = false;
                    }
                }
            }
            next_persist = Instant::now() + Duration::from_secs(5);
            dirty = false;
        }
        if Instant::now() >= next_publish || finish.is_some() {
            view.store(Arc::new(View {
                report: Arc::new(report.clone()),
                durable_unix_ms: last_durable,
                storage_failure: failure,
                committed_warning: warning,
                dropped_observations: dropped.load(Ordering::Relaxed),
            }));
            next_publish = Instant::now() + Duration::from_millis(250);
        }
        if let Some((exit, receipt)) = finish {
            let clean = run.as_ref().is_some_and(|run| match exit {
                Exit::Clean => run.mark_clean().is_ok(),
                Exit::StartupFailed => run.mark_startup_failed().is_ok(),
                _ => false,
            });
            let _ = receipt.send(clean);
            break;
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        match receive.recv_timeout(Duration::from_millis(50)) {
            Ok(observation) => {
                apply(&mut report, observation);
                dirty = true;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
}
