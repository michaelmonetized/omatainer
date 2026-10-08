//! One prepared set, one active set and worker-owned retirement.
use super::{project::{Applied, Error, Prepared}, *};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use super::audio::routing::model::{Direction, Group, Tap, MAX_PHYSICAL_CHANNELS};
#[cfg(test)]
mod tests;

const QUEUED: u8 = 0;
const READY: u8 = 1;
const REQUESTED: u8 = 2;
const COMMITTED: u8 = 3;
const SETTLED: u8 = 4;

fn physical_input(rt: &RtEngine) -> bool {
    rt.routing.as_ref().is_some_and(|graph| graph.model.input.is_some()
        || graph.model.ports.iter().any(|port| port.direction == Direction::Input))
}

fn cue_free(rt: &RtEngine) -> bool {
    rt.routing.as_ref().is_none_or(|graph| graph.monitor_channels_free || graph.monitor_output.is_some_and(|(_, pair)| pair == [2, 3]))
}

fn priming(rt: &RtEngine) -> u32 {
    rt.routing.as_ref().map_or(0, |graph| graph.latency_status().priming_frames)
}

fn missing_outputs(rt: &RtEngine, channels: usize) -> bool {
    rt.routing.as_ref().is_some_and(|graph| graph.model.connections.iter().any(|route| {
        let Group::Output(id) = route.destination else { return false; };
        graph.model.ports.iter().find(|port| port.id == id).is_some_and(|port| route.map.iter().any(|map|
            map.gain != 0.0 && usize::from(port.channels[usize::from(map.destination)]) >= channels))
    }))
}

#[derive(Clone, Copy)]
enum Refusal {
    PhysicalInput,
    MissingOutputs,
    Priming(u32),
}
impl Refusal {
    fn message(self, rate: u32) -> String {
        match self {
            Self::PhysicalInput => "Sets with physical input routes must be opened while stopped; current performance and saved routes are retained".into(),
            Self::MissingOutputs => "Remap unavailable next-set outputs while stopped; current performance and saved routes are retained".into(),
            Self::Priming(frames) => format!("The next set still needs {:.4} seconds of processing history. Cue it first or use a longer fade; current performance is retained", f64::from(frames) / f64::from(rate)),
        }
    }
}

fn main_pair(rt: &RtEngine) -> Option<[usize; 2]> {
    let Some(graph) = &rt.routing else { return Some([0, 1]); };
    let mut pair = None;
    for port in graph.model.ports.iter().filter(|port| port.direction == Direction::Output && port.channels.len() == 2) {
        let mut routes = graph.model.connections.iter().filter(|route| route.destination == Group::Output(port.id));
        let Some(route) = routes.next() else { continue; };
        if routes.next().is_some() || route.source.group != Group::Main || route.source.tap != Tap::PostMixer || route.map.len() != 2
            || !(0..2).all(|channel| route.map.iter().any(|map| usize::from(map.source) == channel && usize::from(map.destination) == channel && map.gain == 1.0))
            || graph.model.ports.iter().any(|other| other.direction == Direction::Output && other.id != port.id && other.channels.iter().any(|channel| port.channels.contains(channel))) {
            continue;
        }
        if pair.is_some() { return None; }
        pair = Some([usize::from(port.channels[0]), usize::from(port.channels[1])]);
    }
    pair
}

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
    priming_frames: AtomicU64,
    request: Sender<Transition>,
    incoming: Receiver<Transition>,
}
impl Control {
    /// Review readiness without touching the renderer graph.
    /// Takes this control; returns whether cue/transition admission is ready.
    pub(crate) fn ready(&self) -> bool { self.phase.load(Ordering::Acquire) == READY }
    /// Read the next set's unfilled processing delay.
    /// Takes this control and output rate; returns seconds of history still needed before a short fade.
    pub(crate) fn priming_seconds(&self, rate: u32) -> f64 {
        self.priming_frames.load(Ordering::Acquire) as f64 / f64::from(rate)
    }
    /// Claim a reviewed transition.
    /// Takes the active owner, current revision and fade seconds; queues one transition or preserves the current mix.
    pub(crate) fn transition(&self, owner: &project::Handle, revision: u64, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || !(0.01..=30.0).contains(&seconds) { return Err("Fade must be between 0.01 and 30 seconds".into()); }
        let frames = (seconds * f64::from(owner.sample_rate())).round().max(2.0) as u64;
        let remaining = self.priming_frames.load(Ordering::Acquire);
        if remaining != 0 && frames <= remaining { return Err(Refusal::Priming(remaining as u32).message(owner.sample_rate())); }
        let permit = owner.performance().project_change().map_err(|error| error.to_string())?;
        self.phase.compare_exchange(READY, REQUESTED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "The next set is not ready for a transition".to_string())?;
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
    scratch: Box<[f32]>,
    preview_pair: Option<[usize; 2]>,
    refusal: Option<Refusal>,
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
        if physical_input(&prepared.rt) { return Err(Refusal::PhysicalInput.message(prepared.rt.sr as u32)); }
        if prepared.rt.routing.as_ref().is_some_and(|graph| graph.model.plugins.iter().any(|plugin| !plugin.bypass && plugin.unavailable.is_some())) {
            return Err("Next-set preflight refused an unavailable native plugin; current performance is retained".into());
        }
        prepared.rt.playing = true;
        prepared.rt.recording = false;
        prepared.rt.resume_project_clips();
        for deck in &mut prepared.rt.decks { deck.playing = deck.audio.is_some(); }
        let (request, incoming) = bounded(1);
        let control = Arc::new(Control { phase: AtomicU8::new(QUEUED), cancel, preview: AtomicBool::new(false),
            cue_available: AtomicBool::new(false), priming_frames: AtomicU64::new(u64::from(priming(&prepared.rt))), request, incoming });
        let preview_pair = main_pair(&prepared.rt);
        let stage = Box::new(Stage { prepared, control: control.clone(), namespace, scratch: vec![0.0; 128 * MAX_PHYSICAL_CHANNELS].into_boxed_slice(),
            preview_pair, refusal: None,
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
        let result = if let Some(refusal) = stage.refusal {
            Err(Error::Invalid(refusal.message(stage.prepared.rt.sr as u32)))
        } else { match stage.error.as_ref() {
            Some(Error::Conflict) => Err(Error::Conflict),
            Some(Error::Busy) => Err(Error::Busy),
            Some(Error::Protected(error)) => Err(Error::Protected(*error)),
            _ if stage.fade.is_none() => Err(Error::Cancelled),
            _ => Ok(()),
        }};
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
        let preview_channels = channels.min(MAX_PHYSICAL_CHANNELS);
        stage.control.cue_available.store(channels >= 4 && cue_free(self) && cue_free(&stage.prepared.rt)
            && stage.preview_pair.is_some_and(|pair| pair.iter().all(|channel| *channel < preview_channels)), Ordering::Release);
        if stage.finished { self.live_set = Some(stage); return; }
        let committed = stage.fade.is_some();
        if !committed { stage.control.priming_frames.store(u64::from(priming(&stage.prepared.rt)), Ordering::Release); }
        if self.performance.status().recovery {
            stage.error = Some(Error::Protected(performance::Error::Recovery));
            stage.finished = true;
        } else if !committed && stage.control.cancel.load(Ordering::Acquire) {
            stage.finished = true;
        } else if !committed && physical_input(self) {
            stage.refusal = Some(Refusal::PhysicalInput);
            stage.finished = true;
        } else if !committed && missing_outputs(&stage.prepared.rt, preview_channels) {
            stage.refusal = Some(Refusal::MissingOutputs);
            stage.finished = true;
        } else if !committed && (stage.namespace != self.session.namespace || self.sr != stage.prepared.rt.sr
            || self.project_sealed || !self.sampler_assets.same_owner(&stage.prepared.rt.sampler_assets)) {
            stage.error = Some(Error::Conflict);
            stage.finished = true;
        } else if !committed {
            if stage.control.phase.load(Ordering::Acquire) == QUEUED { stage.control.phase.store(READY, Ordering::Release); }
            if stage.transition.is_none() { stage.transition = stage.control.incoming.try_recv().ok(); }
            if let Some(request) = &stage.transition {
                let remaining = priming(&stage.prepared.rt);
                if remaining != 0 && request.frames <= u64::from(remaining) {
                    stage.refusal = Some(Refusal::Priming(remaining));
                    stage.finished = true;
                } else if self.recording || self.project.revision() != request.revision || self.cmd_rx.pending_project_ui_requests() {
                    stage.error = Some(Error::Conflict);
                    stage.finished = true;
                } else if self.undo.available() && self.cmd_rx.begin_project_install() && self.cmd_rx.is_empty() {
                    if self.undo.clear() {
                        stage.prepared.rt.monitor = self.monitor.clone();
                        stage.prepared.swap_into(self);
                        self.maintain_monitor(channels, true);
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
            let width = channels.min(MAX_PHYSICAL_CHANNELS);
            let preview = stage.fade.is_none() && stage.control.cue_available.load(Ordering::Acquire) && stage.control.preview.load(Ordering::Acquire);
            if stage.fade.is_none() && !stage.control.cue_available.load(Ordering::Acquire) { stage.preview_gain = 0.0; }
            if stage.fade.is_some() || preview || stage.preview_gain != 0.0 {
                for block in output.chunks_mut(channels * 128) {
                    let frames = block.len() / channels;
                    let active = if let Some((elapsed, total)) = stage.fade {
                        frames.min(total.saturating_sub(elapsed) as usize)
                    } else if !preview {
                        frames.min((stage.preview_gain * (self.sr * 0.02).max(1.0)).ceil() as usize)
                    } else { frames };
                    if active > 0 { stage.prepared.rt.process_interleaved(&mut stage.scratch[..active * width], width); }
                    stage.scratch[active * width..frames * width].fill(0.0);
                    for (frame, auxiliary) in block.chunks_exact_mut(channels).zip(stage.scratch.chunks_exact(width)) {
                        if let Some((elapsed, total)) = &mut stage.fade {
                            let incoming = (*elapsed as f64 / (*total - 1) as f64).min(1.0) as f32;
                            for (channel, value) in frame.iter_mut().enumerate() {
                                *value = *value * incoming + auxiliary.get(channel).copied().unwrap_or(0.0) * (1.0 - incoming);
                            }
                            *elapsed += 1;
                            if *elapsed >= *total { stage.finished = true; }
                        } else if channels >= 4 && cue_free(self) {
                            let step = 1.0 / (self.sr * 0.02).max(1.0);
                            stage.preview_gain = if preview { (stage.preview_gain + step).min(1.0) } else { (stage.preview_gain - step).max(0.0) };
                            if let Some(pair) = stage.preview_pair.filter(|pair| pair.iter().all(|channel| *channel < width)) {
                                frame[2] = auxiliary[pair[0]] * stage.preview_gain;
                                frame[3] = auxiliary[pair[1]] * stage.preview_gain;
                            }
                        }
                    }
                }
            }
        }
        if stage.fade.is_none() { stage.control.priming_frames.store(u64::from(priming(&stage.prepared.rt)), Ordering::Release); }
        if let Some(applied) = stage.applied.take() {
            if let Err(error) = self.project.live_sets().0.applied.try_send(applied) { stage.applied = Some(error.into_inner()); }
        }
        if stage.finished && stage.applied.is_none() {
            let phase = stage.control.phase.load(Ordering::Acquire);
            if stage.transition.is_none() {
                let pending = if phase == REQUESTED {
                    stage.transition = stage.control.incoming.try_recv().ok();
                    stage.transition.is_none()
                } else {
                    stage.control.phase.compare_exchange(phase, SETTLED, Ordering::AcqRel, Ordering::Acquire).is_err()
                };
                if pending { self.live_set = Some(stage); return; }
            }
            stage.control.phase.store(SETTLED, Ordering::Release);
            if let Err(error) = self.project.live_sets().0.retired.try_send(stage) { self.live_set = Some(error.into_inner()); }
        } else { self.live_set = Some(stage); }
    }
}
