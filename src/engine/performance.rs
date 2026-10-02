//! Show protection is shared by every producer and checked again by the
//! renderer. No callback lock, allocation, driver call, or worker wait lives here.
use super::Command;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc,
};

const PROTECTED: u64 = 1;
const RECOVERY: u64 = 2;
const STOPPED: u64 = 4;
const EXCLUSIVE: u64 = 8;
const WRITER: u64 = 1 << 8;
// Low flags, 24 bits of producer leases, then a safety generation. The
// generation prevents a complete stop/ack cycle from recreating an old CAS
// word while the renderer is trying to reopen recovery admission.
const SAFETY_GENERATION: u64 = 1 << 32;
const WRITERS: u64 = (SAFETY_GENERATION - 1) & !(WRITER - 1);
const OPTIONAL_SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Protected,
    PlayingDeck,
    Recovery,
    Changing,
    PendingStop,
    BackgroundBusy,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Protected => "Performance protection: this destructive action was not applied. Leave performance mode deliberately to edit it.",
            Self::PlayingDeck => "Performance protection: pause and release the target deck before loading or unloading it.",
            Self::Recovery => "Recovery is latched. Release physical inputs, then explicitly acknowledge recovery. Playback will remain stopped.",
            Self::Changing => "A project/device change or irreversible background commit is already pending. Finish or cancel it before changing performance protection.",
            Self::PendingStop => "The renderer has not completed the safety stop yet. Recovery cannot be acknowledged early.",
            Self::BackgroundBusy => "Optional work is already at its bounded capacity. Retry after it finishes.",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Safety {
    Stop = 1,
    Silence = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub protected: bool,
    pub recovery: bool,
    pub stopped: bool,
    pub changing: bool,
    pub safety_requested: u64,
    pub safety_applied: u64,
    pub output_muted: bool,
    pub observation_complete: bool,
    pub observed_frames: u64,
    pub quiet_frames: u64,
    pub nonfinite_tail: bool,
    pub optional_active: u32,
    pub rejected: u64,
    pub last_rejection: Option<Error>,
}

struct Shared {
    admission: AtomicU64,
    safety: AtomicU64,
    input_epoch: AtomicU64,
    applied: AtomicU64,
    recover: AtomicU64,
    deck_activity: AtomicU8,
    output: AtomicU8,
    observed: AtomicU64,
    quiet: AtomicU64,
    rejected: AtomicU64,
    last_rejection: AtomicU8,
    work: [Arc<AtomicBool>; OPTIONAL_SLOTS],
    work_claims: AtomicU64,
}
#[derive(Clone)]
pub struct Handle(Arc<Shared>);
impl Default for Handle {
    fn default() -> Self {
        Self(Arc::new(Shared {
            admission: AtomicU64::new(0),
            safety: AtomicU64::new(0),
            input_epoch: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            recover: AtomicU64::new(0),
            output: AtomicU8::new(0),
            observed: AtomicU64::new(0),
            quiet: AtomicU64::new(0),
            deck_activity: AtomicU8::new(0),
            rejected: AtomicU64::new(0),
            last_rejection: AtomicU8::new(0),
            work: std::array::from_fn(|_| Arc::new(AtomicBool::new(false))),
            work_claims: AtomicU64::new(0),
        }))
    }
}

/// Taken by every command sender, including the GUI intent bridge. Recovery
/// opens only after earlier senders finish AND their queued work is drained.
pub(super) struct Writer<'a>(&'a AtomicU64);
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
    }
}

/// Worker-side exclusion independent of the project's CloseGuard. Enabling
/// protection cannot race a queued/active device or document replacement.
pub struct ExclusivePermit {
    handle: Handle,
    audio: bool,
}
impl ExclusivePermit {
    pub fn valid_for_audio(&self, handle: &Handle) -> bool {
        self.audio
            && Arc::ptr_eq(&self.handle.0, &handle.0)
            && handle.0.admission.load(Ordering::Acquire) & EXCLUSIVE != 0
    }
}
impl Drop for ExclusivePermit {
    fn drop(&mut self) {
        self.handle
            .0
            .admission
            .fetch_and(!EXCLUSIVE, Ordering::Release);
    }
}

/// Optional jobs borrow one of the preallocated cancellation flags. A slot is
/// reused only after all UI/worker clones of its flag have gone away, so a late
/// Cancel cannot affect a different job. Destruction occurs on UI/worker paths.
pub struct WorkPermit {
    handle: Handle,
    slot: usize,
    generation: u64,
}
impl WorkPermit {
    /// Worker/UI publication boundary. Winning this claim keeps mode entry out
    /// until commit acknowledgement completes; losing to protection cancels the
    /// optional publication without pretending an already committed write lost.
    pub fn commit(&self) -> Result<ExclusivePermit, Error> {
        self.commit_after_check(|| {})
    }
    fn commit_after_check(&self, after_check: impl FnOnce()) -> Result<ExclusivePermit, Error> {
        if self.cancelled() {
            return Err(self.handle.reject(Error::Protected));
        }
        after_check();
        let permit = self.handle.project_change()?;
        // A quick protect/unprotect can finish between the first cancellation
        // read and acquiring the commit guard. That old job remains cancelled.
        if self.cancelled() {
            return Err(self.handle.reject(Error::Protected));
        }
        Ok(permit)
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.handle.0.work[self.slot].load(Ordering::Acquire)
            || self.handle.0.admission.load(Ordering::Acquire) / SAFETY_GENERATION
                != self.generation
    }
    pub fn cancel(&self) -> Arc<AtomicBool> {
        self.handle.0.work[self.slot].clone()
    }
}
impl Drop for WorkPermit {
    fn drop(&mut self) {
        self.handle
            .0
            .work_claims
            .fetch_and(!(1 << self.slot), Ordering::Release);
    }
}

impl Handle {
    pub fn status(&self) -> Status {
        let state = self.0.admission.load(Ordering::Acquire);
        Status {
            protected: state & PROTECTED != 0,
            recovery: state & RECOVERY != 0,
            stopped: state & STOPPED != 0,
            changing: state & EXCLUSIVE != 0,
            safety_requested: self.0.safety.load(Ordering::Acquire) >> 2,
            safety_applied: self.0.applied.load(Ordering::Acquire) >> 2,
            output_muted: self.0.output.load(Ordering::Acquire) & 1 != 0,
            observation_complete: self.0.output.load(Ordering::Acquire) & 2 != 0,
            nonfinite_tail: self.0.output.load(Ordering::Acquire) & 4 != 0,
            observed_frames: self.0.observed.load(Ordering::Relaxed),
            quiet_frames: self.0.quiet.load(Ordering::Relaxed),
            optional_active: self.0.work_claims.load(Ordering::Acquire).count_ones(),
            rejected: self.0.rejected.load(Ordering::Relaxed),
            last_rejection: match self.0.last_rejection.load(Ordering::Relaxed) {
                1 => Some(Error::Protected),
                2 => Some(Error::PlayingDeck),
                3 => Some(Error::Recovery),
                4 => Some(Error::Changing),
                5 => Some(Error::PendingStop),
                6 => Some(Error::BackgroundBusy),
                _ => None,
            },
        }
    }
    pub fn protected(&self) -> bool {
        self.0.admission.load(Ordering::Acquire) & (PROTECTED | RECOVERY) != 0
    }
    pub fn reject(&self, error: Error) -> Error {
        self.0.last_rejection.store(
            match error {
                Error::Protected => 1,
                Error::PlayingDeck => 2,
                Error::Recovery => 3,
                Error::Changing => 4,
                Error::PendingStop => 5,
                Error::BackgroundBusy => 6,
            },
            Ordering::Relaxed,
        );
        self.0.rejected.fetch_add(1, Ordering::Relaxed);
        error
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        let mut previous = self.0.admission.load(Ordering::Acquire);
        loop {
            if previous & EXCLUSIVE != 0 {
                return Err(self.reject(Error::Changing));
            }
            if previous & RECOVERY != 0 && !enabled {
                return Err(self.reject(Error::Recovery));
            }
            let next = if enabled {
                previous.wrapping_add(SAFETY_GENERATION) | PROTECTED
            } else {
                previous & !PROTECTED
            };
            match self.0.admission.compare_exchange_weak(
                previous,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => previous = current,
            }
        }
        if enabled {
            for cancel in &self.0.work {
                cancel.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
    pub(super) fn writer(&self) -> Writer<'_> {
        self.0.admission.fetch_add(WRITER, Ordering::AcqRel);
        Writer(&self.0.admission)
    }
    pub fn audio_change(&self) -> Result<ExclusivePermit, Error> {
        self.exclusive(true)
    }
    pub(super) fn project_change(&self) -> Result<ExclusivePermit, Error> {
        self.exclusive(false)
    }
    fn exclusive(&self, audio: bool) -> Result<ExclusivePermit, Error> {
        let mut old = self.0.admission.load(Ordering::Acquire);
        loop {
            if old & EXCLUSIVE != 0 {
                return Err(self.reject(Error::Changing));
            }
            let recovery =
                old & (RECOVERY | STOPPED) == RECOVERY | STOPPED || self.status().output_muted;
            if old & (PROTECTED | RECOVERY) != 0 && !(audio && recovery) {
                return Err(self.reject(Error::Protected));
            }
            match self.0.admission.compare_exchange_weak(
                old,
                old | EXCLUSIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(ExclusivePermit {
                        handle: self.clone(),
                        audio,
                    })
                }
                Err(next) => old = next,
            }
        }
    }
    pub fn optional_work(&self) -> Result<WorkPermit, Error> {
        let admission = self.0.admission.load(Ordering::Acquire);
        if admission & (PROTECTED | RECOVERY) != 0 {
            return Err(self.reject(Error::Protected));
        }
        let generation = admission / SAFETY_GENERATION;
        for slot in 0..OPTIONAL_SLOTS {
            if Arc::strong_count(&self.0.work[slot]) != 1 {
                continue;
            }
            let bit = 1 << slot;
            if self.0.work_claims.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
                continue;
            }
            self.0.work[slot].store(false, Ordering::Release);
            let permit = WorkPermit {
                handle: self.clone(),
                slot,
                generation,
            };
            if self.protected() || permit.cancelled() {
                self.0.work[slot].store(true, Ordering::Release);
                return Err(self.reject(Error::Protected));
            }
            return Ok(permit);
        }
        Err(self.reject(Error::BackgroundBusy))
    }
    pub(crate) fn input_epoch(&self) -> u64 {
        self.0.input_epoch.load(Ordering::Acquire)
    }
    pub fn request_safety(&self, safety: Safety) {
        let _writer = self.writer();
        self.0
            .admission
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some((state.wrapping_add(SAFETY_GENERATION) | RECOVERY) & !STOPPED)
            })
            .unwrap();
        self.0.input_epoch.fetch_add(1, Ordering::AcqRel);
        self.0
            .safety
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
                let pending = previous != self.0.applied.load(Ordering::Acquire);
                let severity = if pending {
                    (previous & 3).max(safety as u64)
                } else {
                    safety as u64
                };
                Some((((previous >> 2).wrapping_add(1).max(1)) << 2) | severity)
            })
            .unwrap();
        for cancel in &self.0.work {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn acknowledge_inputs_released(&self) -> Result<(), Error> {
        let state = self.0.admission.load(Ordering::Acquire);
        if state & EXCLUSIVE != 0 {
            return Err(self.reject(Error::Changing));
        }
        let request = self.0.safety.load(Ordering::Acquire);
        if state & (RECOVERY | STOPPED) != RECOVERY | STOPPED
            || self.0.applied.load(Ordering::Acquire) != request
        {
            return Err(self.reject(Error::PendingStop));
        }
        self.0.recover.store(request, Ordering::Release);
        Ok(())
    }
    pub(super) fn safety_epoch(&self) -> u64 {
        self.0.safety.load(Ordering::Acquire) >> 2
    }
    pub(super) fn safety_request(&self) -> Option<(u64, Safety)> {
        let request = self.0.safety.load(Ordering::Acquire);
        (request != self.0.applied.load(Ordering::Acquire)).then(|| {
            (
                request,
                if request & 3 == 2 {
                    Safety::Silence
                } else {
                    Safety::Stop
                },
            )
        })
    }
    pub(super) fn stopped(&self, request: u64) {
        self.stopped_after_check(request, || {});
    }
    fn stopped_after_check(&self, request: u64, after_check: impl FnOnce()) {
        let state = self.0.admission.load(Ordering::Acquire);
        if request != self.0.safety.load(Ordering::Acquire) {
            return;
        }
        after_check();
        // A newer stop changes the generation, so an older renderer completion
        // cannot publish STOPPED for it. One attempt only; keep the mailbox
        // unapplied on contention and safely retry at the next block boundary.
        if self
            .0
            .admission
            .compare_exchange(state, state | STOPPED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.0.applied.store(request, Ordering::Release);
        }
    }
    /// Audio makes one attempt. Read writer count before queue emptiness: an
    /// earlier sender publishes its command before releasing that count.
    pub(super) fn try_recover(&self, drained: impl FnOnce() -> bool) -> bool {
        let state = self.0.admission.load(Ordering::Acquire);
        let request = self.0.safety.load(Ordering::Acquire);
        if state & WRITERS != 0
            || state & EXCLUSIVE != 0
            || state & (RECOVERY | STOPPED) != RECOVERY | STOPPED
            || self.0.recover.load(Ordering::Acquire) != request
            || self.0.applied.load(Ordering::Acquire) != request
            || !drained()
        {
            return false;
        }
        self.0.input_epoch.fetch_add(1, Ordering::AcqRel);
        self.0
            .admission
            .compare_exchange(
                state,
                state & !(RECOVERY | STOPPED),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    pub(super) fn publish_output(&self, output: &Output) {
        self.0.observed.store(output.observed, Ordering::Relaxed);
        self.0.quiet.store(output.quiet, Ordering::Relaxed);
        self.0.output.store(
            u8::from(output.muted)
                | (u8::from(!output.observing && output.observed > 0) << 1)
                | (u8::from(output.nonfinite) << 2),
            Ordering::Release,
        );
    }
    pub(super) fn publish_decks(&self, activity: u8) {
        self.0.deck_activity.store(activity, Ordering::Release);
    }
    pub(crate) fn check(&self, command: &Command, activity: Option<u8>) -> Result<(), Error> {
        let state = self.0.admission.load(Ordering::Acquire);
        if state & RECOVERY != 0 && !recovery_safe(command) {
            return Err(Error::Recovery);
        }
        if state & PROTECTED == 0 {
            return Ok(());
        }
        if destructive(command) {
            return Err(Error::Protected);
        }
        if let Some(deck) = media_target(command) {
            if deck < super::DECKS
                && activity.unwrap_or_else(|| self.0.deck_activity.load(Ordering::Acquire))
                    & (1 << deck)
                    != 0
            {
                return Err(Error::PlayingDeck);
            }
        }
        Ok(())
    }
}

fn media_target(command: &Command) -> Option<usize> {
    match command {
        Command::DeckAudio { deck, .. }
        | Command::DeckLoadRequested { deck, .. }
        | Command::DeckLoadSelected { deck }
        | Command::LoadBuiltin { deck, .. }
        | Command::DeckUnload { deck }
        | Command::DeckRestorePreparation { deck, .. } => Some(*deck as usize % super::DECKS),
        Command::DeckDecoded { request, .. } => Some(request.deck as usize % super::DECKS),
        Command::SessionControl(scoped) => media_target(&scoped.command),
        Command::Gesture { command, .. } => media_target(command),
        _ => None,
    }
}
fn destructive(command: &Command) -> bool {
    match command {
        Command::SessionEdit(request) => request.disruptive(),
        Command::MidiImport(_)
        | Command::MidiEdit(_)
        | Command::MidiAudition { on: true, .. }
        | Command::SetNotes { .. }
        | Command::SamplerEdit(_)
        | Command::SamplerAudition(_)
        | Command::DeckGrid { .. }
        | Command::Undo
        | Command::Redo
        | Command::FxAdd(_)
        | Command::DeckHotCue { del: true, .. }
        | Command::DeckCuePoint { del: true, .. } => true,
        Command::SessionControl(scoped) => destructive(&scoped.command),
        Command::Gesture { command, .. } => destructive(command),
        Command::PerformanceMode(_)
        | Command::SafetyStop(_)
        | Command::RecoverPerformance
        | Command::ReservedStop { .. }
        | Command::Play
        | Command::Stop
        | Command::TogglePlay
        | Command::Record
        | Command::Tap(_)
        | Command::MidiClock { .. }
        | Command::SetBpm(_)
        | Command::LaunchClip { .. }
        | Command::LaunchScene { .. }
        | Command::StopTrack { .. }
        | Command::DeckPlay { .. }
        | Command::DeckCue { .. }
        | Command::DeckSync { .. }
        | Command::DeckJog { .. }
        | Command::DeckTouch { .. }
        | Command::MidiDeckTouch { .. }
        | Command::DeckPitch { .. }
        | Command::DeckGain { .. }
        | Command::DeckEq { .. }
        | Command::DeckFilter { .. }
        | Command::DeckPfl { .. }
        | Command::DeckHotCue { del: false, .. }
        | Command::DeckCuePoint { del: false, .. }
        | Command::DeckCueStyle { .. }
        | Command::DeckLoop { .. }
        | Command::DeckLoopIn { .. }
        | Command::DeckLoopOut { .. }
        | Command::DeckLoadSelected { .. }
        | Command::DeckVinyl { .. }
        | Command::DeckKeylock { .. }
        | Command::DeckAudio { .. }
        | Command::DeckDecoded { .. }
        | Command::DeckLoadRequested { .. }
        | Command::DeckRestorePreparation { .. }
        | Command::LibraryFence { .. }
        | Command::DeckSeek { .. }
        | Command::DeckUnload { .. }
        | Command::LoadBuiltin { .. }
        | Command::Xfader(_)
        | Command::Master(_)
        | Command::CueMix(_)
        | Command::TrackGain { .. }
        | Command::ClipGain { .. }
        | Command::TrackPan { .. }
        | Command::Mute { .. }
        | Command::Solo { .. }
        | Command::Arm { .. }
        | Command::Browse(_)
        | Command::Select { .. }
        | Command::ComposeArm { .. }
        | Command::ComposeDisarm
        | Command::SelectDeck(_)
        | Command::SelectDeckRequested { .. }
        | Command::SetView(_)
        | Command::LiveNoteOn { .. }
        | Command::RoutedNoteOn { .. }
        | Command::LiveNoteOff { .. }
        | Command::FxWet { .. }
        | Command::FxSelect { .. }
        | Command::Quant(_)
        | Command::Metronome
        | Command::LearnCapture { .. }
        | Command::NudgeBpm(_)
        | Command::ToggleQuant
        | Command::DeckLoopDouble { .. }
        | Command::DeckLoopHalf { .. }
        | Command::DeckReloop { .. }
        | Command::DeckMatch
        | Command::DeckEqCut { .. }
        | Command::DeckEqSolo { .. }
        | Command::DeckPitchRange { .. }
        | Command::FireClip { .. }
        | Command::ToggleScene { .. }
        | Command::RestartScene { .. }
        | Command::AddScene { .. }
        | Command::SamplerPad { .. }
        | Command::MidiAudition { on: false, .. }
        | Command::SamplerAuditionStop { .. }
        | Command::SamplerBank(_)
        | Command::SamplerInst(_)
        | Command::SamplerOct(_)
        | Command::OpenFxTrack(_)
        | Command::OpenFxScene(_)
        | Command::CloseFx
        | Command::FxToggle(_)
        | Command::FxMix { .. }
        | Command::FxParam { .. } => false,
    }
}
pub(super) fn recovery_safe(command: &Command) -> bool {
    matches!(
        command,
        Command::Stop
            | Command::StopTrack { .. }
            | Command::ReservedStop { .. }
            | Command::LiveNoteOff { .. }
            | Command::LiveNoteOn { vel: 0, .. }
            | Command::RoutedNoteOn { vel: 0, .. }
            | Command::SamplerPad { on: false, .. }
            | Command::MidiAudition { on: false, .. }
            | Command::SamplerAuditionStop { .. }
            | Command::DeckTouch { on: false, .. }
            | Command::MidiDeckTouch { on: false, .. }
            | Command::ComposeDisarm
            | Command::LibraryFence { .. }
            | Command::Select { .. }
            | Command::SelectDeck(_)
            | Command::SelectDeckRequested { .. }
            | Command::SetView(_)
            | Command::Browse(_)
            | Command::OpenFxTrack(_)
            | Command::OpenFxScene(_)
            | Command::CloseFx
            | Command::MidiClock { .. }
    )
}

/// Fixed renderer-only output state. Quiet is an observation, not proof that
/// bypassed/delayed histories have been erased. Emergency mute stays latched
/// until an explicit off-callback stopped-graph reset.
#[derive(Default)]
pub(super) struct Output {
    muted: bool,
    ramp: u32,
    ramp_total: u32,
    observed: u64,
    quiet: u64,
    nonfinite: bool,
    observing: bool,
}
impl Output {
    pub(crate) fn retain_mute(&mut self) {
        self.muted = true;
        self.ramp = 0;
    }
    fn stop(&mut self, safety: Safety, sr: f32) {
        self.observed = 0;
        self.quiet = 0;
        self.nonfinite = false;
        self.observing = true;
        if safety == Safety::Silence && !self.muted {
            self.muted = true;
            self.ramp_total = (sr * 0.002).ceil().max(2.0) as u32;
            self.ramp = self.ramp_total;
        }
    }
    pub(super) fn observe(&mut self, frame: [f32; 2], sr: f32) {
        if !self.observing {
            return;
        }
        self.observed = self.observed.saturating_add(1);
        let finite = frame.iter().all(|v| v.is_finite());
        self.nonfinite |= !finite;
        if finite && frame.iter().all(|v| v.abs() <= 0.0001) {
            self.quiet += 1;
        } else {
            self.quiet = 0;
        }
        // Continue the DSP, but bound this diagnostic window to two seconds.
        if self.observed >= (sr * 2.0).ceil() as u64 {
            self.observing = false;
        }
    }
    pub(super) fn output(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let output = self.preview(frame);
        if self.muted && self.ramp > 0 { self.ramp -= 1; }
        output
    }
    pub(super) fn preview(&self, mut frame: [f32; 2]) -> [f32; 2] {
        if !self.muted {
            return frame;
        }
        if self.ramp == 0 {
            return [0.0; 2];
        }
        let gain = (self.ramp - 1) as f32 / (self.ramp_total - 1) as f32;
        for sample in &mut frame {
            *sample = if sample.is_finite() {
                *sample * gain
            } else {
                0.0
            };
        }
        frame
    }
}

pub(super) fn reject_receipt(command: &Command) {
    match command {
        Command::SessionEdit(request) => request.ack.reject(),
        Command::DeckLoadRequested { receipt, .. } if receipt.claim() => {
            receipt.finish(super::load_receipt::State::Protected)
        }
        Command::SessionControl(scoped) => reject_receipt(&scoped.command),
        Command::Gesture { command, .. } => reject_receipt(command),
        _ => {}
    }
}
impl super::RtEngine {
    pub(super) fn deck_activity(&self) -> u8 {
        self.decks.iter().enumerate().fold(0, |bits, (i, deck)| {
            bits | (u8::from(deck.playing || deck.touching) << i)
        })
    }
    pub(super) fn performance_tick(&mut self) {
        let Some((request, safety)) = self.performance.safety_request() else {
            return;
        };
        // Finalize exact original recording targets before any gate or arm is
        // released. All bounded synth voices enter their normal release stage.
        self.history_finish_take();
        self.finish_recording_all();
        self.playing = false;
        self.recording = false;
        self.compose_target = None;
        self.metro.reset();
        self.count_in = None;
        for track in &mut self.tracks {
            track.stop_clip();
            for voice in &mut track.poly.voices {
                if matches!(voice.env.stage, 1..=3) {
                    voice.env.off();
                }
            }
        }
        for voice in &mut self.sampler_poly.voices {
            if matches!(voice.env.stage, 1..=3) {
                voice.env.off();
            }
        }
        self.pad_targets.fill(None);
        self.finish_sampler_audition();
        // Finite sample one-shots keep their existing Arc ownership and natural
        // tails. Emergency silence is applied after the entire output chain.
        for deck in &mut self.decks {
            deck.playing = false;
            deck.touching = false;
            deck.touch_sources.fill(None);
            deck.scratch = 0.0;
        }
        self.safety_output.stop(safety, self.sr);
        self.performance.publish_decks(0);
        self.performance.stopped(request);
        self.performance.publish_output(&self.safety_output);
    }
}

#[cfg(test)]
mod tests;
