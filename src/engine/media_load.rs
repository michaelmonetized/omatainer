//! One active decoder with a replaceable pending/result slot per deck.
//! Filesystem and decoder calls remain on the worker. Cancellation is observed
//! at safe decode boundaries; it cannot interrupt an OS read already in progress.
use super::decode::{DecodeFailure, DecodedAudio};
use super::media_source::{FileFingerprint,LibSource};
use super::DECKS;
use crate::sampler_bank::{assets, prepare};
use super::{performance, sampler, media_analysis, media_health};
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
    source: LibSource,
    expected: Option<FileFingerprint>,
}
pub struct Completion {
    pub tags: Option<Result<crate::media_tags::Observation, String>>,
    pub fingerprint: Option<FileFingerprint>,
    pub content_hash: Option<[u8;32]>,
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
struct HealthJob { token: AnalysisToken, request: media_health::Request, work: performance::WorkPermit }
pub(crate) struct HealthCompletion { pub token: AnalysisToken, pub result: Result<media_health::Observation, AnalysisFailure> }
enum Work { Deck(Request), Sampler(SamplerRequest), Analysis(AnalysisJob), Health(HealthJob) }
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
    health_pending: Option<HealthJob>,
    health_ready: Option<HealthCompletion>,
    health_token: Option<AnalysisToken>,
    health_active: Option<AnalysisToken>,
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
    health_generation: Arc<AtomicU64>,
    health_next: AtomicU64,
    performance: performance::Handle,
}
impl Loader {
    pub fn start_with_performance(performance: super::performance::Handle) -> io::Result<Self> {
        let decode_performance = performance.clone();
        Self::with_backend(move |path, token, file| super::decode::decode_deck_file(path, file.expect("verified descriptor"), || !token.is_current(), &decode_performance),
            media_analysis::run, performance, true, crate::media_location::Snapshot::discover)
    }
    pub fn start() -> io::Result<Self> {
        Self::start_with_performance(performance::Handle::default())
    }

    #[cfg(test)]
    pub(crate) fn with_inventory(inventory:impl FnMut()->Result<crate::media_location::Snapshot,crate::media_location::Failure>+Send+'static)->io::Result<Self> {
        let performance=performance::Handle::default();let foreground=performance.clone();
        Self::with_backend(move |path,token,file|super::decode::decode_deck_file(path,file.expect("verified descriptor"),||!token.is_current(),&foreground),media_analysis::run,performance,true,inventory)
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
    #[cfg(test)]
    pub(crate) fn with_analysis_hook(
        performance: performance::Handle,
        mut before: impl FnMut(&AnalysisToken) + Send + 'static,
    ) -> io::Result<Self> {
        let foreground = performance.clone();
        Self::with_backend(
            move |path, token, file| super::decode::decode_deck_file(path, file.expect("verified descriptor"), || !token.is_current(), &foreground),
            move |request, token, work| { before(token); media_analysis::run(request, token, work) },
            performance, true, crate::media_location::Snapshot::discover,
        )
    }
    fn with_workers(
        mut decode: impl FnMut(&Path, &LoadToken) -> Result<DecodedAudio, DecodeFailure> + Send + 'static,
        analyze: impl FnMut(AnalysisRequest, &AnalysisToken, &performance::WorkPermit)
            -> Result<crate::track_analysis::Prepared, AnalysisFailure> + Send + 'static,
        performance: performance::Handle,
    ) -> io::Result<Self> {
        // Path decoders are explicit synthetic test adapters. Production
        // constructors always supply the same opened descriptor to decoding.
        Self::with_backend(move |path,token,_file|decode(path,token),analyze,performance,false,crate::media_location::Snapshot::discover)
    }
    fn with_backend(
        mut decode: impl FnMut(&Path,&LoadToken,Option<std::fs::File>)->Result<DecodedAudio,DecodeFailure> + Send + 'static,
        mut analyze: impl FnMut(AnalysisRequest,&AnalysisToken,&performance::WorkPermit)->Result<crate::track_analysis::Prepared,AnalysisFailure> + Send + 'static,
        performance:performance::Handle,
        verified:bool,
        mut inventory:impl FnMut()->Result<crate::media_location::Snapshot,crate::media_location::Failure> +Send+'static,
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
                health_pending: None,
                health_ready: None,
                health_token: None,
                health_active: None,
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
                        if let Some(job) = state.health_pending.take() {
                            state.health_active = Some(job.token.clone());
                            break Work::Health(job);
                        }
                        state = worker.wake.wait(state).unwrap();
                    }
                };
                let request = match request {
                    Work::Deck(request) => request,
                    Work::Health(job) => {
                        let result = media_health::run(job.request, &job.token, &job.work);
                        let old = {
                            let mut state = worker.state.lock().unwrap();
                            state.health_active = None;
                            if state.stop || !job.token.same_generation() { drop(state); continue; }
                            state.health_ready.replace(HealthCompletion { token:job.token, result })
                        };
                        drop(old);
                        continue;
                    }
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
                let (fingerprint,content_hash,result) = if verified || request.expected.is_some() || matches!(request.source,LibSource::Removable {..}) {
                    guarded_decode(&request.source,&request.token,request.expected,&mut decode,&mut inventory)
                } else if let LibSource::File(path)=&request.source {
                    let before=FileFingerprint::read(path);let result=decode(path,&request.token,None);
                    let after=FileFingerprint::read(path);(before.filter(|before|Some(*before)==after),None,result)
                } else {(None,None,Err(source_failure("Unsupported media namespace")))};
                struct TagCancellation<'a>(&'a LoadToken);
                impl crate::media_tags::Cancellation for TagCancellation<'_> {
                    fn cancelled(&self) -> bool { !self.0.is_current() }
                }
                let tags = if verified && result.is_ok() {
                    fingerprint.map(|fingerprint| crate::media_location::Location::resolve(&request.source)
                        .map_err(|error| error.to_string())
                        .and_then(|location| crate::media_tags::inspect_cancellable(&location, fingerprint, &TagCancellation(&request.token))))
                } else { None };
                if !request.token.is_current() { continue; }
                let deck = request.token.deck as usize;
                let old = {
                    let mut state = worker.state.lock().unwrap();
                    if state.stop || !request.token.is_current() {
                        // Drop potentially large sample storage after unlocking.
                        drop(state);
                        continue;
                    }
                    state.ready[deck].replace(Completion {
                        tags,
                        fingerprint,
                        content_hash,
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
            health_generation: Arc::new(AtomicU64::new(0)),
            health_next: AtomicU64::new(0),
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
            preempt_optional_decode(&state);
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

    /// Queue one read-only media check on the shared decoder worker.
    /// Takes a captured source/version; returns a cancellable token or refuses unavailable, protected or occupied optional work.
    pub(crate) fn request_health(&self, request: media_health::Request) -> Result<AnalysisToken, String> {
        crate::library::validate_source(&request.source)?;
        let work = self.performance.optional_work().map_err(|e| e.to_string())?;
        let (token, old) = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stop { return Err("decoder is unavailable".into()); }
            if state.analysis_pending.is_some() || state.analysis_active.is_some() { return Err("Finish or cancel the active analysis request first".into()); }
            let id = self.health_next.fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
                .map_err(|_| "media validation identity exhausted")? + 1;
            let token = AnalysisToken::new(id, self.health_generation.clone());
            if let Some(previous) = state.health_token.replace(token.clone()) { previous.cancel(); }
            self.health_generation.store(id, Ordering::Release);
            let pending = state.health_pending.replace(HealthJob { token:token.clone(), request, work });
            (token, (pending, state.health_ready.take()))
        };
        drop(old);
        self.shared.wake.notify_one();
        Ok(token)
    }
    /// Poll the bounded media-check result without waiting on decoding.
    /// Takes the loader; returns its latest completed check if the worker lock is available.
    pub(crate) fn take_health_ready(&self) -> Option<HealthCompletion> {
        self.shared.state.try_lock().ok()?.health_ready.take()
    }
    /// One replaceable low-priority job and result; no batch is stored here.
    pub(crate) fn request_analysis(&self, request: AnalysisRequest) -> Result<AnalysisToken, String> {
        request.validate().map_err(|e| e.to_string())?;
        let work = self.performance.optional_work().map_err(|e| e.to_string())?;
        let (token, old) = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stop { return Err("decoder is unavailable".into()); }
            if state.health_pending.is_some() || state.health_active.is_some() { return Err("Finish or cancel the active media validation first".into()); }
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
        self.enqueue(deck,LibSource::File(path),None)
    }
    pub fn request_source(&self,deck:u8,source:LibSource)->Result<LoadToken,String> {
        self.request_source_expected(deck,source,None)
    }
    /// Decode the reviewed source version.
    /// Takes the deck, immutable source and optional captured fingerprint; returns admission or an explicit error.
    pub fn request_source_expected(&self,deck:u8,source:LibSource,expected:Option<FileFingerprint>)->Result<LoadToken,String> {
        crate::library::validate_source(&source)?;
        crate::media_location::validate_root_source(&source).map_err(|e|e.to_string())?;
        self.enqueue(deck,source,expected)
    }
    fn enqueue(&self,deck:u8,source:LibSource,expected:Option<FileFingerprint>)->Result<LoadToken,String> {
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
            preempt_optional_decode(&state);
            let token = LoadToken {
                deck,
                id: previous + 1,
                current,
            };
            let pending = state.pending[deck as usize].replace(Request {
                token: token.clone(),
                source,
                expected,
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
            if let Some(token) = state.health_token.take() { token.cancel(); }
            if let Some(token) = state.sampler_token.take() { token.cancel(); }
            if let Some(token) = state.analysis_token.take() { token.cancel(); }
            (state.sampler_pending.take(), state.sampler_ready.take(),
                state.analysis_pending.take(), state.analysis_ready.take(),
                state.health_pending.take(), state.health_ready.take(),
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

fn source_failure(detail:impl ToString)->DecodeFailure {
    DecodeFailure {kind:super::decode::DecodeFailureKind::Io,stage:super::decode::DecodeStage::Open,
        diagnostics:Default::default(),detail:detail.to_string().chars().take(256).collect()}
}
fn guarded_decode(
    source:&LibSource,token:&LoadToken,expected:Option<FileFingerprint>,
    decode:&mut impl FnMut(&Path,&LoadToken,Option<std::fs::File>)->Result<DecodedAudio,DecodeFailure>,
    inventory:&mut impl FnMut()->Result<crate::media_location::Snapshot,crate::media_location::Failure>,
)->(Option<FileFingerprint>,Option<[u8;32]>,Result<DecodedAudio,DecodeFailure>) {
    use crate::media_location::{Location,Failure};
    use std::{os::unix::fs::OpenOptionsExt,io::{Read,Seek}};
    use sha2::{Digest,Sha256};
    let mut operation=|| -> Result<(FileFingerprint,Option<[u8;32]>,DecodedAudio),DecodeFailure> {
        let location=if matches!(source,LibSource::Removable {..}) {
            let snapshot=inventory().map_err(source_failure)?;let location=snapshot.resolve(source).map_err(source_failure)?;
            snapshot.inspect(&location).map_err(source_failure)?;location
        } else {Location::resolve(source).map_err(source_failure)?};
        let mut file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&location.path).map_err(source_failure)?;
        let meta=file.metadata().map_err(source_failure)?;let fingerprint=FileFingerprint::from_metadata(&meta);
        if expected.is_some_and(|expected|expected!=fingerprint) || !meta.is_file() || FileFingerprint::read(&location.path)!=Some(fingerprint) {return Err(source_failure(Failure::Changed));}
        if !token.is_current() {return Err(source_failure("Load superseded before decoding"));}
        let content_hash=if matches!(source,LibSource::Removable {..}) {
            if meta.len()>8*1024*1024*1024 {return Err(source_failure("removable source exceeds 8 GiB verification limit"));}
            let mut hash=Sha256::new();let mut buffer=[0u8;64*1024];let mut total=0u64;
            loop {
                if !token.is_current() {return Err(source_failure("Load superseded during content verification"));}
                let n=file.read(&mut buffer).map_err(source_failure)?;if n==0 {break;}
                total+=n as u64;if total>meta.len() {return Err(source_failure(Failure::Changed));}hash.update(&buffer[..n]);
            }
            if total!=meta.len() {return Err(source_failure(Failure::Changed));}
            file.rewind().map_err(source_failure)?;Some(hash.finalize().into())
        } else {None};
        let result=decode(&location.path,token,Some(file.try_clone().map_err(source_failure)?));
        if matches!(source,LibSource::Removable {..}) {location.recheck_with(&inventory().map_err(source_failure)?).map_err(source_failure)?;}
        if FileFingerprint::from_metadata(&file.metadata().map_err(source_failure)?)!=fingerprint || FileFingerprint::read(&location.path)!=Some(fingerprint) {return Err(source_failure(Failure::Changed));}
        result.map(|audio|(fingerprint,content_hash,audio))
    };
    match operation() {Ok((fingerprint,hash,audio))=>(Some(fingerprint),hash,Ok(audio)),Err(error)=>(None,None,Err(error))}
}
fn preempt_optional_decode(state: &State) {
    if let Some(token) = &state.analysis_active { token.preempt(); }
    if let Some(job) = &state.analysis_pending { job.token.preempt(); }
    if let Some(token) = &state.health_active { token.preempt(); }
    if let Some(job) = &state.health_pending { job.token.preempt(); }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod analysis_tests;
