use super::{Index, Values};
use crate::engine::{
    midi_data::{Lanes, TickTiming},
    midi_edit::NoteId,
    midi_tools::{event_orders, Content},
};
use crate::midi_file::Message;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dimension {
    Pitch,
    Pressure,
    Timbre,
}
impl Dimension {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Pitch => 0,
            Self::Pressure => 1,
            Self::Timbre => 2,
        }
    }
    pub(crate) fn of(message: Message) -> Option<Self> {
        match message.bytes[0] & 0xf0 {
            0xe0 => Some(Self::Pitch),
            0xd0 => Some(Self::Pressure),
            0xb0 if message.bytes[1] == 74 => Some(Self::Timbre),
            _ => None,
        }
    }
    pub(crate) fn length(self) -> u8 {
        if self == Self::Pressure {
            2
        } else {
            3
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Point {
    pub message: Message,
    pub dimension: Dimension,
    pub beat: f64,
    pub value: f64,
    pub range: u8,
}
#[derive(Clone, Debug)]
pub(crate) struct Envelope {
    pub note: NoteId,
    pub channel: u8,
    pub manager: u8,
    pub duration: f64,
    pub initial: Values,
    pub range: u8,
    pub points: Vec<Point>,
    pub dimensions: [Vec<usize>; 3],
}
#[derive(Clone, Copy)]
pub(crate) struct Edit {
    pub dimension: Dimension,
    pub beat: f64,
    pub value: Option<f64>,
    pub original: Option<Message>,
}
#[derive(Debug)]
pub(crate) struct Edited {
    pub content: Content,
    pub channel: u8,
    pub maximum_rounding_beats: f64,
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Note expression operation cancelled".into())
    } else {
        Ok(())
    }
}
fn prepared(content: &Content, cancel: &AtomicBool) -> Result<Arc<Lanes>, String> {
    Lanes::named_with_cancel(
        content.ppqn,
        content.end_tick,
        content.messages.clone(),
        content.meta.clone(),
        content.labels.clone(),
        &mut || cancel.load(Ordering::Acquire),
    )
}
fn source(content: &Content, id: NoteId) -> Result<usize, String> {
    if !id.valid() || content.notes.len() > crate::engine::project::MAX_NOTES_PER_CLIP {
        return Err("Note expression requires a valid captured note identity".into());
    }
    let index = content
        .notes
        .iter()
        .position(|note| note.id == id)
        .ok_or("The expression note was removed")?;
    let note = &content.notes[index];
    if note.channel > 15
        || !note.interchange_valid()
        || !note.source_start().is_finite()
        || !note.source_duration().is_finite()
        || note.source_duration() <= 0.0
    {
        return Err("Note expression coordinates are invalid".into());
    }
    Ok(index)
}
fn coordinate(note: &crate::engine::MidiNote, order: u32, end: bool, ppqn: u16) -> (f64, u32) {
    (
        (note.source_start() + if end { note.source_duration() } else { 0.0 }) * f64::from(ppqn),
        order,
    )
}
fn inspect_prepared(
    content: &Content,
    id: NoteId,
    lanes: &Lanes,
    index: &Index,
    cancel: &AtomicBool,
) -> Result<Envelope, String> {
    let selected = source(content, id)?;
    let note = &content.notes[selected];
    let orders = event_orders(content)?;
    let start = coordinate(note, orders[selected].0, false, content.ppqn);
    let end = coordinate(note, orders[selected].1, true, content.ppqn);
    let initial = index.before_event(start.0, start.1, &lanes.state);
    let (manager, true) = initial
        .role(note.channel)
        .ok_or("This note is outside an enabled MPE zone")?
    else {
        return Err("Select a note on an MPE member channel".into());
    };
    let mut state = initial;
    let mut points = Vec::new();
    for (position, message) in lanes.messages.iter().copied().enumerate() {
        if position % 1024 == 0 {
            cancelled(cancel)?;
        }
        let key = (message.tick as f64, message.order);
        if key < start || key >= end {
            continue;
        }
        let update = state.input(&message.bytes[..usize::from(message.length)]);
        if update.reset & (1 << note.channel) != 0 {
            return Err("An MPE zone change ends this note; split the clip at that change before editing its expression".into());
        }
        if message.bytes[0] & 15 != note.channel {
            continue;
        }
        let Some(dimension) = Dimension::of(message) else {
            continue;
        };
        let range = state.range(note.channel);
        let value = match dimension {
            Dimension::Pitch => f64::from(state.member(note.channel).pitch),
            Dimension::Pressure => f64::from(message.bytes[1]),
            Dimension::Timbre => f64::from(message.bytes[2]),
        };
        points.push(Point {
            message,
            dimension,
            beat: message.tick as f64 / f64::from(content.ppqn) - note.source_start(),
            value,
            range,
        });
    }
    let dimensions = std::array::from_fn(|dimension| {
        points
            .iter()
            .enumerate()
            .filter_map(|(position, point)| {
                (point.dimension.index() == dimension).then_some(position)
            })
            .collect()
    });
    Ok(Envelope {
        note: id,
        channel: note.channel,
        manager,
        duration: note.source_duration(),
        initial: initial.member(note.channel),
        range: initial.range(note.channel),
        points,
        dimensions,
    })
}

/// Inspect a stable note's three expression envelopes.
/// Takes captured MIDI content, note identity and cancellation; returns member pitch in semitones, pressure and timbre without altering the source.
pub(crate) fn inspect(
    content: &Content,
    id: NoteId,
    cancel: &AtomicBool,
) -> Result<Envelope, String> {
    cancelled(cancel)?;
    let lanes = prepared(content, cancel)?;
    let index = lanes
        .expression
        .as_ref()
        .ok_or("This clip has no MPE zone setup")?;
    inspect_prepared(content, id, &lanes, index, cancel)
}

fn packet(dimension: Dimension, channel: u8, value: f64, range: u8) -> Result<[u8; 3], String> {
    if !value.is_finite() {
        return Err("Expression value must be finite".into());
    }
    match dimension {
        Dimension::Pitch => {
            let raw = if range == 0 {
                if value != 0.0 {
                    return Err("A zero bend range accepts only zero semitones".into());
                }
                8192.0
            } else {
                (value * 8192.0 / f64::from(range) + 8192.0).round()
            };
            if !(0.0..=16383.0).contains(&raw) {
                return Err("Pitch expression is outside this member's bend range".into());
            }
            let raw = raw as u16;
            Ok([0xe0 | channel, (raw & 127) as u8, (raw >> 7) as u8])
        }
        Dimension::Pressure | Dimension::Timbre => {
            if !(0.0..=127.0).contains(&value) {
                return Err("Pressure and timbre accept values from zero through 127".into());
            }
            let value = value.round() as u8;
            Ok(if dimension == Dimension::Pressure {
                [0xd0 | channel, value, 0]
            } else {
                [0xb0 | channel, 74, value]
            })
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Message(Message),
    On(usize),
    Off(usize),
    Meta(usize),
}
struct Event {
    order: (u32, u8, usize),
    kind: Kind,
}

/// Edit one note without changing another member's expression.
/// Takes captured MIDI content, stable note identity, one point edit and cancellation; returns a complete draft using a free member channel with restored reuse state and explicit tick rounding.
pub(crate) fn edit(
    content: &Content,
    id: NoteId,
    edit: Edit,
    cancel: &AtomicBool,
) -> Result<Edited, String> {
    cancelled(cancel)?;
    if content.notes.iter().any(|note| {
        !note.id.valid()
            || !note.interchange_valid()
            || !note.source_start().is_finite()
            || !(0.0..=262_144.0).contains(&note.source_start())
            || !note.source_duration().is_finite()
            || note.source_duration() <= 0.0
            || note.source_start() + note.source_duration() > 262_144.0
    }) {
        return Err("MPE editing requires valid captured note coordinates".into());
    }
    if edit.value.is_none() && edit.original.is_none() {
        return Err("Select an existing expression point before removing it".into());
    }
    let selected = source(content, id)?;
    let note = &content.notes[selected];
    if !edit.beat.is_finite() || edit.beat < 0.0 || edit.beat >= note.source_duration() {
        return Err("A per-note expression point must be before that note's release".into());
    }
    let lanes = prepared(content, cancel)?;
    let index = lanes
        .expression
        .as_ref()
        .ok_or("This clip has no MPE zone setup")?;
    let envelope = inspect_prepared(content, id, &lanes, index, cancel)?;
    if edit.original.is_some_and(|message| {
        !envelope
            .points
            .iter()
            .any(|point| point.message == message && point.dimension == edit.dimension)
    }) {
        return Err("The selected expression point changed; inspect this note again".into());
    }
    let orders = event_orders(content)?;
    let start = coordinate(note, orders[selected].0, false, content.ppqn);
    let end = coordinate(note, orders[selected].1, true, content.ppqn);
    let state = index.before_event(start.0, start.1, &lanes.state);
    let mut candidate = None;
    for channel in
        std::iter::once(note.channel).chain((0..16).filter(|channel| *channel != note.channel))
    {
        cancelled(cancel)?;
        if state.role(channel) != Some((envelope.manager, true)) {
            continue;
        }
        if content.notes.iter().enumerate().any(|(position, other)| {
            position != selected
                && other.channel == channel
                && coordinate(other, orders[position].0, false, content.ppqn) < end
                && coordinate(other, orders[position].1, true, content.ppqn) > start
        }) {
            continue;
        }
        let mut probe = state;
        let mut safe = true;
        for (position, message) in lanes.messages.iter().enumerate() {
            if position % 1024 == 0 {
                cancelled(cancel)?;
            }
            if (message.tick as f64, message.order) < start
                || (message.tick as f64, message.order) >= end
            {
                continue;
            }
            if probe
                .input(&message.bytes[..usize::from(message.length)])
                .reset
                & (1 << channel)
                != 0
            {
                safe = false;
                break;
            }
        }
        if safe {
            candidate = Some(channel);
            break;
        }
    }
    let channel = candidate.ok_or("All member channels in this zone are occupied for this note; expand the zone or route a separate MPE part before editing independent expression")?;
    let mut result = content.clone();
    let mut maximum_rounding_beats = 0.0f64;
    for (position, note) in result.notes.iter_mut().enumerate() {
        let start = note.source_start();
        let duration = note.source_duration();
        let tick = (start * f64::from(content.ppqn)).round();
        let length = (duration * f64::from(content.ppqn)).round().max(1.0);
        maximum_rounding_beats = maximum_rounding_beats
            .max((tick / f64::from(content.ppqn) - start).abs())
            .max((length / f64::from(content.ppqn) - duration).abs());
        note.start = (tick / f64::from(content.ppqn)) as f32;
        note.len = (length / f64::from(content.ppqn)) as f32;
        note.source_timing = Some(TickTiming {
            ppqn: content.ppqn,
            start: tick as u64,
            duration: length as u64,
            start_order: orders[position].0,
            end_order: orders[position].1,
        });
    }
    result.notes[selected].channel = channel;
    let selected_timing = result.notes[selected].source_timing.unwrap();
    let tick = ((note.source_start() + edit.beat) * f64::from(content.ppqn)).round() as u64;
    if tick >= selected_timing.start + selected_timing.duration {
        return Err("Tick rounding places that point at the note release; move it earlier".into());
    }
    maximum_rounding_beats = maximum_rounding_beats
        .max((tick as f64 / f64::from(content.ppqn) - note.source_start() - edit.beat).abs());
    let initial = index.before_event(start.0, start.1, &lanes.state);
    let restore = index.before_event(end.0, u32::MAX, &lanes.state);
    let point_state = if edit.beat == 0.0 {
        initial
    } else {
        index.before_event(tick as f64, u32::MAX, &lanes.state)
    };
    let value = edit
        .value
        .map(|value| packet(edit.dimension, channel, value, point_state.range(channel)))
        .transpose()?;
    let mut events = Vec::with_capacity(
        content.messages.len()
            + content.meta.len()
            + content.notes.len() * 2
            + envelope.points.len()
            + 7,
    );
    let owned: std::collections::BTreeSet<_> = envelope
        .points
        .iter()
        .map(|point| (point.message.tick, point.message.order))
        .collect();
    for message in &content.messages {
        if channel == note.channel && owned.contains(&(message.tick, message.order)) {
            continue;
        }
        events.push(Event {
            order: (message.order, 1, 0),
            kind: Kind::Message(*message),
        });
    }
    for (position, meta) in content.meta.iter().enumerate() {
        events.push(Event {
            order: (meta.order, 1, 1),
            kind: Kind::Meta(position),
        });
    }
    for (position, (on, off)) in orders.iter().copied().enumerate() {
        events.push(Event {
            order: (on, 1, 2),
            kind: Kind::On(position),
        });
        events.push(Event {
            order: (off, 1, 3),
            kind: Kind::Off(position),
        });
    }
    let mut initial_packets = initial.member_packets(note.channel);
    for bytes in &mut initial_packets {
        bytes[0] = (bytes[0] & 0xf0) | channel;
    }
    if tick == selected_timing.start {
        if let Some(value) = value {
            initial_packets[edit.dimension.index()] = value;
        }
    }
    for (dimension, bytes) in initial_packets.into_iter().enumerate() {
        let length = [3, 2, 3][dimension];
        events.push(Event {
            order: (orders[selected].0, 0, dimension),
            kind: Kind::Message(Message {
                tick: selected_timing.start,
                order: 0,
                bytes,
                length,
            }),
        });
    }
    for (position, point) in envelope.points.iter().enumerate() {
        if Some(point.message) == edit.original {
            continue;
        }
        let mut message = point.message;
        message.bytes[0] = (message.bytes[0] & 0xf0) | channel;
        events.push(Event {
            order: (message.order, 2, position + 3),
            kind: Kind::Message(message),
        });
    }
    if tick != selected_timing.start {
        if let Some(value) = value {
            events.push(Event {
                order: (u32::MAX, 3, 0),
                kind: Kind::Message(Message {
                    tick,
                    order: 0,
                    bytes: value,
                    length: edit.dimension.length(),
                }),
            });
        }
    }
    for (dimension, bytes) in restore.member_packets(channel).into_iter().enumerate() {
        events.push(Event {
            order: (orders[selected].1, 2, dimension),
            kind: Kind::Message(Message {
                tick: selected_timing.start + selected_timing.duration,
                order: 0,
                bytes,
                length: [3, 2, 3][dimension],
            }),
        });
    }
    if events.len() > crate::midi_file::MAX_EVENTS {
        return Err("Note expression edit exceeds the MIDI event limit".into());
    }
    events.sort_unstable_by_key(|event| event.order);
    result.messages.clear();
    for (order, event) in events.into_iter().enumerate() {
        if order % 1024 == 0 {
            cancelled(cancel)?;
        }
        let order = u32::try_from(order).map_err(|_| "MIDI event order is exhausted")?;
        match event.kind {
            Kind::Message(mut message) => {
                message.order = order;
                result.messages.push(message);
            }
            Kind::On(position) => {
                result.notes[position]
                    .source_timing
                    .as_mut()
                    .unwrap()
                    .start_order = order
            }
            Kind::Off(position) => {
                result.notes[position]
                    .source_timing
                    .as_mut()
                    .unwrap()
                    .end_order = order
            }
            Kind::Meta(position) => result.meta[position].order = order,
        }
    }
    result
        .messages
        .sort_unstable_by_key(|message| (message.tick, message.order));
    result.end_tick = result
        .end_tick
        .max(selected_timing.start + selected_timing.duration);
    prepared(&result, cancel)?;
    Ok(Edited {
        content: result,
        channel,
        maximum_rounding_beats,
    })
}

#[cfg(test)]
mod tests;
