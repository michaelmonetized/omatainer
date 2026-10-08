use super::{midi_data::Label, midi_edit::NoteId, MidiNote};
use crate::midi_file::{Message, Meta};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

const LIMIT: f64 = 262_144.0;
pub(crate) const CURVE_POINTS: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Content {
    pub notes: Vec<MidiNote>,
    pub ppqn: u16,
    pub end_tick: u64,
    pub messages: Vec<Message>,
    pub meta: Vec<Meta>,
    pub labels: Vec<Label>,
}
impl Content {
    fn lane_bytes(&self) -> usize {
        let base = self
            .messages
            .len()
            .saturating_mul(std::mem::size_of::<Message>())
            .saturating_add(self.meta.len().saturating_mul(std::mem::size_of::<Meta>()))
            .saturating_add(
                self.labels
                    .len()
                    .saturating_mul(std::mem::size_of::<Label>()),
            );
        self.meta
            .iter()
            .fold(base, |bytes, m| {
                bytes.saturating_add(match &m.value {
                    crate::midi_file::MetaValue::Text { bytes, .. } => bytes.len(),
                    _ => 0,
                })
            })
            .saturating_add(
                self.labels
                    .iter()
                    .fold(0usize, |bytes, l| bytes.saturating_add(l.name.len())),
            )
    }
    /// Compare exact controller content.
    /// Takes another draft payload; returns whether source ticks, messages, metadata and labels match.
    pub(crate) fn same_lanes(&self, other: &Self) -> bool {
        self.ppqn == other.ppqn
            && self.end_tick == other.end_tick
            && self.messages == other.messages
            && self.meta == other.meta
            && self.labels == other.labels
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Quantize,
    Recombine,
    Velocity,
    Stretch,
    Reverse,
    Warp,
    ScaleTranspose,
    Harmony,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Property {
    Pitch,
    Position,
    Length,
    Velocity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expression {
    PolyPressure,
    Lower(u8),
    Upper(u8),
}
impl Expression {
    fn member(self, channel: u8) -> bool {
        match self {
            Self::PolyPressure => false,
            Self::Lower(n) => channel > 0 && channel <= n,
            Self::Upper(n) => channel < 15 && channel >= 15 - n,
        }
    }
    fn valid(self) -> bool {
        match self {
            Self::PolyPressure => true,
            Self::Lower(n) | Self::Upper(n) => (1..=15).contains(&n),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Parameters {
    pub kind: Kind,
    pub property: Property,
    pub rotation: i32,
    pub mirror: bool,
    pub shuffle: bool,
    pub seed: u64,
    pub grid: f64,
    pub strength: f64,
    pub starts: bool,
    pub ends: bool,
    pub stretch: f64,
    pub warp_time: f64,
    pub warp_speed: [f64; 3],
    pub preserve_range: bool,
    pub curve: [f64; CURVE_POINTS],
    pub cycles: u8,
    pub phase: f64,
    pub velocity: [u8; 2],
    pub expression: Expression,
    pub context: Option<super::musical_context::Context>,
    pub degrees: i16,
    pub include_chromatic: bool,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            kind: Kind::Quantize,
            property: Property::Pitch,
            rotation: 0,
            mirror: false,
            shuffle: false,
            seed: 1,
            grid: 0.25,
            strength: 1.0,
            starts: true,
            ends: false,
            stretch: 1.0,
            warp_time: 0.5,
            warp_speed: [1.0; 3],
            preserve_range: true,
            curve: std::array::from_fn(|i| i as f64 / (CURVE_POINTS - 1) as f64),
            cycles: 1,
            phase: 0.0,
            velocity: [32, 112],
            expression: Expression::PolyPressure,
            context: None,
            degrees: 2,
            include_chromatic: false,
        }
    }
}
impl Parameters {
    fn valid(&self) -> bool {
        self.rotation.unsigned_abs() < super::project::MAX_NOTES_PER_CLIP as u32
            && self.grid.is_finite()
            && (1.0 / 1024.0..=4.0).contains(&self.grid)
            && self.strength.is_finite()
            && (0.0..=1.0).contains(&self.strength)
            && (self.kind != Kind::Quantize || self.starts || self.ends)
            && self.stretch.is_finite()
            && (0.125..=8.0).contains(&self.stretch)
            && self.warp_time.is_finite()
            && (0.01..=0.99).contains(&self.warp_time)
            && self
                .warp_speed
                .iter()
                .all(|v| v.is_finite() && (0.125..=8.0).contains(v))
            && self
                .curve
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && (1..=16).contains(&self.cycles)
            && self.phase.is_finite()
            && (0.0..=1.0).contains(&self.phase)
            && self.velocity[0] > 0
            && self.velocity[0] <= self.velocity[1]
            && self.velocity[1] <= 127
            && self.expression.valid()
            && (-128..=128).contains(&self.degrees)
            && self
                .context
                .is_none_or(super::musical_context::Context::valid)
            && (!matches!(self.kind, Kind::ScaleTranspose | Kind::Harmony)
                || self.context.is_some())
    }
    fn integral(&self, phase: f64) -> f64 {
        let integrate = |width: f64, a: f64, b: f64, part: f64| {
            let slope = (b - a) / width;
            if slope.abs() < 1e-12 {
                part / a
            } else {
                (slope * part / a).ln_1p() / slope
            }
        };
        let first = phase.min(self.warp_time);
        let mut area = integrate(
            self.warp_time,
            self.warp_speed[0],
            self.warp_speed[1],
            first,
        );
        if phase > self.warp_time {
            area += integrate(
                1.0 - self.warp_time,
                self.warp_speed[1],
                self.warp_speed[2],
                phase - self.warp_time,
            );
        }
        area
    }
    fn warp(&self, beat: f64, first: f64, span: f64) -> f64 {
        let phase = ((beat - first) / span).clamp(0.0, 1.0);
        first
            + span * self.integral(phase)
                / if self.preserve_range {
                    self.integral(1.0)
                } else {
                    1.0
                }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Timing {
    pub notes: usize,
    pub first: f64,
    pub end: f64,
    pub total_length: f64,
    pub maximum_overlap: usize,
    pub total_gap: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Summary {
    pub original: Timing,
    pub transformed: Timing,
    pub mean_start_shift: f64,
    pub maximum_rounding_beats: f64,
    pub expression_events: usize,
}
#[derive(Debug)]
pub(crate) struct Prepared {
    pub content: Content,
    pub summary: Summary,
}
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("MIDI transformation cancelled; the draft is unchanged".into())
    } else {
        Ok(())
    }
}
fn random(value: &mut u64) -> u64 {
    *value = value.wrapping_add(0x9e3779b97f4a7c15);
    let mut result = *value;
    result = (result ^ (result >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    result = (result ^ (result >> 27)).wrapping_mul(0x94d049bb133111eb);
    result ^ (result >> 31)
}
fn timing(notes: &[MidiNote], indices: &[usize]) -> Timing {
    let mut result = Timing {
        notes: indices.len(),
        first: f64::INFINITY,
        end: 0.0,
        total_length: 0.0,
        maximum_overlap: 0,
        total_gap: 0.0,
    };
    let mut events = Vec::with_capacity(indices.len() * 2);
    for &i in indices {
        let n = &notes[i];
        let start = n.source_start();
        let end = start + n.source_duration();
        result.first = result.first.min(start);
        result.end = result.end.max(end);
        result.total_length += n.source_duration();
        if end > start {
            events.push((start, 1i32));
            events.push((end, -1));
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut count = 0i32;
    let mut last = result.first;
    for (beat, delta) in events {
        if count == 0 && last.is_finite() {
            result.total_gap += (beat - last).max(0.0);
        }
        last = beat;
        count += delta;
        result.maximum_overlap = result.maximum_overlap.max(count.max(0) as usize);
    }
    if count == 0 && last.is_finite() {
        result.total_gap += (result.end - last).max(0.0);
    }
    result
}
fn coordinates(note: &mut MidiNote, start: f64, end: f64) -> Result<f64, String> {
    if !start.is_finite()
        || !end.is_finite()
        || !(0.0..=LIMIT).contains(&start)
        || !(start..=LIMIT).contains(&end)
        || end == start && note.len > 0.0
    {
        return Err("The transformation would collapse a note or leave the supported beat range; reduce its amount or change its targets".into());
    }
    if let Some(mut ticks) = note.source_timing {
        let next_start = (start * f64::from(ticks.ppqn)).round() as u64;
        let next_end = (end * f64::from(ticks.ppqn)).round() as u64;
        if next_start == next_end && note.len > 0.0 {
            return Err(
                "The source tick resolution would collapse a note; choose a gentler transformation"
                    .into(),
            );
        }
        ticks.start = next_start;
        ticks.duration = next_end - next_start;
        if !ticks.valid() {
            return Err("Transformed source ticks leave their supported range".into());
        }
        note.start = ticks.start_beats() as f32;
        note.len = ticks.duration_beats() as f32;
        note.source_timing = Some(ticks);
    } else {
        note.start = start as f32;
        note.len = (end - start) as f32;
    }
    Ok((note.source_start() - start)
        .abs()
        .max((note.source_start() + note.source_duration() - end).abs()))
}
fn expression(message: Message, mode: Expression) -> bool {
    let status = message.bytes[0] & 0xf0;
    status == 0xa0
        || mode.member(message.bytes[0] & 15)
            && (matches!(status, 0xd0 | 0xe0) || status == 0xb0 && message.bytes[1] == 74)
}
#[derive(Clone, Copy)]
enum Event {
    On(usize),
    Off(usize),
    Expression(usize),
}
fn event_orders(content: &Content) -> Result<Vec<(u32, u32)>, String> {
    let mut next = content
        .messages
        .iter()
        .map(|m| m.order)
        .chain(content.meta.iter().map(|m| m.order))
        .chain(
            content
                .notes
                .iter()
                .filter_map(|n| n.source_timing.map(|t| t.end_order)),
        )
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("MIDI event order is exhausted")?;
    content
        .notes
        .iter()
        .map(|n| {
            if let Some(t) = n.source_timing {
                Ok((t.start_order, t.end_order))
            } else {
                let start = next;
                next = next.checked_add(2).ok_or("MIDI event order is exhausted")?;
                Ok((start, start + 1))
            }
        })
        .collect()
}
#[derive(Default)]
struct Held {
    notes: BTreeSet<usize>,
    changes: usize,
}
impl Held {
    fn insert(&mut self, index: usize, changed: bool) {
        if self.notes.insert(index) && changed {
            self.changes += 1;
        }
    }
    fn remove(&mut self, index: usize, changed: bool) {
        if self.notes.remove(&index) && changed {
            self.changes -= 1;
        }
    }
}
/// Capture uniquely owned note expression for a prepared clip.
/// Takes validated content, changed-note flags, explicit expression mode and cancellation; returns one owner per source message or a refusal for ambiguous voices.
pub(crate) fn expression_owners(
    content: &Content, changed: &[bool], mode: Expression, cancel: &AtomicBool,
) -> Result<Vec<Option<usize>>, String> {
    if changed.len() != content.notes.len() { return Err("Expression ownership flags do not match the clip notes".into()); }
    owners(content, changed, mode, cancel)
}
fn owners(
    content: &Content,
    changed: &[bool],
    mode: Expression,
    cancel: &AtomicBool,
) -> Result<Vec<Option<usize>>, String> {
    let orders = event_orders(content)?;
    let mut events = Vec::with_capacity(content.notes.len() * 2 + content.messages.len());
    let mut upcoming = BTreeMap::<(u64, u8, u8), Held>::new();
    for (i, n) in content.notes.iter().enumerate() {
        let start = n.source_start();
        let end = start + n.source_duration();
        events.push((start, orders[i].0, Event::On(i)));
        events.push((end, orders[i].1, Event::Off(i)));
        for pitch in [n.pitch, 128] {
            upcoming
                .entry((start.to_bits(), n.channel, pitch))
                .or_default()
                .insert(i, changed[i]);
        }
    }
    for (i, &m) in content
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| expression(**m, mode))
    {
        events.push((
            m.tick as f64 / f64::from(content.ppqn),
            m.order,
            Event::Expression(i),
        ));
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut active = BTreeMap::<(u8, u8), Held>::new();
    let mut result = vec![None; content.messages.len()];
    for (step, (beat, _, event)) in events.into_iter().enumerate() {
        if step % 64 == 0 {
            cancelled(cancel)?;
        }
        match event {
            Event::On(i) => {
                let n = &content.notes[i];
                for pitch in [n.pitch, 128] {
                    upcoming
                        .get_mut(&(beat.to_bits(), n.channel, pitch))
                        .unwrap()
                        .remove(i, changed[i]);
                    active
                        .entry((n.channel, pitch))
                        .or_default()
                        .insert(i, changed[i]);
                }
            }
            Event::Off(i) => {
                let n = &content.notes[i];
                for pitch in [n.pitch, 128] {
                    if let Some(held) = active.get_mut(&(n.channel, pitch)) {
                        held.remove(i, changed[i]);
                    }
                }
            }
            Event::Expression(i) => {
                let m = content.messages[i];
                let channel = m.bytes[0] & 15;
                let pitch = if m.bytes[0] & 0xf0 == 0xa0 {
                    m.bytes[1]
                } else {
                    128
                };
                let candidates = active
                    .get(&(channel, pitch))
                    .filter(|set| !set.notes.is_empty())
                    .or_else(|| upcoming.get(&(beat.to_bits(), channel, pitch)));
                if let Some(set) = candidates {
                    if set.notes.len() > 1 && set.changes > 0 {
                        return Err(format!("Expression has multiple possible notes on channel {}. Leave their pitch/time unchanged or use a separate voice",channel+1));
                    }
                    if set.notes.len() == 1 {
                        result[i] = set.notes.first().copied();
                    }
                }
            }
        }
    }
    Ok(result)
}
fn validate_wire(content: &Content, cancel: &AtomicBool) -> Result<(), String> {
    let orders = event_orders(content)?;
    let notes = content
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let start = (n.source_start() * f64::from(content.ppqn)).round() as u64;
            let end =
                ((n.source_start() + n.source_duration()) * f64::from(content.ppqn)).round() as u64;
            crate::midi_file::Note {
                channel: n.channel,
                pitch: n.pitch,
                velocity: n.vel,
                release_velocity: n.release_vel,
                start_tick: start,
                duration_ticks: end - start,
                start_order: orders[i].0,
                end_order: orders[i].1,
            }
        })
        .collect::<Vec<_>>();
    let end_tick = content
        .end_tick
        .max(
            notes
                .iter()
                .map(|n| n.start_tick + n.duration_ticks)
                .max()
                .unwrap_or(0),
        )
        .max(content.messages.iter().map(|m| m.tick).max().unwrap_or(0));
    crate::midi_file::encode_with_cancel(
        &crate::midi_file::File {
            format: crate::midi_file::Format::Single,
            ppqn: content.ppqn,
            tracks: vec![crate::midi_file::Track {
                end_tick,
                notes,
                messages: content.messages.clone(),
                meta: content.meta.clone(),
            }],
            warnings: vec![],
        },
        false,
        || cancel.load(Ordering::Acquire),
    )
    .map(|_| ())
    .map_err(|e| format!("The transformed MIDI cannot retain its event pairing: {e}"))
}
fn protect_voices(
    original: &Content,
    next: &Content,
    changed: &[bool],
    ownership: &[Option<usize>],
    mode: Expression,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut protected = BTreeSet::new();
    for channel in 0..16 {
        if mode.member(channel) {
            protected.insert((channel, 128));
        }
    }
    for (i, &m) in original
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.bytes[0] & 0xf0 == 0xa0)
    {
        protected.insert((
            m.bytes[0] & 15,
            ownership[i].map_or(m.bytes[1], |n| next.notes[n].pitch),
        ));
    }
    let orders = event_orders(next)?;
    let mut events = Vec::with_capacity(next.notes.len() * 2 + original.messages.len());
    let mut upcoming = BTreeMap::<(u64, u8, u8), Held>::new();
    for (i, n) in next.notes.iter().enumerate() {
        let start = n.source_start();
        let end = start + n.source_duration();
        events.push((start, orders[i].0, Event::On(i)));
        events.push((end, orders[i].1, Event::Off(i)));
        for pitch in [n.pitch, 128] {
            if protected.contains(&(n.channel, pitch)) {
                upcoming
                    .entry((start.to_bits(), n.channel, pitch))
                    .or_default()
                    .insert(i, changed[i]);
            }
        }
    }
    for (i, &m) in original
        .messages
        .iter()
        .enumerate()
        .filter(|(i, m)| ownership[*i].is_none() && expression(**m, mode))
    {
        events.push((
            m.tick as f64 / f64::from(original.ppqn),
            m.order,
            Event::Expression(i),
        ));
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut active = BTreeMap::<(u8, u8), Held>::new();
    for (step, (beat, _, event)) in events.into_iter().enumerate() {
        if step % 64 == 0 {
            cancelled(cancel)?;
        }
        match event {
            Event::On(i) => {
                let n = &next.notes[i];
                for pitch in [n.pitch, 128] {
                    if protected.contains(&(n.channel, pitch)) {
                        upcoming
                            .get_mut(&(beat.to_bits(), n.channel, pitch))
                            .unwrap()
                            .remove(i, changed[i]);
                        let held = active.entry((n.channel, pitch)).or_default();
                        if n.source_duration() > 0.0
                            && !held.notes.is_empty()
                            && (changed[i] || held.changes > 0)
                        {
                            return Err(format!("The transformation would overlap expression voices on channel {}; their expression must keep a unique note",n.channel+1));
                        }
                        held.insert(i, changed[i]);
                    }
                }
            }
            Event::Off(i) => {
                let n = &next.notes[i];
                for pitch in [n.pitch, 128] {
                    if let Some(held) = active.get_mut(&(n.channel, pitch)) {
                        held.remove(i, changed[i]);
                    }
                }
            }
            Event::Expression(i) => {
                let m = original.messages[i];
                let channel = m.bytes[0] & 15;
                let pitch = if m.bytes[0] & 0xf0 == 0xa0 {
                    m.bytes[1]
                } else {
                    128
                };
                if active
                    .get(&(channel, pitch))
                    .is_some_and(|set| set.changes > 0)
                    || upcoming
                        .get(&(beat.to_bits(), channel, pitch))
                        .is_some_and(|set| set.changes > 0)
                {
                    return Err("An unowned expression event would enter a transformed note. Select a phrase that keeps that channel's expression separate".into());
                }
            }
        }
    }
    Ok(())
}

/// Prepare one reversible MIDI transformation.
/// Takes immutable draft content, stable selected identities, parameters and cancellation; returns validated notes, preserved expression events and exact timing summaries without renderer or device access.
pub(crate) fn prepare(
    original: &Content,
    selected: &BTreeSet<NoteId>,
    params: &Parameters,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    if params.kind == Kind::Harmony {
        return scale::harmonize(original, selected, params, cancel);
    }
    cancelled(cancel)?;
    if original.lane_bytes() > super::midi_data::MAX_LANE_BYTES
        || original.end_tick > u64::from(original.ppqn) * 262144
        || original.labels.len() > 4096
        || original.labels.iter().any(|l| {
            l.channel >= 16
                || !l.control.valid()
                || l.name.is_empty()
                || l.name.len() > 256
                || l.name.chars().any(char::is_control)
        })
        || !params.valid()
        || original.ppqn == 0
        || original.ppqn > 32767
        || original.notes.len() > super::project::MAX_NOTES_PER_CLIP
        || original.messages.len() + original.meta.len() + original.notes.len() * 2 + 1
            > crate::midi_file::MAX_EVENTS
    {
        return Err("MIDI transformation parameters or content exceed supported limits".into());
    }
    let mut ids = BTreeSet::new();
    if original.notes.iter().any(|n| {
        !n.id.valid()
            || !ids.insert(n.id)
            || !n.interchange_valid()
            || n.pitch > 127
            || n.vel == 0
            || n.vel > 127
            || !n.start.is_finite()
            || !n.len.is_finite()
            || !(0.0..=LIMIT).contains(&n.source_start())
            || !(0.0..=LIMIT).contains(&n.source_duration())
    }) || selected.is_empty()
        || !selected.is_subset(&ids)
    {
        return Err("Choose a nonempty stable note selection with valid MIDI values".into());
    }
    let mut indices = original
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| selected.contains(&n.id))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    indices.sort_by(|&a, &b| {
        original.notes[a]
            .source_start()
            .total_cmp(&original.notes[b].source_start())
            .then(
                original.notes[a]
                    .source_timing
                    .map(|t| t.start_order)
                    .cmp(&original.notes[b].source_timing.map(|t| t.start_order)),
            )
            .then(original.notes[a].id.cmp(&original.notes[b].id))
    });
    let original_timing = timing(&original.notes, &indices);
    let first = original_timing.first;
    let span = original_timing.end - first;
    if matches!(params.kind, Kind::Warp | Kind::Reverse) && span <= 0.0 {
        return Err("This transformation needs a phrase with positive duration".into());
    }
    let mut permutation = (0..indices.len()).collect::<Vec<_>>();
    if params.shuffle {
        let mut state = params.seed;
        for i in (1..permutation.len()).rev() {
            let j = (random(&mut state) % (i as u64 + 1)) as usize;
            permutation.swap(i, j);
        }
    }
    if params.mirror {
        permutation.reverse();
    }
    let rotation = params.rotation.rem_euclid(permutation.len() as i32) as usize;
    permutation.rotate_right(rotation);
    let mut content = original.clone();
    let mut rounding = 0.0f64;
    let last_start = indices
        .iter()
        .map(|&i| original.notes[i].source_start())
        .fold(first, f64::max);
    for (position, &index) in indices.iter().enumerate() {
        if position % 64 == 0 {
            cancelled(cancel)?;
        }
        let old = &original.notes[index];
        let n = &mut content.notes[index];
        let start = old.source_start();
        let length = old.source_duration();
        let mut new_start = start;
        let mut new_end = start + length;
        match params.kind {
            Kind::ScaleTranspose => {
                n.pitch = params
                    .context
                    .ok_or("Choose a saved scale before transposing")?
                    .transpose(n.pitch, params.degrees, params.include_chromatic)?;
            }
            Kind::Harmony => unreachable!("Harmony uses the bounded copy preparation path"),
            Kind::Quantize => {
                let snap =
                    |v: f64| v + ((v / params.grid).round() * params.grid - v) * params.strength;
                if params.starts {
                    new_start = snap(start);
                }
                new_end = if params.ends {
                    snap(start + length)
                } else {
                    new_start + length
                };
            }
            Kind::Stretch => {
                new_start = first + (start - first) * params.stretch;
                new_end = first + (start + length - first) * params.stretch;
            }
            Kind::Reverse => {
                new_start = first + original_timing.end - (start + length);
                new_end = new_start + length;
            }
            Kind::Warp => {
                new_start = params.warp(start, first, span);
                new_end = if params.ends {
                    params.warp(start + length, first, span)
                } else {
                    new_start + length
                };
            }
            Kind::Recombine => {
                let value = &original.notes[indices[permutation[position]]];
                match params.property {
                    Property::Pitch => n.pitch = value.pitch,
                    Property::Velocity => n.vel = value.vel,
                    Property::Position => {
                        new_start = value.source_start();
                        new_end = new_start + length;
                    }
                    Property::Length => new_end = start + value.source_duration(),
                };
            }
            Kind::Velocity => {
                let phase = if last_start > first {
                    (start - first) / (last_start - first)
                } else {
                    0.0
                };
                let cycle = phase * f64::from(params.cycles) + params.phase;
                let phase = if cycle > 0.0 && cycle.fract().abs() < 1e-12 {
                    1.0
                } else {
                    cycle.rem_euclid(1.0)
                };
                let x = phase * (CURVE_POINTS - 1) as f64;
                let a = (x.floor() as usize).min(CURVE_POINTS - 1);
                let b = (a + 1).min(CURVE_POINTS - 1);
                let value = params.curve[a] + (params.curve[b] - params.curve[a]) * (x - a as f64);
                let velocity = f64::from(params.velocity[0])
                    + value * f64::from(params.velocity[1] - params.velocity[0]);
                n.vel = (f64::from(old.vel) + (velocity - f64::from(old.vel)) * params.strength)
                    .round()
                    .clamp(1.0, 127.0) as u8;
            }
        }
        if new_start != start || new_end != start + length {
            rounding = rounding.max(coordinates(n, new_start, new_end)?);
        }
    }
    let changed = original
        .notes
        .iter()
        .zip(&content.notes)
        .map(|(old, n)| {
            old.pitch != n.pitch
                || old.source_start() != n.source_start()
                || old.source_duration() != n.source_duration()
        })
        .collect::<Vec<_>>();
    let mut expression_events = 0;
    if changed.iter().any(|v| *v)
        && (params.expression != Expression::PolyPressure
            || original
                .messages
                .iter()
                .any(|&m| expression(m, params.expression)))
    {
        let ownership = owners(original, &changed, params.expression, cancel)?;
        protect_voices(
            original,
            &content,
            &changed,
            &ownership,
            params.expression,
            cancel,
        )?;
        for (i, owner) in ownership.into_iter().enumerate() {
            if i % 64 == 0 {
                cancelled(cancel)?;
            }
            if let Some(index) = owner.filter(|&i| changed[i]) {
                let old = &original.notes[index];
                let new = &content.notes[index];
                let m = &mut content.messages[i];
                let beat = m.tick as f64 / f64::from(content.ppqn);
                let fraction = if old.source_duration() > 0.0 {
                    ((beat - old.source_start()) / old.source_duration()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let next = if params.kind == Kind::Warp && params.ends {
                    params.warp(beat, first, span)
                } else {
                    new.source_start()
                        + new.source_duration()
                            * if params.kind == Kind::Reverse {
                                1.0 - fraction
                            } else {
                                fraction
                            }
                };
                let tick = (next * f64::from(content.ppqn)).round();
                if !tick.is_finite() || tick < 0.0 || tick > LIMIT * f64::from(content.ppqn) {
                    return Err("Expression leaves its supported source tick range".into());
                }
                m.tick = tick as u64;
                rounding = rounding.max((tick / f64::from(content.ppqn) - next).abs());
                if m.bytes[0] & 0xf0 == 0xa0 {
                    m.bytes[1] = new.pitch;
                }
                expression_events += 1;
            }
        }
        content.messages.sort_unstable_by_key(|m| (m.tick, m.order));
    }
    content.end_tick = content
        .end_tick
        .max(
            content
                .notes
                .iter()
                .map(|n| {
                    ((n.source_start() + n.source_duration()) * f64::from(content.ppqn)).ceil()
                        as u64
                })
                .max()
                .unwrap_or(0),
        )
        .max(content.messages.iter().map(|m| m.tick).max().unwrap_or(0));
    validate_wire(&content, cancel)?;
    cancelled(cancel)?;
    let transformed = timing(&content.notes, &indices);
    let mean_start_shift = indices
        .iter()
        .map(|&i| content.notes[i].source_start() - original.notes[i].source_start())
        .sum::<f64>()
        / indices.len() as f64;
    Ok(Prepared {
        content,
        summary: Summary {
            original: original_timing,
            transformed,
            mean_start_shift,
            maximum_rounding_beats: rounding,
            expression_events,
        },
    })
}

#[cfg(test)]
mod tests;

mod scale;
