//! Optional label publication is owned outside the audio callback.
use super::Source;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config { pub enabled: bool, pub title: bool, pub artist: bool, pub identity: bool }
impl Default for Config { fn default() -> Self { Self { enabled: false, title: true, artist: true, identity: false } } }
impl Config { pub fn is_default(&self) -> bool { *self == Self::default() } }

#[derive(Default)]
struct State { config: Config, generation: u64, available: bool, published: Option<Instant>, decks: [Option<Source>; 2] }
#[derive(Default)]
pub(crate) struct Shared(Mutex<State>);
impl Shared {
    /// Apply explicit publication and redaction intent.
    /// Takes saved configuration; immediately invalidates previously published labels when any setting changes.
    pub fn configure(&self, config: Config) {
        let mut state = self.0.lock();
        if state.config != config { state.config = config; state.generation = state.generation.saturating_add(1); state.published = None; state.decks = [None, None]; }
    }
    pub fn config(&self) -> (u64, Config) { let state = self.0.lock(); (state.generation, state.config) }
    /// Publish a recent renderer-confirmed label snapshot.
    /// Takes the configuration generation, availability and two bounded sources; refuses stale configuration and never queues consumers.
    pub fn publish(&self, generation: u64, available: bool, decks: [Option<Source>; 2]) {
        let mut state = self.0.lock();
        if state.generation != generation || !state.config.enabled { return; }
        state.available = available; state.published = Some(Instant::now()); state.decks = decks;
    }
    pub fn disconnect(&self) { let mut state=self.0.lock(); state.available=false; state.decks=[None,None]; state.published=None; }
    /// Read only the permitted fields from a fresh snapshot.
    /// Takes no arguments; returns at most two labels, with disabled or stale data removed.
    pub fn read(&self) -> serde_json::Value {
        let state = self.0.lock();
        let fresh = state.published.is_some_and(|time|time.elapsed() <= Duration::from_secs(1));
        let status = if !state.config.enabled { "disabled" } else if !state.available { "unavailable" } else if !fresh { "stale" } else { "current" };
        let decks: Vec<_> = if state.config.enabled && state.available && fresh {
            state.decks.iter().enumerate().filter_map(|(deck,source)| {
                let source = source.as_ref()?;
                let mut value = serde_json::json!({"deck":deck});
                let mut truncated = false;
                if state.config.title { let (text, cut)=label(source.title()); value["title"]=text.into(); truncated |= cut; }
                if state.config.artist { let (text, cut)=label(source.artist()); value["artist"]=text.into(); truncated |= cut; }
                if truncated { value["labels_truncated"]=true.into(); }
                if state.config.identity { if let Source::Catalog {track_id,version,..}=source {value["track_id"]=track_id.clone().into();value["version"]=(*version).into();} }
                Some(value)
            }).collect()
        } else { Vec::new() };
        serde_json::json!({"schema":1,"enabled":state.config.enabled,"status":status,"generation":state.generation.to_string(),"decks":decks})
    }
}

/// Fit a label within the local feed response budget.
/// Takes valid UTF-8; returns at most 512 complete bytes and an explicit truncation flag.
fn label(value: &str) -> (&str, bool) { let mut end=value.len().min(512); while !value.is_char_boundary(end) {end-=1;} (&value[..end], end<value.len()) }
