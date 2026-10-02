//! Bounded Standard MIDI File interchange. Tick coordinates remain integers;
//! conversion to a renderer/editor's musical coordinates belongs to its adapter.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TRACKS: usize = 128;
pub(crate) const MAX_EVENTS: usize = 262_144;
pub(crate) const MAX_NOTES: usize = 65_536;
pub(crate) const MAX_TICK: u64 = 32_767 * 262_144;
const MAX_DELTA: u64 = 0x0fff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Single,
    Parallel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct File {
    pub format: Format,
    pub ppqn: u16,
    pub tracks: Vec<Track>,
    /// Unsupported data is never silently included in a musical import.
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Track {
    pub end_tick: u64,
    pub notes: Vec<Note>,
    pub messages: Vec<Message>,
    pub meta: Vec<Meta>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    pub channel: u8,
    pub pitch: u8,
    pub velocity: u8,
    pub release_velocity: u8,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub start_order: u32,
    pub end_order: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Message {
    pub tick: u64,
    pub order: u32,
    /// Full channel status, followed by one or two seven-bit data bytes.
    pub bytes: [u8; 3],
    pub length: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Meta {
    pub tick: u64,
    pub order: u32,
    pub value: MetaValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MetaValue {
    Tempo(u32), // integer microseconds per quarter note
    Meter {
        numerator: u8,
        denominator_power: u8,
        clocks: u8,
        thirty_seconds: u8,
    },
    Text {
        kind: u8,
        bytes: Vec<u8>,
    }, // standard text 01..09, kept byte-for-byte
    Key {
        sharps: i8,
        minor: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Warning {
    pub track: Option<usize>,
    pub tick: u64,
    pub kind: Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unsupported {
    HeaderExtension(usize),
    Chunk([u8; 4], usize),
    SystemExclusive(u8, usize),
    Meta(u8, usize),
    UnpairedRelease(u8, u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Error {
    pub message: String,
    pub cancelled: bool,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn error(text: impl Into<String>) -> Error {
    Error {
        message: text.into(),
        cancelled: false,
    }
}

fn check_cancel(cancelled: &mut dyn FnMut() -> bool) -> Result<()> {
    if cancelled() {
        Err(Error {
            message: "MIDI operation cancelled".into(),
            cancelled: true,
        })
    } else {
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| error("MIDI byte offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| error("Truncated MIDI data"))?;
        self.offset = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn vlq(&mut self) -> Result<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            let byte = self.byte()?;
            value = (value << 7) | u32::from(byte & 127);
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(error("MIDI variable-length quantity exceeds four bytes"))
    }
    fn chunk(&mut self) -> Result<([u8; 4], &'a [u8])> {
        let kind = self.take(4)?.try_into().unwrap();
        let len = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        Ok((kind, self.take(len)?))
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<File> {
    decode_with_cancel(bytes, || false)
}

pub(crate) fn decode_with_cancel(
    bytes: &[u8],
    mut cancelled: impl FnMut() -> bool,
) -> Result<File> {
    check_cancel(&mut cancelled)?;
    if bytes.len() > MAX_BYTES {
        return Err(error("MIDI file exceeds 16 MiB"));
    }
    let mut input = Reader { bytes, offset: 0 };
    let (kind, header) = input.chunk()?;
    if &kind != b"MThd" || header.len() < 6 {
        return Err(error("Expected a Standard MIDI File header"));
    }
    let format = match u16::from_be_bytes(header[0..2].try_into().unwrap()) {
        0 => Format::Single,
        1 => Format::Parallel,
        2 => {
            return Err(error(
                "Asynchronous SMF format 2 is unsupported; export a format 0 or 1 file",
            ))
        }
        _ => return Err(error("Unknown MIDI file format")),
    };
    let count = u16::from_be_bytes(header[2..4].try_into().unwrap()) as usize;
    if count == 0 || count > MAX_TRACKS || (format == Format::Single && count != 1) {
        return Err(error(
            "MIDI file must contain 1..128 tracks; format 0 requires exactly one",
        ));
    }
    let ppqn = u16::from_be_bytes(header[4..6].try_into().unwrap());
    if ppqn & 0x8000 != 0 {
        return Err(error(
            "SMPTE MIDI time division is unsupported; use PPQN musical timing",
        ));
    }
    if ppqn == 0 {
        return Err(error("MIDI PPQN division must be 1..32767"));
    }
    let mut file = File {
        format,
        ppqn,
        tracks: Vec::with_capacity(count),
        warnings: Vec::new(),
    };
    if header.len() > 6 {
        file.warnings.push(Warning {
            track: None,
            tick: 0,
            kind: Unsupported::HeaderExtension(header.len() - 6),
        });
    }
    let mut events = 0usize;
    let mut notes = 0usize;
    let mut chunks = 0usize;
    while !input.done() {
        check_cancel(&mut cancelled)?;
        chunks += 1;
        if chunks > MAX_TRACKS + 1024 {
            return Err(error("MIDI file contains too many chunks"));
        }
        let (kind, chunk) = input.chunk()?;
        if &kind != b"MTrk" {
            file.warnings.push(Warning {
                track: None,
                tick: 0,
                kind: Unsupported::Chunk(kind, chunk.len()),
            });
            continue;
        }
        if file.tracks.len() >= count {
            return Err(error("MIDI track count differs from its header"));
        }
        let index = file.tracks.len();
        let track = decode_track(
            chunk,
            index,
            &mut events,
            &mut notes,
            &mut file.warnings,
            &mut cancelled,
        )
        .map_err(|mut e| {
            e.message = format!("MIDI track {}: {e}", index + 1);
            e
        })?;
        file.tracks.push(track);
    }
    if file.tracks.len() != count {
        return Err(error("MIDI track count differs from its header"));
    }
    check_cancel(&mut cancelled)?;
    Ok(file)
}

fn decode_track(
    bytes: &[u8],
    track: usize,
    events: &mut usize,
    notes: &mut usize,
    warnings: &mut Vec<Warning>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Track> {
    let mut input = Reader { bytes, offset: 0 };
    let mut output = Track::default();
    let mut tick = 0u64;
    let mut running = None;
    let mut held = BTreeMap::<(u8, u8), VecDeque<(u64, u8, u32)>>::new();
    let mut ended = false;
    while !input.done() {
        if *events % 64 == 0 {
            check_cancel(cancelled)?;
        }
        *events += 1;
        if *events > MAX_EVENTS {
            return Err(error("MIDI file exceeds 262144 events"));
        }
        let order = *events as u32;
        tick = tick
            .checked_add(u64::from(input.vlq()?))
            .filter(|tick| *tick <= MAX_TICK)
            .ok_or_else(|| error("MIDI tick timeline exceeds supported bounds"))?;
        let first = input.byte()?;
        let (status, peek) = if first < 128 {
            (
                running
                    .ok_or_else(|| error("MIDI running status has no preceding channel message"))?,
                Some(first),
            )
        } else {
            (first, None)
        };
        match status {
            0x80..=0xef => {
                running = Some(status);
                let length = if matches!(status >> 4, 0xc | 0xd) {
                    2
                } else {
                    3
                };
                let first = match peek {
                    Some(value) => value,
                    None => input.byte()?,
                };
                let second = if length == 3 { input.byte()? } else { 0 };
                if first >= 128 || second >= 128 {
                    return Err(error("MIDI channel data must be seven-bit"));
                }
                let channel = status & 15;
                if status >> 4 == 9 && second > 0 {
                    *notes += 1;
                    if *notes > MAX_NOTES {
                        return Err(error("MIDI file exceeds 65536 notes"));
                    }
                    held.entry((channel, first))
                        .or_default()
                        .push_back((tick, second, order));
                } else if status >> 4 == 8 || status >> 4 == 9 {
                    let start = held
                        .get_mut(&(channel, first))
                        .and_then(VecDeque::pop_front);
                    if let Some((start_tick, velocity, start_order)) = start {
                        output.notes.push(Note {
                            channel,
                            pitch: first,
                            velocity,
                            release_velocity: second,
                            start_tick,
                            duration_ticks: tick - start_tick,
                            start_order,
                            end_order: order,
                        });
                    } else {
                        warnings.push(Warning {
                            track: Some(track),
                            tick,
                            kind: Unsupported::UnpairedRelease(channel, first),
                        });
                        // A standalone release still has real MIDI semantics and
                        // remains available to the writer after explicit review.
                        output.messages.push(Message {
                            tick,
                            order,
                            bytes: [status, first, second],
                            length,
                        });
                    }
                } else {
                    output.messages.push(Message {
                        tick,
                        order,
                        bytes: [status, first, second],
                        length,
                    });
                }
            }
            0xff if peek.is_none() => {
                // Meta messages do not replace the previous channel status.
                let kind = input.byte()?;
                let count = input.vlq()? as usize;
                let payload = input.take(count)?;
                let value = match kind {
                    0x2f => {
                        if !payload.is_empty() || !input.done() {
                            return Err(error(
                                "End-of-track must be empty and the final track event",
                            ));
                        }
                        ended = true;
                        break;
                    }
                    0x51 => {
                        if payload.len() != 3 {
                            return Err(error("MIDI tempo must contain three bytes"));
                        }
                        let value = (u32::from(payload[0]) << 16)
                            | (u32::from(payload[1]) << 8)
                            | u32::from(payload[2]);
                        if value == 0 {
                            return Err(error("MIDI tempo cannot be zero"));
                        }
                        Some(MetaValue::Tempo(value))
                    }
                    0x58 => {
                        if payload.len() != 4 || payload[0] == 0 {
                            return Err(error("Invalid MIDI time signature"));
                        }
                        Some(MetaValue::Meter {
                            numerator: payload[0],
                            denominator_power: payload[1],
                            clocks: payload[2],
                            thirty_seconds: payload[3],
                        })
                    }
                    0x59 => {
                        if payload.len() != 2
                            || !(-7..=7).contains(&(payload[0] as i8))
                            || payload[1] > 1
                        {
                            return Err(error("Invalid MIDI key signature"));
                        }
                        Some(MetaValue::Key {
                            sharps: payload[0] as i8,
                            minor: payload[1] != 0,
                        })
                    }
                    1..=9 => {
                        // Empty text is also the harmless long-delta spacer
                        // used by the canonical writer; it has no content.
                        if kind == 1 && payload.is_empty() {
                            None
                        } else {
                            Some(MetaValue::Text {
                                kind,
                                bytes: payload.to_vec(),
                            })
                        }
                    }
                    _ => {
                        warnings.push(Warning {
                            track: Some(track),
                            tick,
                            kind: Unsupported::Meta(kind, count),
                        });
                        None
                    }
                };
                if let Some(value) = value {
                    output.meta.push(Meta { tick, order, value });
                }
            }
            0xf0 | 0xf7 if peek.is_none() => {
                running = None;
                let count = input.vlq()? as usize;
                input.take(count)?;
                warnings.push(Warning {
                    track: Some(track),
                    tick,
                    kind: Unsupported::SystemExclusive(status, count),
                });
            }
            _ => {
                return Err(error(
                    "Unsupported or invalid MIDI system status inside SMF track",
                ))
            }
        }
    }
    if !ended {
        return Err(error("MIDI track has no end-of-track event"));
    }
    if held.values().any(|notes| !notes.is_empty()) {
        return Err(error(
            "MIDI track ends with a note lacking its release; no duration was guessed",
        ));
    }
    output.end_tick = tick;
    output
        .notes
        .sort_by_key(|note| (note.start_tick, note.start_order));
    Ok(output)
}

#[derive(Clone, Copy)]
enum WriteEvent<'a> {
    On(&'a Note),
    Off(&'a Note),
    Message(&'a Message),
    Meta(&'a MetaValue),
}

pub(crate) fn encode(file: &File, allow_unsupported_omission: bool) -> Result<Vec<u8>> {
    encode_with_cancel(file, allow_unsupported_omission, || false)
}

pub(crate) fn encode_with_cancel(
    file: &File,
    allow_unsupported_omission: bool,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<u8>> {
    check_cancel(&mut cancelled)?;
    if !file.warnings.is_empty() && !allow_unsupported_omission {
        return Err(error("MIDI file contains reported unsupported events; explicitly approve their omission first"));
    }
    if file.ppqn == 0
        || file.ppqn > 32767
        || file.tracks.is_empty()
        || file.tracks.len() > MAX_TRACKS
        || (file.format == Format::Single && file.tracks.len() != 1)
    {
        return Err(error("Invalid MIDI format, track count or PPQN"));
    }
    let mut output = Vec::new();
    output.extend_from_slice(b"MThd\0\0\0\x06");
    output.extend_from_slice(
        &(if file.format == Format::Single {
            0u16
        } else {
            1
        })
        .to_be_bytes(),
    );
    output.extend_from_slice(&(file.tracks.len() as u16).to_be_bytes());
    output.extend_from_slice(&file.ppqn.to_be_bytes());
    let mut total_events = 0usize;
    let mut total_notes = 0usize;
    for track in &file.tracks {
        check_cancel(&mut cancelled)?;
        if track.end_tick > MAX_TICK {
            return Err(error("MIDI end tick exceeds supported bounds"));
        }
        total_notes = total_notes
            .checked_add(track.notes.len())
            .ok_or_else(|| error("MIDI note count overflow"))?;
        let count = track
            .notes
            .len()
            .checked_mul(2)
            .and_then(|v| v.checked_add(track.messages.len()))
            .and_then(|v| v.checked_add(track.meta.len()))
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| error("MIDI event count overflow"))?;
        total_events = total_events
            .checked_add(count)
            .ok_or_else(|| error("MIDI event count overflow"))?;
        if total_events > MAX_EVENTS || total_notes > MAX_NOTES {
            return Err(error("MIDI event/note count exceeds supported bounds"));
        }
        let mut events = Vec::with_capacity(count - 1);
        let mut orders = BTreeSet::new();
        let mut add = |tick: u64, order: u32, event| -> Result<()> {
            if events.len() % 64 == 0 {
                check_cancel(&mut cancelled)?;
            }
            if tick > track.end_tick || !orders.insert(order) {
                return Err(error("MIDI event order/timing is invalid"));
            }
            events.push((tick, order, event));
            Ok(())
        };
        for note in &track.notes {
            if note.channel > 15
                || note.pitch > 127
                || note.velocity == 0
                || note.velocity > 127
                || note.release_velocity > 127
                || note.end_order <= note.start_order
            {
                return Err(error("Invalid MIDI note channel, value or event order"));
            }
            let end = note
                .start_tick
                .checked_add(note.duration_ticks)
                .ok_or_else(|| error("MIDI note time overflow"))?;
            add(note.start_tick, note.start_order, WriteEvent::On(note))?;
            add(end, note.end_order, WriteEvent::Off(note))?;
        }
        for message in &track.messages {
            let status = message.bytes[0];
            let expected = if matches!(status >> 4, 0xc | 0xd) {
                2
            } else {
                3
            };
            if !(0x80..=0xef).contains(&status)
                || message.length != expected
                || message.bytes[1] > 127
                || message.bytes[2] > 127
                || (expected == 2 && message.bytes[2] != 0)
            {
                return Err(error("Invalid MIDI channel message"));
            }
            if status >> 4 == 9 && message.bytes[2] != 0 {
                return Err(error("A note onset requires paired note timing"));
            }
            add(message.tick, message.order, WriteEvent::Message(message))?;
        }
        for meta in &track.meta {
            add(meta.tick, meta.order, WriteEvent::Meta(&meta.value))?;
        }
        events.sort_by_key(|(tick, order, _)| (*tick, *order));
        // Arbitrary models must retain the same FIFO association the reader
        // uses for overlapping notes of one pitch/channel.
        let mut held = BTreeMap::<(u8, u8), VecDeque<u32>>::new();
        let mut chunk = Vec::new();
        let mut previous = 0;
        for (index, (tick, _, event)) in events.into_iter().enumerate() {
            if index % 64 == 0 {
                check_cancel(&mut cancelled)?;
            }
            total_events += delta(&mut chunk, &mut previous, tick)?;
            if total_events > MAX_EVENTS {
                return Err(error("MIDI spacers exceed the event count bound"));
            }
            match event {
                WriteEvent::On(note) => {
                    held.entry((note.channel, note.pitch))
                        .or_default()
                        .push_back(note.start_order);
                    append(
                        &mut chunk,
                        &[0x90 | note.channel, note.pitch, note.velocity],
                    )?;
                }
                WriteEvent::Off(note) => {
                    if held
                        .get_mut(&(note.channel, note.pitch))
                        .and_then(VecDeque::pop_front)
                        != Some(note.start_order)
                    {
                        return Err(error(
                            "Overlapping MIDI note releases must retain FIFO pairing",
                        ));
                    }
                    append(
                        &mut chunk,
                        &[0x80 | note.channel, note.pitch, note.release_velocity],
                    )?;
                }
                WriteEvent::Message(message) => {
                    if matches!(message.bytes[0] >> 4, 8 | 9)
                        && held
                            .get(&(message.bytes[0] & 15, message.bytes[1]))
                            .is_some_and(|notes| !notes.is_empty())
                    {
                        return Err(error(
                            "Standalone MIDI release conflicts with a paired note",
                        ));
                    }
                    append(&mut chunk, &message.bytes[..message.length as usize])?;
                }
                WriteEvent::Meta(meta) => match meta {
                    MetaValue::Tempo(value) => {
                        if *value == 0 || *value > 0xffffff {
                            return Err(error("Invalid MIDI tempo"));
                        }
                        append(
                            &mut chunk,
                            &[
                                0xff,
                                0x51,
                                3,
                                (*value >> 16) as u8,
                                (*value >> 8) as u8,
                                *value as u8,
                            ],
                        )?;
                    }
                    MetaValue::Meter {
                        numerator,
                        denominator_power,
                        clocks,
                        thirty_seconds,
                    } => {
                        if *numerator == 0 {
                            return Err(error("Invalid MIDI meter numerator"));
                        }
                        append(
                            &mut chunk,
                            &[
                                0xff,
                                0x58,
                                4,
                                *numerator,
                                *denominator_power,
                                *clocks,
                                *thirty_seconds,
                            ],
                        )?;
                    }
                    MetaValue::Key { sharps, minor } => {
                        if !(-7..=7).contains(sharps) {
                            return Err(error("Invalid MIDI key signature"));
                        }
                        append(
                            &mut chunk,
                            &[0xff, 0x59, 2, *sharps as u8, u8::from(*minor)],
                        )?;
                    }
                    MetaValue::Text { kind, bytes } => {
                        if !(1..=9).contains(kind) || bytes.len() > MAX_BYTES {
                            return Err(error("Invalid or oversized MIDI text"));
                        }
                        append(&mut chunk, &[0xff, *kind])?;
                        vlq(&mut chunk, bytes.len() as u32)?;
                        append(&mut chunk, bytes)?;
                    }
                },
            }
        }
        total_events += delta(&mut chunk, &mut previous, track.end_tick)?;
        if total_events > MAX_EVENTS {
            return Err(error("MIDI spacers exceed the event count bound"));
        }
        append(&mut chunk, &[0xff, 0x2f, 0])?;
        append(&mut output, b"MTrk")?;
        append(&mut output, &(chunk.len() as u32).to_be_bytes())?;
        append(&mut output, &chunk)?;
    }
    check_cancel(&mut cancelled)?;
    Ok(output)
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if output
        .len()
        .checked_add(bytes.len())
        .is_none_or(|len| len > MAX_BYTES)
    {
        return Err(error("Encoded MIDI exceeds 16 MiB"));
    }
    output.extend_from_slice(bytes);
    Ok(())
}
fn vlq(output: &mut Vec<u8>, value: u32) -> Result<()> {
    if u64::from(value) > MAX_DELTA {
        return Err(error("MIDI delta exceeds four-byte quantity"));
    }
    let mut bytes = [0u8; 4];
    let mut slot = 3;
    let mut value = value;
    bytes[slot] = (value & 127) as u8;
    value >>= 7;
    while value != 0 {
        slot -= 1;
        bytes[slot] = (value & 127) as u8 | 128;
        value >>= 7;
    }
    append(output, &bytes[slot..])
}
fn delta(output: &mut Vec<u8>, previous: &mut u64, tick: u64) -> Result<usize> {
    let mut gap = tick
        .checked_sub(*previous)
        .ok_or_else(|| error("MIDI events are not in time order"))?;
    let mut spacers = 0;
    while gap > MAX_DELTA {
        spacers += 1;
        vlq(output, MAX_DELTA as u32)?;
        append(output, &[0xff, 1, 0])?; // content-free standard text, not proprietary
        gap -= MAX_DELTA;
    }
    vlq(output, gap as u32)?;
    *previous = tick;
    Ok(spacers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(ppqn: u16, format: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut output = b"MThd\0\0\0\x06".to_vec();
        output.extend_from_slice(&format.to_be_bytes());
        output.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        output.extend_from_slice(&ppqn.to_be_bytes());
        for track in tracks {
            output.extend_from_slice(b"MTrk");
            output.extend_from_slice(&(track.len() as u32).to_be_bytes());
            output.extend_from_slice(track);
        }
        output
    }
    #[test]
    fn hand_encoded_running_status_overlaps_channels_velocities_and_controller_lanes() {
        let data = raw(
            960,
            0,
            &[&[
                0, 0xc2, 17, 0, 0xb2, 7, 100, 0, 0x92, 60, 90, 0x83, 0x60, 60,
                70, // another same pitch at 480, running status
                0, 0xff, 1, 1, b'x', // meta preserves channel running status
                0, 64, 80, // same status, independent pitch
                0, 0x93, 60, 60, // independent channel
                0x83, 0x60, 0x82, 60, 11, // FIFO off at 960
                0, 60, 12, 0, 64, 13, 0, 0x83, 60, 14, 0, 0xff, 0x2f, 0,
            ]],
        );
        let file = decode(&data).unwrap();
        let notes = &file.tracks[0].notes;
        assert_eq!(file.ppqn, 960);
        assert_eq!(notes.len(), 4);
        assert!(file.warnings.is_empty());
        assert_eq!(
            (
                notes[0].start_tick,
                notes[0].duration_ticks,
                notes[0].velocity,
                notes[0].release_velocity
            ),
            (0, 960, 90, 11)
        );
        assert_eq!(
            (
                notes[1].start_tick,
                notes[1].duration_ticks,
                notes[1].velocity,
                notes[1].release_velocity
            ),
            (480, 480, 70, 12)
        );
        assert_eq!(
            (
                notes[3].channel,
                notes[3].duration_ticks,
                notes[3].release_velocity
            ),
            (3, 480, 14)
        );
        assert_eq!(file.tracks[0].messages[0].bytes, [0xc2, 17, 0]);
        assert_eq!(file.tracks[0].messages[1].bytes, [0xb2, 7, 100]);
        let again = decode(&encode(&file, false).unwrap()).unwrap();
        // Canonical output uses explicit status; musical fields stay exact.
        assert_eq!(again, file);
    }
    #[test]
    fn parallel_conductor_preserves_tempo_meter_text_key_and_end_silence() {
        let conductor = &[
            0, 0xff, 0x51, 3, 7, 0xa1, 0x20, 0, 0xff, 0x58, 4, 7, 3, 24, 8, 0, 0xff, 3, 3, b'A',
            0xff, b'B', 0, 0xff, 0x59, 2, 0xfe, 1, 0x83, 0x60, 0xff, 0x51, 3, 6, 0x1a, 0x80, 0,
            0xff, 0x58, 4, 3, 2, 24, 8, 0x83, 0x60, 0xff, 0x2f, 0,
        ];
        let notes = &[
            0, 0x99, 36, 100, 0x81, 0x70, 0x89, 36, 64, 0x85, 0x50, 0xff, 0x2f, 0,
        ];
        let file = decode(&raw(480, 1, &[conductor, notes])).unwrap();
        assert_eq!(file.tracks[0].end_tick, 960);
        assert_eq!(file.tracks[1].end_tick, 960);
        assert_eq!(file.tracks[0].meta[0].value, MetaValue::Tempo(500_000));
        assert_eq!(file.tracks[0].meta[4].value, MetaValue::Tempo(400_000));
        assert_eq!(file.tracks[0].meta[5].tick, 480);
        assert_eq!(file.tracks[1].notes[0].duration_ticks, 240);
        assert_eq!(decode(&encode(&file, false).unwrap()).unwrap(), file);
    }
    #[test]
    fn zero_duration_and_late_high_ppqn_notes_do_not_round_through_float() {
        let track = Track {
            end_tick: MAX_TICK,
            notes: vec![
                Note {
                    channel: 15,
                    pitch: 127,
                    velocity: 127,
                    release_velocity: 126,
                    start_tick: MAX_TICK - 2,
                    duration_ticks: 1,
                    start_order: 1,
                    end_order: 2,
                },
                Note {
                    channel: 0,
                    pitch: 60,
                    velocity: 1,
                    release_velocity: 0,
                    start_tick: MAX_TICK,
                    duration_ticks: 0,
                    start_order: 3,
                    end_order: 4,
                },
            ],
            ..Track::default()
        };
        let file = File {
            format: Format::Single,
            ppqn: 32767,
            tracks: vec![track],
            warnings: vec![],
        };
        let encoded = encode(&file, false).unwrap();
        assert!(encoded.len() < 512);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(
            decoded.tracks[0]
                .notes
                .iter()
                .map(|n| (
                    n.start_tick,
                    n.duration_ticks,
                    n.channel,
                    n.release_velocity
                ))
                .collect::<Vec<_>>(),
            vec![(MAX_TICK - 2, 1, 15, 126), (MAX_TICK, 0, 0, 0)]
        );
        assert_eq!(decoded.tracks[0].end_tick, MAX_TICK);
        assert!(decoded.warnings.is_empty());
    }
    #[test]
    fn unsupported_sysex_proprietary_meta_and_unpaired_releases_require_deliberate_omission() {
        let file = decode(&raw(
            480,
            0,
            &[&[
                0, 0xf0, 3, 0x7d, 1, 0xf7, 0, 0xff, 0x7f, 2, 9, 8, 0, 0x80, 60, 0, 0, 0xff, 0x2f, 0,
            ]],
        ))
        .unwrap();
        assert_eq!(file.warnings.len(), 3);
        assert!(encode(&file, false).is_err());
        let written = decode(&encode(&file, true).unwrap()).unwrap();
        assert_eq!(
            written.tracks[0]
                .messages
                .iter()
                .map(|m| (m.tick, m.bytes, m.length))
                .collect::<Vec<_>>(),
            file.tracks[0]
                .messages
                .iter()
                .map(|m| (m.tick, m.bytes, m.length))
                .collect::<Vec<_>>()
        );
        assert_eq!(written.warnings.len(), 1); // standalone release is retained
    }
    #[test]
    fn every_truncation_and_invalid_status_or_division_fails_without_partial_file() {
        let valid = raw(
            480,
            0,
            &[&[0, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0, 0xff, 0x2f, 0]],
        );
        for length in 0..valid.len() {
            assert!(decode(&valid[..length]).is_err(), "length {length}");
        }
        for data in [
            vec![0, 60, 100, 0, 0xff, 0x2f, 0],
            vec![0, 0x90, 128, 100, 0, 0xff, 0x2f, 0],
            vec![0x80, 0x80, 0x80, 0x80, 0, 0xff, 0x2f, 0],
            vec![0, 0x90, 60, 100, 0, 0xff, 0x2f, 0],
            vec![0, 0xff, 0x2f, 0, 0],
            vec![0, 0xf0, 0, 0, 60, 100, 0, 0xff, 0x2f, 0],
        ] {
            assert!(decode(&raw(480, 0, &[&data])).is_err());
        }
        assert!(decode(&raw(0, 0, &[&[0, 0xff, 0x2f, 0]])).is_err());
        assert!(decode(&raw(0xe728, 0, &[&[0, 0xff, 0x2f, 0]])).is_err());
        assert!(decode(&raw(480, 2, &[&[0, 0xff, 0x2f, 0]])).is_err());
        assert!(decode(&raw(480, 0, &[&[0, 0xff, 0x2f, 0], &[0, 0xff, 0x2f, 0]])).is_err());
    }
    #[test]
    fn cancellation_returns_no_partial_read_or_write_and_keeps_its_typed_outcome() {
        let mut track = Vec::new();
        for _ in 0..512 {
            track.extend_from_slice(&[0, 0xb0, 1, 64]);
        }
        track.extend_from_slice(&[0, 0xff, 0x2f, 0]);
        let bytes = raw(480, 0, &[&track]);
        let mut polls = 0;
        let result = decode_with_cancel(&bytes, || {
            polls += 1;
            polls >= 4
        })
        .unwrap_err();
        assert!(result.cancelled);
        assert!(polls < 8);
        let file = decode(&bytes).unwrap();
        let mut polls = 0;
        let result = encode_with_cancel(&file, false, || {
            polls += 1;
            polls >= 5
        })
        .unwrap_err();
        assert!(result.cancelled);
        assert!(polls < 8);
        assert!(decode_with_cancel(&[], || true).unwrap_err().cancelled);
        assert!(!decode(&[]).unwrap_err().cancelled);
    }
    #[test]
    fn all_channel_message_widths_and_extended_chunks_keep_their_supported_content() {
        let mut bytes = raw(
            480,
            0,
            &[&[
                0, 0xa1, 60, 90, 0, 0xb1, 0, 127, 0, 0xc1, 12, 0, 0xd1, 13, 0, 0xe1, 0, 64, 0,
                0xff, 0x2f, 0,
            ]],
        );
        bytes.splice(14..14, b"JUNK\0\0\0\x01z".iter().copied());
        let file = decode(&bytes).unwrap();
        assert_eq!(file.warnings[0].kind, Unsupported::Chunk(*b"JUNK", 1));
        assert_eq!(
            file.tracks[0]
                .messages
                .iter()
                .map(|m| (m.bytes, m.length))
                .collect::<Vec<_>>(),
            vec![
                ([0xa1, 60, 90], 3),
                ([0xb1, 0, 127], 3),
                ([0xc1, 12, 0], 2),
                ([0xd1, 13, 0], 2),
                ([0xe1, 0, 64], 3)
            ]
        );
        assert_eq!(
            decode(&encode(&file, true).unwrap()).unwrap().tracks,
            file.tracks
        );
    }
    #[test]
    fn stored_original_fixture_retains_all_integer_note_and_conductor_coordinates() {
        let file = decode(include_bytes!(
            "../tests/fixtures/midi/sixteen-bars-ppqn960.mid"
        ))
        .unwrap();
        assert_eq!(file.ppqn, 960);
        assert_eq!(file.format, Format::Parallel);
        assert!(file.warnings.is_empty());
        assert_eq!(file.tracks.len(), 2);
        for track in &file.tracks {
            assert_eq!(track.end_tick, 61440);
        }
        assert_eq!(file.tracks[1].notes.len(), 64);
        for (i, note) in file.tracks[1].notes.iter().enumerate() {
            assert_eq!(
                (
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.channel,
                    note.velocity,
                    note.release_velocity
                ),
                (
                    960 * i as u64 + 1,
                    719,
                    60 + (i % 8) as u8,
                    if i % 2 == 0 { 0 } else { 2 },
                    80 + (i % 32) as u8,
                    (i % 64) as u8
                )
            );
        }
        assert_eq!(
            file.tracks[0]
                .meta
                .iter()
                .map(|m| (m.tick, &m.value))
                .collect::<Vec<_>>(),
            vec![
                (0, &MetaValue::Tempo(500000)),
                (
                    0,
                    &MetaValue::Meter {
                        numerator: 4,
                        denominator_power: 2,
                        clocks: 24,
                        thirty_seconds: 8
                    }
                ),
                (30720, &MetaValue::Tempo(666667)),
                (
                    30720,
                    &MetaValue::Meter {
                        numerator: 7,
                        denominator_power: 3,
                        clocks: 24,
                        thirty_seconds: 8
                    }
                )
            ]
        );
        assert_eq!(file.tracks[1].messages.len(), 11);
        assert_eq!(decode(&encode(&file, false).unwrap()).unwrap(), file);
    }
    #[test]
    fn invalid_model_timing_note_pairing_and_values_cannot_escape_through_writer() {
        let mut file = decode(&raw(
            480,
            0,
            &[&[
                0, 0x90, 60, 100, 1, 60, 90, 1, 0x80, 60, 0, 1, 60, 0, 0, 0xff, 0x2f, 0,
            ]],
        ))
        .unwrap();
        let original = file.clone();
        file.tracks[0].notes[0].duration_ticks = 4;
        assert!(encode(&file, false).is_err());
        file = original.clone();
        file.tracks[0].notes[0].end_order = file.tracks[0].notes[1].end_order;
        assert!(encode(&file, false).is_err());
        file = original.clone();
        file.tracks[0].notes[0].duration_ticks = 3;
        file.tracks[0].notes[1].duration_ticks = 1;
        // Independent MIDI off order would pair this model's overlapping notes
        // differently; do not silently export a different pair of durations.
        file.tracks[0].notes[0].end_order = 4;
        file.tracks[0].notes[1].end_order = 3;
        assert!(encode(&file, false).is_err());
        file = original;
        file.tracks[0].notes[0].velocity = 128;
        assert!(encode(&file, false).is_err());
    }
}
