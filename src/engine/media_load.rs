//! One active decoder with a replaceable pending/result slot per deck.
//! Filesystem and decoder calls remain on the worker. Cancellation is observed
//! at safe decode boundaries; it cannot interrupt an OS read already in progress.
use super::decode::{decode_audio_with_cancel, DecodeFailure, DecodedAudio};
use super::media_source::FileFingerprint;
use super::DECKS;
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
struct State {
    pending: [Option<Request>; DECKS],
    ready: [Option<Completion>; DECKS],
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
}
impl Loader {
    pub fn start() -> io::Result<Self> {
        Self::with_decoder(|path, token| decode_audio_with_cancel(path, || !token.is_current()))
    }

    pub(crate) fn with_decoder(
        mut decode: impl FnMut(&Path, &LoadToken) -> Result<DecodedAudio, DecodeFailure>
            + Send
            + 'static,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: std::array::from_fn(|_| None),
                ready: std::array::from_fn(|_| None),
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
                        let selected = (0..DECKS)
                            .map(|offset| (state.next_deck + offset) % DECKS)
                            .find(|&deck| state.pending[deck].is_some());
                        if let Some(deck) = selected {
                            state.next_deck = (deck + 1) % DECKS;
                            break state.pending[deck].take().unwrap();
                        }
                        state = worker.wake.wait(state).unwrap();
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
        })
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
            (
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

#[cfg(test)]
mod tests;
