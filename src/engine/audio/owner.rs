//! CPAL streams never leave this owner thread. Only immutable status and bounded
//! requests cross threads; the renderer itself moves through a single return slot.
use super::*;
use crate::engine::project::{CloseGuard, Handle as Project};
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    Switching,
    Calibrating,
    Offline,
}
#[derive(Clone, Debug)]
pub struct Status {
    pub generation: u64,
    pub phase: Phase,
    /// Backend-accepted logical settings; CPAL cannot expose physical negotiation.
    pub active: Option<OutputInfo>,
    pub requested: crate::preferences::Audio,
    pub message: String,
    pub measurement: Option<Arc<calibration::Evidence>>,
    pub callback_floor: u64,
    pub recovery: Option<recovery::Target>,
}
#[derive(Clone)]
pub struct Handle {
    requests: Sender<Request>,
    status: Arc<ArcSwap<Status>>,
    project: Project,
    stopped: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    safe_mode: bool,
}
pub struct AudioOut {
    pub handle: Handle,
}
impl Drop for AudioOut {
    fn drop(&mut self) {
        // Backend teardown may join an OS thread. The owner performs it, never
        // the GUI destructor. The detached owner exits after its current call.
        self.handle.stopped.store(true, Ordering::Release);
    }
}
enum Operation {
    Switch(crate::preferences::Audio, Option<config::Plan>),
    Calibrate(calibration::Request),
    Reset,
    Reconnect(u64),
}
struct Request {
    operation: Operation,
    seal: CloseGuard,
    cancel: Arc<AtomicBool>,
    result: Sender<Result<Arc<Status>, String>>,
}
impl Handle {
    pub fn status(&self) -> Arc<Status> {
        self.status.load_full()
    }
    pub fn safe_mode(&self) -> bool { self.safe_mode }
    /// Worker only. The GUI previews first and explicitly confirms stopping.
    pub fn switch(
        &self,
        settings: crate::preferences::Audio,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Status>, String> {
        self.transact(Operation::Switch(settings, None), cancel, self.performance_permit()?)
    }
    pub fn apply_preview(
        &self,
        settings: crate::preferences::Audio,
        expected: config::Plan,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Status>, String> {
        self.apply_preview_permitted(settings, expected, cancel, self.performance_permit()?)
    }
    pub fn calibrate(
        &self,
        request: calibration::Request,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Status>, String> {
        request.validate()?;
        self.calibrate_permitted(request, cancel, self.performance_permit()?)
    }
    pub(crate) fn performance_permit(&self) -> Result<crate::engine::performance::ExclusivePermit, String> {
        if self.safe_mode {return Err("Audio devices are disabled in safe mode; save your work and explicitly restart normally".into());}
        self.project.performance().audio_change().map_err(|error| error.to_string())
    }
    pub(crate) fn apply_preview_permitted(&self, settings: crate::preferences::Audio, expected: config::Plan, cancel: Arc<AtomicBool>, permit: crate::engine::performance::ExclusivePermit) -> Result<Arc<Status>, String> {
        self.transact(Operation::Switch(settings, Some(expected)), cancel, permit)
    }
    pub(crate) fn calibrate_permitted(&self, request: calibration::Request, cancel: Arc<AtomicBool>, permit: crate::engine::performance::ExclusivePermit) -> Result<Arc<Status>, String> {
        request.validate()?;
        self.transact(Operation::Calibrate(request), cancel, permit)
    }
    pub(crate) fn reset_permitted(&self, cancel: Arc<AtomicBool>, permit: crate::engine::performance::ExclusivePermit) -> Result<Arc<Status>, String> {
        self.transact(Operation::Reset, cancel, permit)
    }
    /// Reconnect the retained physical output without resuming playback.
    /// Takes cancellation and an admitted audio permit; returns actual owner status.
    pub(crate) fn reconnect_permitted(&self, cancel: Arc<AtomicBool>, permit: crate::engine::performance::ExclusivePermit, generation:u64) -> Result<Arc<Status>, String> {
        self.transact(Operation::Reconnect(generation), cancel, permit)
    }
    fn transact(
        &self,
        operation: Operation,
        cancel: Arc<AtomicBool>,
        permit: crate::engine::performance::ExclusivePermit,
    ) -> Result<Arc<Status>, String> {
        // Guard the permitted/internal entry points too. Preview cannot bypass
        // startup isolation by retaining a permit acquired elsewhere.
        if self.safe_mode {return Err("Audio devices are disabled in safe mode; save your work and explicitly restart normally".into());}
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("An audio operation is still pending".into());
        }
        struct Busy<'a>(&'a AtomicBool);
        impl Drop for Busy<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _busy = Busy(&self.busy);
        if self.stopped.load(Ordering::Acquire) {
            return Err("Audio owner has closed".into());
        }
        // A new accepted attempt invalidates prior evidence, even if cancelled
        // while waiting for the exclusive renderer boundary.
        self.status.rcu(|old| {
            let mut status = (**old).clone();
            status.measurement = None;
            Arc::new(status)
        });
        let seal = self
            .project
            .seal_for_audio_permitted(permit, &cancel)
            .map_err(|e| e.to_string())?;
        let (result, receipt) = bounded(1);
        self.requests
            .try_send(Request {
                operation,
                seal,
                cancel,
                result,
            })
            .map_err(|_| "Audio request lane is unavailable".to_string())?;
        // Once admitted, only the owner can say whether cancellation preceded
        // activation. A late cancellation must not misreport committed audio.
        loop {
            match receipt.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err("Audio owner closed before acknowledging the change".into())
                }
                Err(_) => {}
            }
        }
    }
}

pub(super) trait Backend: 'static {
    type Stream;
    fn select(&mut self, settings: &crate::preferences::Audio) -> Result<config::Plan, String>;
    /// Any failed open must drop the callback, returning its graph. A successful
    /// stream owns the callback until close, including after play fails.
    fn open(
        &mut self,
        plan: &config::Plan,
        callback: OutputCallback,
        fault: Arc<AtomicBool>,
        identity: Option<&str>,
    ) -> Result<Self::Stream, String>;
    fn identity(&mut self, plan: &config::Plan) -> Option<String>;
    fn reconnect(&mut self, target: &recovery::Target) -> Result<config::Plan, String>;
    fn play(&mut self, stream: &Self::Stream) -> Result<(), String>;
    fn calibrate(
        &mut self,
        request: &calibration::Request,
        cancel: &AtomicBool,
        stopped: &Arc<AtomicBool>,
    ) -> Result<calibration::Measurement, String>;
    fn close(&mut self, stream: Self::Stream) {
        drop(stream);
    }
}
struct Active<S> {
    stream: S,
    graph: Receiver<Box<RtEngine>>,
    enabled: Arc<AtomicBool>,
    fault: Arc<AtomicBool>,
    plan: config::Plan,
    callback_floor: u64,
    identity: Option<String>,
    telemetry: Arc<crate::engine::audio_metrics::Telemetry>,
    watchdog: recovery::Watchdog,
}
struct Owner<B: Backend> {
    backend: B,
    active: Option<Active<B::Stream>>,
    graph: Option<Box<RtEngine>>,
    status: Arc<ArcSwap<Status>>,
    stopped: Arc<AtomicBool>,
    last_offline_publish: std::time::Instant,
    recovery: Option<recovery::Target>,
}
impl<B: Backend> Owner<B> {
    fn publish(
        &self,
        phase: Phase,
        requested: crate::preferences::Audio,
        message: String,
    ) -> Arc<Status> {
        let previous = self.status.load();
        let value = Arc::new(Status {
            generation: previous.generation + 1,
            phase,
            active: self
                .active
                .as_ref()
                .map(|active| OutputInfo::from(active.plan.clone())),
            requested,
            message,
            measurement: None,
            callback_floor: self
                .active
                .as_ref()
                .map(|a| a.callback_floor)
                .unwrap_or(u64::MAX),
            recovery: self.recovery.clone(),
        });
        self.status.store(value.clone());
        value
    }
    fn reclaim(&mut self) -> Option<config::Plan> {
        let active = self.active.take()?;
        active.enabled.store(false, Ordering::Release);
        self.backend.close(active.stream);
        // On Linux CPAL/ALSA, Stream::drop wakes and joins the callback worker.
        // Keep the return receiver alive until the unique lease arrives. If a
        // backend violates this contract, fail closed here rather than drop or
        // manufacture a second graph. No creative admission is reopened.
        self.graph = active.graph.recv().ok();
        Some(active.plan)
    }
    fn open(&mut self, plan: config::Plan, cancel: &AtomicBool) -> Result<(), String> {
        self.open_bound(plan, cancel, None)
    }
    fn open_bound(&mut self, plan: config::Plan, cancel: &AtomicBool, required: Option<&str>) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
            return Err("Audio operation cancelled or owner shutting down".into());
        }
        let identity = self.backend.identity(&plan);
        if required.is_some_and(|expected| identity.as_deref() != Some(expected)) {
            return Err("Retained physical output changed; no other output was opened".into());
        }
        let mut graph = self
            .graph
            .take()
            .ok_or("Renderer ownership has not returned")?;
        if let Err(error) = graph.set_sample_rate(plan.rate) {
            self.graph = Some(graph);
            return Err(error);
        }
        let (returned, receiver) = bounded(1);
        let enabled = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(AtomicBool::new(false));
        let callback_floor = graph.telemetry.read().callbacks;
        let telemetry = graph.telemetry.clone();
        graph.cmd_rx.set_audio_offline(false);
        let callback = OutputCallback::managed(
            graph,
            plan.channels as usize,
            returned,
            enabled.clone(),
            self.stopped.clone(),
        );
        let stream = match self.backend.open(&plan, callback, fault.clone(), identity.as_deref()) {
            Ok(stream) => stream,
            Err(error) => {
                self.graph = receiver.recv().ok();
                return Err(error);
            }
        };
        let result = self.backend.play(&stream);
        // This RMW is the commit point against cancellation: cancellation
        // ordered before it wins; a later request cannot undo applied output.
        let commit = result.is_ok()
            && !fault.load(Ordering::Acquire)
            && !self.stopped.load(Ordering::Acquire)
            && cancel
                .compare_exchange(false, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
        if !commit {
            self.backend.close(stream);
            self.graph = receiver.recv().ok();
            return Err(result.err().unwrap_or_else(|| {
                if cancel.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
                    "Audio change cancelled before activation".into()
                } else {
                    "Output failed before activation".into()
                }
            }));
        }
        // Clear offline admission while the request's exclusive seal is still
        // held. Releasing that seal is separately acknowledged by the callback.
        // The graph cannot be touched here after transfer.
        enabled.store(true, Ordering::Release);
        let watchdog = recovery::Watchdog::new(recovery::boot_time().unwrap_or_default(), callback_floor,
            Duration::from_secs_f64(plan.buffer.unwrap_or(0) as f64 / plan.rate.max(1) as f64));
        self.active = Some(Active {
            stream,
            graph: receiver,
            enabled,
            fault,
            plan,
            callback_floor,
            identity,
            telemetry,
            watchdog,
        });
        self.recovery = None;
        Ok(())
    }
    fn dispatch(&mut self, request: Request) {
        let Request {
            operation,
            seal,
            cancel,
            result,
        } = request;
        match operation {
            Operation::Switch(settings, expected) => {
                self.switch(settings, expected, seal, cancel, result, false, None)
            }
            Operation::Calibrate(request) => self.calibrate(request, seal, cancel, result),
            Operation::Reconnect(generation) => {
                let state=self.status.load_full();
                if state.generation!=generation || state.phase!=Phase::Offline {drop(seal);let _=result.send(Err("Recovery output changed; review it again".into()));return;}
                let requested = state.requested.clone();
                let target = self.recovery.clone();
                let attempted = if self.active.is_some() {
                    Err("Output is already active; reconnect refused".into())
                } else {
                    target.as_ref().ok_or_else(|| "No retained output; preview and confirm an output explicitly".into())
                        .and_then(|target| self.backend.reconnect(target)
                            .and_then(|plan| self.open_bound(plan, &cancel, target.identity.as_deref())))
                };
                let status = match attempted {
                    Ok(()) => {
                        self.publish(Phase::Running, requested, "Retained physical output reconnected. Playback remains stopped; release physical inputs and acknowledge recovery before pressing Play.".into())
                    }
                    Err(error) => {
                        if self.active.is_none() {
                            if let Some(graph)=&mut self.graph {graph.cmd_rx.set_audio_offline(true);}
                            self.publish(Phase::Offline, requested, format!("Reconnect failed: {error}. Session retained; no fallback was opened"));
                        }
                        drop(seal); let _=result.send(Err(error)); return;
                    }
                };
                drop(seal); let _=result.send(Ok(status));
            }
            Operation::Reset => {
                let state = self.status.load_full();
                let Some(active) = &state.active else {
                    drop(seal); let _=result.send(Err("Reconnect the retained output or preview and confirm a fallback before resetting DSP".into())); return;
                };
                let identity=self.active.as_ref().and_then(|active|active.identity.clone());
                self.switch(recovery::settings(&active.plan), Some(active.plan.clone()), seal, cancel, result, true, identity);
            }
        }
    }
    fn switch(
        &mut self,
        settings: crate::preferences::Audio,
        expected: Option<config::Plan>,
        seal: CloseGuard,
        cancel: Arc<AtomicBool>,
        result: Sender<Result<Arc<Status>, String>>,
        reset: bool,
        identity: Option<String>,
    ) {
        if cancel.load(Ordering::Acquire) {
            let _ = result.send(Err("Audio change cancelled; active output preserved".into()));
            return;
        }
        // Validate against current backend capabilities before stopping audio.
        let plan = match self.backend.select(&settings) {
            Ok(plan) if expected.as_ref().is_none_or(|expected| expected == &plan) => plan,
            Ok(_) => {
                let _ = result.send(Err(
                    "Device capabilities/default changed since preview; preview again".into(),
                ));
                return;
            }
            Err(error) => {
                let _ = result.send(Err(error));
                return;
            }
        };
        self.publish(
            Phase::Switching,
            settings.clone(),
            "Stopping playback for confirmed device change".into(),
        );
        let previous_target = self.active.as_ref().map(|active| recovery::Target {
            plan: active.plan.clone(), identity: active.identity.clone(),
        }).or_else(|| self.recovery.clone());
        let previous = self.reclaim();
        if let Some(graph) = &mut self.graph {
            graph.stop_for_audio();
            if reset { graph.safety_output = crate::engine::performance::Output::default(); }
            graph.cmd_rx.set_audio_offline(false);
        }
        let attempted = self.open_bound(plan, &cancel, identity.as_deref());
        let status = match attempted {
            Ok(()) => self.publish(
                Phase::Running,
                settings,
                if reset { "Stopped DSP reset completed; output unmuted, playback remains stopped".into() } else { "Audio change applied; playback remains stopped".into() },
            ),
            Err(error) => {
                if reset { if let Some(graph) = &mut self.graph { graph.safety_output.retain_mute(); graph.performance.publish_output(&graph.safety_output); } }
                let rollback =
                    previous.and_then(|plan| self.open_bound(plan, &AtomicBool::new(false),previous_target.as_ref().and_then(|target|target.identity.as_deref())).err());
                if self.active.is_some() {
                    self.publish(Phase::Running, settings, format!("Audio change failed: {error}. Previous output restored; playback remains stopped"))
                } else {
                    self.recovery=previous_target;
                    if let Some(graph) = &mut self.graph {
                        graph.cmd_rx.set_audio_offline(true);
                    }
                    self.publish(Phase::Offline, settings, format!("Audio change failed: {error}. Recovery output unavailable: {}. Session retained; Save, New/Open and Close remain available", rollback.unwrap_or_else(|| "no previous active output".into())))
                }
            }
        };
        // Does not clear another operation's gate. Offline owner processing
        // releases only this guard, then services later project requests.
        drop(seal);
        let _ = result.send(Ok(status));
    }
    fn calibrate(
        &mut self,
        request: calibration::Request,
        seal: CloseGuard,
        cancel: Arc<AtomicBool>,
        result: Sender<Result<Arc<Status>, String>>,
    ) {
        if cancel.load(Ordering::Acquire)
            || self
                .active
                .as_ref()
                .is_none_or(|active| active.plan != request.output)
        {
            let _ = result.send(Err(
                "Calibration cancelled or the active output changed; preview again".into(),
            ));
            return;
        }
        let settings = self.status.load().requested.clone();
        self.publish(
            Phase::Calibrating,
            settings.clone(),
            "Confirmed loopback calibration; session playback stopped".into(),
        );
        let identity=self.active.as_ref().and_then(|active|active.identity.clone());
        let previous = self.reclaim().expect("validated active stream");
        if let Some(graph) = &mut self.graph {
            graph.stop_for_audio();
        }
        let measurement = self.backend.calibrate(&request, &cancel, &self.stopped);
        let restored = self.open_bound(previous.clone(), &AtomicBool::new(false), identity.as_deref());
        let mut status = match restored {
            Ok(()) => (*self.publish(
                Phase::Running,
                settings,
                "Calibration finished; session output restored, playback remains stopped".into(),
            ))
            .clone(),
            Err(error) => {
                self.recovery=Some(recovery::Target {plan:previous,identity});
                if let Some(graph) = &mut self.graph {
                    graph.cmd_rx.set_audio_offline(true);
                }
                (*self.publish(Phase::Offline,settings,format!("Calibration finished but session output could not reopen: {error}. Session retained; Save and Close remain available"))).clone()
            }
        };
        match measurement {
            Ok(measured) if !cancel.load(Ordering::Acquire) && status.phase == Phase::Running => {
                status.measurement = Some(Arc::new(calibration::Evidence {
                    identity: request,
                    measured,
                }))
            }
            Ok(_) => status
                .message
                .push_str(". No current measurement: cancelled or output unavailable"),
            Err(error) => status.message.push_str(&format!(". {error}")),
        }
        let status = Arc::new(status);
        self.status.store(status.clone());
        drop(seal);
        let _ = result.send(Ok(status));
    }
    fn tick_offline(&mut self) {
        if let Some(graph) = &mut self.graph {
            graph.process(&mut []);
            graph.stop_offline_onsets();
            // Zero-frame servicing cannot trigger the renderer's frame-count
            // publication cadence. Retry independently so a temporarily held
            // reader cannot strand project install's display acknowledgment.
            if self.last_offline_publish.elapsed() >= Duration::from_millis(125) {
                graph.publish();
                self.last_offline_publish = std::time::Instant::now();
            }
        }
    }
    fn run(&mut self, requests: Receiver<Request>) {
        while !self.stopped.load(Ordering::Acquire) {
            if self.active.as_mut().is_some_and(|active| active.fault.load(Ordering::Acquire)
                || recovery::boot_time().is_some_and(|now| active.watchdog.lost(now, active.telemetry.read().callbacks)))
            {
                self.recovery = self.active.as_ref().map(|active| recovery::Target {
                    plan: active.plan.clone(), identity: active.identity.clone(),
                });
                self.reclaim();
                if let Some(graph) = &mut self.graph {
                    graph.cmd_rx.set_audio_offline(true);
                    graph.performance.request_safety(crate::engine::performance::Safety::Stop);
                    graph.stop_for_audio();
                    graph.process(&mut []);
                }
                self.publish(
                    Phase::Offline,
                    self.status.load().requested.clone(),
                    "Audio output was lost or stopped calling back. Session retained and recording finalized; reconnect the retained output or explicitly preview a fallback. Release physical inputs before resuming. Save and Close remain available."
                        .into(),
                );
            }
            self.tick_offline();
            match requests.recv_timeout(Duration::from_millis(2)) {
                Ok(request) => self.dispatch(request),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
        }
        self.reclaim();
        // Graph/streams/media retire on this owner, never the UI or callback.
    }
}

pub(super) fn start_with<B: Backend>(
    rt: RtEngine,
    settings: crate::preferences::Audio,
    create: impl FnOnce() -> B + Send + 'static,
) -> anyhow::Result<AudioOut> {
    start_owned(rt,settings,create,false)
}

/// No CPAL host, device enumeration, input stream or MIDI manager is created.
/// The same retained-graph service loop as backend failure still supports real
/// project capture/install, recovery and clean close while transport is offline.
pub fn start_safe(rt:RtEngine)->anyhow::Result<AudioOut> {
    start_owned(rt,crate::preferences::Audio::default(),||Unavailable,true)
}
struct Unavailable;
impl Backend for Unavailable {
    type Stream=();
    fn select(&mut self,_:&crate::preferences::Audio)->Result<config::Plan,String>{Err("Safe mode has no audio backend".into())}
    fn open(&mut self,_:&config::Plan,_:OutputCallback,_:Arc<AtomicBool>,_:Option<&str>)->Result<(),String>{Err("Safe mode has no audio backend".into())}
    fn identity(&mut self,_:&config::Plan)->Option<String>{None}
    fn reconnect(&mut self,_:&recovery::Target)->Result<config::Plan,String>{Err("Safe mode has no audio backend".into())}
    fn play(&mut self,_:&())->Result<(),String>{Err("Safe mode has no audio backend".into())}
    fn calibrate(&mut self,_:&calibration::Request,_:&AtomicBool,_:&Arc<AtomicBool>)->Result<calibration::Measurement,String>{Err("Safe mode has no audio backend".into())}
}
fn start_owned<B:Backend>(
    rt:RtEngine,
    settings:crate::preferences::Audio,
    create:impl FnOnce()->B+Send+'static,
    safe_mode:bool,
)->anyhow::Result<AudioOut> {
    let project = rt.project.clone();
    let status = Arc::new(ArcSwap::from_pointee(Status {
        generation: 0,
        phase: Phase::Switching,
        active: None,
        requested: settings.clone(),
        message: "Opening configured output".into(),
        measurement: None,
        callback_floor: u64::MAX,
        recovery: None,
    }));
    let stopped = Arc::new(AtomicBool::new(false));
    let (send, requests) = bounded(1);
    let handle = Handle {
        requests: send,
        status: status.clone(),
        project,
        stopped: stopped.clone(),
        busy: Arc::new(AtomicBool::new(false)),
        safe_mode,
    };
    let (ready, started) = bounded(1);
    std::thread::Builder::new()
        .name("omatainer-audio-owner".into())
        .spawn(move || {
            let mut owner = Owner {
                backend: create(),
                active: None,
                graph: Some(Box::new(rt)),
                status,
                stopped,
                last_offline_publish: std::time::Instant::now(),
                recovery: None,
            };
            if safe_mode {
                let graph=owner.graph.as_mut().expect("safe owner keeps its graph");
                graph.cmd_rx.set_audio_offline(true);
                graph.stop_for_audio();
                owner.publish(Phase::Offline, settings, "Safe mode: audio and MIDI devices are disabled. Project Open, recovery and Save remain available; restart normally to enable playback.".into());
                let _=ready.send(Ok(()));
                owner.run(requests);
                return;
            }
            let result = owner
                .backend
                .select(&settings)
                .and_then(|plan| owner.open(plan, &AtomicBool::new(false)));
            match result {
                Ok(()) => {
                    owner.publish(Phase::Running, settings, "Configured output running".into());
                    let _ = ready.send(Ok(()));
                    owner.run(requests);
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
        })?;
    started.recv()?.map_err(anyhow::Error::msg)?;
    Ok(AudioOut { handle })
}

impl RtEngine {
    /// Unique audio-owner access only, after callback retirement. Preserve clip
    /// launch positions for explicit Play, but end captured/physical gates.
    fn stop_for_audio(&mut self) {
        let launches = std::array::from_fn::<_, { crate::engine::TRACKS }, _>(|i| {
            self.tracks[i].playing.or(self.tracks[i].project_resume)
        });
        self.performance_tick();
        self.apply(crate::engine::Command::Stop);
        let midi_beat = self.precise_midi_beat();
        for (track, launch) in self.tracks.iter_mut().zip(launches) {
            track.project_resume = launch.map(|mut launch| {
                launch.last_beat = -0.0001;
                launch
            });
            track.rebuild_midi_schedule(self.beat, midi_beat);
            track.poly.set_sample_rate(self.sr);
            track.eq.set_sample_rate(self.sr);
            track.eq_right.set_sample_rate(self.sr);
            track.drum_pos.fill(None);
            for slot in &mut track.fx.slots {
                slot.reset_for_audio();
            }
        }
        self.sampler_poly.set_sample_rate(self.sr);
        self.pad_targets.fill(None);
        self.pad_voices.fill(None);
        if let Some(active) = &self.sampler_audition { active.ended(); }
        self.sampler_audition = None;
        self.pad_output.fill([0.0; 2]);
        for chain in &mut self.scene_fx {
            for slot in &mut chain.slots {
                slot.reset_for_audio();
            }
        }
        for (slot, kind) in self.master_fx.iter_mut().zip(self.fx_kind) {
            slot.reset(kind);
        }
        if let Some(history) = &mut self.history_measurement { history.reset_dsp(); }
        for deck in &mut self.decks {
            deck.playing = false;
            deck.touching = false;
            deck.touch_sources.fill(None);
            deck.scratch = 0.0;
            for eq in &mut deck.eq {
                eq.set_sample_rate(self.sr);
            }
            deck.filter = [crate::engine::deck_filter::ChannelFilter::default(); 2];
            deck.last_output = [0.0; 2];
            deck.transition_to(deck.pos, self.sr, crate::engine::DeckTransition::Jump);
        }
        self.publish();
    }
    fn stop_offline_onsets(&mut self) {
        // Commands admitted before a spontaneous device failure may still be
        // in flight. Service their data edits, but never advance or leave a gate
        // sounding when the output is subsequently recovered.
        if self.playing
            || self.recording
            || self.compose_target.is_some()
            || self.decks.iter().any(|deck| deck.playing || deck.touching)
            || self.has_held_project_notes()
            || self.sampler_audition.is_some()
        {
            self.stop_for_audio();
        }
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::{Command, CommandPort, Engine, SubmissionError};
    use std::sync::atomic::AtomicUsize;
    use std::thread::JoinHandle;
    use std::time::Instant;
    #[derive(Default)]
    pub(crate) struct Controls {
        pub failures: parking_lot::Mutex<std::collections::VecDeque<bool>>,
        pub play_failures: parking_lot::Mutex<std::collections::VecDeque<bool>>,
        pub active_fault: parking_lot::Mutex<Option<Arc<AtomicBool>>>,
        pub block_open: AtomicBool,
        pub entering_open: AtomicBool,
        pub calibration_mode: AtomicUsize,
        pub calibration_block: AtomicBool,
        pub entering_calibration: AtomicBool,
        pub opens: AtomicUsize,
        pub alternate_device: AtomicBool,
        pub device_renamed: AtomicBool,
        pub identity_unavailable: AtomicBool,
        pub stalled: AtomicBool,
        pub renders: AtomicUsize,
        pub dropped: AtomicUsize,
    }
    pub(crate) struct Fake {
        pub controls: Arc<Controls>,
    }
    pub(crate) struct Stream {
        stop: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
    }
    impl Drop for Stream {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(join) = self.join.take() {
                join.join().unwrap();
            }
        }
    }
    impl Backend for Fake {
        type Stream = Stream;
        fn select(&mut self, settings: &crate::preferences::Audio) -> Result<config::Plan, String> {
            let device = if self.controls.alternate_device.load(Ordering::Acquire) { "Alternate fixture" }
                else if self.controls.device_renamed.load(Ordering::Acquire) { "Renumbered fixture" } else { "Fixture" };
            if settings
                .device
                .as_deref()
                .is_some_and(|name| name != device)
            {
                return Err("Requested fixture output missing".into());
            }
            let rate = settings.sample_rate.unwrap_or(48000);
            if ![44100, 48000, 96000, 192000].contains(&rate) {
                return Err("Unsupported fixture rate".into());
            }
            Ok(config::Plan {
                backend: "Fixture".into(),
                device: device.into(),
                channels: settings.channels.unwrap_or(2),
                rate,
                format: cpal::SampleFormat::F32,
                buffer: settings.buffer_frames,
                warning: None,
            })
        }
        fn open(
            &mut self,
            _plan: &config::Plan,
            mut callback: OutputCallback,
            fault: Arc<AtomicBool>,
            identity: Option<&str>,
        ) -> Result<Stream, String> {
            if identity.is_some_and(|expected| self.identity(_plan).as_deref() != Some(expected)) {
                return Err("Fixture identity changed before opening".into());
            }
            self.controls.opens.fetch_add(1, Ordering::AcqRel);
            self.controls.entering_open.store(true, Ordering::Release);
            while self.controls.block_open.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.controls.entering_open.store(false, Ordering::Release);
            if self.controls.failures.lock().pop_front().unwrap_or(false) {
                return Err("Injected backend open failure".into());
            }
            *self.controls.active_fault.lock() = Some(fault);
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let controls = self.controls.clone();
            let join = std::thread::spawn(move || {
                let mut out = [0.0_f32; 256];
                while !stopped.load(Ordering::Acquire) {
                    if !controls.stalled.load(Ordering::Acquire) {
                        callback.render(&mut out);
                        controls.renders.fetch_add(1, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                drop(callback);
                controls.dropped.fetch_add(1, Ordering::Release);
            });
            Ok(Stream {
                stop,
                join: Some(join),
            })
        }
        fn identity(&mut self,_:&config::Plan)->Option<String> {
            if self.controls.identity_unavailable.load(Ordering::Acquire) {None}
            else {Some(if self.controls.alternate_device.load(Ordering::Acquire) {"unit-2"} else {"unit-1"}.into())}
        }
        fn reconnect(&mut self,target:&recovery::Target)->Result<config::Plan,String> {
            let expected=target.identity.as_ref().ok_or("No verified fixture identity")?;
            let mut settings=recovery::settings(&target.plan);settings.device=None;
            let plan=self.select(&settings)?;
            if self.identity(&plan).as_ref()!=Some(expected) {return Err("Retained fixture identity unavailable".into());}
            Ok(plan)
        }
        fn play(&mut self, _stream: &Stream) -> Result<(), String> {
            if self
                .controls
                .play_failures
                .lock()
                .pop_front()
                .unwrap_or(false)
            {
                Err("Injected backend play failure".into())
            } else {
                Ok(())
            }
        }
        fn calibrate(
            &mut self,
            request: &calibration::Request,
            cancel: &AtomicBool,
            stopped: &Arc<AtomicBool>,
        ) -> Result<calibration::Measurement, String> {
            self.controls
                .entering_calibration
                .store(true, Ordering::Release);
            while self.controls.calibration_block.load(Ordering::Acquire)
                && !cancel.load(Ordering::Acquire)
                && !stopped.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.controls
                .entering_calibration
                .store(false, Ordering::Release);
            if cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
                return Err("Calibration cancelled; no measurement".into());
            }
            match self.controls.calibration_mode.load(Ordering::Acquire) {
                0 => calibration::tests::synthetic(&request.output, 20_000_000),
                1 => Err("No measurement: a reliable loopback probe was not found".into()),
                _ => Err("Calibration timed out; insufficient callback evidence".into()),
            }
        }
    }
    pub(crate) fn fixture() -> (Engine, AudioOut, Arc<Controls>) {
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let controls = Arc::new(Controls::default());
        let c = controls.clone();
        let audio = start_with(rt, crate::preferences::Audio::default(), move || Fake {
            controls: c,
        })
        .unwrap();
        (engine, audio, controls)
    }
    pub(crate) fn engine_fixture() -> (Engine, Arc<Controls>) {
        let (mut engine, audio, controls) = fixture();
        engine._audio = Some(audio);
        (engine, controls)
    }
    pub(crate) fn publish_phase_for_ui_test(handle: &Handle, phase: Phase) {
        handle.status.rcu(|old| {
            let mut status = (**old).clone();
            status.phase = phase.clone();
            Arc::new(status)
        });
    }
    fn cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    fn settings(rate: u32) -> crate::preferences::Audio {
        let mut a = crate::preferences::Audio::default();
        a.sample_rate = Some(rate);
        a
    }
    fn wait(mut ready: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ready() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "fixture did not progress"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn accepted(commands: &CommandPort, command: Command) {
        for _ in 0..100 {
            match commands.send(command.clone()) {
                Ok(_) => return,
                Err(SubmissionError::ProjectChanging) => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                other => panic!("{other:?}"),
            }
        }
        panic!("gate remained closed");
    }
    #[test]
    fn supported_switches_preserve_graph_controls_stop_performance_and_refresh_project_rate() {
        let (engine, audio, controls) = fixture();
        for rate in [44100, 48000, 96000, 192000] {
            accepted(&engine.cmd, Command::Master(0.37));
            accepted(
                &engine.cmd,
                Command::LiveNoteOn {
                    source: 7,
                    ch: 0,
                    note: 64,
                    vel: 100,
                },
            );
            let status = audio.handle.switch(settings(rate), cancel()).unwrap();
            assert_eq!(status.phase, Phase::Running);
            assert_eq!(status.active.as_ref().unwrap().plan.rate, rate);
            assert_eq!(engine.sr(), rate);
            let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
            assert_eq!(captured.state.master, 0.37);
            wait(|| engine.snapshot().decks.iter().all(|deck| !deck.playing));
            assert!(controls.renders.load(Ordering::Acquire) > 0);
        }
        assert_eq!(controls.dropped.load(Ordering::Acquire), 4);
    }
    #[test]
    fn failed_target_rolls_back_and_double_failure_still_services_project_save_open_close() {
        let (engine, audio, controls) = fixture();
        controls.failures.lock().extend([true, false]);
        let status = audio.handle.switch(settings(96000), cancel()).unwrap();
        assert_eq!(status.phase, Phase::Running);
        assert_eq!(status.active.as_ref().unwrap().plan.rate, 48000);
        assert!(status.message.contains("Previous output restored"));
        controls.failures.lock().extend([true, true]);
        let status = audio.handle.switch(settings(96000), cancel()).unwrap();
        assert_eq!(status.phase, Phase::Offline);
        assert!(status.active.is_none());
        assert!(matches!(
            engine.cmd.send(Command::Master(0.1)),
            Err(SubmissionError::ProjectChanging | SubmissionError::AudioUnavailable)
        ));
        assert!(engine
            .cmd
            .send(Command::LiveNoteOff {
                source: 7,
                ch: 0,
                note: 64
            })
            .is_ok());
        let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
        let prepared = crate::engine::project::Prepared::empty(engine.sr()).unwrap();
        let applied = engine
            .project
            .install(prepared, captured.revision, &AtomicBool::new(false))
            .unwrap();
        let guard = engine
            .project
            .seal_for_close(Some(applied.revision), &AtomicBool::new(false))
            .unwrap();
        drop(guard);
        let status = audio.handle.switch(settings(44100), cancel()).unwrap();
        assert_eq!(status.phase, Phase::Running);
        assert_eq!(status.active.as_ref().unwrap().plan.rate, 44100);
        accepted(&engine.cmd, Command::Master(0.12));
        assert_eq!(
            engine
                .project
                .capture(&AtomicBool::new(false))
                .unwrap()
                .state
                .master,
            0.12
        );
    }
    #[test]
    fn cancellation_during_open_restores_prior_output_and_cannot_unseal_another_close() {
        let (engine, audio, controls) = fixture();
        controls.block_open.store(true, Ordering::Release);
        let handle = audio.handle.clone();
        let token = cancel();
        let cancelled = token.clone();
        let task = std::thread::spawn(move || handle.switch(settings(96000), token));
        wait(|| controls.entering_open.load(Ordering::Acquire));
        cancelled.store(true, Ordering::Release);
        controls.block_open.store(false, Ordering::Release);
        let status = task.join().unwrap().unwrap();
        assert_eq!(status.active.as_ref().unwrap().plan.rate, 48000);
        let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
        let guard = engine
            .project
            .seal_for_close(Some(captured.revision), &AtomicBool::new(false))
            .unwrap();
        let token = cancel();
        token.store(true, Ordering::Release);
        assert!(audio.handle.switch(settings(44100), token).is_err());
        assert_eq!(
            engine.cmd.send(Command::Master(0.2)),
            Err(SubmissionError::ProjectChanging)
        );
        drop(guard);
        accepted(&engine.cmd, Command::Master(0.2));
    }
    #[test]
    fn delayed_switch_serializes_project_capture_and_offline_capture_can_write_a_real_project() {
        let (engine, audio, controls) = fixture();
        controls.block_open.store(true, Ordering::Release);
        let handle = audio.handle.clone();
        let switch = std::thread::spawn(move || handle.switch(settings(96000), cancel()));
        wait(|| controls.entering_open.load(Ordering::Acquire));
        let project = engine.project.clone();
        let (done, result) = bounded(1);
        let capture = std::thread::spawn(move || {
            done.send(project.capture(&AtomicBool::new(false))).unwrap();
        });
        assert!(result.recv_timeout(Duration::from_millis(20)).is_err());
        controls.failures.lock().extend([true, true]);
        controls.block_open.store(false, Ordering::Release);
        assert_eq!(switch.join().unwrap().unwrap().phase, Phase::Offline);
        let captured = result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        capture.join().unwrap();
        let path = std::env::temp_dir().join(format!(
            "omatainer-offline-save-{}.omat",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let bundle = crate::project_file::Bundle {
            state: captured.state,
            media: captured.media,
        };
        crate::project_file::save(
            &path,
            &bundle,
            crate::project_file::Overwrite::Never,
            &crate::project_file::Limits::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let reopened = crate::project_file::load::<crate::engine::project::State>(
            &path,
            &crate::project_file::Limits::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&reopened.state).unwrap(),
            serde_json::to_value(&bundle.state).unwrap()
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn shutdown_during_blocked_open_never_enables_a_new_callback_and_late_cancel_does_not_undo_commit(
    ) {
        let (engine, audio, controls) = fixture();
        let token = cancel();
        let status = audio.handle.switch(settings(44100), token.clone()).unwrap();
        token.store(true, Ordering::Release);
        assert_eq!(status.active.as_ref().unwrap().plan.rate, 44100);
        assert_eq!(audio.handle.status().phase, Phase::Running);
        controls.block_open.store(true, Ordering::Release);
        let handle = audio.handle.clone();
        let switch = std::thread::spawn(move || handle.switch(settings(96000), cancel()));
        wait(|| controls.entering_open.load(Ordering::Acquire));
        let before = engine.cmd.audio_metrics().callbacks;
        drop(audio);
        controls.block_open.store(false, Ordering::Release);
        let status = switch.join().unwrap().unwrap();
        assert_eq!(status.phase, Phase::Offline);
        wait(|| !engine.cmd.is_connected());
        assert_eq!(engine.cmd.audio_metrics().callbacks, before);
    }
    #[test]
    fn managed_callback_is_silent_before_commit_and_after_shutdown_without_hot_path_heap_work() {
        let (_engine, rt) = Engine::headless_for_test(48000, 256);
        let graph = Box::new(rt);
        let identity = &*graph as *const RtEngine as usize;
        let (returned, receiver) = bounded(1);
        let enabled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let mut callback =
            OutputCallback::managed(graph, 2, returned, enabled.clone(), stopped.clone());
        let mut output = [1.0_f32; 256];
        callback.render(&mut output);
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(callback.rt.frames_done, 0);
        enabled.store(true, Ordering::Release);
        callback.render(&mut output);
        assert_eq!(
            crate::engine::test_alloc::measure(|| callback.render(&mut output)),
            crate::engine::test_alloc::Counts::default()
        );
        let before = callback.rt.frames_done;
        stopped.store(true, Ordering::Release);
        assert_eq!(
            crate::engine::test_alloc::measure(|| callback.render(&mut output)),
            crate::engine::test_alloc::Counts::default()
        );
        assert_eq!(callback.rt.frames_done, before);
        assert!(output.iter().all(|sample| *sample == 0.0));
        drop(callback);
        let graph = receiver.recv().unwrap();
        assert_eq!(&*graph as *const RtEngine as usize, identity);
    }
    #[test]
    fn play_failure_rolls_back_and_runtime_failure_cannot_release_a_project_close_seal() {
        let (engine, audio, controls) = fixture();
        controls.play_failures.lock().extend([true, false]);
        let status = audio.handle.switch(settings(96000), cancel()).unwrap();
        assert_eq!(status.active.as_ref().unwrap().plan.rate, 48000);
        assert!(status.message.contains("play failure"));
        let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
        let guard = engine
            .project
            .seal_for_close(Some(captured.revision), &AtomicBool::new(false))
            .unwrap();
        controls
            .active_fault
            .lock()
            .as_ref()
            .unwrap()
            .store(true, Ordering::Release);
        wait(|| audio.handle.status().phase == Phase::Offline);
        assert!(engine.cmd.performance().status().recovery);
        assert!(engine.cmd.performance().status().changing);
        assert_eq!(engine.cmd.send(Command::Master(0.2)), Err(SubmissionError::Performance(crate::engine::performance::Error::Recovery)));
        drop(guard);
        wait(|| !engine.cmd.performance().status().changing);
        engine.cmd.performance().acknowledge_inputs_released().unwrap();
        wait(|| !engine.cmd.performance().status().recovery);
        engine.project.capture(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            engine.cmd.send(Command::Master(0.2)),
            Err(SubmissionError::AudioUnavailable)
        );
        assert_eq!(
            audio
                .handle
                .switch(settings(44100), cancel())
                .unwrap()
                .phase,
            Phase::Running
        );
    }
    #[test]
    fn same_rate_stop_and_explicit_resume_rebuilds_midi_and_arp_without_render_allocation() {
        for arp in [false, true] {
            let (_engine, mut rt) = Engine::headless_for_test(48000, 256);
            rt.tracks[1].clips[0].notes = vec![crate::engine::MidiNote {
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 60,
                start: 0.0,
                len: 4.0,
                vel: 100,
            }];
            if arp {
                rt.tracks[1].fx.slots.push(crate::engine::fx::FxSlot::new(
                    crate::engine::fx::FxId::Arp,
                    48000.0,
                ));
            }
            rt.apply(Command::LaunchClip { track: 1, scene: 0 });
            let mut out = [0.0; 256];
            rt.process(&mut out);
            let before = rt.tracks[1].poly.note_on_events;
            assert!(before > 0);
            rt.stop_for_audio();
            rt.set_sample_rate(48000).unwrap();
            rt.process(&mut out);
            assert!(out.iter().all(|sample| *sample == 0.0));
            assert_eq!(rt.tracks[1].poly.note_on_events, before);
            rt.apply(Command::Play);
            assert_eq!(
                crate::engine::test_alloc::measure(|| rt.process(&mut out)),
                crate::engine::test_alloc::Counts::default()
            );
            assert!(
                rt.tracks[1].poly.note_on_events > before,
                "no resumed onset for arp={arp}"
            );
            assert!(out.iter().any(|sample| sample.abs() > 1e-7));
        }
    }
    #[test]
    fn offline_snapshot_publication_retries_after_held_reader_and_project_install() {
        let (engine, audio, controls) = fixture();
        controls.failures.lock().extend([true, true]);
        audio.handle.switch(settings(96000), cancel()).unwrap();
        let held = engine.snap.lock();
        std::thread::sleep(Duration::from_millis(600));
        let prepared = crate::engine::project::Prepared::empty(engine.sr()).unwrap();
        let applied = engine
            .project
            .install(prepared, engine.project.revision(), &AtomicBool::new(false))
            .unwrap();
        assert!(held.project_revision < applied.revision);
        drop(held);
        wait(|| engine.snapshot().project_revision >= applied.revision);
        assert_eq!(audio.handle.status().phase, Phase::Offline);
    }
    #[test]
    fn stale_rate_prepared_graph_is_rejected_without_replacing_session() {
        let (engine, audio, _) = fixture();
        let prepared = crate::engine::project::Prepared::empty(48000).unwrap();
        audio.handle.switch(settings(96000), cancel()).unwrap();
        let revision = engine.project.revision();
        assert!(matches!(
            engine
                .project
                .install(prepared, revision, &AtomicBool::new(false)),
            Err(crate::engine::project::Error::Conflict)
        ));
        let state = engine.project.capture(&AtomicBool::new(false)).unwrap();
        assert_eq!(state.revision, revision);
        assert!(state.media.len() > 0);
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    use crate::engine::{Command, performance::{Error, Safety}};
    fn wait(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while !ready() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
    }
    #[test]
    fn protected_owner_refuses_disruptions_and_explicit_reset_is_the_only_emergency_unmute() {
        let (engine, audio, controls) = tests::fixture();
        engine.send(Command::PerformanceMode(true)).unwrap();
        let opens = controls.opens.load(Ordering::Acquire);
        assert!(audio.handle.switch(crate::preferences::Audio::default(), Arc::new(AtomicBool::new(false))).is_err());
        assert_eq!(controls.opens.load(Ordering::Acquire), opens);
        engine.send(Command::SafetyStop(Safety::Silence)).unwrap();
        wait(|| engine.cmd.performance().status().stopped && engine.cmd.performance().status().output_muted);
        // A failed reset/rollback retains mute, even though histories were reset.
        controls.failures.lock().extend([true, false]);
        let permit = audio.handle.performance_permit().unwrap();
        let status = audio.handle.reset_permitted(Arc::new(AtomicBool::new(false)), permit).unwrap();
        assert!(status.message.contains("failed"));
        assert!(engine.cmd.performance().status().output_muted);
        engine.send(Command::RecoverPerformance).unwrap();
        wait(|| !engine.cmd.performance().status().recovery);
        assert!(engine.cmd.performance().status().protected);
        assert!(engine.cmd.performance().status().output_muted);
        let permit = audio.handle.performance_permit().unwrap();
        let status = audio.handle.reset_permitted(Arc::new(AtomicBool::new(false)), permit).unwrap();
        assert!(status.message.contains("reset completed"));
        wait(|| !engine.cmd.performance().status().output_muted);
        let capture = engine.project.capture(&AtomicBool::new(false)).unwrap();
        assert_eq!(capture.state.bpm, 124.0);
        wait(|| { let snapshot = engine.snapshot(); !snapshot.playing && snapshot.decks.iter().all(|deck| !deck.playing) });
    }
    #[test]
    fn queued_audio_permit_prevents_mode_entry_without_stealing_another_seal() {
        let (engine, audio, controls) = tests::fixture();
        let permit = audio.handle.performance_permit().unwrap();
        assert_eq!(engine.cmd.performance().set_enabled(true), Err(Error::Changing));
        drop(permit);
        controls.block_open.store(true, Ordering::Release);
        let handle = audio.handle.clone();
        let worker = std::thread::spawn(move || handle.switch(crate::preferences::Audio::default(), Arc::new(AtomicBool::new(false))));
        wait(|| controls.entering_open.load(Ordering::Acquire));
        assert_eq!(engine.cmd.performance().set_enabled(true), Err(Error::Changing));
        controls.block_open.store(false, Ordering::Release);
        worker.join().unwrap().unwrap();
        wait(|| !engine.cmd.performance().status().changing);
        engine.cmd.performance().set_enabled(true).unwrap();
    }
}
