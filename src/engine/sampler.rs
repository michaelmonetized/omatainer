//! Renderer bank identities and immutable prepared edit requests.
use super::*;
use crate::sampler_bank::{assets, resident, BankId, Factory, Slot, Source, SLOTS};
use std::sync::atomic::{AtomicU8, Ordering};

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
pub(crate) struct Ack(Arc<AtomicU8>);
impl Ack {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }
    pub fn state(&self) -> EditState {
        match self.0.load(Ordering::Acquire) {
            1 => EditState::Applied,
            2 => EditState::Rejected,
            _ => EditState::Pending,
        }
    }
    /// A cancellation before renderer ownership wins. Once claimed, the
    /// renderer reports its actual commit or rejection; late cancellation never
    /// rewrites an applied edit as a failure.
    pub fn cancel(&self) -> bool {
        self.0.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
    pub(in crate::engine) fn claim(&self) -> bool {
        self.0.compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
    pub(super) fn applied(&self) {
        self.0.store(1, Ordering::Release);
    }
    pub(crate) fn reject(&self) {
        let _ = self.0.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |state| matches!(state, 0 | 3).then_some(2));
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub epoch: u64,
    pub target: Target,
    pub bank: Bank,
    pub select: bool,
    pub ack: Ack,
}

pub(super) fn factory_data(owner: &assets::Owner, sr: u32) -> Result<[assets::Bank; 3], String> {
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

pub(crate) fn reject(command: &Command) {
    match command {
        Command::SamplerEdit(edit) => edit.ack.reject(),
        Command::Gesture { command, .. } => reject(command),
        _ => (),
    }
}
pub(super) fn admission_ack(mut command: &Command) -> Option<Ack> {
    loop {
        match command {
            Command::SamplerEdit(edit) => return Some(edit.ack.clone()),
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
