use super::*;
mod batch;
mod group;
mod selection;
pub(super) mod view;
use std::collections::BTreeMap;
type Key = (u8, u16);
pub(super) struct Companion {
    pub draft: Draft,
    pub offset: f64,
    pub tuning_hz: f32,
    pub editable: bool,
}
struct Loading {
    receiver: mpsc::Receiver<Result<Companion, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
pub(super) struct Comparison {
    pub clips: Vec<Companion>,
    pub offset: f64,
    pub tuning_hz: f32,
    pub multi: bool,
    pub criteria: selection::Criteria,
    loading: Option<Loading>,
    pub group: group::Group,
    pub batch: batch::Batch,
    pub add_track: u8,
    pub add_scene: u16,
}
impl Default for Comparison {
    fn default() -> Self {
        Self {
            clips: Vec::new(),
            offset: 0.0,
            tuning_hz: 440.0,
            multi: false,
            criteria: selection::Criteria::default(),
            loading: None,
            group: group::Group::default(),
            batch: batch::Batch::default(),
            add_track: 0,
            add_scene: 0,
        }
    }
}
fn key(draft: &Draft) -> Key {
    (draft.baseline.track, draft.baseline.scene)
}
impl Comparison {
    pub fn busy(&self) -> bool {
        self.loading.is_some() || self.group.busy() || self.batch.busy()
    }
    pub fn editing(&self, focus: &Draft) -> bool {
        !self.busy()
            && !self.group.previewed()
            && focus.tools.editing()
            && self.clips.iter().all(|c| c.draft.tools.editing())
    }
    pub fn dirty(&self) -> bool {
        self.clips.iter().any(|c| c.draft.dirty)
    }
    pub fn cancel(&self) {
        if let Some(loading) = &self.loading {
            loading.cancel.store(true, Ordering::Release);
        }
        self.group.cancel();
        self.batch.cancel();
    }
    /// Inspect one explicitly chosen session clip as a protected companion.
    /// Takes the engine, original focused owner and logical target; starts coherent worker capture without editing either clip.
    pub fn add(&mut self, engine: &Engine, focus: &Draft, target: Key) -> Result<(), String> {
        if !self.editing(focus) {
            return Err("Finish current preview or Apply before adding a comparison clip".into());
        }
        if self.clips.len() >= 63
            || target == key(focus)
            || self.clips.iter().any(|c| key(&c.draft) == target)
        {
            return Err(
                "Choose a different MIDI clip; comparison supports at most 64 owners".into(),
            );
        }
        let project = engine.project.clone();
        let baseline = focus.baseline.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("midi-comparison-inspect".into())
            .spawn(move || {
                let result = (|| {
                    let captured = project.capture(&worker_cancel).map_err(|e| e.to_string())?;
                    let layout = captured
                        .state
                        .session
                        .as_ref()
                        .ok_or("Session identity is unavailable")?;
                    if baseline.epoch != captured.checkpoint.epoch
                        || baseline.track_identity.is_none_or(|r| {
                            !layout.resolves(
                                crate::engine::session::Axis::Track,
                                baseline.track as usize,
                                r,
                            )
                        })
                        || baseline.scene_identity.is_none_or(|r| {
                            !layout.resolves(
                                crate::engine::session::Axis::Scene,
                                baseline.scene as usize,
                                r,
                            )
                        })
                    {
                        return Err(
                            "The original session owner changed; current draft retained".into()
                        );
                    }
                    if layout
                        .reference(crate::engine::session::Axis::Track, target.0 as usize)
                        .is_none()
                        || layout
                            .reference(crate::engine::session::Axis::Scene, target.1 as usize)
                            .is_none()
                    {
                        return Err("Choose an active MIDI clip for comparison".into());
                    }
                    let tuning_hz = captured
                        .state
                        .tracks
                        .get(target.0 as usize)
                        .ok_or("Unknown comparison track")?
                        .synth
                        .tuning_hz;
                    let baseline = Document::capture(captured, target.0, target.1)?;
                    if worker_cancel.load(Ordering::Acquire) {
                        return Err("Comparison inspection cancelled".into());
                    }
                    let draft = Draft::new(baseline);
                    let offset = draft.region.start;
                    Ok(Companion {
                        draft,
                        offset,
                        tuning_hz,
                        editable: false,
                    })
                })();
                let _ = sender.send(result);
            })
            .map_err(|e| format!("Comparison inspection worker unavailable: {e}"))?;
        self.loading = Some(Loading { receiver, cancel });
        Ok(())
    }
    pub fn poll_added(&mut self, focus: &Draft) -> Result<bool, String> {
        let Some(loading) = &self.loading else {
            return Ok(false);
        };
        let result = match loading.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Comparison worker disconnected; existing owners retained".into())
            }
        };
        let loading = self.loading.take().unwrap();
        if loading.cancel.load(Ordering::Acquire) {
            return Err("Comparison inspection cancelled".into());
        }
        let peer = result?;
        let notes = focus.notes.len()
            + peer.draft.notes.len()
            + self
                .clips
                .iter()
                .map(|c| c.draft.notes.len())
                .sum::<usize>();
        if notes > crate::engine::project::MAX_TOTAL_NOTES {
            return Err(
                "Comparison exceeds the project note limit; existing owners retained".into(),
            );
        }
        self.clips.push(peer);
        Ok(true)
    }
    /// Focus a captured companion while retaining all unapplied drafts.
    /// Takes the active draft and owned row; swaps only focus while preserving shared viewport, source regions, offsets and tuning.
    pub fn focus(&mut self, focus: &mut Draft, index: usize) -> Result<(), String> {
        if !self.editing(focus) {
            return Err("Keep or restore the current previews before switching focus".into());
        }
        let peer = self
            .clips
            .get_mut(index)
            .ok_or("Comparison owner is unavailable")?;
        let shared_left = focus.view_beat - focus.region.start + self.offset;
        let view = (focus.view_high, focus.beat_pixels, focus.row_pixels);
        std::mem::swap(focus, &mut peer.draft);
        std::mem::swap(&mut self.offset, &mut peer.offset);
        std::mem::swap(&mut self.tuning_hz, &mut peer.tuning_hz);
        peer.editable = false;
        focus.view_beat = shared_left + focus.region.start - self.offset;
        (focus.view_high, focus.beat_pixels, focus.row_pixels) = view;
        focus.drag = None;
        peer.draft.drag = None;
        Ok(())
    }
    /// Select by exact pitch, shared time and velocity over enabled owners.
    /// Takes the current focus; publishes all validated identity sets together and preserves ghost selections and musical content.
    pub fn select(&mut self, focus: &mut Draft) -> Result<(), String> {
        if !self.editing(focus) {
            return Err("Finish the current preview before changing group selection".into());
        }
        let mut selected = BTreeMap::new();
        selected.insert(
            key(focus),
            self.criteria
                .select(&focus.notes, focus.region, self.offset)?,
        );
        if self.multi {
            for peer in &self.clips {
                if peer.editable {
                    selected.insert(
                        key(&peer.draft),
                        self.criteria
                            .select(&peer.draft.notes, peer.draft.region, peer.offset)?,
                    );
                }
            }
        }
        focus.selected = selected.remove(&key(focus)).unwrap();
        for peer in &mut self.clips {
            if let Some(ids) = selected.remove(&key(&peer.draft)) {
                peer.draft.selected = ids;
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, focus: &Draft, index: usize) -> Result<(), String> {
        if !self.editing(focus) {
            return Err("Finish current preview or Apply before removing a comparison".into());
        }
        let peer = self
            .clips
            .get(index)
            .ok_or("Comparison owner is unavailable")?;
        if peer.draft.dirty {
            return Err("Apply or explicitly discard this group's changes before removing a modified comparison".into());
        }
        self.clips.remove(index);
        Ok(())
    }
    pub fn layers(&self) -> Vec<canvas::Layer<'_>> {
        self.clips
            .iter()
            .map(|peer| canvas::Layer {
                notes: &peer.draft.notes,
                source_start: peer.draft.region.start,
                shared_offset: peer.offset,
                owner: &peer.draft.name,
                track: peer.draft.baseline.track,
                scene: peer.draft.baseline.scene,
                editable: self.multi && peer.editable,
            })
            .collect()
    }
}

impl Comparison {
    fn owners<'a>(
        clips: &'a [Companion],
        focus: &'a Draft,
        multi: bool,
    ) -> BTreeMap<Key, &'a Draft> {
        let mut drafts = BTreeMap::from([(key(focus), focus)]);
        if multi {
            drafts.extend(
                clips
                    .iter()
                    .filter(|c| c.editable)
                    .map(|c| (key(&c.draft), &c.draft)),
            );
        }
        drafts
    }
    fn all_mut<'a>(
        clips: &'a mut [Companion],
        focus: Option<&'a mut Draft>,
    ) -> BTreeMap<Key, &'a mut Draft> {
        let mut drafts: BTreeMap<_, _> = clips
            .iter_mut()
            .map(|c| (key(&c.draft), &mut c.draft))
            .collect();
        if let Some(focus) = focus {
            drafts.insert(key(focus), focus);
        }
        drafts
    }
    pub fn poll(
        &mut self,
        engine: &Engine,
        mut focus: Option<&mut Draft>,
    ) -> Result<Option<&'static str>, String> {
        if let Some(draft) = focus.as_deref() {
            self.poll_added(draft)?;
        }
        self.group
            .poll(Self::all_mut(&mut self.clips, focus.as_deref_mut()))?;
        let event = self
            .batch
            .poll(engine, Self::all_mut(&mut self.clips, focus))?;
        if matches!(event, Some(batch::Event::Applied)) {
            self.group.clear();
        }
        Ok(event.map(batch::Event::message))
    }
    pub fn apply(&mut self, engine: &Engine, focus: &Draft) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for current comparison work before Apply".into());
        }
        let mut drafts = Self::owners(&self.clips, focus, true);
        drafts.extend(self.clips.iter().map(|c| (key(&c.draft), &c.draft)));
        self.batch.start(engine, drafts)
    }
    pub fn action(
        &mut self,
        engine: &Engine,
        focus: &mut Draft,
        action: view::Action,
    ) -> Result<(), String> {
        match action {
            view::Action::Add(target) => self.add(engine, focus, target),
            view::Action::Focus(index) => self.focus(focus, index),
            view::Action::Remove(index) => self.remove(focus, index),
            view::Action::Select => self.select(focus),
            view::Action::Preview => self.group.start(
                Self::owners(&self.clips, focus, self.multi)
                    .into_iter()
                    .filter(|(_, draft)| !draft.selected.is_empty())
                    .collect(),
                focus.tools.parameters()?,
            ),
            view::Action::Restore => self
                .group
                .restore(Self::all_mut(&mut self.clips, Some(focus))),
            view::Action::Keep => self.group.keep(),
            view::Action::Cancel => {
                self.cancel();
                Ok(())
            }
        }
    }
    pub fn discard(&mut self) {
        self.cancel();
        self.loading = None;
        self.group.clear();
        self.batch.discard_preparation();
        self.clips.clear();
    }

}

#[cfg(test)]
mod tests;
