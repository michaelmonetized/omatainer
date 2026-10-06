//! Stable MIDI note identity and explicit clip playback coordinates. Entropy is
//! acquired before renderer ownership; note creation uses an atomic counter.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc, OnceLock,
};

/// Immutable editor inspection obtained from the coherent project worker.
/// GUI selection is intentionally separate from this persisted musical data.
#[derive(Clone, Debug)]
pub(crate) struct Document {
    pub track_identity: Option<super::session::Reference>,
    pub scene_identity: Option<super::session::Reference>,
    pub track: u8,
    pub scene: u16,
    pub epoch: u64,
    pub kind: super::ClipKind,
    pub name: String,
    pub bars: f32,
    pub region: Option<Region>,
    pub notes: Vec<super::MidiNote>,
    pub lanes: Option<Arc<super::midi_data::Lanes>>,
}
impl Document {
    pub fn capture(
        captured: super::project::Captured,
        track: u8,
        scene: u16,
    ) -> Result<Arc<Self>, String> {
        let clip = captured
            .state
            .tracks
            .get(track as usize)
            .and_then(|track| track.clips.get(scene as usize))
            .ok_or("MIDI editor target is outside the session")?;
        if clip.kind == super::ClipKind::Audio {
            return Err("This slot contains audio; choose an empty or MIDI clip".into());
        }
        Ok(Arc::new(Self {
            track_identity: captured.state.session.as_ref().and_then(|layout| layout.reference(super::session::Axis::Track, track as usize)),
            scene_identity: captured.state.session.as_ref().and_then(|layout| layout.reference(super::session::Axis::Scene, scene as usize)),
            track,
            scene,
            epoch: captured.checkpoint.epoch,
            kind: clip.kind,
            name: clip.name.clone(),
            bars: clip.bars,
            region: clip.region,
            notes: clip.notes.clone(),
            lanes: clip.lanes.clone(),
        }))
    }
    pub fn playback_region(&self) -> Region {
        self.region.unwrap_or_else(|| Region::full(self.bars))
    }
    pub(super) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.capacity()
            + self.notes.capacity() * std::mem::size_of::<super::MidiNote>()
            + self.lanes.as_ref().map_or(0, |lanes| lanes.bytes())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Ack(Arc<AtomicU8>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Pending,
    Applied,
    Rejected,
    Cancelled,
}
impl Ack {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }
    pub fn state(&self) -> Outcome {
        match self.0.load(Ordering::Acquire) {
            1 => Outcome::Applied,
            2 => Outcome::Rejected,
            3 => Outcome::Cancelled,
            _ => Outcome::Pending,
        }
    }
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(super) fn claim(&self) -> bool {
        self.0
            .compare_exchange(0, 4, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(super) fn applied(&self) {
        self.0.store(1, Ordering::Release);
    }
    pub(super) fn reject(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                matches!(v, 0 | 4).then_some(2)
            });
    }
}

/// Private prepared content cannot be modified after producer validation.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(super) baseline: Arc<Document>,
    pub(super) name: String,
    pub(super) region: Region,
    pub(super) notes: Vec<super::MidiNote>,
    pub(super) lanes: Option<Arc<super::midi_data::Lanes>>,
    metadata_baseline: Option<super::undo::Checkpoint>,
    pub(super) ack: Ack,
}
impl Request {
    pub fn new(
        baseline: Arc<Document>,
        name: String,
        region: Region,
        notes: Vec<super::MidiNote>,
    ) -> Result<(Self, Ack, Arc<Document>), String> {
        let lanes = baseline.lanes.clone();
        Self::with_lanes(baseline, name, region, notes, lanes)
    }
    /// Prepare one note and controller edit.
    /// Takes the captured target, name, region, notes and immutable prepared lanes; returns the guarded request, acknowledgment and next document.
    pub fn with_lanes(
        baseline: Arc<Document>,
        mut name: String,
        region: Region,
        mut notes: Vec<super::MidiNote>,
        lanes: Option<Arc<super::midi_data::Lanes>>,
    ) -> Result<(Self, Ack, Arc<Document>), String> {
        if name.len() > 4096
            || notes.len() > super::project::MAX_NOTES_PER_CLIP
            || !region.allows(&notes)
        {
            return Err(
                "Clip name, note count or loop density exceeds the supported MIDI limits".into(),
            );
        }
        let mut ids = std::collections::HashSet::with_capacity(notes.len());
        for note in &notes {
            if !note.id.valid() || !ids.insert(note.id) || !valid_note(note) {
                return Err("Each note needs a unique identity, MIDI pitch/velocity and finite nonnegative beat values".into());
            }
        }
        if lanes.as_ref().is_some_and(|l| !l.prepared()) {
            return Err("Controller lanes must finish preparation before Apply".into());
        }
        notes.shrink_to_fit();
        name.shrink_to_fit();
        let next = Arc::new(Document {
            track_identity: baseline.track_identity,
            scene_identity: baseline.scene_identity,
            track: baseline.track,
            scene: baseline.scene,
            epoch: baseline.epoch,
            kind: super::ClipKind::Midi,
            name: name.clone(),
            bars: (region.end / 4.0) as f32,
            region: Some(region),
            notes: notes.clone(),
            lanes: lanes.clone(),
        });
        let ack = Ack::new();
        Ok((
            Self {
                baseline,
                name,
                region,
                notes,
                lanes,
                metadata_baseline: None,
                ack: ack.clone(),
            },
            ack,
            next,
        ))
    }
    pub fn bytes(&self) -> usize {
        self.baseline.bytes()
            + self.name.capacity()
            + self.notes.capacity() * std::mem::size_of::<super::MidiNote>()
            + self.lanes.as_ref().map_or(0, |l| l.bytes())
    }
    /// Check that an edited clip can still be saved.
    /// Takes a coherent project capture and cancellation flag; returns a request guarded against concurrent project edits or a metadata limit error.
    pub fn guard_metadata(mut self, mut captured: super::project::Captured, cancel: &AtomicBool) -> Result<Self, String> {
        let clip = captured.state.tracks.get_mut(self.baseline.track as usize).and_then(|t| t.clips.get_mut(self.baseline.scene as usize)).ok_or("MIDI target no longer exists")?;
        clip.name = self.name.clone(); clip.kind = super::ClipKind::Midi; clip.region = Some(self.region); clip.bars = (self.region.end / 4.0) as f32;
        clip.notes = self.notes.clone(); clip.lanes = self.lanes.clone();
        captured.state.validate(&captured.media).map_err(|e| e.to_string())?;
        struct Size<'a> { bytes: usize, limit: usize, cancel: &'a AtomicBool }
        impl std::io::Write for Size<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.cancel.load(Ordering::Acquire) { return Err(std::io::Error::other("MIDI preparation cancelled")); }
                self.bytes = self.bytes.saturating_add(bytes.len());
                if self.bytes > self.limit { return Err(std::io::Error::other("MIDI edit exceeds the native project's metadata limit")); }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let limit = captured.state.import_metadata_limits().max_metadata_bytes.min(crate::project_file::DEFAULT_METADATA_LIMIT - 128 * 1024);
        serde_json::to_writer(Size { bytes: 0, limit, cancel }, &captured.state).map_err(|e| e.to_string())?;
        self.metadata_baseline = Some(captured.checkpoint);
        Ok(self)
    }
    pub(super) fn unchanged(&self) -> bool {
        self.baseline.kind == super::ClipKind::Midi
            && self.baseline.name == self.name
            && self.baseline.region == Some(self.region)
            && self.baseline.notes == self.notes
            && self.baseline.lanes == self.lanes
    }
}
fn valid_note(note: &super::MidiNote) -> bool {
    note.interchange_valid()
        && note.pitch <= 127
        && note.vel <= 127
        && [note.start, note.len]
            .into_iter()
            .all(|v| v.is_finite() && (0.0..=262_144.0).contains(&v))
}
/// Retain a prepared edit acknowledgment during producer submission.
/// Takes any command, including its ownership wrappers; returns the shared acknowledgment so rejected submissions cannot remain pending.
pub(super) fn admission_ack(mut command: &super::Command) -> Option<Ack> {
    loop {
        match command {
            super::Command::ClipManage(request) => return Some(request.ack.clone()),
            super::Command::ArrangementEdit(request) => return Some(request.ack.clone()),
            super::Command::AudioClipEdit(request) => return Some(request.ack.clone()),
            super::Command::MicAuxConfigure(request) => return Some(request.ack.clone()),
            super::Command::SessionEdit(request) => return Some(request.ack.clone()),
            super::Command::MidiImport(request) => return Some(request.ack.clone()),
            super::Command::MidiEdit(request) => return Some(request.ack.clone()),
            super::Command::Gesture { command: inner, .. } => command = inner,
            super::Command::SessionControl(scoped)=>command=&scoped.command,
            _ => return None,
        }
    }
}
pub(super) fn reject_retired(command: &super::Command) {
    if let Some(ack) = admission_ack(command) {ack.reject();}
}
impl super::RtEngine {
    pub(super) fn midi_note_count(&self) -> usize {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.notes.len())
            .sum()
    }
    #[cfg(test)]
    pub(crate) fn begin_midi_trace_for_test(&mut self, track: usize) {
        self.tracks[track].midi_schedule.trace = Some(Vec::with_capacity(1024));
    }
    #[cfg(test)]
    pub(crate) fn take_midi_trace_for_test(&mut self, track: usize) -> Vec<(u64, bool, u8, u8)> {
        let frames_per_beat = self.sr as f64 * 60.0 / self.bpm as f64;
        self.tracks[track]
            .midi_schedule
            .trace
            .as_mut()
            .unwrap()
            .drain(..)
            .map(|(elapsed, gate)| {
                // The renderer passes the end of the output sample's interval.
                let frame = (elapsed * frames_per_beat - 1.0).round().max(0.0) as u64;
                match gate {
                    super::midi_schedule::Gate::On(p, v) => (frame, true, p, v),
                    super::midi_schedule::Gate::Off(p) => (frame, false, p, 0),
                }
            })
            .collect()
    }
    pub(super) fn midi_edit_current(&self, request: &Request) -> bool {
        let baseline = &request.baseline;
        let Some(clip) = self
            .tracks
            .get(baseline.track as usize)
            .and_then(|track| track.clips.get(baseline.scene as usize))
        else {
            return false;
        };
        baseline.epoch == self.undo.checkpoint().epoch
            && request.metadata_baseline.is_none_or(|checkpoint| checkpoint == self.undo.checkpoint())
            && baseline.track_identity.is_none_or(|r| self.session.resolves(super::session::Axis::Track, baseline.track as usize, r))
            && baseline.scene_identity.is_none_or(|r| self.session.resolves(super::session::Axis::Scene, baseline.scene as usize, r))
            && !self.recording_clip_held(baseline.track as usize, baseline.scene as usize)
            && clip.kind == baseline.kind
            && clip.kind != super::ClipKind::Audio
            && clip.name == baseline.name
            && clip.bars == baseline.bars
            && clip.region == baseline.region
            && clip.notes == baseline.notes
            && clip.lanes == baseline.lanes
            && request.name.len() <= 4096
            && request.notes.len() <= super::project::MAX_NOTES_PER_CLIP
            && request.notes.capacity() <= super::project::MAX_NOTES_PER_CLIP
            && self
                .midi_note_count()
                .saturating_sub(clip.notes.len())
                .saturating_add(request.notes.len())
                <= super::project::MAX_TOTAL_NOTES
            && request.region.allows(&request.notes)
            && request.notes.iter().all(|n| n.id.valid() && valid_note(n))
            && request.lanes.as_ref().is_none_or(|l| l.prepared())
            && self.tracks.iter().flat_map(|t| &t.clips).map(|c| c.lanes.as_ref().map_or(0, |l| l.bytes())).sum::<usize>()
                .saturating_sub(clip.lanes.as_ref().map_or(0, |l| l.bytes()))
                .saturating_add(request.lanes.as_ref().map_or(0, |l| l.bytes()))
                .saturating_add(self.conductor.as_ref().map_or(0, |c| c.bytes())) <= super::midi_data::MAX_LANE_BYTES
    }
    pub(super) fn apply_midi_edit(&mut self, mut request: Request) {
        let t = request.baseline.track as usize;
        let s = request.baseline.scene as usize;
        self.cancel_recording_clip(t, s);
        if self.tracks[t]
            .playing
            .or(self.tracks[t].project_resume)
            .map(|p| p.scene)
            == Some(s as u16)
        {
            self.tracks[t].release_clip_notes();
        }
        let clip = &mut self.tracks[t].clips[s];
        std::mem::swap(&mut clip.notes, &mut request.notes);
        std::mem::swap(&mut clip.name, &mut request.name);
        std::mem::swap(&mut clip.lanes, &mut request.lanes);
        clip.kind = super::ClipKind::Midi;
        clip.bars = (request.region.end / 4.0) as f32;
        clip.region = Some(request.region);
        let midi_beat = self.precise_midi_beat();
        self.tracks[t].clip_notes_changed(s, self.beat, midi_beat);
        request.ack.applied();
        self.undo.retire_command(super::Command::MidiEdit(request));
    }
}

static NAMESPACE: OnceLock<u64> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) fn initialize() -> Result<(), String> {
    if NAMESPACE.get().is_some() {
        return Ok(());
    }
    let bytes = crate::sampler_bank::BankId::new()?.bytes();
    let namespace = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    if namespace == 0 {
        return Err("MIDI identity namespace is reserved".into());
    }
    let _ = NAMESPACE.set(namespace);
    Ok(())
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub(crate) struct NoteId([u64; 2]);
impl NoteId {
    pub(crate) fn words(self) -> [u64; 2] { self.0 }
    pub fn new() -> Self {
        if NAMESPACE.get().is_none() && initialize().is_err() {
            return Self::default();
        }
        let Some(namespace) = NAMESPACE.get() else {
            return Self::default();
        };
        let sequence = SEQUENCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .unwrap_or(0);
        Self([*namespace, sequence])
    }
    pub fn valid(self) -> bool {
        self.0[1] != 0
    }
    pub(super) fn legacy(track: usize, scene: usize, index: usize) -> Self {
        Self([
            0,
            1 + ((track * super::SCENES + scene) * super::project::MAX_NOTES_PER_CLIP + index)
                as u64,
        ])
    }
}
impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}{:016x}", self.0[0], self.0[1])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Region {
    pub start: f64,
    pub end: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_enabled: bool,
}
impl Region {
    pub fn full(bars: f32) -> Self {
        let end = bars as f64 * 4.0;
        Self {
            start: 0.0,
            end,
            loop_start: 0.0,
            loop_end: end,
            loop_enabled: true,
        }
    }
    pub fn valid(self) -> bool {
        [self.start, self.end, self.loop_start, self.loop_end]
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=262_144.0).contains(&value))
            && self.start < self.end
            && self.start <= self.loop_start
            && self.loop_start < self.loop_end
            && self.loop_end <= self.end
            && self.end - self.start >= 1.0 / 1024.0
            && self.period() >= 1.0 / 1024.0
    }
    pub fn repeating(self, launch_loops: bool) -> bool {
        launch_loops && self.loop_enabled
    }
    pub fn period(self) -> f64 {
        self.loop_end - self.loop_start
    }
    pub fn position(self, elapsed: f64, launch_loops: bool) -> Option<f64> {
        if !self.valid() || !elapsed.is_finite() || elapsed < 0.0 {
            return None;
        }
        let position = self.start + elapsed;
        if self.repeating(launch_loops) && position >= self.loop_end {
            Some(self.loop_start + (position - self.loop_end).rem_euclid(self.period()))
        } else {
            (position < self.end).then_some(position)
        }
    }
    pub fn allows(self, notes: &[super::MidiNote]) -> bool {
        if !self.valid() {
            return false;
        }
        let recurring = notes
            .iter()
            .filter(|note| {
                !note.muted
                    && note.len > 0.0
                    && note.source_start() >= self.loop_start
                    && (note.source_start()) < self.loop_end
            })
            .count();
        !self.loop_enabled
            || recurring as f64 <= super::project::MAX_NOTES_PER_CLIP as f64 * self.period()
    }
}

/// The legacy whole-list setter remains usable without supplied identities.
/// A copied note receives a new ID; the first occurrence retains its identity.
/// This runs on the producer, before renderer ownership of the payload.
pub(super) fn qualify_legacy_notes(command: &mut super::Command) -> bool {
    let command = if let super::Command::Gesture { command, .. } = command {
        command.as_mut()
    } else {
        command
    };
    let super::Command::SetNotes { notes, .. } = command else {
        return true;
    };
    if notes.len() > super::project::MAX_NOTES_PER_CLIP
        || notes.capacity() > super::project::MAX_NOTES_PER_CLIP
    {
        return true;
    }
    let mut used = std::collections::HashSet::with_capacity(notes.len());
    for note in notes {
        if !note.id.valid() || !used.insert(note.id) {
            note.id = NoteId::new();
            if !note.id.valid() || !used.insert(note.id) {
                return false;
            }
        }
    }
    true
}

pub(super) fn reject_legacy_fields(state: &serde_json::Value) -> Result<(), &'static str> {
    let Some(version) = state.get("version").and_then(serde_json::Value::as_u64) else {
        return Ok(());
    };
    if version >= 6 {
        return Ok(());
    }
    if state.get("conductor").is_some() { return Err("Conductor maps require project schema 6"); }
    for track in state
        .get("tracks")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        for clip in track
            .get("clips")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if clip.get("lanes").is_some() { return Err("MIDI lanes require project schema 6"); }
            if version < 5 && clip.get("region").is_some() {
                return Err("MIDI clip regions are unsupported in legacy project schemas");
            }
            for note in clip
                .get("notes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                if ["channel", "release_vel", "source_timing"]
                    .iter()
                    .any(|field| note.get(field).is_some())
                {
                    return Err(
                        "MIDI channel and tick fields are unsupported before project schema 6",
                    );
                }
                if version < 5 && (note.get("id").is_some() || note.get("muted").is_some()) {
                    return Err(
                        "MIDI note identity and mute are unsupported in legacy project schemas",
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(rt: &super::super::RtEngine, track: u8, scene: u16) -> Arc<Document> {
        let clip = &rt.tracks[track as usize].clips[scene as usize];
        Arc::new(Document {
            track_identity: rt.session.reference(super::super::session::Axis::Track, track as usize),
            scene_identity: rt.session.reference(super::super::session::Axis::Scene, scene as usize),
            track,
            scene,
            epoch: rt.undo.checkpoint().epoch,
            kind: clip.kind,
            name: clip.name.clone(),
            bars: clip.bars,
            region: clip.region,
            notes: clip.notes.clone(),
            lanes: clip.lanes.clone(),
        })
    }
    fn tick(rt: &mut super::super::RtEngine) {
        rt.process(&mut [0.0; 128]);
    }
    fn note() -> super::super::MidiNote {
        super::super::MidiNote {
            channel: 0,
            release_vel: 64,
            source_timing: None,
            id: NoteId::new(),
            pitch: 64,
            start: 1.0,
            len: 0.5,
            vel: 92,
            muted: false,
        }
    }
    #[test]
    fn prepared_edit_ack_undo_redo_retain_note_ids_and_ranges_without_audio_heap_work() {
        use super::super::{test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        let old = document(&rt, 2, 7);
        let notes = vec![note()];
        let id = notes[0].id;
        let region = Region {
            start: 0.5,
            loop_start: 1.0,
            ..Region::full(16.0)
        };
        let (request, ack, next) =
            Request::new(old.clone(), "Sixteen bars".into(), region, notes).unwrap();
        engine.send(Command::MidiEdit(request)).unwrap();
        assert_eq!(ack.state(), Outcome::Pending);
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Applied);
        assert_eq!(rt.tracks[2].clips[7].notes, next.notes);
        assert_eq!(rt.tracks[2].clips[7].region, Some(region));
        engine.send(Command::Undo).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].clips[7].notes, old.notes);
        assert_eq!(rt.tracks[2].clips[7].kind, old.kind);
        engine.send(Command::Redo).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].clips[7].notes[0].id, id);
        assert_eq!(rt.tracks[2].clips[7].region, Some(region));
    }
    #[test]
    fn stale_cancelled_and_budget_rejected_edits_leave_current_creative_work_intact() {
        use super::super::{test_alloc, Command, Engine};
        for reason in ["stale", "epoch", "cancelled", "budget"] {
            let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
            let old = document(&rt, 2, 7);
            let (request, ack, _) =
                Request::new(old, "Draft".into(), Region::full(16.0), vec![note()]).unwrap();
            match reason {
                "stale" => {
                    engine
                        .send(Command::SetNotes {
                            track: 2,
                            scene: 7,
                            notes: vec![note()],
                        })
                        .unwrap();
                    tick(&mut rt);
                }
                "epoch" => rt.clear_undo_for_test(),
                "cancelled" => assert!(ack.cancel()),
                "budget" => rt.set_undo_budget_for_test(1),
                _ => unreachable!(),
            }
            let before = document(&rt, 2, 7);
            let checkpoint = rt.undo.checkpoint();
            engine.send(Command::MidiEdit(request)).unwrap();
            assert_eq!(
                test_alloc::measure(|| tick(&mut rt)),
                test_alloc::Counts::default(),
                "{reason}"
            );
            assert_eq!(rt.tracks[2].clips[7].notes, before.notes, "{reason}");
            assert_eq!(rt.tracks[2].clips[7].name, before.name, "{reason}");
            assert_eq!(rt.undo.checkpoint(), checkpoint, "{reason}");
            assert_eq!(
                ack.state(),
                if reason == "cancelled" {
                    Outcome::Cancelled
                } else {
                    Outcome::Rejected
                }
            );
        }
    }
    #[test]
    fn audition_is_an_independent_non_recording_owner_with_reserved_release() {
        use super::super::{dsp::InputKey, test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 32);
        rt.recording = true;
        rt.playing = true;
        rt.selected_track = 2;
        rt.selected_scene = 7;
        let before = document(&rt, 2, 7);
        engine
            .send(Command::MidiAudition {
                id: 42,
                track: 2,
                note: 64,
                vel: 100,
                on: true,
            })
            .unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert!(rt.tracks[2]
            .poly
            .voices
            .iter()
            .any(|v| v.input == Some(InputKey::Preview(42)) && matches!(v.env.stage, 1..=3)));
        assert_eq!(rt.tracks[2].clips[7].notes, before.notes);
        engine.cmd.performance().set_enabled(true).unwrap();
        engine
            .send(Command::MidiAudition {
                id: 42,
                track: 2,
                note: 64,
                vel: 100,
                on: false,
            })
            .unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert!(rt.tracks[2]
            .poly
            .voices
            .iter()
            .all(|v| v.input != Some(InputKey::Preview(42)) || !matches!(v.env.stage, 1..=3)));
        assert_eq!(rt.tracks[2].clips[7].notes, before.notes);
    }
    #[test]
    fn late_protection_retires_last_document_owner_without_callback_destruction() {
        use super::super::{test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        let baseline = document(&rt, 2, 7);
        let weak = Arc::downgrade(&baseline);
        let (request, ack, next) =
            Request::new(baseline, "Draft".into(), Region::full(16.0), vec![note()]).unwrap();
        drop(next);
        engine.send(Command::MidiEdit(request)).unwrap();
        engine.cmd.performance().set_enabled(true).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Rejected);
        assert!(rt.tracks[2].clips[7].notes.is_empty());
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while weak.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < end,
                "retirement worker retained document"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn an_active_recording_cannot_be_replaced_even_by_an_exact_inspection() {
        use super::super::{test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        engine.send(Command::Select { track: 2, scene: 7 }).unwrap();
        engine.send(Command::Play).unwrap();
        engine.send(Command::Record).unwrap();
        engine
            .send(Command::LiveNoteOn {
                source: 7,
                ch: 1,
                note: 60,
                vel: 100,
            })
            .unwrap();
        tick(&mut rt);
        assert!(rt.recording_clip_held(2, 7));
        let baseline = document(&rt, 2, 7);
        let id = baseline.notes[0].id;
        let (request, ack, _) = Request::new(
            baseline,
            "Unsafe overwrite".into(),
            Region::full(16.0),
            vec![],
        )
        .unwrap();
        engine.send(Command::MidiEdit(request)).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Rejected);
        assert_eq!(rt.tracks[2].clips[7].notes[0].id, id);
        assert!(rt.recording_clip_held(2, 7));
        engine
            .send(Command::LiveNoteOff {
                source: 7,
                ch: 1,
                note: 60,
            })
            .unwrap();
        tick(&mut rt);
        assert!(!rt.recording_clip_held(2, 7));
        assert!(rt.tracks[2].clips[7].notes[0].len > 0.0);
    }
    #[test]
    fn bounded_loop_density_is_enforced_for_prepared_edits_and_live_recording() {
        use super::super::{Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        let region = Region::full(1.0 / 4096.0);
        let notes: Vec<_> = (0..8)
            .map(|_| {
                let mut n = note();
                n.start = 0.0;
                n
            })
            .collect();
        assert!(region.allows(&notes));
        let mut too_dense = notes.clone();
        too_dense.push({
            let mut n = note();
            n.start = 0.0;
            n
        });
        assert!(Request::new(document(&rt, 2, 7), "Too dense".into(), region, too_dense).is_err());
        let (request, ack, _) =
            Request::new(document(&rt, 2, 7), "Tiny loop".into(), region, notes).unwrap();
        engine.send(Command::MidiEdit(request)).unwrap();
        tick(&mut rt);
        assert_eq!(ack.state(), Outcome::Applied);
        engine.send(Command::Select { track: 2, scene: 7 }).unwrap();
        engine.send(Command::Play).unwrap();
        engine.send(Command::Record).unwrap();
        engine
            .send(Command::LiveNoteOn {
                source: 8,
                ch: 1,
                note: 61,
                vel: 100,
            })
            .unwrap();
        tick(&mut rt);
        assert_eq!(rt.tracks[2].clips[7].notes.len(), 8);
        assert!(region.allows(&rt.tracks[2].clips[7].notes));
        assert!(!rt.recording_clip_held(2, 7));
    }
    #[test]
    fn whole_project_note_limit_rejects_expansion_but_accepts_reduction_and_keeps_recording_saveable(
    ) {
        use super::super::{test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        for track in &mut rt.tracks {
            for clip in &mut track.clips {
                clip.notes.clear();
            }
            let clip = &mut track.clips[7];
            clip.kind = super::super::ClipKind::Midi;
            clip.notes = (0..super::super::project::MAX_NOTES_PER_CLIP)
                .map(|_| note())
                .collect();
        }
        assert_eq!(rt.midi_note_count(), super::super::project::MAX_TOTAL_NOTES);
        let (request, ack, _) = Request::new(
            document(&rt, 2, 6),
            "Over full project".into(),
            Region::full(16.0),
            vec![note()],
        )
        .unwrap();
        engine.send(Command::MidiEdit(request)).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Rejected);
        assert!(rt.tracks[2].clips[6].notes.is_empty());
        engine.send(Command::Select { track: 2, scene: 6 }).unwrap();
        engine.send(Command::Play).unwrap();
        engine.send(Command::Record).unwrap();
        engine
            .send(Command::LiveNoteOn {
                source: 78,
                ch: 1,
                note: 61,
                vel: 100,
            })
            .unwrap();
        tick(&mut rt);
        assert!(rt.tracks[2].clips[6].notes.is_empty());
        assert!(!rt.recording_clip_held(2, 6));
        let (request, ack, _) = Request::new(
            document(&rt, 2, 7),
            "Reduced".into(),
            Region::full(16.0),
            vec![note()],
        )
        .unwrap();
        engine.send(Command::MidiEdit(request)).unwrap();
        assert_eq!(
            test_alloc::measure(|| tick(&mut rt)),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Applied);
        assert!(rt.midi_note_count() < super::super::project::MAX_TOTAL_NOTES);
    }
    #[test]
    fn identities_are_stable_distinct_and_legacy_migration_is_deterministic() {
        initialize().unwrap();
        let first = NoteId::new();
        let second = NoteId::new();
        assert!(first.valid());
        assert_ne!(first, second);
        assert_eq!(
            serde_json::from_slice::<NoteId>(&serde_json::to_vec(&first).unwrap()).unwrap(),
            first
        );
        assert_eq!(NoteId::legacy(2, 3, 4), NoteId::legacy(2, 3, 4));
        assert_ne!(NoteId::legacy(2, 3, 4), NoteId::legacy(2, 3, 5));
        assert_ne!(NoteId::legacy(2, 3, 4), first);
        assert!(!NoteId::default().valid());
    }
    #[test]
    fn region_start_end_and_loop_coordinates_preserve_first_pass_and_repeat() {
        let region = Region {
            start: 2.0,
            end: 16.0,
            loop_start: 4.0,
            loop_end: 12.0,
            loop_enabled: true,
        };
        assert!(region.valid());
        assert_eq!(region.position(0.0, true), Some(2.0));
        assert_eq!(region.position(9.0, true), Some(11.0));
        assert_eq!(region.position(10.0, true), Some(4.0));
        assert_eq!(region.position(18.0, true), Some(4.0));
        assert_eq!(region.position(14.0, false), None);
        assert_eq!(
            Region {
                loop_enabled: false,
                ..region
            }
            .position(14.0, true),
            None
        );
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(!Region {
                start: value,
                ..region
            }
            .valid());
        }
        assert!(!Region {
            loop_end: 17.0,
            ..region
        }
        .valid());
        assert_eq!(Region::full(16.0).end, 64.0);
    }
}
