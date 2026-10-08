use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub model: Arc<Model>,
    pub looping: bool,
    pub next_id: u32,
}
impl Saved {
    /// Validate remembered navigation without retaining a pending jump.
    /// Takes the immutable locator model and loop flag; returns a refusal for invalid metadata or an enabled loop without braces.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.model.validate()?;
        if self.next_id < self.model.next_id || self.next_id > 65536 {
            return Err("Song locator identity counter is invalid".into());
        }
        if self.looping && self.model.loop_region.is_none() {
            return Err("Draw loop braces before enabling song looping".into());
        }
        Ok(())
    }
    /// Compare one exact reviewed navigation state.
    /// Takes the current state; returns true only for the same metadata identity and loop flag, without allocating.
    pub(crate) fn same(&self, current: &Self) -> bool {
        Arc::ptr_eq(&self.model, &current.model)
            && self.looping == current.looping
            && self.next_id == current.next_id
    }
}

pub(crate) const MAX_LOCATORS: usize = 256;
pub(crate) const MAX_NAME_BYTES: usize = 256;
pub(crate) const MAX_BEATS: f64 = 262144.0;
pub(crate) const MIN_LOOP_BEATS: f64 = 1.0 / 960.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Locator {
    pub id: u16,
    pub name: String,
    pub beat: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Loop {
    pub start: f64,
    pub end: f64,
}
impl Loop {
    /// Validate a half-open musical loop.
    /// Takes its start/end; returns whether both are finite, in range and at least one MIDI tick apart.
    pub(crate) fn valid(self) -> bool {
        position(self.start) && position(self.end) && self.end - self.start >= MIN_LOOP_BEATS
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub next_id: u32,
    pub locators: Vec<Locator>,
    pub loop_region: Option<Loop>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            next_id: 1,
            locators: Vec::new(),
            loop_region: None,
        }
    }
}
/// Check one musical position.
/// Takes a quarter-note beat; returns whether native storage can represent it.
pub(crate) fn position(beat: f64) -> bool {
    beat.is_finite() && (0.0..=MAX_BEATS).contains(&beat)
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= MAX_NAME_BYTES && !name.chars().any(char::is_control)
}
impl Model {
    /// Validate bounded locator metadata before publication.
    /// Takes one saved or edited model; returns a refusal for bad names, identities, positions, ordering or loops.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.next_id == 0
            || self.next_id > u32::from(u16::MAX) + 1
            || self.locators.len() > MAX_LOCATORS
        {
            return Err("Song navigation exceeds its locator or identity limit".into());
        }
        for (index, locator) in self.locators.iter().enumerate() {
            if locator.id == 0
                || u32::from(locator.id) >= self.next_id
                || !name_valid(&locator.name)
                || !position(locator.beat)
                || self.locators[..index]
                    .iter()
                    .any(|other| other.id == locator.id)
                || index > 0
                    && (self.locators[index - 1].beat > locator.beat
                        || self.locators[index - 1].beat == locator.beat
                            && self.locators[index - 1].id > locator.id)
            {
                return Err("Song locators need unique stable IDs, visible names and sorted finite positions".into());
            }
        }
        if self.loop_region.is_some_and(|region| !region.valid()) {
            return Err(
                "Song loop braces need in-range positions at least 1/960 beat apart".into(),
            );
        }
        Ok(())
    }
    /// Count retained storage for native Undo admission.
    /// Takes this immutable model; returns bytes including reserved vector and string capacities.
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.locators.capacity() * std::mem::size_of::<Locator>()
            + self
                .locators
                .iter()
                .map(|locator| locator.name.capacity())
                .sum::<usize>()
    }
    /// Create a named locator without reusing deleted identities.
    /// Takes a visible name and musical position; returns its stable MIDI-addressable ID or leaves the draft unchanged on refusal.
    pub(crate) fn add(&mut self, name: String, beat: f64) -> Result<u16, String> {
        if self.locators.len() >= MAX_LOCATORS
            || !name_valid(&name)
            || !position(beat)
            || self.next_id == 0
            || self.next_id > u32::from(u16::MAX)
        {
            return Err("Choose a visible locator name and finite beat; at most 256 locators and 65,535 lifetime identities are supported".into());
        }
        let id = self.next_id as u16;
        self.next_id += 1;
        self.locators.push(Locator { id, name, beat });
        self.sort();
        Ok(id)
    }
    /// Edit one retained locator.
    /// Takes its stable ID, replacement name and beat; returns success only when the complete edit is valid.
    pub(crate) fn update(&mut self, id: u16, name: String, beat: f64) -> Result<(), String> {
        if !name_valid(&name) || !position(beat) {
            return Err("Choose a visible locator name and finite beat".into());
        }
        let locator = self
            .locators
            .iter_mut()
            .find(|locator| locator.id == id)
            .ok_or("Locator no longer exists")?;
        locator.name = name;
        locator.beat = beat;
        self.sort();
        Ok(())
    }
    /// Delete exactly one retained locator.
    /// Takes its stable ID; returns success without renumbering other locators or changing independent loop braces.
    pub(crate) fn delete(&mut self, id: u16) -> Result<(), String> {
        let index = self
            .locators
            .iter()
            .position(|locator| locator.id == id)
            .ok_or("Locator no longer exists")?;
        self.locators.remove(index);
        Ok(())
    }
    fn sort(&mut self) {
        for locator in &mut self.locators {
            if locator.beat == 0.0 {
                locator.beat = 0.0;
            }
        }
        self.locators
            .sort_unstable_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
    }
    /// Resolve a mapped locator independently of display order.
    /// Takes its stable ID; returns the retained musical destination without allocation.
    pub(crate) fn destination(&self, id: u16) -> Option<f64> {
        self.locators
            .iter()
            .find(|locator| locator.id == id)
            .map(|locator| locator.beat)
    }
    /// Find the next distinct named section.
    /// Takes the current beat and direction; returns a chronological destination without allocation or wrapping.
    pub(crate) fn adjacent(&self, beat: f64, forward: bool) -> Option<f64> {
        if !position(beat) {
            return None;
        }
        if forward {
            self.locators
                .iter()
                .find(|locator| locator.beat > beat + super::super::midi_schedule::BEAT_EPSILON)
                .map(|locator| locator.beat)
        } else {
            self.locators
                .iter()
                .rev()
                .find(|locator| locator.beat < beat - super::super::midi_schedule::BEAT_EPSILON)
                .map(|locator| locator.beat)
        }
    }
    /// Build editable loop braces between named sections.
    /// Takes the first locator ID; returns its range through the next distinct position, refusing the final section.
    pub(crate) fn loop_to_next(&self, id: u16) -> Result<Loop, String> {
        let start = self.destination(id).ok_or("Locator no longer exists")?;
        let end = self
            .adjacent(start, true)
            .ok_or("Place a later locator to define this loop")?;
        let region = Loop { start, end };
        if !region.valid() {
            return Err("Named sections are too close to form a loop".into());
        }
        Ok(region)
    }
}
