//! Persistent performance sessions are independent of project state and Undo.
//! The typed export contains labels and opaque identity, never media locations.
use crate::engine::history_measurement::{Classification, Observation};
use serde::{Deserialize, Serialize};
pub(crate) mod storage;
pub(crate) mod worker;

pub(crate) const MAX_ENTRIES: usize = 4096;
const MAX_RATES: usize = 64;
pub(crate) const MAX_LABEL: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Catalog { track_id: String, version: u32, title: String, artist: String },
    Unresolved,
    External { title: String, artist: String },
}
impl Source {
    pub fn title(&self) -> &str { match self { Self::Catalog { title, .. } | Self::External { title, .. } => title, Self::Unresolved => "Unresolved track" } }
    pub fn artist(&self) -> &str { match self { Self::Catalog { artist, .. } | Self::External { artist, .. } => artist, Self::Unresolved => "" } }
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Catalog { track_id, .. } = self { valid_id(track_id)?; }
        label(self.title(), false)?; label(self.artist(), true)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateCount {
    pub rate: u32,
    pub observed: u64,
    pub active: u64,
    pub ambiguous: u64,
    pub invalid: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: u32,
    pub source: Source,
    pub deck: Option<u8>,
    pub load_key: Option<u64>,
    pub rates: Vec<RateCount>,
    pub played_override: Option<bool>,
    pub first_active_ns: Option<u64>,
    pub last_active_ns: Option<u64>,
    pub last_frame: Option<u64>,
}
impl Entry {
    pub fn measured_seconds(&self) -> f64 { self.rates.iter().map(|r| r.active as f64 / f64::from(r.rate)).sum() }
    pub fn played(&self) -> bool { self.played_override.unwrap_or_else(|| self.rates.iter().any(|r| r.active > 0)) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State { Active, Ended, Unclean }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    pub schema: u32,
    pub id: String,
    pub renderer_session: u64,
    pub started_ns: u64,
    pub ended_ns: Option<u64>,
    pub last_confirmed_ns: u64,
    pub start_frame: u64,
    pub confirmed_frame: u64,
    pub end_frame: Option<u64>,
    pub state: State,
    pub incomplete: bool,
    pub dropped_observation_frames: u64,
    pub edit_revision: u64,
    pub entries: Vec<Entry>,
}
impl Session {
    pub fn new(id: String, renderer_session: u64, started_ns: u64, start_frame: u64) -> Result<Self, String> {
        let session = Self { schema: 1, id, renderer_session, started_ns, ended_ns: None, last_confirmed_ns: started_ns,
            start_frame, confirmed_frame: start_frame, end_frame: None, state: State::Active, incomplete: false,
            dropped_observation_frames: 0, edit_revision: 0, entries: Vec::new() };
        session.validate()?; Ok(session)
    }
    pub fn validate(&self) -> Result<(), String> {
        valid_id(&self.id)?;
        if self.schema != 1 || self.renderer_session == 0 || self.started_ns == 0 || self.last_confirmed_ns < self.started_ns
            || self.entries.len() > MAX_ENTRIES
            || (self.state == State::Active) != self.ended_ns.is_none()
            || (self.state == State::Active) != self.end_frame.is_none()
            || self.ended_ns.is_some_and(|end| end < self.started_ns || end < self.last_confirmed_ns)
            || self.confirmed_frame < self.start_frame
            || self.end_frame.is_some_and(|end| end < self.confirmed_frame)
        { return Err("invalid history session boundaries or limits".into()); }
        let mut ids = std::collections::HashSet::new();
        let mut loads = std::collections::HashSet::new();
        for entry in &self.entries {
            if entry.id == 0 || !ids.insert(entry.id) || entry.rates.len() > MAX_RATES { return Err("invalid history entry identity or limits".into()); }
            entry.source.validate()?;
            match (&entry.source, entry.deck, entry.load_key) {
                (Source::External { .. }, None, None) if entry.rates.is_empty() && entry.last_frame.is_none()
                    && entry.first_active_ns.is_none() && entry.last_active_ns.is_none() => {},
                (Source::Catalog { .. } | Source::Unresolved, Some(deck @ 0..=1), Some(key))
                    if key != 0 && loads.insert((deck, key)) => {},
                _ => return Err("manual and measured history fields conflict".into()),
            }
            if entry.first_active_ns.is_some() != entry.last_active_ns.is_some()
                || entry.first_active_ns.is_some_and(|time| time < self.started_ns)
                || entry.last_active_ns.is_some_and(|time| time > self.last_confirmed_ns
                    || entry.first_active_ns.is_some_and(|first| time < first))
                || entry.last_frame.is_some_and(|frame| frame < self.start_frame || frame > self.confirmed_frame)
            { return Err("invalid history entry timeline".into()); }
            if entry.rates.iter().any(|r| r.active > 0) != entry.first_active_ns.is_some()
                || (entry.rates.iter().any(|r| r.observed > 0) && entry.last_frame.is_none())
            { return Err("history sample counts do not match entry timeline".into()); }
            let mut rates = std::collections::HashSet::new();
            for count in &entry.rates {
                if !(8_000..=384_000).contains(&count.rate) || !rates.insert(count.rate)
                    || count.active.checked_add(count.ambiguous).and_then(|v| v.checked_add(count.invalid))
                        .is_none_or(|classified| classified > count.observed)
                { return Err("invalid history sample counts".into()); }
            }
        }
        Ok(())
    }
    pub fn loaded(&mut self, key: u64, deck: u8, source: Source) -> Result<u32, String> {
        if key == 0 || deck > 1 || matches!(source, Source::External { .. }) { return Err("invalid loaded history identity".into()); }
        source.validate()?;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.load_key == Some(key) && e.deck == Some(deck)) {
            if entry.source == Source::Unresolved { entry.source = source; }
            else if source != Source::Unresolved && entry.source != source { return Err("history load identity already has different metadata".into()); }
            return Ok(entry.id);
        }
        let id = self.next_entry()?;
        self.entries.push(Entry { id, source, deck: Some(deck), load_key: Some(key), rates: Vec::new(),
            played_override: None, first_active_ns: None, last_active_ns: None, last_frame: None });
        Ok(id)
    }
    fn next_entry(&self) -> Result<u32, String> {
        if self.entries.len() >= MAX_ENTRIES { return Err("history session reached its 4096-entry limit".into()); }
        self.entries.iter().map(|e| e.id).max().unwrap_or(0).checked_add(1).ok_or_else(|| "history entry IDs exhausted".into())
    }
    pub fn observe(&mut self, observation: Observation, source: Source) -> Result<(), String> {
        let frame_end = observation.first_frame.checked_add(u64::from(observation.frames)).ok_or("history sample clock exhausted")?;
        if observation.session != self.renderer_session || self.state == State::Unclean
            || observation.frames == 0 || !(8_000..=384_000).contains(&observation.sample_rate)
            || observation.first_frame < self.start_frame || self.end_frame.is_some_and(|end| frame_end > end)
            || observation.wall_ns < self.started_ns || self.ended_ns.is_some_and(|end| observation.wall_ns > end)
        { return Err("history observation is outside its session boundary".into()); }
        let id = self.loaded(observation.episode.load, observation.episode.deck, source)?;
        let entry = self.entries.iter_mut().find(|entry| entry.id == id).unwrap();
        if entry.last_frame.is_some_and(|last| observation.first_frame < last) { return Err("duplicate or reordered history observation".into()); }
        let position = match entry.rates.iter().position(|r| r.rate == observation.sample_rate) {
            Some(position) => position,
            None if entry.rates.len() < MAX_RATES => { entry.rates.push(RateCount { rate: observation.sample_rate, ..Default::default() }); entry.rates.len() - 1 },
            None => return Err("history entry reached its 64-rate limit".into()),
        };
        let rate = &mut entry.rates[position];
        let frames = u64::from(observation.frames);
        let mut candidate = rate.clone();
        candidate.observed = candidate.observed.checked_add(frames).ok_or("history duration exhausted")?;
        let count = match observation.classification {
            Classification::Active => Some(&mut candidate.active),
            Classification::Ambiguous => Some(&mut candidate.ambiguous),
            Classification::Nonfinite | Classification::ClockOverflow => Some(&mut candidate.invalid),
            Classification::BelowFloor => None,
        };
        if let Some(count) = count { *count = count.checked_add(frames).ok_or("history duration exhausted")?; }
        *rate = candidate;
        entry.last_frame = Some(frame_end);
        self.last_confirmed_ns = self.last_confirmed_ns.max(observation.wall_ns);
        self.confirmed_frame = self.confirmed_frame.max(frame_end);
        if observation.classification == Classification::Active {
            entry.first_active_ns.get_or_insert(observation.wall_ns);
            entry.last_active_ns = Some(observation.wall_ns);
        }
        self.incomplete |= matches!(observation.classification, Classification::Nonfinite | Classification::ClockOverflow);
        Ok(())
    }
    pub fn end(&mut self, wall_ns: u64, frame: u64, incomplete: bool, dropped: u64) -> Result<(), String> {
        if self.state != State::Active || wall_ns < self.last_confirmed_ns || frame < self.start_frame
            || self.entries.iter().any(|entry| entry.last_frame.is_some_and(|last| last > frame))
        { return Err("invalid history end receipt".into()); }
        self.state = State::Ended; self.ended_ns = Some(wall_ns); self.end_frame = Some(frame); self.confirmed_frame = frame;
        self.incomplete |= incomplete; self.dropped_observation_frames = self.dropped_observation_frames.max(dropped);
        Ok(())
    }
    pub fn recover_unclean(&mut self) {
        if self.state == State::Active {
            self.state = State::Unclean;
            self.ended_ns = Some(self.last_confirmed_ns);
            self.end_frame = Some(self.confirmed_frame);
            self.incomplete = true;
        }
    }
    pub fn mark(&mut self, revision: u64, entry: u32, played: Option<bool>) -> Result<(), String> {
        if revision != self.edit_revision { return Err("history edit changed; refresh before editing".into()); }
        let next = self.edit_revision.checked_add(1).ok_or("history edit revision exhausted")?;
        let entry = self.entries.iter_mut().find(|e| e.id == entry).ok_or("history entry no longer exists")?;
        entry.played_override = played; self.edit_revision = next; Ok(())
    }
    pub fn external(&mut self, revision: u64, title: String, artist: String) -> Result<u32, String> {
        if revision != self.edit_revision { return Err("history edit changed; refresh before editing".into()); }
        let source = Source::External { title, artist }; source.validate()?;
        let next = self.edit_revision.checked_add(1).ok_or("history edit revision exhausted")?;
        let id = self.next_entry()?;
        self.entries.push(Entry { id, source, deck: None, load_key: None, rates: Vec::new(),
            played_override: Some(true), first_active_ns: None, last_active_ns: None, last_frame: None });
        self.edit_revision = next; Ok(id)
    }
    pub fn export(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        #[derive(Serialize)]
        struct ExportEntry<'a> { id: u32, source: &'a Source, deck: Option<u8>, played: bool,
            played_override: Option<bool>, digital_main_frames_by_rate: &'a [RateCount],
            first_active_callback_ns: Option<u64>, last_active_callback_ns: Option<u64> }
        let entries: Vec<_> = self.entries.iter().map(|e| ExportEntry { id: e.id, source: &e.source,
            deck: e.deck, played: e.played(), played_override: e.played_override,
            digital_main_frames_by_rate: &e.rates, first_active_callback_ns: e.first_active_ns,
            last_active_callback_ns: e.last_active_ns }).collect();
        serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"session_id":self.id,
            "started_ns":self.started_ns,"ended_ns":self.ended_ns,"last_confirmed_ns":self.last_confirmed_ns,
            "state":self.state,"incomplete":self.incomplete,"dropped_observation_frames":self.dropped_observation_frames,
            "scope":"Submitted digital main-output activity, -90 dBFS RMS, 10 ms windows rounded up to an output sample; timestamps have callback-entry resolution. No physical audibility or backend delivery claim. Manual marks do not change measured frames.",
            "entries":entries})).map_err(|e| e.to_string())
    }
}

pub(crate) fn valid_id(id: &str) -> Result<(), String> {
    if id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { Ok(()) }
    else { Err("invalid opaque history identity".into()) }
}
fn label(value: &str, empty: bool) -> Result<(), String> {
    if value.len() > MAX_LABEL || (!empty && value.trim().is_empty()) || value.chars().any(char::is_control) {
        Err("history labels need non-control text within 1024 UTF-8 bytes".into())
    } else { Ok(()) }
}

#[cfg(test)]
mod tests;
