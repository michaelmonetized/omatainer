//! One prepared set, one active set and worker-owned retirement.
use super::{project::{Applied, Error, Prepared}, *};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
#[cfg(test)]
mod tests;

const QUEUED: u8 = 0;
const READY: u8 = 1;
const REQUESTED: u8 = 2;
const COMMITTED: u8 = 3;
const SETTLED: u8 = 4;

struct Transition {
    revision: u64,
    frames: u64,
    _permit: performance::ExclusivePermit,
}

pub(crate) struct Control {
    phase: AtomicU8,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) preview: AtomicBool,
    pub(crate) cue_available: AtomicBool,
    request: Sender<Transition>,
    incoming: Receiver<Transition>,
}
impl Control {
    /// Review readiness without touching the renderer graph.
    /// Takes this control; returns whether cue/transition admission is ready.
    pub(crate) fn ready(&self) -> bool { self.phase.load(Ordering::Acquire) == READY }
    /// Claim a reviewed transition.
    /// Takes the active owner, current revision and fade seconds; queues one transition or preserves the current mix.
    pub(crate) fn transition(&self, owner: &project::Handle, revision: u64, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || !(0.01..=30.0).contains(&seconds) { return Err("Fade must be between 0.01 and 30 seconds".into()); }
        let permit = owner.performance().project_change().map_err(|error| error.to_string())?;
        self.phase.compare_exchange(READY, REQUESTED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "The next set is not ready for a transition".to_string())?;
        let frames = (seconds * f64::from(owner.sample_rate())).round().max(2.0) as u64;
        if self.request.try_send(Transition { revision, frames, _permit: permit }).is_err() {
            self.phase.store(READY, Ordering::Release);
            return Err("The transition slot is unavailable".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Handle(Arc<Shared>);
pub(crate) struct Reservation {
    handle: Handle,
    loading: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) { if self.loading { self.handle.0.busy.store(false, Ordering::Release); } }
}
struct Shared {
    busy: AtomicBool,
    request: Sender<Box<Stage>>,
    incoming: Receiver<Box<Stage>>,
    retired: Sender<Box<Stage>>,
    returned: Receiver<Box<Stage>>,
    applied: Sender<Applied>,
    installed: Receiver<Applied>,
}

pub(crate) struct Stage {
    prepared: Prepared,
    control: Arc<Control>,
    namespace: [u64; 2],
    scratch: Box<[f32; 256]>,
    transition: Option<Transition>,
    fade: Option<(u64, u64)>,
    preview_gain: f32,
    finished: bool,
    error: Option<Error>,
    applied: Option<Applied>,
}
impl Stage {
    pub(super) fn transitioning(&self) -> bool { self.fade.is_some() && !self.finished }
}
impl Handle {
    pub(super) fn new() -> Self {
        let (request, incoming) = bounded(1);
        let (retired, returned) = bounded(1);
        let (applied, installed) = bounded(1);
        Self(Arc::new(Shared { busy: AtomicBool::new(false), request, incoming, retired, returned, applied, installed }))
    }
    /// Observe the reserved graph slot.
    /// Takes this handle; returns true until worker retirement completes.
    pub(crate) fn busy(&self) -> bool { self.0.busy.load(Ordering::Acquire) }
    /// Reserve the next-set slot before reading its file.
    /// Takes this handle; returns an owner that prevents project/device replacement until worker failure or graph retirement.
    pub(crate) fn reserve(&self) -> Result<Reservation, String> {
        self.0.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A next set is already reserved".to_string())?;
        Ok(Reservation { handle: self.clone(), loading: true })
    }
    /// Stage a complete graph from a worker.
    /// Takes prepared DSP/media, active namespace and a shared cancellation flag; returns controls for that one graph.
    pub(crate) fn stage(&self, mut reservation: Reservation, mut prepared: Prepared, namespace: [u64; 2], cancel: Arc<AtomicBool>) -> Result<Arc<Control>, String> {
        if !Arc::ptr_eq(&self.0, &reservation.handle.0) { return Err("The reserved next-set owner changed".into()); }
        if prepared.rt.routing.is_some() { return Err("Live-set transitions currently require the standard stereo master route".into()); }
        prepared.rt.playing = true;
        prepared.rt.recording = false;
        prepared.rt.resume_project_clips();
        for deck in &mut prepared.rt.decks { deck.playing = deck.audio.is_some(); }
        let (request, incoming) = bounded(1);
        let control = Arc::new(Control { phase: AtomicU8::new(QUEUED), cancel, preview: AtomicBool::new(false),
            cue_available: AtomicBool::new(false), request, incoming });
        let stage = Box::new(Stage { prepared, control: control.clone(), namespace, scratch: Box::new([0.0; 256]),
            transition: None, fade: None, preview_gain: 0.0, finished: false, error: None, applied: None });
        if self.0.request.try_send(stage).is_err() {
            return Err("The renderer staging slot is unavailable".into());
        }
        reservation.loading = false;
        Ok(control)
    }
    /// Receive the renderer's committed identity.
    /// Takes this handle on a polling worker; returns one application receipt without parking on audio's channel.
    pub(crate) fn applied(&self) -> Option<Applied> { self.0.installed.try_recv().ok() }
    /// Retire graph storage on a worker.
    /// Takes this handle; returns the authoritative outcome after dropping all retired media and DSP here.
    pub(crate) fn retire(&self) -> Option<Result<(), Error>> {
        let stage = self.0.returned.try_recv().ok()?;
        let result = match stage.error.as_ref() {
            Some(Error::Conflict) => Err(Error::Conflict),
            Some(Error::Busy) => Err(Error::Busy),
            Some(Error::Protected(error)) => Err(Error::Protected(*error)),
            _ if stage.fade.is_none() => Err(Error::Cancelled),
            _ => Ok(()),
        };
        drop(stage);
        self.0.busy.store(false, Ordering::Release);
        Some(result)
    }
}

impl RtEngine {
    pub(super) fn live_set_tick(&mut self, channels: usize) {
        let handle = self.project.live_sets();
        if self.live_set.is_none() { self.live_set = handle.0.incoming.try_recv().ok(); }
        let Some(mut stage) = self.live_set.take() else { return; };
        stage.control.cue_available.store(channels >= 4 && self.routing.is_none(), Ordering::Release);
        if stage.finished { self.live_set = Some(stage); return; }
        let committed = stage.fade.is_some();
        if self.performance.status().recovery {
            stage.error = Some(Error::Protected(performance::Error::Recovery));
            stage.finished = true;
        } else if !committed && stage.control.cancel.load(Ordering::Acquire) {
            stage.finished = true;
        } else if !committed && (stage.namespace != self.session.namespace || self.sr != stage.prepared.rt.sr
            || self.routing.is_some() || self.project_sealed || !self.sampler_assets.same_owner(&stage.prepared.rt.sampler_assets)) {
            stage.error = Some(Error::Conflict);
            stage.finished = true;
        } else if !committed {
            if stage.control.phase.load(Ordering::Acquire) == QUEUED { stage.control.phase.store(READY, Ordering::Release); }
            if stage.transition.is_none() { stage.transition = stage.control.incoming.try_recv().ok(); }
            if let Some(request) = &stage.transition {
                if self.recording || self.project.revision() != request.revision || self.cmd_rx.pending_project_ui_requests() {
                    stage.error = Some(Error::Conflict);
                    stage.finished = true;
                } else if self.undo.available() && self.cmd_rx.begin_project_install() && self.cmd_rx.is_empty() {
                    if self.undo.clear() {
                        stage.prepared.swap_into(self);
                        self.project.edited();
                        self.publish();
                        stage.applied = Some(Applied { checkpoint: self.undo.checkpoint(), revision: self.project.revision(),
                            playback_receipts: std::array::from_fn(|deck| self.decks[deck].load_receipt.clone()) });
                        stage.fade = Some((0, request.frames));
                        stage.control.phase.store(COMMITTED, Ordering::Release);
                    }
                    self.cmd_rx.end_project_install();
                }
            }
        }
        if stage.finished && stage.transition.is_some() { self.cmd_rx.end_project_install(); }
        if stage.transitioning() { self.audible.restart(); }
        self.live_set = Some(stage);
    }

    pub(super) fn live_set_mix(&mut self, output: &mut [f32], channels: usize) {
        let Some(mut stage) = self.live_set.take() else { return; };
        if !stage.finished {
            let preview = stage.fade.is_none() && channels >= 4 && stage.control.preview.load(Ordering::Acquire);
            if stage.fade.is_some() || preview || stage.preview_gain != 0.0 {
                for block in output.chunks_mut(channels * 128) {
                    let frames = block.len() / channels;
                    let active = if let Some((elapsed, total)) = stage.fade {
                        frames.min(total.saturating_sub(elapsed) as usize)
                    } else if !preview {
                        frames.min((stage.preview_gain * (self.sr * 0.02).max(1.0)).ceil() as usize)
                    } else { frames };
                    if active > 0 { stage.prepared.rt.process(&mut stage.scratch[..active * 2]); }
                    stage.scratch[active * 2..frames * 2].fill(0.0);
                    for (frame, auxiliary) in block.chunks_exact_mut(channels).zip(stage.scratch.chunks_exact(2)) {
                        if let Some((elapsed, total)) = &mut stage.fade {
                            let incoming = (*elapsed as f64 / (*total - 1) as f64).min(1.0) as f32;
                            if channels == 1 { frame[0] = frame[0] * incoming + (auxiliary[0] + auxiliary[1]) * 0.5 * (1.0 - incoming); }
                            else { for channel in 0..2 { frame[channel] = frame[channel] * incoming + auxiliary[channel] * (1.0 - incoming); } }
                            *elapsed += 1;
                            if *elapsed >= *total { stage.finished = true; }
                        } else if channels >= 4 {
                            let step = 1.0 / (self.sr * 0.02).max(1.0);
                            stage.preview_gain = if preview { (stage.preview_gain + step).min(1.0) } else { (stage.preview_gain - step).max(0.0) };
                            frame[2] = auxiliary[0] * stage.preview_gain;
                            frame[3] = auxiliary[1] * stage.preview_gain;
                        }
                    }
                }
            }
        }
        if let Some(applied) = stage.applied.take() {
            if let Err(error) = self.project.live_sets().0.applied.try_send(applied) { stage.applied = Some(error.into_inner()); }
        }
        if stage.finished && stage.applied.is_none() {
            stage.control.phase.store(SETTLED, Ordering::Release);
            if let Err(error) = self.project.live_sets().0.retired.try_send(stage) { self.live_set = Some(error.into_inner()); }
        } else { self.live_set = Some(stage); }
    }
}
