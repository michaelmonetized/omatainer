use super::*;
use crate::engine::midi_interchange;
use std::collections::BTreeMap;
type Key = (u8, u16);
struct Input {
    baseline: Arc<Document>,
    name: String,
    region: Region,
    notes: Vec<MidiNote>,
    controls: control::Controls,
}
impl Input {
    fn capture(draft: &Draft) -> Self {
        Self {
            baseline: draft.baseline.clone(),
            name: draft.name.clone(),
            region: draft.region,
            notes: draft.notes.clone(),
            controls: draft.controls.clone(),
        }
    }
    fn current(&self, draft: &Draft) -> bool {
        Arc::ptr_eq(&self.baseline, &draft.baseline)
            && self.name == draft.name
            && self.region == draft.region
            && self.notes == draft.notes
            && draft.controls.matches(&self.controls.content(&self.notes))
    }
}
type Prepared = (midi_interchange::Request, Ack, Vec<Arc<Document>>);
struct Preparing {
    receiver: mpsc::Receiver<Result<Prepared, String>>,
    cancel: Arc<AtomicBool>,
    guards: BTreeMap<Key, Input>,
}
impl Drop for Preparing {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Pending {
    ack: Ack,
    next: Vec<Arc<Document>>,
}
#[derive(Clone, Copy)]
pub(super) enum Event {
    Queued,
    Applied,
    Cancelled,
}
impl Event {
    pub fn message(self) -> &'static str {
        match self {
            Self::Queued => "Applying all changed clips… Waiting for confirmation.",
            Self::Applied => "MIDI clips updated. Undo restores all of these changes together.",
            Self::Cancelled => "Apply cancelled. Your drafts are kept.",
        }
    }
}
#[derive(Default)]
pub(super) struct Batch {
    preparing: Option<Preparing>,
    pending: Option<Pending>,
}
impl Batch {
    pub fn discard_preparation(&mut self) {
        self.preparing = None;
    }

    pub fn busy(&self) -> bool {
        self.preparing.is_some() || self.pending.is_some()
    }
    pub fn cancel(&self) {
        if let Some(preparing) = &self.preparing {
            preparing.cancel.store(true, Ordering::Release);
        }
        if let Some(pending) = &self.pending {
            pending.ack.cancel();
        }
    }
    /// Prepare all changed MIDI drafts for one renderer history change.
    /// Takes captured owners and the existing engine; starts bounded cancellable lane/metadata preparation or retains every draft.
    pub fn start(&mut self, engine: &Engine, drafts: BTreeMap<Key, &Draft>) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the current combined Apply outcome".into());
        }
        if drafts.is_empty() || drafts.len() > 64 {
            return Err("Choose 1–64 captured MIDI drafts".into());
        }
        let guards: BTreeMap<_, _> = drafts
            .iter()
            .filter(|(_, d)| d.dirty)
            .map(|(key, d)| (*key, Input::capture(d)))
            .collect();
        if guards.is_empty() {
            return Err("The captured MIDI drafts contain no changes".into());
        }
        let inputs: Vec<_> = drafts
            .values()
            .filter(|draft| draft.dirty)
            .map(|draft| Input::capture(draft))
            .collect();
        let project = engine.project.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("midi-group-apply".into())
            .spawn(move || {
                let result = (|| {
                    let mut requests = Vec::with_capacity(inputs.len());
                    for input in inputs {
                        let lanes = input.controls.prepared(
                            &input.notes,
                            input.region.end,
                            &worker_cancel,
                        )?;
                        if worker_cancel.load(Ordering::Acquire) {
                            return Err("Combined Apply cancelled; drafts retained".into());
                        }
                        let (request, _, _) = Request::with_lanes(
                            input.baseline,
                            input.name,
                            input.region,
                            input.notes,
                            lanes,
                        )?;
                        requests.push(request);
                    }
                    let captured = project.capture(&worker_cancel).map_err(|e| e.to_string())?;
                    midi_interchange::Request::prepare_edits(captured, requests, &worker_cancel)
                })();
                let _ = sender.send(result);
            })
            .map_err(|e| format!("Combined MIDI preparation worker unavailable: {e}"))?;
        self.preparing = Some(Preparing {
            receiver,
            cancel,
            guards,
        });
        Ok(())
    }
    /// Resolve prepared admission and the single actual renderer acknowledgement.
    /// Takes current mutable owners and the engine; returns a truthful outcome, retaining drafts when publication is refused.
    pub fn poll(
        &mut self,
        engine: &Engine,
        mut drafts: BTreeMap<Key, &mut Draft>,
    ) -> Result<Option<Event>, String> {
        if let Some(preparing) = &self.preparing {
            let result = match preparing.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Combined preparation worker disconnected; drafts retained".into(),
                )),
            };
            if let Some(result) = result {
                let preparing = self.preparing.take().unwrap();
                if preparing.cancel.load(Ordering::Acquire) {
                    return Err("Combined Apply cancelled; drafts retained".into());
                }
                if preparing
                    .guards
                    .iter()
                    .any(|(key, input)| drafts.get(key).is_none_or(|draft| !input.current(draft)))
                {
                    return Err("A captured draft changed during preparation; no combined Apply was submitted".into());
                }
                let (request, ack, next) = result?;
                engine.send(Command::MidiImport(request)).map_err(|e| {
                    format!("Combined MIDI edit was not accepted: {e}; drafts retained")
                })?;
                self.pending = Some(Pending { ack, next });
                return Ok(Some(Event::Queued));
            }
        }
        if let Some(pending) = &self.pending {
            match pending.ack.state() {
                Outcome::Pending if engine.cmd.is_connected() => {}
                Outcome::Pending => {
                    self.pending = None;
                    return Err("Audio engine disconnected before confirming Apply. Outcome unknown; drafts kept.".into());
                }
                Outcome::Cancelled => {
                    self.pending = None;
                    return Ok(Some(Event::Cancelled));
                }
                Outcome::Rejected => {
                    self.pending = None;
                    return Err("Apply rejected because the project or a clip changed. All drafts are kept.".into());
                }
                Outcome::Applied => {
                    let pending = self.pending.take().unwrap();
                    for next in pending.next {
                        if let Some(draft) = drafts.get_mut(&(next.track, next.scene)) {
                            draft.baseline = next;
                            draft.dirty = false;
                            draft.controls.dirty = false;
                            draft.steps.clear();
                            draft.step_chord.clear();
                            draft.rhythm.committed();
                            draft.tools.committed();
                        }
                    }
                    return Ok(Some(Event::Applied));
                }
            }
        }
        Ok(None)
    }
}
