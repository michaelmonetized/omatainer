//! One active decoder with a replaceable pending/result slot per deck.
//! Filesystem and decoder calls remain on the worker. Cancellation is observed
//! at safe decode boundaries; it cannot interrupt an OS read already in progress.
use super::decode::{decode_audio_with_cancel, DecodeFailure, DecodedAudio};
use super::media_source::FileFingerprint;
use super::DECKS;
use crate::sampler_bank::{assets, prepare};
use super::{performance, sampler, media_analysis};
pub(crate) use media_analysis::{Request as AnalysisRequest, Token as AnalysisToken, Failure as AnalysisFailure};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Debug)]
pub struct LoadToken {
    pub deck: u8,
    pub id: u64,
    current: Arc<AtomicU64>,
}
impl LoadToken {
    pub fn is_current(&self) -> bool {
        self.current.load(Ordering::Acquire) == self.id
    }
}
struct Request {
    token: LoadToken,
    path: PathBuf,
}
pub struct Completion {
    pub fingerprint: Option<FileFingerprint>,
    pub token: LoadToken,
    pub result: Result<DecodedAudio, DecodeFailure>,
}
#[derive(Clone, Debug)]
pub(crate) struct SamplerToken {
    pub id: u64,
    pub ack: sampler::Ack,
    current: Arc<AtomicU64>,
}
impl SamplerToken {
    pub fn is_current(&self) -> bool {
        self.current.load(Ordering::Acquire) == self.id && self.ack.state() != sampler::EditState::Rejected
    }
    pub fn cancel(&self) {
        // Applied or renderer-claimed requests retain their truthful outcome.
        self.ack.cancel();
        let _ = self.current.compare_exchange(self.id, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}
struct SamplerRequest {
    token: SamplerToken,
    request: prepare::Request,
    owner: assets::Owner,
    work: performance::WorkPermit,
}
pub(crate) struct SamplerCompletion {
    pub token: SamplerToken,
    pub result: Result<prepare::Prepared, String>,
    pub work: performance::WorkPermit,
}
struct AnalysisJob {
    token: AnalysisToken,
    request: AnalysisRequest,
    work: performance::WorkPermit,
}
pub(crate) struct AnalysisCompletion {
    pub token: AnalysisToken,
    pub result: Result<crate::track_analysis::Prepared, AnalysisFailure>,
    pub work: performance::WorkPermit,
}
enum Work { Deck(Request), Sampler(SamplerRequest), Analysis(AnalysisJob) }
struct State {
    pending: [Option<Request>; DECKS],
    ready: [Option<Completion>; DECKS],
    sampler_pending: Option<SamplerRequest>,
    sampler_ready: Option<SamplerCompletion>,
    sampler_token: Option<SamplerToken>,
    analysis_pending: Option<AnalysisJob>,
    analysis_ready: Option<AnalysisCompletion>,
    analysis_token: Option<AnalysisToken>,
    analysis_active: Option<AnalysisToken>,
    stop: bool,
    next_deck: usize,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
pub struct Loader {
    shared: Arc<Shared>,
    generations: [Arc<AtomicU64>; DECKS],
    sampler_generation: Arc<AtomicU64>,
    sampler_next: AtomicU64,
    analysis_generation: Arc<AtomicU64>,
    analysis_next: AtomicU64,
    performance: performance::Handle,
}
impl Loader {
    pub fn start_with_performance(performance: super::performance::Handle) -> io::Result<Self> {
        let decode_performance = performance.clone();
        Self::with_worker(move |path, token| super::decode::decode_audio_for_show(path, || !token.is_current(), &decode_performance), performance)
    }
    pub fn start() -> io::Result<Self> {
        Self::with_decoder(|path, token| decode_audio_with_cancel(path, || !token.is_current()))
    }

    pub(crate) fn with_decoder(
        decode: impl FnMut(&Path, &LoadToken) -> Result<DecodedAudio, DecodeFailure>
            + Send + 'static,
    ) -> io::Result<Self> {
        Self::with_worker(decode, performance::Handle::default())
    }
    fn with_worker(
        decode: impl FnMut(&Path, &LoadToken) -> Result<DecodedAudio, DecodeFailure>
            + Send + 'static,
        performance: performance::Handle,
    ) -> io::Result<Self> {
        Self::with_workers(decode, media_analysis::run, performance)
    }
    fn with_workers(
        mut decode: impl FnMut(&Path, &LoadToken) -> Result<DecodedAudio, DecodeFailure> + Send + 'static,
        mut analyze: impl FnMut(AnalysisRequest, &AnalysisToken, &performance::WorkPermit)
            -> Result<crate::track_analysis::Prepared, AnalysisFailure> + Send + 'static,
        performance: performance::Handle,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: std::array::from_fn(|_| None),
                ready: std::array::from_fn(|_| None),
                sampler_pending: None,
                sampler_ready: None,
                sampler_token: None,
                analysis_pending: None,
                analysis_ready: None,
                analysis_token: None,
                analysis_active: None,
                stop: false,
                next_deck: 0,
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("omatainer-decode".into())
            .spawn(move || loop {
                let request = {
                    let mut state = worker.state.lock().unwrap();
                    loop {
                        if state.stop {
                            return;
                        }
                        // Three typed lanes share one decoder. A bank import
                        // decodes at most 16 sources sequentially in its turn;
                        // neither a fake deck ID nor another worker is needed.
                        let selected = (0..=DECKS)
                            .map(|offset| (state.next_deck + offset) % (DECKS + 1))
                            .find(|&lane| if lane == DECKS { state.sampler_pending.is_some() }
                                else { state.pending[lane].is_some() });
                        if let Some(lane) = selected {
                            state.next_deck = (lane + 1) % (DECKS + 1);
                            break if lane == DECKS { Work::Sampler(state.sampler_pending.take().unwrap()) }
                                else { Work::Deck(state.pending[lane].take().unwrap()) };
                        }
                        // Optional work never consumes a turn while any explicit
                        // deck or sampler request is waiting.
                        if let Some(job) = state.analysis_pending.take() {
                            state.analysis_active = Some(job.token.clone());
                            break Work::Analysis(job);
                        }
                        state = worker.wake.wait(state).unwrap();
                    }
                };
                let request = match request {
                    Work::Deck(request) => request,
                    Work::Analysis(job) => {
                        let mut result = job.token.check(&job.work)
                            .and_then(|_| analyze(job.request, &job.token, &job.work));
                        // Keep user cancellation and protection observable even
                        // if a decoder reached completion before its last check.
                        if let Err(reason) = job.token.check(&job.work) { result = Err(reason); }
                        let old = {
                            let mut state = worker.state.lock().unwrap();
                            state.analysis_active = None;
                            if state.stop || !job.token.same_generation() { drop(state); continue; }
                            state.analysis_ready.replace(AnalysisCompletion {
                                token: job.token, result, work: job.work,
                            })
                        };
                        drop(old);
                        continue;
                    }
                    Work::Sampler(request) => {
                        if !request.token.is_current() { continue; }
                        let result = prepare::run(request.request, &request.owner,
                            || !request.token.is_current() || request.work.cancelled());
                        if !request.token.is_current() { continue; }
                        let old = {
                            let mut state = worker.state.lock().unwrap();
                            if state.stop || !request.token.is_current() { drop(state); continue; }
                            state.sampler_ready.replace(SamplerCompletion {
                                token: request.token, result, work: request.work,
                            })
                        };
                        drop(old);
                        continue;
                    }
                };
                if !request.token.is_current() {
                    continue;
                }
                let before = FileFingerprint::read(&request.path);
                let result = decode(&request.path, &request.token);
                let after = FileFingerprint::read(&request.path);
                let fingerprint = before.filter(|before| Some(*before) == after);
                if !request.token.is_current() {
                    continue;
                }
                let deck = request.token.deck as usize;
                let old = {
                    let mut state = worker.state.lock().unwrap();
                    if state.stop || !request.token.is_current() {
                        // Drop potentially large sample storage after unlocking.
                        drop(state);
                        continue;
                    }
                    state.ready[deck].replace(Completion {
                        fingerprint,
                        token: request.token,
                        result,
                    })
                };
                drop(old);
            })?;
        Ok(Self {
            shared,
            generations: std::array::from_fn(|_| Arc::new(AtomicU64::new(0))),
            sampler_generation: Arc::new(AtomicU64::new(0)),
            sampler_next: AtomicU64::new(0),
            analysis_generation: Arc::new(AtomicU64::new(0)),
            analysis_next: AtomicU64::new(0),
            performance,
        })
    }

    pub(crate) fn request_sampler(&self, request: prepare::Request, owner: assets::Owner) -> Result<SamplerToken, String> {
        let work = self.performance.optional_work().map_err(|e| e.to_string())?;
        let id = self.sampler_next.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |id| id.checked_add(1)).map_err(|_| "sampler request identity exhausted")? + 1;
        let token = SamplerToken { id, ack: sampler::Ack::new(), current: self.sampler_generation.clone() };
        let old = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stop { return Err("decoder is unavailable".into()); }
            if let Some(previous) = state.sampler_token.replace(token.clone()) { previous.cancel(); }
            preempt_analysis(&state);
            self.sampler_generation.store(id, Ordering::Release);
            let pending = state.sampler_pending.replace(SamplerRequest { token: token.clone(), request, owner, work });
            (pending, state.sampler_ready.take())
        };
        drop(old);
        self.shared.wake.notify_one();
        Ok(token)
    }
    pub(crate) fn take_sampler_ready(&self) -> Option<SamplerCompletion> {
        self.shared.state.try_lock().ok()?.sampler_ready.take()
    }
    pub(crate) fn invalidate_sampler(&self) {
        let old = {
            let mut state = self.shared.state.lock().unwrap();
            if let Some(token) = state.sampler_token.take() { token.cancel(); }
            (state.sampler_pending.take(), state.sampler_ready.take())
        };
        drop(old);
    }

    /// One replaceable low-priority job and result; no batch is stored here.
    pub(crate) fn request_analysis(&self, request: AnalysisRequest) -> Result<AnalysisToken, String> {
        request.validate().map_err(|e| e.to_string())?;
        let work = self.performance.optional_work().map_err(|e| e.to_string())?;
        let (token, old) = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stop { return Err("decoder is unavailable".into()); }
            let id = self.analysis_next.fetch_update(Ordering::AcqRel, Ordering::Acquire,
                |id| id.checked_add(1)).map_err(|_| "analysis request identity exhausted")? + 1;
            let token = AnalysisToken::new(id, self.analysis_generation.clone());
            // Cancellation precedes generation replacement, so a concurrent
            // metadata publication either claims first or observes cancellation.
            if let Some(previous) = state.analysis_token.replace(token.clone()) { previous.cancel(); }
            self.analysis_generation.store(id, Ordering::Release);
            let pending = state.analysis_pending.replace(AnalysisJob { token: token.clone(), request, work });
            (token, (pending, state.analysis_ready.take()))
        };
        drop(old);
        self.shared.wake.notify_one();
        Ok(token)
    }
    pub(crate) fn take_analysis_ready(&self) -> Option<AnalysisCompletion> {
        self.shared.state.try_lock().ok()?.analysis_ready.take()
    }
    pub(crate) fn invalidate_analysis(&self) {
        let old = {
            let mut state = self.shared.state.lock().unwrap();
            if let Some(token) = state.analysis_token.take() { token.cancel(); }
            (state.analysis_pending.take(), state.analysis_ready.take())
        };
        drop(old);
    }

    /// Invalidates queued, active, completed, and already-submitted completions.
    pub fn invalidate(&self, deck: u8) -> Result<(), String> {
        let generation = self.generations.get(deck as usize).ok_or("invalid deck")?;
        let discarded = {
            let mut state = self.shared.state.lock().unwrap();
            generation
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
                .map_err(|_| "media request identity exhausted")?;
            (
                state.pending[deck as usize].take(),
                state.ready[deck as usize].take(),
            )
        };
        drop(discarded);
        Ok(())
    }

    pub fn request(&self, deck: u8, path: PathBuf) -> Result<LoadToken, String> {
        let current = self
            .generations
            .get(deck as usize)
            .ok_or("invalid deck")?
            .clone();
        let (token, old) = {
            let mut state = self.shared.state.lock().unwrap();
            let previous = current
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
                .map_err(|_| "media request identity exhausted")?;
            preempt_analysis(&state);
            let token = LoadToken {
                deck,
                id: previous + 1,
                current,
            };
            let pending = state.pending[deck as usize].replace(Request {
                token: token.clone(),
                path,
            });
            let ready = state.ready[deck as usize].take();
            (token, (pending, ready))
        };
        drop(old);
        self.shared.wake.notify_one();
        Ok(token)
    }

    /// At most one result per deck; polling never waits on decoding or I/O.
    pub fn take_ready(&self) -> [Option<Completion>; DECKS] {
        let Ok(mut state) = self.shared.state.try_lock() else {
            return std::array::from_fn(|_| None);
        };
        std::array::from_fn(|deck| state.ready[deck].take())
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        for generation in &self.generations {
            generation.store(0, Ordering::Release);
        }
        let discarded = {
            let mut state = self.shared.state.lock().unwrap();
            state.stop = true;
            if let Some(token) = state.sampler_token.take() { token.cancel(); }
            if let Some(token) = state.analysis_token.take() { token.cancel(); }
            (state.sampler_pending.take(), state.sampler_ready.take(),
                state.analysis_pending.take(), state.analysis_ready.take(),
                std::mem::replace(&mut state.pending, std::array::from_fn(|_| None)),
                std::mem::replace(&mut state.ready, std::array::from_fn(|_| None)),
            )
        };
        self.shared.wake.notify_one();
        drop(discarded);
        // A blocked OS read cannot be cancelled safely. The worker owns its
        // remaining resources and exits at the next safe boundary; UI teardown
        // does not wait for storage. No new worker is spawned per request.
    }
}

fn preempt_analysis(state: &State) {
    if let Some(token) = &state.analysis_active { token.preempt(); }
    if let Some(job) = &state.analysis_pending { job.token.preempt(); }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod analysis_tests;
