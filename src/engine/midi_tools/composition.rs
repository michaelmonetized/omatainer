use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Voicing {
    Closed,
    Open,
    DropTwo,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Articulation {
    Arpeggio,
    Strum,
    Grace,
    Flam,
    Glissando,
    Repeat,
    Legato,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    pub degree: i16,
    pub inversion: u8,
    pub alteration: i8,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub start: f64,
    pub length: f64,
    pub register: [u8; 2],
    pub channel: u8,
    pub replace: bool,
    pub chords: Vec<Chord>,
    pub voices: u8,
    pub voicing: Voicing,
    pub spread: u8,
    pub voice_leading: bool,
    pub density: f64,
    pub pitch_variation: f64,
    pub gate: [f64; 2],
    pub articulation: Articulation,
    pub descending: bool,
    pub interval: i8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            start: 0.0,
            length: 8.0,
            register: [48, 84],
            channel: 0,
            replace: false,
            chords: [0, 5, 3, 4, 0, 3, 4, 0]
                .into_iter()
                .map(|degree| Chord {
                    degree,
                    inversion: 0,
                    alteration: 0,
                })
                .collect(),
            voices: 3,
            voicing: Voicing::Closed,
            spread: 0,
            voice_leading: true,
            density: 1.0,
            pitch_variation: 0.0,
            gate: [0.7, 0.9],
            articulation: Articulation::Strum,
            descending: false,
            interval: -1,
        }
    }
}
impl Settings {
    pub(super) fn valid(&self) -> bool {
        self.start.is_finite()
            && (0.0..=LIMIT).contains(&self.start)
            && self.length.is_finite()
            && (1.0 / 1024.0..=LIMIT).contains(&self.length)
            && self.start + self.length <= LIMIT
            && self.register[0] <= self.register[1]
            && self.register[1] < 128
            && self.channel < 16
            && !self.chords.is_empty()
            && self.chords.len() <= 64
            && self.chords.iter().all(|c| {
                (-128..=128).contains(&c.degree)
                    && c.inversion < self.voices
                    && (-11..=11).contains(&c.alteration)
            })
            && (3..=4).contains(&self.voices)
            && self.spread <= 3
            && self.density.is_finite()
            && (0.0..=1.0).contains(&self.density)
            && self.pitch_variation.is_finite()
            && (0.0..=1.0).contains(&self.pitch_variation)
            && self
                .gate
                .iter()
                .all(|v| v.is_finite() && (0.01..=1.0).contains(v))
            && self.gate[0] <= self.gate[1]
            && (-24..=24).contains(&self.interval)
    }
}
#[derive(Clone, Copy)]
struct Generated {
    source: Option<usize>,
    channel: u8,
    pitch: u8,
    start: f64,
    end: f64,
    velocity: u8,
}
fn unit(state: &mut u64) -> f64 {
    (random(state) >> 11) as f64 / (1u64 << 53) as f64
}
fn pitches(params: &Parameters) -> Result<Vec<u8>, String> {
    let settings = &params.composition;
    let allowed: Vec<_> = (settings.register[0]..=settings.register[1])
        .filter(|pitch| {
            params.include_chromatic
                || params
                    .context
                    .is_some_and(|context| context.contains(*pitch))
        })
        .collect();
    if allowed.is_empty() {
        return Err("Choose a saved scale with notes inside this register, or explicitly allow chromatic pitches".into());
    }
    Ok(allowed)
}
fn nearest(allowed: &[u8], pitch: f64) -> u8 {
    *allowed
        .iter()
        .min_by(|a, b| {
            (f64::from(**a) - pitch)
                .abs()
                .total_cmp(&(f64::from(**b) - pitch).abs())
                .then(a.cmp(b))
        })
        .unwrap()
}
fn push(result: &mut Vec<Generated>, value: Generated) -> Result<(), String> {
    if result.len() >= super::super::project::MAX_NOTES_PER_CLIP {
        return Err("The generated phrase exceeds 8192 notes; reduce its length or density".into());
    }
    if !value.start.is_finite()
        || !value.end.is_finite()
        || value.start < 0.0
        || value.end > LIMIT
        || value.start >= value.end
        || value.pitch > 127
        || value.velocity == 0
        || value.velocity > 127
    {
        return Err(
            "The generated phrase leaves the supported pitch or positive-duration range".into(),
        );
    }
    result.push(value);
    Ok(())
}
fn closest_voicing(
    options: &[Vec<u8>],
    previous: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    fn visit(
        options: &[Vec<u8>],
        previous: &[u8],
        current: &mut Vec<u8>,
        best: &mut Option<(u32, Vec<u8>)>,
        work: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        *work += 1;
        if *work % 64 == 0 {
            cancelled(cancel)?;
        }
        if *work > 65_536 {
            return Err("Voice-leading search exceeds its bound; narrow the register".into());
        }
        if current.len() == options.len() {
            let cost = current
                .iter()
                .zip(previous)
                .map(|(a, b)| u32::from(a.abs_diff(*b)))
                .sum();
            if best.as_ref().is_none_or(|(old, notes)| {
                cost < *old || cost == *old && current.as_slice() < notes.as_slice()
            }) {
                *best = Some((cost, current.clone()));
            }
            return Ok(());
        }
        for &pitch in &options[current.len()] {
            if current.last().is_some_and(|last| pitch <= *last) {
                continue;
            }
            current.push(pitch);
            visit(options, previous, current, best, work, cancel)?;
            current.pop();
        }
        Ok(())
    }
    let mut best = None;
    visit(
        options,
        previous,
        &mut Vec::new(),
        &mut best,
        &mut 0,
        cancel,
    )?;
    best.map(|(_, notes)| notes)
        .ok_or_else(|| "This chord cannot keep ordered voices within the chosen register".into())
}
fn chords(params: &Parameters, cancel: &AtomicBool) -> Result<Vec<Generated>, String> {
    let settings = &params.composition;
    let context = params
        .context
        .ok_or("Choose a saved key and scale for this progression")?;
    let root = (settings.register[0]..=settings.register[1])
        .find(|pitch| pitch % 12 == context.tonic)
        .ok_or("The register does not contain this tonic")?;
    let root = if settings.voicing == Voicing::DropTwo {
        root.checked_add(12)
            .filter(|pitch| *pitch <= settings.register[1])
            .ok_or("Drop-two voicing needs at least another octave in the register")?
    } else {
        root
    };
    let step = settings.length / settings.chords.len() as f64;
    let mut result = Vec::new();
    let mut previous = Vec::new();
    for (index, chord) in settings.chords.iter().enumerate() {
        cancelled(cancel)?;
        if chord.alteration != 0 && !params.include_chromatic {
            return Err("Borrowed chord alterations require explicit chromatic permission".into());
        }
        let mut notes = (0..settings.voices)
            .map(|voice| {
                context
                    .transpose(root, chord.degree + i16::from(voice) * 2, false)
                    .map(i16::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for pitch in notes.iter_mut().take(chord.inversion as usize) {
            *pitch += 12;
        }
        notes.sort_unstable();
        match settings.voicing {
            Voicing::Closed => {}
            Voicing::Open => {
                for (voice, pitch) in notes.iter_mut().enumerate() {
                    if voice % 2 == 1 {
                        *pitch += 12;
                    }
                }
            }
            Voicing::DropTwo => {
                let second = notes.len() - 2;
                notes[second] -= 12;
            }
        }
        for (voice, pitch) in notes.iter_mut().enumerate() {
            *pitch += i16::from(chord.alteration) + voice as i16 * i16::from(settings.spread) * 12;
        }
        notes.sort_unstable();
        let mut bounded = notes.iter().map(|pitch|u8::try_from(*pitch).ok().filter(|pitch| (settings.register[0]..=settings.register[1]).contains(pitch)).ok_or_else(||"Chord inversion, voicing or spread leaves the register; widen it or reduce spread".to_string())).collect::<Result<Vec<_>,_>>()?;
        if settings.voice_leading && !previous.is_empty() {
            let options: Vec<Vec<u8>> = bounded
                .iter()
                .map(|pitch| {
                    (settings.register[0]..=settings.register[1])
                        .filter(|candidate| candidate % 12 == pitch % 12)
                        .collect()
                })
                .collect();
            bounded = closest_voicing(&options, &previous, cancel)?;
        }
        let start = settings.start + index as f64 * step;
        for &pitch in &bounded {
            push(
                &mut result,
                Generated {
                    source: None,
                    channel: settings.channel,
                    pitch,
                    start,
                    end: start + step * settings.gate[1],
                    velocity: params.velocity[1],
                },
            )?;
        }
        previous = bounded;
    }
    Ok(result)
}
fn melody(params: &Parameters, cancel: &AtomicBool) -> Result<Vec<Generated>, String> {
    let settings = &params.composition;
    let allowed = pitches(params)?;
    let steps = (settings.length / params.grid).ceil() as usize;
    if steps > 65_536 {
        return Err("The melody exceeds 65536 steps; use a longer grid or shorter range".into());
    }
    let mut result = Vec::new();
    let mut state = params.seed;
    for step in 0..steps {
        if step % 64 == 0 {
            cancelled(cancel)?;
        }
        let density = unit(&mut state);
        let jitter = unit(&mut state);
        let gate = unit(&mut state);
        let dynamic = unit(&mut state);
        if density >= settings.density {
            continue;
        }
        let x = step as f64 / steps.saturating_sub(1).max(1) as f64 * (CURVE_POINTS - 1) as f64;
        let first = x.floor() as usize;
        let second = (first + 1).min(CURVE_POINTS - 1);
        let level =
            params.curve[first] + (params.curve[second] - params.curve[first]) * (x - first as f64);
        let span = f64::from(settings.register[1] - settings.register[0]);
        let pitch = nearest(
            &allowed,
            f64::from(settings.register[0])
                + (level + (jitter * 2.0 - 1.0) * settings.pitch_variation).clamp(0.0, 1.0) * span,
        );
        let start = settings.start + step as f64 * params.grid;
        let length = (settings.start + settings.length - start).min(params.grid)
            * (settings.gate[0] + gate * (settings.gate[1] - settings.gate[0]));
        let velocity = (f64::from(params.velocity[0])
            + dynamic * f64::from(params.velocity[1] - params.velocity[0]))
        .round() as u8;
        push(
            &mut result,
            Generated {
                source: None,
                channel: settings.channel,
                pitch,
                start,
                end: start + length,
                velocity,
            },
        )?;
    }
    Ok(result)
}
fn articulate(
    original: &Content,
    selected: &BTreeSet<NoteId>,
    params: &Parameters,
    cancel: &AtomicBool,
) -> Result<Vec<Generated>, String> {
    let settings = &params.composition;
    let mut indices: Vec<_> = original
        .notes
        .iter()
        .enumerate()
        .filter(|(_, note)| selected.contains(&note.id))
        .map(|(index, _)| index)
        .collect();
    indices.sort_unstable_by(|a, b| {
        original.notes[*a]
            .source_start()
            .total_cmp(&original.notes[*b].source_start())
            .then(original.notes[*a].pitch.cmp(&original.notes[*b].pitch))
            .then(original.notes[*a].id.cmp(&original.notes[*b].id))
    });
    if indices
        .iter()
        .any(|index| original.notes[*index].variation.is_some())
    {
        return Err("Clear the selected note choices before changing their articulation; this preview does not discard probability groups".into());
    }
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < indices.len() {
        cancelled(cancel)?;
        let onset = original.notes[indices[cursor]].source_start();
        let end = indices[cursor..]
            .iter()
            .position(|index| original.notes[*index].source_start() != onset)
            .map_or(indices.len(), |offset| cursor + offset);
        let mut group = indices[cursor..end].to_vec();
        if settings.descending {
            group.reverse();
        }
        if settings.articulation == Articulation::Arpeggio {
            let finish = group
                .iter()
                .map(|index| {
                    original.notes[*index].source_start() + original.notes[*index].source_duration()
                })
                .fold(onset, f64::max);
            let steps = ((finish - onset) / params.grid).ceil() as usize;
            if steps > 65_536 {
                return Err("Arpeggiation exceeds 65536 steps; lengthen its grid".into());
            }
            for step in 0..steps {
                if step % 64 == 0 {
                    cancelled(cancel)?;
                }
                let index = group[step % group.len()];
                let old = &original.notes[index];
                let start = onset + step as f64 * params.grid;
                let length = (finish - start).min(params.grid) * settings.gate[1];
                push(
                    &mut result,
                    Generated {
                        source: Some(index),
                        channel: old.channel,
                        pitch: old.pitch,
                        start,
                        end: start + length,
                        velocity: old.vel,
                    },
                )?;
            }
        } else {
            for (rank, &index) in group.iter().enumerate() {
                let old = &original.notes[index];
                let start = old.source_start();
                let finish = start + old.source_duration();
                let value = |pitch, start, end, velocity| Generated {
                    source: Some(index),
                    channel: old.channel,
                    pitch,
                    start,
                    end,
                    velocity,
                };
                match settings.articulation {
                    Articulation::Strum => {
                        let next = start + rank as f64 * params.grid;
                        push(
                            &mut result,
                            value(
                                old.pitch,
                                next,
                                next + (finish - next) * settings.gate[1],
                                old.vel,
                            ),
                        )?;
                    }
                    Articulation::Grace => {
                        let pitch = i16::from(old.pitch) + i16::from(settings.interval);
                        let pitch = u8::try_from(pitch)
                            .ok()
                            .filter(|p| *p < 128)
                            .ok_or("Grace interval leaves MIDI pitches 0–127")?;
                        push(
                            &mut result,
                            value(
                                pitch,
                                start - params.grid,
                                start - params.grid * (1.0 - settings.gate[1]),
                                params.velocity[0],
                            ),
                        )?;
                        push(&mut result, value(old.pitch, start, finish, old.vel))?;
                    }
                    Articulation::Flam => {
                        if start + params.grid >= finish {
                            return Err(
                                "A flam needs room for both hits before the source release".into(),
                            );
                        }
                        push(
                            &mut result,
                            value(
                                old.pitch,
                                start,
                                start + params.grid * settings.gate[1],
                                params.velocity[0],
                            ),
                        )?;
                        push(
                            &mut result,
                            value(old.pitch, start + params.grid, finish, old.vel),
                        )?;
                    }
                    Articulation::Glissando | Articulation::Repeat => {
                        let steps = (old.source_duration() / params.grid).ceil() as usize;
                        if steps > 65_536 {
                            return Err(
                                "Articulated repetitions exceed 65536 steps; lengthen the grid"
                                    .into(),
                            );
                        }
                        for step in 0..steps {
                            if step % 64 == 0 {
                                cancelled(cancel)?;
                            }
                            let next = start + step as f64 * params.grid;
                            let pitch = if settings.articulation == Articulation::Glissando {
                                i16::from(old.pitch)
                                    + (f64::from(settings.interval) * step as f64
                                        / steps.saturating_sub(1).max(1) as f64)
                                        .round() as i16
                            } else {
                                i16::from(old.pitch)
                            };
                            let pitch = u8::try_from(pitch)
                                .ok()
                                .filter(|p| *p < 128)
                                .ok_or("Glissando leaves MIDI pitches 0–127")?;
                            push(
                                &mut result,
                                value(
                                    pitch,
                                    next,
                                    next + (finish - next).min(params.grid) * settings.gate[1],
                                    old.vel,
                                ),
                            )?;
                        }
                    }
                    Articulation::Legato => {
                        let next = indices
                            .iter()
                            .filter_map(|other| {
                                let note = &original.notes[*other];
                                (note.channel == old.channel && note.source_start() > start)
                                    .then_some(note.source_start())
                            })
                            .min_by(f64::total_cmp)
                            .unwrap_or(finish);
                        push(
                            &mut result,
                            value(
                                old.pitch,
                                start,
                                start + (next - start) * settings.gate[1],
                                old.vel,
                            ),
                        )?;
                    }
                    Articulation::Arpeggio => unreachable!(),
                }
            }
        }
        cursor = end;
    }
    Ok(result)
}
fn positive_timing(mut value: Timing, start: f64) -> Timing {
    if value.notes == 0 {
        value.first = start;
        value.end = start;
    }
    value
}

/// Prepare ordinary editable musical notes.
/// Takes validated source content, captured note selection, chord/melody/articulation settings and cancellation; returns one reversible candidate with retained automation or refuses all changes.
pub(super) fn prepare(
    original: &Content,
    selected: &BTreeSet<NoteId>,
    params: &Parameters,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let generated = match params.kind {
        Kind::Chords => chords(params, cancel)?,
        Kind::Melody => melody(params, cancel)?,
        Kind::Articulate => articulate(original, selected, params, cancel)?,
        _ => return Err("Choose a composition tool".into()),
    };
    let replaced: Vec<_> = original
        .notes
        .iter()
        .map(|note| params.composition.replace && selected.contains(&note.id))
        .collect();
    let original_owners = owners(original, &replaced, params.expression, cancel)?;
    let mut owned_messages = vec![Vec::new(); original.notes.len()];
    for (index, owner) in original_owners.iter().enumerate() {
        if let Some(owner) = owner {
            owned_messages[*owner].push(index);
        }
    }
    let mut content = original.clone();
    content
        .notes
        .retain(|note| !params.composition.replace || !selected.contains(&note.id));
    content.messages = original
        .messages
        .iter()
        .enumerate()
        .filter(|(index, _)| !original_owners[*index].is_some_and(|owner| replaced[owner]))
        .map(|(_, message)| *message)
        .collect();
    if content.notes.len() + generated.len() > super::super::project::MAX_NOTES_PER_CLIP {
        return Err("The complete generated draft exceeds 8192 notes".into());
    }
    let retained = content.notes.len();
    let mut next_order = event_orders(original)?
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .chain(original.messages.iter().map(|message| message.order))
        .chain(original.meta.iter().map(|event| event.order))
        .max()
        .unwrap_or(0);
    let mut order = || {
        next_order = next_order
            .checked_add(1)
            .ok_or("Generated MIDI event order is exhausted")?;
        Ok::<u32, String>(next_order)
    };
    let mut rounding = 0.0f64;
    let mut expression_events = 0;
    for (index, value) in generated.into_iter().enumerate() {
        if index % 64 == 0 {
            cancelled(cancel)?;
        }
        let start = (value.start * f64::from(content.ppqn)).round() as u64;
        let finish = (value.end * f64::from(content.ppqn)).round() as u64;
        if finish <= start {
            return Err("The source tick resolution would collapse a generated note; lengthen the note or grid".into());
        }
        let start_order = order()?;
        if let Some(source) = value.source {
            let old = &original.notes[source];
            for &message_index in &owned_messages[source] {
                if content.messages.len() + content.meta.len() + (content.notes.len() + 1) * 2 + 1
                    >= crate::midi_file::MAX_EVENTS
                {
                    return Err("Generated expression exceeds the MIDI event limit".into());
                }
                let mut message = original.messages[message_index];
                let fraction = ((message.tick as f64 / f64::from(content.ppqn)
                    - old.source_start())
                    / old.source_duration())
                .clamp(0.0, 1.0);
                message.tick =
                    (start + ((finish - start) as f64 * fraction).round() as u64).min(finish - 1);
                if message.bytes[0] & 0xf0 == 0xa0 {
                    message.bytes[1] = value.pitch;
                }
                message.order = order()?;
                content.messages.push(message);
                expression_events += 1;
            }
        }
        let timing = super::super::midi_data::TickTiming {
            ppqn: content.ppqn,
            start,
            duration: finish - start,
            start_order,
            end_order: order()?,
        };
        if !timing.valid() {
            return Err("Generated note ticks leave their supported range".into());
        }
        let id = NoteId::new();
        if !id.valid() {
            return Err("A stable generated note identity could not be created".into());
        }
        rounding = rounding
            .max((timing.start_beats() - value.start).abs())
            .max((timing.start_beats() + timing.duration_beats() - value.end).abs());
        content.notes.push(MidiNote {
            id,
            channel: value.channel,
            pitch: value.pitch,
            start: timing.start_beats() as f32,
            len: timing.duration_beats() as f32,
            vel: value.velocity,
            release_vel: value
                .source
                .map_or(64, |source| original.notes[source].release_vel),
            muted: false,
            source_timing: Some(timing),
            variation: None,
        });
    }
    let mut voices = BTreeMap::<(u8, u8), Vec<usize>>::new();
    for (index, note) in content.notes.iter().enumerate() {
        voices
            .entry((note.channel, note.pitch))
            .or_default()
            .push(index);
    }
    for indices in voices.values_mut() {
        cancelled(cancel)?;
        indices.sort_unstable_by(|a, b| {
            content.notes[*a]
                .source_start()
                .total_cmp(&content.notes[*b].source_start())
        });
        let mut original_end = 0.0f64;
        let mut generated_end = 0.0f64;
        for &index in indices.iter() {
            let note = &content.notes[index];
            let start = note.source_start();
            let end = start + note.source_duration();
            if start < generated_end || index >= retained && start < original_end {
                return Err("Generated notes would overlap the same MIDI voice; shorten gates, change channels or replace the selection".into());
            }
            if index >= retained {
                generated_end = generated_end.max(end);
            } else {
                original_end = original_end.max(end);
            }
        }
    }
    super::super::note_variation::validate(&content.notes)?;
    content
        .messages
        .sort_unstable_by_key(|message| (message.tick, message.order));
    let changed: Vec<_> = (0..content.notes.len())
        .map(|index| index >= retained)
        .collect();
    let next_owners = owners(&content, &changed, params.expression, cancel)?;
    let original_orders: BTreeSet<_> = original
        .messages
        .iter()
        .map(|message| message.order)
        .collect();
    for (index, message) in content.messages.iter().enumerate() {
        if next_owners[index].is_some_and(|owner| owner >= retained)
            && original_orders.contains(&message.order)
        {
            return Err("Generated voices would take expression from the original material; change their channel or range".into());
        }
    }
    content.end_tick = content.end_tick.max(
        content
            .notes
            .iter()
            .map(|note| {
                ((note.source_start() + note.source_duration()) * f64::from(content.ppqn)).ceil()
                    as u64
            })
            .max()
            .unwrap_or(0),
    );
    if content.messages.len() + content.meta.len() + content.notes.len() * 2 + 1
        > crate::midi_file::MAX_EVENTS
        || content.lane_bytes() > super::super::midi_data::MAX_LANE_BYTES
    {
        return Err("Generated expression exceeds the MIDI event or lane limit".into());
    }
    validate_wire(&content, cancel)?;
    cancelled(cancel)?;
    let before: Vec<_> = original
        .notes
        .iter()
        .enumerate()
        .filter(|(_, note)| selected.contains(&note.id))
        .map(|(index, _)| index)
        .collect();
    let after: Vec<_> = (retained..content.notes.len()).collect();
    let summary = Summary {
        original: positive_timing(timing(&original.notes, &before), params.composition.start),
        transformed: positive_timing(timing(&content.notes, &after), params.composition.start),
        mean_start_shift: 0.0,
        maximum_rounding_beats: rounding,
        expression_events,
    };
    Ok(Prepared { content, summary })
}

#[cfg(test)]
mod tests;
