use super::*;

/// Prepare a harmony without changing the existing phrase.
/// Takes captured notes, explicit selection, saved scale parameters and cancellation; returns bounded new voices with distinct identities and uniquely owned pressure, or refuses the whole edit.
pub(super) fn harmonize(
    original: &Content,
    selected: &BTreeSet<NoteId>,
    params: &Parameters,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let mut transposition = params.clone();
    transposition.kind = Kind::ScaleTranspose;
    let shifted = prepare(original, selected, &transposition, cancel)?;
    let copied: Vec<_> = original
        .notes
        .iter()
        .zip(&shifted.content.notes)
        .enumerate()
        .filter(|(_, (old, next))| selected.contains(&old.id) && old.pitch != next.pitch)
        .map(|(i, _)| i)
        .collect();
    if copied.is_empty() {
        return Err(
            "Choose a nonzero scale interval and notes that participate in this scale".into(),
        );
    }
    if original.notes.len() + copied.len() > super::super::project::MAX_NOTES_PER_CLIP {
        return Err("Harmony exceeds the clip's 8192-note limit".into());
    }
    if copied
        .iter()
        .any(|&i| params.expression.member(original.notes[i].channel))
    {
        return Err("Harmony needs separate MPE voices; choose ordinary MIDI channels before copying member-channel notes".into());
    }
    let mut changed = vec![false; original.notes.len()];
    for &i in &copied {
        changed[i] = true;
    }
    let ownership = owners(original, &changed, params.expression, cancel)?;
    let orders = event_orders(original)?;
    let mut next_order = orders
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .chain(original.messages.iter().map(|m| m.order))
        .chain(original.meta.iter().map(|m| m.order))
        .max()
        .unwrap_or(0);
    let mut order = || {
        next_order = next_order
            .checked_add(1)
            .ok_or("Harmony MIDI event order is exhausted")?;
        Ok::<_, String>(next_order)
    };
    let mut content = original.clone();
    let mut copies = BTreeMap::new();
    let mut rounding = shifted.summary.maximum_rounding_beats;
    for (step, &i) in copied.iter().enumerate() {
        if step % 64 == 0 {
            cancelled(cancel)?;
        }
        let mut note = shifted.content.notes[i].clone();
        note.id = NoteId::new();
        if !note.id.valid() {
            return Err("A stable harmony note identity could not be created".into());
        }
        let start_order = order()?;
        let end_order = order()?;
        if let Some(timing) = &mut note.source_timing {
            timing.start_order = start_order;
            timing.end_order = end_order;
        } else {
            let start = (note.source_start() * f64::from(content.ppqn)).round() as u64;
            let end = ((note.source_start() + note.source_duration()) * f64::from(content.ppqn))
                .round() as u64;
            let timing = super::super::midi_data::TickTiming {
                ppqn: content.ppqn,
                start,
                duration: end - start,
                start_order,
                end_order,
            };
            if !timing.valid() {
                return Err("Harmony cannot retain a valid MIDI note duration".into());
            }
            rounding = rounding
                .max((timing.start_beats() - note.source_start()).abs())
                .max(
                    (timing.start_beats() + timing.duration_beats()
                        - note.source_start()
                        - note.source_duration())
                    .abs(),
                );
            note.start = timing.start_beats() as f32;
            note.len = timing.duration_beats() as f32;
            note.source_timing = Some(timing);
        }
        copies.insert(i, content.notes.len());
        content.notes.push(note);
    }
    let original_message_count = content.messages.len();
    for (step, owner) in ownership.iter().enumerate() {
        if step % 64 == 0 {
            cancelled(cancel)?;
        }
        if let Some(copy) = owner.and_then(|index| copies.get(&index).copied()) {
            let mut message = original.messages[step];
            if message.bytes[0] & 0xf0 == 0xa0 {
                message.bytes[1] = content.notes[copy].pitch;
                message.order = order()?;
                content.messages.push(message);
            }
        }
    }
    if content.messages.len() + content.meta.len() + content.notes.len() * 2 + 1
        > crate::midi_file::MAX_EVENTS
        || content.lane_bytes() > super::super::midi_data::MAX_LANE_BYTES
    {
        return Err("Harmony exceeds the existing MIDI event or lane limit".into());
    }
    let mut new_voices = vec![false; content.notes.len()];
    for &index in copies.values() {
        new_voices[index] = true;
    }
    let next_ownership = owners(&content, &new_voices, params.expression, cancel)?;
    if ownership
        .iter()
        .zip(&next_ownership)
        .any(|(old, next)| old.is_none() && next.is_some_and(|i| new_voices[i]))
    {
        return Err("Harmony would capture pressure that had no original note; choose a different interval or phrase".into());
    }
    content.messages.sort_unstable_by_key(|m| (m.tick, m.order));
    validate_wire(&content, cancel)?;
    cancelled(cancel)?;
    let selected_indices: Vec<_> = original
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| selected.contains(&n.id))
        .map(|(i, _)| i)
        .collect();
    let after_indices: Vec<_> = selected_indices
        .iter()
        .copied()
        .chain(copies.values().copied())
        .collect();
    let expression_events = content.messages.len() - original_message_count;
    let summary = Summary {
        original: timing(&original.notes, &selected_indices),
        transformed: timing(&content.notes, &after_indices),
        mean_start_shift: 0.0,
        maximum_rounding_beats: rounding,
        expression_events,
    };
    Ok(Prepared { content, summary })
}

#[cfg(test)]
mod tests;
