//! Renderer bank identities and immutable prepared edit requests.
use super::*;
use crate::sampler_bank::{assets, resident, BankId, Factory, Slot, Source, SLOTS};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub(crate) const MAX_BANKS: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct Bank {
    pub id: BankId,
    pub revision: u64,
    /// Only freshly generated factory instances have this marker. Native
    /// project loads, including legacy banks named "Kit", are embedded PCM.
    pub factory: Option<Factory>,
    pub data: assets::Bank,
}
impl Bank {
    pub fn name(&self) -> &str {
        &self.data.settings.name
    }
    pub fn imported(data: assets::Bank) -> Result<Self, String> {
        let id = BankId::new()?;
        if data.settings.definition == Some(id) {
            return Err("working bank identity collided with reusable definition".into());
        }
        Ok(Self {
            id,
            revision: 0,
            factory: None,
            data,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Replace { id: BankId, revision: u64 },
    Append { revision: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditState {
    Pending,
    Applied,
    Rejected,
}
#[derive(Clone, Debug)]
pub(crate) struct Ack(Arc<AckState>);
#[derive(Debug)]
struct AckState { state: AtomicU8, ended: AtomicBool }
impl Ack {
    pub fn new() -> Self {
        Self(Arc::new(AckState { state: AtomicU8::new(0), ended: AtomicBool::new(false) }))
    }
    pub fn state(&self) -> EditState {
        match self.0.state.load(Ordering::Acquire) {
            1 => EditState::Applied,
            2 => EditState::Rejected,
            _ => EditState::Pending,
        }
    }
    /// Audition completion is renderer-owned and observable even when a short
    /// voice starts and finishes between two snapshot publications.
    pub fn ended(&self) -> bool { self.0.ended.load(Ordering::Acquire) }
    /// A cancellation before renderer ownership wins. Once claimed, the
    /// renderer reports its actual commit or rejection; late cancellation never
    /// rewrites an applied edit as a failure.
    pub fn cancel(&self) -> bool {
        self.0.state.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
    pub(in crate::engine) fn claim(&self) -> bool {
        self.0.state.compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
    pub(super) fn applied(&self) {
        self.0.state.store(1, Ordering::Release);
    }
    pub(crate) fn reject(&self) {
        let _ = self.0.state.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |state| matches!(state, 0 | 3).then_some(2));
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Audition {
    pub id: u64,
    pub bank: assets::Bank,
    pub slot: u8,
    pub track: u8,
    pub ack: Ack,
}
#[derive(Debug)]
pub(crate) struct ActiveAudition {
    pub id: u64,
    pub voice: resident::Voice,
    request: Audition,
}
impl ActiveAudition {
    pub(super) fn ended(&self) { self.request.ack.0.ended.store(true, Ordering::Release); }
}
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub epoch: u64,
    pub target: Target,
    pub bank: Bank,
    pub select: bool,
    pub ack: Ack,
}

pub(crate) fn factory_data(owner: &assets::Owner, sr: u32) -> Result<[assets::Bank; 3], String> {
    let mut cached = [None, None, None];
    for factory in Factory::ALL {
        cached[factory.index()] = owner.factory(factory, sr).map_err(|e| e.to_string())?;
    }
    if cached.iter().any(Option::is_none) {
        for (factory, samples) in Factory::ALL.into_iter().zip(build_pad_banks(sr)) {
            if cached[factory.index()].is_some() {
                continue;
            }
            let settings = resident::Settings {
                name: factory.name().into(),
                definition: None,
                slots: std::array::from_fn(|slot| Slot {
                    source: Some(Source::Factory {
                        bank: factory,
                        slot: slot as u8,
                    }),
                    ..Slot::default()
                }),
            };
            let data = resident::Data::prepare(
                Arc::new(settings),
                samples.map(Some),
                std::array::from_fn(|_| None),
            )?;
            cached[factory.index()] = Some(
                owner
                    .pin_factory(factory, sr, data)
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    Ok(cached.map(Option::unwrap))
}
pub(super) fn initial(sr: u32) -> Result<(assets::Owner, Vec<Bank>), String> {
    let owner = assets::Owner::acquire().map_err(|e| e.to_string())?;
    let mut banks = Vec::with_capacity(MAX_BANKS);
    for (factory, data) in Factory::ALL.into_iter().zip(factory_data(&owner, sr)?) {
        banks.push(Bank {
            id: BankId::new()?,
            revision: 1,
            factory: Some(factory),
            data,
        });
    }
    Ok((owner, banks))
}

impl RtEngine {
    pub(super) fn apply_sampler_audition(&mut self, request: Audition) {
        let slot = request.slot as usize;
        let track = request.track as usize;
        if request.id == 0 || slot >= SLOTS || track >= TRACKS
            || !self.sampler_assets.owns(&request.bank)
            || request.bank.audio[slot].is_none() || request.bank.ranges[slot].is_none()
            || !request.ack.claim()
        {
            request.ack.reject();
        } else {
            self.finish_sampler_audition();
            self.sampler_audition = Some(ActiveAudition { id: request.id,
                voice: resident::Voice::new(request.bank.audio[slot].as_ref().unwrap().clone(),
                    request.bank.ranges[slot].unwrap(), request.bank.settings.slots[slot].controls, 1.0, track),
                request: request.clone() });
            request.ack.applied();
        }
        self.undo.retire_command(Command::SamplerAudition(request));
    }
    pub(super) fn finish_sampler_audition(&mut self) {
        if let Some(active) = self.sampler_audition.take() {
            active.request.ack.0.ended.store(true, Ordering::Release);
            self.undo.retire_command(Command::SamplerAudition(active.request));
            // The source PCM still has the independent asset-worker pin.
            drop(active.voice);
        }
    }
    pub(super) fn sampler_edit_index(&self, edit: &Edit) -> Option<usize> {
        if edit.epoch != self.undo.checkpoint().epoch
            || !self.sampler_assets.owns(&edit.bank.data)
            || edit.bank.data.settings.definition == Some(edit.bank.id)
            || self.sampler_revision == u64::MAX
        {
            return None;
        }
        let index = match edit.target {
            Target::Replace { id, revision } => self
                .sampler_banks
                .iter()
                .position(|bank| bank.id == id && bank.revision == revision)?,
            Target::Append { revision }
                if revision == self.sampler_revision && self.sampler_banks.len() < MAX_BANKS =>
            {
                self.sampler_banks.len()
            }
            _ => return None,
        };
        if self
            .sampler_banks
            .iter()
            .enumerate()
            .any(|(i, bank)| i != index && bank.id == edit.bank.id)
        {
            return None;
        }
        if self
            .sampler_banks
            .get(index)
            .is_some_and(|old| old.factory.is_some() && old.id == edit.bank.id)
        {
            return None;
        }
        // Factory origin is assigned only by initial/rate preparation. Neither
        // an arbitrary UI edit nor a native file may manufacture regeneration.
        if edit.bank.factory.is_some() {
            return None;
        }
        Some(index)
    }
    pub(super) fn apply_sampler_edit(&mut self, edit: Edit) {
        let Some(index) = self.sampler_edit_index(&edit) else {
            edit.ack.reject();
            self.undo.retire_command(Command::SamplerEdit(edit));
            return;
        };
        self.sampler_revision += 1;
        let mut next = edit.bank.clone();
        next.revision = self.sampler_revision;
        if index == self.sampler_banks.len() {
            self.sampler_banks.push(next);
        } else {
            self.sampler_banks[index] = next;
        }
        if edit.select {
            self.sampler_bank = index;
        }
        edit.ack.applied();
        self.undo.retire_command(Command::SamplerEdit(edit));
    }
    pub(super) fn sampler_rate_banks(&self, sr: u32) -> Result<Option<[assets::Bank; 3]>, String> {
        if self.sampler_banks.iter().any(|bank| bank.factory.is_some()) {
            if self.sampler_revision == u64::MAX {
                return Err("sampler revision exhausted".into());
            }
            Ok(Some(factory_data(&self.sampler_assets, sr)?))
        } else {
            Ok(None)
        }
    }
    pub(super) fn install_sampler_rate_banks(&mut self, prepared: Option<[assets::Bank; 3]>) {
        if let Some(prepared) = prepared {
            self.sampler_revision += 1;
            for bank in &mut self.sampler_banks {
                if let Some(factory) = bank.factory {
                    bank.data = prepared[factory.index()].clone();
                    bank.revision = self.sampler_revision;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod playback_tests;

pub(crate) fn reject(command: &Command) {
    match command {
        Command::SamplerEdit(edit) => edit.ack.reject(),
        Command::SamplerAudition(request) => request.ack.reject(),
        Command::Gesture { command, .. } => reject(command),
        _ => (),
    }
}
pub(super) fn admission_ack(mut command: &Command) -> Option<Ack> {
    loop {
        match command {
            Command::SamplerEdit(edit) => return Some(edit.ack.clone()),
            Command::SamplerAudition(request) => return Some(request.ack.clone()),
            Command::Gesture { command: inner, .. } => command = inner,
            _ => return None,
        }
    }
}

#[cfg(test)]
pub(crate) fn test_bank(rt: &RtEngine, name: String, audio: [Option<Arc<Sample>>; SLOTS]) -> Bank {
    let settings = Arc::new(resident::Settings::empty(name).unwrap());
    let data = resident::Data::prepare(settings, audio, std::array::from_fn(|_| None)).unwrap();
    Bank::imported(rt.sampler_assets.pin(data).unwrap()).unwrap()
}
#[cfg(test)]
impl RtEngine {
    pub(crate) fn set_test_pad_sample(&mut self, bank: usize, slot: usize, sample: Arc<Sample>) {
        let old = &self.sampler_banks[bank];
        let mut settings = (*old.data.settings).clone();
        settings.slots[slot] = Slot::default();
        let mut audio = old.data.audio.clone();
        audio[slot] = Some(sample);
        let mut issues = old.data.issues.clone();
        issues[slot] = None;
        let data = resident::Data::prepare(Arc::new(settings), audio, issues).unwrap();
        self.sampler_banks[bank].data = self.sampler_assets.pin(data).unwrap();
        self.sampler_banks[bank].factory = None;
    }
}

impl RtEngine {
    /// Apply a pad gate with bounded attack pressure without changing its input identity.
    /// Takes pad, gate and normalized pressure; updates playback and records matching note velocity.
    pub(super) fn apply_sampler_pad(&mut self, pad: u8, on: bool, pressure: f32) {
        let input = InputKey::Pad(pad);
        let pitch = sampler_pitch(self.sampler_inst, self.sampler_oct, pad);
        let destination = self.compose_target.unwrap_or(ComposeTarget {
            track: self.selected_track, scene: self.selected_scene,
        });
        if on {
            if self.pad_voices[pad as usize].as_ref().is_some_and(|voice| voice.playback.mode == crate::sampler_bank::PlayMode::Toggle) {
                self.stop_sampler_slot(pad);
                return;
            }
            self.release_input(input);
            self.pad_targets[pad as usize] = None;
            if self.sampler_inst == SamplerInstrument::Samples {
                if let Some(bank) = self.sampler_banks.get(self.sampler_bank) {
                    let slot = pad as usize;
                    self.pad_voices[slot] = bank.data.audio[slot].as_ref().zip(bank.data.ranges[slot]).map(|(sample, range)| {
                        let rate = 2f32.powi((self.sampler_oct - 3) as i32) as f64;
                        let mut voice = crate::sampler_bank::resident::Voice::new(sample.clone(), range, bank.data.settings.slots[slot].controls, rate, destination.track);
                        voice.playback = bank.data.settings.slots[slot].playback;
                        voice.gain *= pressure;
                        voice
                    });
                }
            } else {
                self.sampler_poly.note_on_input(pitch, 0.9 * pressure, input);
                let target = PadTarget { track: destination.track, pitch };
                self.pad_destinations[pad as usize] = target.track;
                self.pad_targets[pad as usize] = Some(target);
            }
            if self.compose_target.is_some() || self.recording {
                if self.recording_position(
                    destination.track, destination.scene,
                ).is_none() { return; }
                if !self.history_record_target(destination.track,destination.scene) {return;}
                let clip = &mut self.tracks[destination.track].clips[destination.scene];
                if clip.kind == ClipKind::Empty {
                    clip.kind = ClipKind::Midi;
                    clip.name.clear();clip.name.push_str("Pad");
                    clip.bars = 1.0;
                }
                self.begin_recording_note(
                    input, destination.track, destination.scene, pitch, (110.0 * pressure).round().clamp(1.0, 127.0) as u8,
                );
            }
        } else {
            self.release_input(input);
            self.pad_targets[pad as usize] = None;
            if self.pad_voices[pad as usize].as_ref().is_some_and(|voice| voice.playback.mode == crate::sampler_bank::PlayMode::Hold) {
                self.pad_voices[pad as usize] = None;
            }
        }
    }
    /// Stop one captured sample or synth pad.
    /// Takes an exact slot in 0–15; returns no value and leaves every other slot and transport untouched.
    pub(super) fn stop_sampler_slot(&mut self, pad: u8) {
        if usize::from(pad) >= SLOTS { return; }
        self.release_input(InputKey::Pad(pad));
        self.pad_targets[usize::from(pad)] = None;
        self.pad_voices[usize::from(pad)] = None;
    }
}
