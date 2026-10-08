use super::*;
use crate::engine::midi_tools::{self, Content, Parameters, Prepared, Summary};
use std::collections::BTreeMap;
type Key = (u8, u16);
struct Original {
    baseline: Arc<Document>,
    name: String,
    content: Arc<Content>,
    region: Region,
    selected: BTreeSet<NoteId>,
    dirty: bool,
    controls_dirty: bool,
    context: Option<crate::engine::musical_context::Context>,
}
impl Original {
    fn capture(draft: &Draft) -> Self {
        Self {
            baseline: draft.baseline.clone(),
            name: draft.name.clone(),
            content: Arc::new(draft.controls.content(&draft.notes)),
            region: draft.region,
            selected: draft.selected.clone(),
            dirty: draft.dirty,
            controls_dirty: draft.controls.dirty,
            context: draft.resolved_context().context,
        }
    }
    fn current(&self, draft: &Draft) -> bool {
        self.name == draft.name
            && Arc::ptr_eq(&self.baseline, &draft.baseline)
            && self.region == draft.region
            && self.selected == draft.selected
            && self.content.notes == draft.notes
            && draft.controls.matches(&self.content)
            && self.dirty == draft.dirty
            && self.controls_dirty == draft.controls.dirty
            && self.context == draft.resolved_context().context
    }
    fn install(&self, draft: &mut Draft) {
        let mut content = (*self.content).clone();
        draft.notes = std::mem::take(&mut content.notes);
        draft.controls.install(&mut content, self.controls_dirty);
        draft.region = self.region;
        draft.selected = self.selected.clone();
        draft.dirty = self.dirty;
        draft.drag = None;
    }
}
struct Worker {
    receiver: mpsc::Receiver<Result<Vec<(Key, Prepared)>, String>>,
    cancel: Arc<AtomicBool>,
    originals: Arc<BTreeMap<Key, Original>>,
    guards: BTreeMap<Key, Original>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub(in crate::ui::piano_roll) struct Group {
    worker: Option<Worker>,
    originals: Option<Arc<BTreeMap<Key, Original>>>,
    current: BTreeMap<Key, Original>,
    pub summaries: Vec<(Key, Summary)>,
}
impl Group {
    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub fn previewed(&self) -> bool {
        self.originals.is_some()
    }
    pub fn cancel(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Release);
        }
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Prepare every explicitly selected editable clip together.
    /// Takes captured draft owners and the shared tool parameters; starts one cancellable worker or retains all drafts on refusal.
    pub fn start(
        &mut self,
        drafts: BTreeMap<Key, &Draft>,
        params: Parameters,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the group preview to finish".into());
        }
        if drafts.is_empty()
            || drafts.len() > 64
            || drafts
                .values()
                .any(|d| d.selected.is_empty() || !d.tools.editing())
        {
            return Err(
                "Select notes in each enabled clip; keep or restore individual previews first"
                    .into(),
            );
        }
        let guards: BTreeMap<_, _> = drafts
            .iter()
            .map(|(key, d)| (*key, Original::capture(d)))
            .collect();
        let originals = if let Some(originals) = &self.originals {
            if originals.keys().ne(guards.keys())
                || self
                    .current
                    .iter()
                    .any(|(key, guard)| drafts.get(key).is_none_or(|draft| !guard.current(draft)))
            {
                return Err(
                    "A group target changed; keep or restore its preview before continuing".into(),
                );
            }
            originals.clone()
        } else {
            Arc::new(
                drafts
                    .iter()
                    .map(|(key, d)| (*key, Original::capture(d)))
                    .collect(),
            )
        };
        let events = originals
            .values()
            .map(|v| v.content.messages.len() + v.content.meta.len() + 2 * v.content.notes.len())
            .sum::<usize>();
        let lane_bytes = originals
            .values()
            .map(|v| {
                v.content.messages.len() * std::mem::size_of::<crate::midi_file::Message>()
                    + v.content
                        .meta
                        .iter()
                        .map(|m| match &m.value {
                            crate::midi_file::MetaValue::Text { bytes, .. } => {
                                std::mem::size_of::<crate::midi_file::Meta>() + bytes.len()
                            }
                            _ => std::mem::size_of::<crate::midi_file::Meta>(),
                        })
                        .sum::<usize>()
                    + v.content
                        .labels
                        .iter()
                        .map(|label| {
                            label.name.len()
                                + std::mem::size_of::<crate::engine::midi_data::Label>()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>();
        if events > crate::midi_file::MAX_EVENTS
            || lane_bytes > crate::engine::midi_data::MAX_LANE_BYTES
        {
            return Err("The group exceeds native MIDI event or lane-memory limits".into());
        }
        if originals
            .values()
            .map(|v| v.content.notes.len())
            .sum::<usize>()
            > crate::engine::project::MAX_TOTAL_NOTES
        {
            return Err("The group exceeds the native session's note limit".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let source = originals.clone();
        std::thread::Builder::new().name("midi-group-preview".into()).spawn(move|| {
            let result=(|| {
                let mut results=Vec::with_capacity(source.len());
                for(key,original)in source.iter() {
                    let mut owner_params=params.clone();
                    if matches!(owner_params.kind,midi_tools::Kind::ScaleTranspose|midi_tools::Kind::Harmony) && owner_params.context.is_none(){owner_params.context=original.context;}
                    let prepared=midi_tools::prepare(&original.content,&original.selected,&owner_params,&worker_cancel)?;
                    if !original.region.allows(&prepared.content.notes) {
                        return Err(format!("Track {} scene {} would exceed its own loop note-density limit; no group preview was published",key.0+1,key.1+1));
                    }
                    results.push((*key,prepared));
                }
                if worker_cancel.load(Ordering::Acquire) {return Err("Group preview cancelled; all drafts retained".into());}
                Ok(results)
            })();let _=sender.send(result);
        }).map_err(|e|format!("Group preview worker unavailable: {e}"))?;
        self.worker = Some(Worker {
            receiver,
            cancel,
            originals,
            guards,
        });
        Ok(())
    }
    /// Publish one completed group after checking every captured owner.
    /// Takes current mutable drafts; changes every prepared target together or retains every target on stale/cancelled/error results.
    pub fn poll(&mut self, mut drafts: BTreeMap<Key, &mut Draft>) -> Result<bool, String> {
        let Some(worker) = &self.worker else {
            return Ok(false);
        };
        let result = match worker.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Group preview worker disconnected; all drafts retained".into())
            }
        };
        let worker = self.worker.take().unwrap();
        if worker
            .guards
            .iter()
            .any(|(key, guard)| drafts.get(key).is_none_or(|draft| !guard.current(draft)))
        {
            return Err(
                "A group draft changed during preparation; every current draft was retained".into(),
            );
        }
        if worker.cancel.load(Ordering::Acquire) {
            return Err("Group preview cancelled; all drafts retained".into());
        }
        let results = result?;
        if results.len() != worker.guards.len()
            || results.iter().map(|(key, _)| key).ne(worker.guards.keys())
        {
            return Err(
                "Group preview did not return every original owner; all drafts retained".into(),
            );
        }
        self.summaries.clear();
        self.current.clear();
        for (key, mut prepared) in results {
            let original = &worker.originals[&key];
            let draft = drafts.get_mut(&key).unwrap();
            let controls_dirty =
                original.controls_dirty || !original.content.same_lanes(&prepared.content);
            draft.notes = std::mem::take(&mut prepared.content.notes);
            draft
                .controls
                .install(&mut prepared.content, controls_dirty);
            draft.dirty = true;
            draft.drag = None;
            self.summaries.push((key, prepared.summary));
            self.current.insert(key, Original::capture(draft));
        }
        self.originals = Some(worker.originals.clone());
        Ok(true)
    }
    /// Restore the exact original group without replacing committed clips.
    /// Takes the current owner map; restores content, selection and dirty state together or retains all stale drafts.
    pub fn restore(&mut self, mut drafts: BTreeMap<Key, &mut Draft>) -> Result<(), String> {
        if self.busy() {
            return Err("Wait or cancel the pending group preview before Restore".into());
        }
        let Some(originals) = &self.originals else {
            return Ok(());
        };
        if self
            .current
            .iter()
            .any(|(key, guard)| drafts.get(key).is_none_or(|draft| !guard.current(draft)))
        {
            return Err("A preview draft changed; current drafts retained rather than overwritten by Restore".into());
        }
        for (key, original) in originals.iter() {
            original.install(drafts.get_mut(key).unwrap());
        }
        self.clear();
        Ok(())
    }
    pub fn keep(&mut self) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the pending preview before Keep".into());
        }
        self.clear();
        Ok(())
    }
}
