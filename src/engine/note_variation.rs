use super::{midi_edit::NoteId, MidiNote};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

pub(crate) const CERTAIN: u16 = 10_000;
pub(crate) const DEFAULT_SEED: u64 = 1;
pub(crate) fn default_seed() -> u64 { DEFAULT_SEED }
pub(crate) fn seed_is_default(seed: &u64) -> bool { *seed == DEFAULT_SEED }

/// Refuse future note settings in an older container.
/// Takes raw native metadata; returns an error when older schemas contain seed or note-variation fields, including null fields.
pub(crate) fn reject_legacy_fields(raw: &serde_json::Value) -> Result<(), &'static str> {
    if raw.get("version").and_then(serde_json::Value::as_u64).is_none_or(|version| version >= 34) { return Ok(()) }
    let clips=raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten()
        .flat_map(|track|track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten())
        .chain(raw.get("arrangement").and_then(|arrangement|arrangement.get("sources")).and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|source|source.get("clip")));
    if raw.get("note_seed").is_some() || clips.into_iter().any(has_future_note_fields) { Err("Note variation requires project state version 34") } else { Ok(()) }
}
/// Inspect a clip's future note fields.
/// Takes raw clip metadata; returns whether any note explicitly contains native variation, regardless of its value.
pub(crate) fn has_future_note_fields(clip: &serde_json::Value) -> bool {
    clip.get("notes").and_then(serde_json::Value::as_array).into_iter().flatten().any(|note|note.get("variation").is_some())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Velocity {
    pub minimum: u8,
    pub maximum: u8,
}
impl Velocity {
    pub(crate) fn valid(self) -> bool {
        self.minimum > 0 && self.minimum <= self.maximum && self.maximum <= 127
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GroupKind { Linked, Exclusive }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Group {
    pub identity: NoteId,
    pub kind: GroupKind,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Expression {
    #[default]
    PolyPressure,
    Lower(u8),
    Upper(u8),
}
impl Expression {
    pub(crate) fn valid(self) -> bool {
        match self { Self::PolyPressure => true, Self::Lower(n) | Self::Upper(n) => (1..=15).contains(&n) }
    }
    pub(crate) fn member(self, channel: u8) -> bool {
        match self { Self::PolyPressure => false, Self::Lower(n) => channel > 0 && channel <= n, Self::Upper(n) => channel < 15 && channel >= 15-n }
    }
    fn tools(self) -> super::midi_tools::Expression {
        match self { Self::PolyPressure => super::midi_tools::Expression::PolyPressure, Self::Lower(n) => super::midi_tools::Expression::Lower(n), Self::Upper(n) => super::midi_tools::Expression::Upper(n) }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Properties {
    pub chance: u16,
    pub velocity: Option<Velocity>,
    pub group: Option<Group>,
    #[serde(default)]
    pub expression: Expression,
}
impl Default for Properties {
    fn default() -> Self { Self { chance: CERTAIN, velocity: None, group: None, expression: Expression::default() } }
}
impl Properties {
    pub(crate) fn valid(self) -> bool {
        self.chance <= CERTAIN && self.velocity.is_none_or(Velocity::valid)
            && self.group.is_none_or(|group| group.identity.valid()) && self.expression.valid()
    }
}
#[derive(Clone, Copy, Debug)]
struct Rule {
    key: NoteId,
    domain: u64,
    note: NoteId,
    minimum: u16,
    maximum: u16,
    velocity: Option<Velocity>,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    rules: Vec<Option<Rule>>,
    expression: Vec<Option<usize>>,
}
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Note variation preparation cancelled".into()) } else { Ok(()) }
}
/// Validate saved note choices before playback.
/// Takes stable notes; returns a refusal for invalid values, inconsistent onset groups or exhausted exclusive weights.
pub(crate) fn validate(notes: &[MidiNote]) -> Result<(), String> {
    groups(notes).map(|_| ())
}
/// Give copied probability groups independent identities.
/// Takes producer-owned copied notes; returns coherent new group IDs or refuses failed identity creation before publication.
pub(crate) fn remap_copied_groups(notes: &mut [MidiNote]) -> Result<(), String> {
    let mut identities = BTreeMap::new();
    for note in notes.iter_mut() {
        if let Some(group) = note.variation.as_mut().and_then(|properties|properties.group.as_mut()) {
            let identity = *identities.entry(group.identity).or_insert_with(NoteId::new);
            if !identity.valid() { return Err("A copied probability group identity could not be created".into()); }
            group.identity = identity;
        }
    }
    validate(notes)
}
fn groups(notes: &[MidiNote]) -> Result<BTreeMap<NoteId, Vec<usize>>, String> {
    let mut groups=BTreeMap::<NoteId,Vec<usize>>::new();
    let mut mode=None;
    for (index,note) in notes.iter().enumerate() {
        let Some(properties)=note.variation else { continue };
        if !note.id.valid() || !properties.valid() { return Err("Note chance, velocity range or identity is invalid".into()); }
        if mode.is_some_and(|old| old!=properties.expression) { return Err("Choose one explicit expression ownership mode for this clip's varied notes".into()); }
        mode=Some(properties.expression);
        if properties.expression!=Expression::PolyPressure && !properties.expression.member(note.channel) { return Err("A varied MPE note must use a declared member channel".into()); }
        if let Some(group)=properties.group { groups.entry(group.identity).or_default().push(index); }
    }
    for members in groups.values_mut() {
        members.sort_unstable_by_key(|&index| notes[index].id);
        let first=&notes[members[0]];
        let settings=first.variation.unwrap();
        let mut weight=0u32;
        for &index in members.iter() {
            let note=&notes[index];let other=note.variation.unwrap();
            if other.group!=settings.group || note.source_start()!=first.source_start() { return Err("Probability group notes must share an onset and group kind".into()); }
            if settings.group.unwrap().kind==GroupKind::Linked && other.chance!=settings.chance { return Err("Linked notes must share one chance".into()); }
            weight+=u32::from(other.chance);
        }
        if settings.group.unwrap().kind==GroupKind::Exclusive && weight>u32::from(CERTAIN) { return Err("Exclusive note chances exceed 100%; lower their weights".into()); }
    }
    Ok(groups)
}
impl Plan {
    /// Prepare immutable chance and expression ownership.
    /// Takes bounded clip notes, retained controller lanes and cancellation; returns worker-owned rules, or none for an ordinary unchanged clip.
    pub(crate) fn prepare(notes: &[MidiNote], lanes: Option<&super::midi_data::Lanes>, cancel: &AtomicBool) -> Result<Option<Arc<Self>>, String> {
        cancelled(cancel)?;
        if notes.len()>super::project::MAX_NOTES_PER_CLIP { return Err("Note variation exceeds the clip note limit".into()); }
        if !notes.iter().any(|note| note.variation.is_some()) { return Ok(None); }
        let groups=groups(notes)?;
        let mut rules:Vec<_>=notes.iter().map(|note| note.variation.map(|properties| Rule { key:properties.group.map_or(note.id,|group|group.identity),domain:properties.group.map_or(0,|group| match group.kind { GroupKind::Linked => 2, GroupKind::Exclusive => 3 }),note:note.id,minimum:0,maximum:properties.chance,velocity:properties.velocity })).collect();
        for members in groups.values() {
            cancelled(cancel)?;let mut lower=0;
            for &index in members {
                let settings=notes[index].variation.unwrap();
                if settings.group.unwrap().kind==GroupKind::Exclusive {
                    let rule=rules[index].as_mut().unwrap();rule.minimum=lower;lower+=settings.chance;rule.maximum=lower;
                }
            }
        }
        let expression=if let Some(lanes)=lanes {
            let content=super::midi_tools::Content { notes:notes.to_vec(),ppqn:lanes.ppqn,end_tick:lanes.end_tick,messages:lanes.messages.clone(),meta:lanes.meta.clone(),labels:lanes.labels.clone() };
            let changed:Vec<_>=notes.iter().map(|note|note.variation.is_some()).collect();
            let mode=notes.iter().find_map(|note|note.variation).unwrap().expression;
            super::midi_tools::expression_owners(&content,&changed,mode.tools(),cancel)?.into_iter().map(|owner|owner.filter(|&i|notes[i].variation.is_some())).collect()
        } else { Vec::new() };
        cancelled(cancel)?;rules.shrink_to_fit();
        Ok(Some(Arc::new(Self { rules, expression })))
    }
    /// Make one repeatable musical-pass decision.
    /// Takes prepared note index, saved project seed, musical cycle and original velocity; returns its varied velocity or silence. It allocates nothing.
    pub(crate) fn velocity(&self, index: usize, seed: u64, cycle: i64, original: u8) -> Option<u8> {
        let rule=self.rules.get(index).copied()?;
        let Some(rule)=rule else { return Some(original) };
        if cycle<0 { return None }
        let chance=draw(seed,rule.key,cycle as u64,rule.domain,CERTAIN as u64) as u16;
        if chance<rule.minimum || chance>=rule.maximum { return None }
        Some(rule.velocity.map_or(original,|range|range.minimum+draw(seed,rule.note,cycle as u64,1,u64::from(range.maximum-range.minimum)+1) as u8))
    }
    /// Read an expression's captured owner.
    /// Takes a source message index; returns only the uniquely owned varied note, leaving unrelated automation unchanged.
    pub(crate) fn expression_owner(&self, index: usize) -> Option<usize> { self.expression.get(index).copied().flatten() }
    /// Budget retained worker storage.
    /// Takes this immutable plan; returns the owned bytes needed by history and project preparation.
    pub(crate) fn bytes(&self) -> usize { std::mem::size_of::<Self>()+self.rules.capacity()*std::mem::size_of::<Option<Rule>>()+self.expression.capacity()*std::mem::size_of::<Option<usize>>() }
}
fn mix(mut value:u64)->u64 {
    value=value.wrapping_add(0x9e3779b97f4a7c15);
    value=(value^(value>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value=(value^(value>>27)).wrapping_mul(0x94d049bb133111eb);
    value^(value>>31)
}
fn draw(seed:u64,identity:NoteId,cycle:u64,lane:u64,bound:u64)->u64 {
    let [namespace,sequence]=identity.words();
    let value=mix(mix(seed^namespace)^mix(sequence)^mix(cycle)^mix(lane));
    ((u128::from(value)*u128::from(bound))>>64) as u64
}

#[cfg(test)]
mod tests;
