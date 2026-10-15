use super::*;
use clap_sys::fixedpoint::*;

pub(super) enum Event {
    Param(clap_event_param_value),
    Note(clap_event_note),
    Midi(clap_event_midi),
    Expression(clap_event_note_expression),
    Modulation(clap_event_param_mod),
}
impl Event {
    pub(super) fn header(&self) -> &clap_event_header {
        match self {
            Self::Param(e) => &e.header,
            Self::Note(e) => &e.header,
            Self::Midi(e) => &e.header,
            Self::Expression(e) => &e.header,
            Self::Modulation(e) => &e.header,
        }
    }
}
fn header<T>(time: u32, kind: u16) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags: 0,
    }
}
/// Build time-ordered CLAP input. Args: validated block and native metadata. Returns exact-offset parameters and channel-preserving MIDI/CLAP notes, or a dialect diagnostic.
pub(super) fn input(frame: &Frame, class: &Class) -> Result<Vec<Event>, String> {
    let metadata = class.native.as_ref().ok_or("CLAP metadata is absent")?;
    let mut result = Vec::with_capacity(frame.parameters.len() + frame.midi.len());
    for &(id, value, time) in &frame.parameters {
        let parameter = metadata
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or("Unknown CLAP parameter")?;
        if parameter.flags & CLAP_PARAM_IS_READONLY != 0 {
            return Err("CLAP parameter is read-only".into());
        }
        let mut value = parameter.min + value * (parameter.max - parameter.min);
        if parameter.flags & CLAP_PARAM_IS_STEPPED != 0 {
            value = value.round().clamp(parameter.min, parameter.max);
        }
        result.push(Event::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(time as u32, CLAP_EVENT_PARAM_VALUE),
            param_id: id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        }));
    }
    for &(time, data) in &frame.midi {
        let port = metadata
            .note_inputs
            .first()
            .ok_or("CLAP processor has no note input")?;
        if port.dialects & (CLAP_NOTE_DIALECT_MIDI | CLAP_NOTE_DIALECT_MIDI_MPE) != 0 {
            result.push(Event::Midi(clap_event_midi {
                header: header::<clap_event_midi>(time as u32, CLAP_EVENT_MIDI),
                port_index: 0,
                data,
            }));
        } else if port.dialects & CLAP_NOTE_DIALECT_CLAP != 0
            && matches!(data[0] & 0xf0, 0x80 | 0x90)
        {
            let kind = if data[0] & 0xf0 == 0x80 || data[2] == 0 {
                CLAP_EVENT_NOTE_OFF
            } else {
                CLAP_EVENT_NOTE_ON
            };
            result.push(Event::Note(clap_event_note {
                header: header::<clap_event_note>(time as u32, kind),
                note_id: -1,
                port_index: 0,
                channel: (data[0] & 15) as i16,
                key: data[1] as i16,
                velocity: data[2] as f64 / 127.,
            }));
        } else if port.dialects & CLAP_NOTE_DIALECT_CLAP != 0 {
            let (kind,key,value)=match data[0]&0xf0 {
                0xa0 => (CLAP_NOTE_EXPRESSION_PRESSURE,data[1] as i16,data[2] as f64/127.),
                0xd0 => (CLAP_NOTE_EXPRESSION_PRESSURE,-1,data[1] as f64/127.),
                0xe0 => (CLAP_NOTE_EXPRESSION_TUNING,-1,((data[1] as u16|((data[2] as u16)<<7)) as f64-8192.)/8192.*2.),
                0xb0 => match data[1] {7=>(CLAP_NOTE_EXPRESSION_VOLUME,-1,data[2] as f64/127.),10=>(CLAP_NOTE_EXPRESSION_PAN,-1,data[2] as f64/127.),11=>(CLAP_NOTE_EXPRESSION_EXPRESSION,-1,data[2] as f64/127.),74=>(CLAP_NOTE_EXPRESSION_BRIGHTNESS,-1,data[2] as f64/127.),_=>return Err("This CLAP-only note port cannot receive the MIDI controller; select a MIDI-capable dialect for sustain, RPN or other raw controllers".into())},
                _ => return Err("CLAP-only note port cannot receive this MIDI message".into()),
            };
            result.push(Event::Expression(clap_event_note_expression {
                header: header::<clap_event_note_expression>(
                    time as u32,
                    CLAP_EVENT_NOTE_EXPRESSION,
                ),
                expression_id: kind,
                note_id: -1,
                port_index: 0,
                channel: (data[0] & 15) as i16,
                key,
                value,
            }));
        } else {
            return Err("CLAP note port has no supported MIDI or native note dialect".into());
        }
    }
    for &(time, event) in &frame.native {
        use expression::NativeEvent::*;
        let target = match event {
            Note { target, .. } | Expression { target, .. } | Modulation { target, .. } => target,
        };
        if !event.valid() {
            return Err("Invalid CLAP native expression event".into());
        }
        match event {
            Note { on, velocity, .. } => {
                require_note_port(metadata, target.port)?;
                result.push(Event::Note(clap_event_note {
                    header: header::<clap_event_note>(
                        time as u32,
                        if on {
                            CLAP_EVENT_NOTE_ON
                        } else {
                            CLAP_EVENT_NOTE_OFF
                        },
                    ),
                    note_id: target.note_id,
                    port_index: target.port,
                    channel: target.channel,
                    key: target.key,
                    velocity,
                }));
            }
            Expression { kind, value, .. } => {
                if target.port >= 0 {
                    require_note_port(metadata, target.port)?;
                } else if !metadata
                    .note_inputs
                    .iter()
                    .any(|p| p.dialects & CLAP_NOTE_DIALECT_CLAP != 0)
                {
                    return Err("CLAP native note-expression dialect is unavailable".into());
                }
                result.push(Event::Expression(clap_event_note_expression {
                    header: header::<clap_event_note_expression>(
                        time as u32,
                        CLAP_EVENT_NOTE_EXPRESSION,
                    ),
                    expression_id: kind,
                    note_id: target.note_id,
                    port_index: target.port,
                    channel: target.channel,
                    key: target.key,
                    value,
                }));
            }
            Modulation {
                parameter, amount, ..
            } => {
                let p = metadata
                    .parameters
                    .iter()
                    .find(|p| p.id == parameter)
                    .ok_or("Unknown CLAP modulation parameter")?;
                if p.flags & CLAP_PARAM_IS_MODULATABLE == 0
                    || target.note_id >= 0 && p.flags & CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID == 0
                    || target.port >= 0 && p.flags & CLAP_PARAM_IS_MODULATABLE_PER_PORT == 0
                    || target.channel >= 0 && p.flags & CLAP_PARAM_IS_MODULATABLE_PER_CHANNEL == 0
                    || target.key >= 0 && p.flags & CLAP_PARAM_IS_MODULATABLE_PER_KEY == 0
                {
                    return Err("CLAP parameter does not support this modulation address".into());
                }
                result.push(Event::Modulation(clap_event_param_mod {
                    header: header::<clap_event_param_mod>(time as u32, CLAP_EVENT_PARAM_MOD),
                    param_id: parameter,
                    cookie: ptr::null_mut(),
                    note_id: target.note_id,
                    port_index: target.port,
                    channel: target.channel,
                    key: target.key,
                    amount,
                }));
            }
        }
    }
    result.sort_by_key(|e| e.header().time);
    Ok(result)
}
pub(super) unsafe extern "C" fn size(list: *const clap_input_events) -> u32 {
    let v = &*((*list).ctx as *const Vec<Event>);
    v.len() as u32
}
pub(super) unsafe extern "C" fn get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let v = &*((*list).ctx as *const Vec<Event>);
    v.get(index as usize).map_or(ptr::null(), |e| e.header())
}
pub(super) struct Output {
    pub midi: Vec<vst3_host::MidiEvent>,
    pub invalid: bool,
    frames: usize,
    count: usize,
}
impl Output {
    pub fn new(frames: usize) -> Self {
        Self {
            midi: Vec::with_capacity(512),
            invalid: false,
            frames,
            count: 0,
        }
    }
}
pub(super) unsafe extern "C" fn push(
    list: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    let out = &mut *((*list).ctx as *mut Output);
    let Some(h) = event.as_ref() else {
        out.invalid = true;
        return false;
    };
    if out.count >= 512 {
        return false;
    }
    out.count += 1;
    if h.time as usize >= out.frames || h.size < (std::mem::size_of::<clap_event_header>() as u32) {
        out.invalid = true;
        return false;
    }
    if h.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return false;
    }
    if h.type_ == CLAP_EVENT_MIDI && h.size as usize >= std::mem::size_of::<clap_event_midi>() {
        let e = &*event.cast::<clap_event_midi>();
        if e.port_index != 0 {
            return false;
        }
        if let Some(m) = midi(e.data) {
            out.midi.push(m);
        } else {
            return false;
        }
    } else if matches!(h.type_, CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF)
        && h.size as usize >= std::mem::size_of::<clap_event_note>()
    {
        let e = &*event.cast::<clap_event_note>();
        if e.port_index != 0
            || !(0..16).contains(&e.channel)
            || !(0..128).contains(&e.key)
            || !e.velocity.is_finite()
            || !(0.0..=1.).contains(&e.velocity)
        {
            out.invalid = true;
            return false;
        }
        if let Some(m) = midi([
            if h.type_ == CLAP_EVENT_NOTE_ON {
                0x90
            } else {
                0x80
            } | e.channel as u8,
            e.key as u8,
            (e.velocity * 127.).round() as u8,
        ]) {
            out.midi.push(m);
        }
    } else if h.type_ == CLAP_EVENT_PARAM_VALUE
        && h.size as usize >= std::mem::size_of::<clap_event_param_value>()
    {
        if !(*event.cast::<clap_event_param_value>()).value.is_finite() {
            out.invalid = true;
            return false;
        }
    } else if !matches!(
        h.type_,
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END | CLAP_EVENT_NOTE_END
    ) {
        return false;
    }
    true
}
pub(in crate::plugin_host) fn midi(b: [u8; 3]) -> Option<vst3_host::MidiEvent> {
    use vst3_host::MidiEvent::*;
    let channel = vst3_host::MidiChannel::from_index(b[0] & 15)?;
    if b[1] > 127 || b[2] > 127 {
        return None;
    }
    Some(match b[0] & 0xf0 {
        0x80 => NoteOff {
            channel,
            note: b[1],
            velocity: b[2],
        },
        0x90 => NoteOn {
            channel,
            note: b[1],
            velocity: b[2],
        },
        0xa0 => PolyAftertouch {
            channel,
            note: b[1],
            pressure: b[2],
        },
        0xb0 => ControlChange {
            channel,
            controller: b[1],
            value: b[2],
        },
        0xc0 => ProgramChange {
            channel,
            program: b[1],
        },
        0xd0 => ChannelAftertouch {
            channel,
            pressure: b[1],
        },
        0xe0 => PitchBend {
            channel,
            value: b[1] as u16 | ((b[2] as u16) << 7),
        },
        _ => return None,
    })
}
/// Build CLAP transport. Args: validated frame and sample rate. Returns tempo, timeline and meter at the first sample.
pub(super) fn transport(f: &Frame, rate: u32) -> clap_event_transport {
    clap_event_transport {
        header: header::<clap_event_transport>(0, CLAP_EVENT_TRANSPORT),
        flags: CLAP_TRANSPORT_HAS_TEMPO
            | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
            | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
            | CLAP_TRANSPORT_HAS_TIME_SIGNATURE
            | if f.playing {
                CLAP_TRANSPORT_IS_PLAYING
            } else {
                0
            },
        song_pos_beats: (f.beat * CLAP_BEATTIME_FACTOR as f64).round() as i64,
        song_pos_seconds: ((f.sample_position as f64 / rate as f64) * CLAP_SECTIME_FACTOR as f64)
            .round() as i64,
        tempo: f.bpm,
        tempo_inc: 0.,
        loop_start_beats: 0,
        loop_end_beats: 0,
        loop_start_seconds: 0,
        loop_end_seconds: 0,
        bar_start: 0,
        bar_number: 0,
        tsig_num: f.signature[0] as u16,
        tsig_denom: f.signature[1] as u16,
    }
}

fn require_note_port(metadata: &Metadata, port: i16) -> Result<(), String> {
    if port < 0
        || !metadata
            .note_inputs
            .get(port as usize)
            .is_some_and(|p| p.dialects & CLAP_NOTE_DIALECT_CLAP != 0)
    {
        return Err("CLAP native note dialect is unavailable on this port".into());
    }
    Ok(())
}
