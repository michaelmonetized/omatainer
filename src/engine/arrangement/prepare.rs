use super::*;
use std::collections::{BTreeMap, BTreeSet};

impl Plan {
    /// Prepare one bounded song without borrowing a renderer.
    /// Takes immutable metadata, native media, retained track layout and cancellation; returns shared audio and exact song-beat events or a capacity/source refusal.
    pub(crate) fn prepare(
        model: Arc<Model>,
        media: &[Arc<Sample>],
        layout: &session::Layout,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Arc<Self>, String> {
        model.validate(media, layout)?;
        let mut sources = Vec::with_capacity(model.sources.len());
        let mut checked = std::collections::HashSet::new();
        for source in &model.sources {
            active(cancel)?;
            let audio = source.clip.audio.map(|i| media[i].clone());
            if let Some(audio) = &audio {
                if checked.insert(Arc::as_ptr(audio) as usize) {
                    for chunk in audio.data.chunks(16384) {
                        active(cancel)?;
                        if chunk.iter().any(|v| !v.is_finite()) {
                            return Err("Arrangement source contains nonfinite PCM".into());
                        }
                    }
                }
            }
            let audio_region = source
                .clip
                .audio_region
                .map(|r| r.prepare(audio.as_ref().unwrap()).map_err(str::to_owned))
                .transpose()?;
            let length = audio_region.map_or_else(
                || {
                    source
                        .clip
                        .region
                        .map_or(f64::from(source.clip.bars) * 4.0, |r| r.end - r.start)
                },
                |r| r.duration_beats,
            );
            if source.clip.kind == ClipKind::Audio
                && audio.as_ref().is_none_or(|s| {
                    s.frames() == 0
                        || s.ch == 0
                        || s.ch > 2
                        || s.sr == 0
                        || s.data.len() % usize::from(s.ch) != 0
                })
            {
                return Err("Arrangement audio source is empty or unsupported".into());
            }
            sources.push(PreparedSource {
                audio,
                audio_region,
                length,
            });
        }
        let mut tracks = (0..layout.tracks.len())
            .map(|slot| {
                layout
                    .reference(session::Axis::Track, slot)
                    .map(|reference| TrackPlan {
                        reference,
                        activity: Vec::new(),
                        audio: Vec::new(),
                        audio_segments: Vec::new(),
                        notes: Vec::new(),
                        note_segments: Vec::new(),
                        events: Vec::new(),
                        gates: Vec::new(),
                        controls: Vec::new(),
                    })
            })
            .collect::<Vec<_>>();
        let mut event_count = 0;
        for instance in &model.instances {
            active(cancel)?;
            let index = model
                .sources
                .iter()
                .position(|s| s.id == instance.source)
                .unwrap();
            let clip = &model.sources[index].clip;
            let source = &sources[index];
            if clip.properties.disabled {continue;}
            let repeating = if clip.kind == ClipKind::Midi {
                clip.region
                    .map_or(instance.repeating, |r| r.repeating(instance.repeating))
            } else {
                instance.repeating
            };
            if !repeating && instance.offset + instance.duration > source.length + 1e-9 {
                return Err("One-shot arrangement placement extends beyond its source; trim it or enable repetition".into());
            }
            let Some(track) = tracks
                .iter_mut()
                .flatten()
                .find(|t| t.reference == instance.track)
            else {
                continue;
            };
            let gain = super::super::clip_gain(clip.gain) * super::super::clip_gain(instance.gain);
            if clip.kind == ClipKind::Audio {
                track.audio.push(AudioSpan {
                    source: index,
                    start: instance.start,
                    end: instance.start + instance.duration,
                    offset: instance.offset,
                    repeating,
                    gain,
                });
            } else {
                let region = clip
                    .region
                    .unwrap_or_else(|| midi_edit::Region::full(clip.bars));
                let intro = if repeating {
                    region.loop_end - region.start
                } else {
                    region.end - region.start
                };
                let limit = instance.offset + instance.duration;
                for note in &clip.notes {
                    active(cancel)?;
                    if note.muted || note.vel == 0 || note.source_duration() <= 0.0 {
                        continue;
                    }
                    let position = if clip.region.is_some() {
                        note.source_start()
                    } else {
                        note.source_start().rem_euclid(source.length)
                    };
                    let end = if clip.region.is_some() {
                        (position + note.source_duration()).min(region.start + intro)
                    } else {
                        position + note.source_duration()
                    };
                    if position < region.start + intro && end > region.start {
                        add_note(
                            track,
                            instance,
                            note,
                            position.max(region.start) - region.start,
                            end - region.start,
                            gain,
                            &mut event_count,
                        )?;
                    }
                    if repeating && position >= region.loop_start && position < region.loop_end {
                        let duration = if clip.region.is_some() {
                            (position + note.source_duration()).min(region.loop_end) - position
                        } else {
                            note.source_duration()
                        };
                        let first = intro + position - region.loop_start;
                        let earliest = ((instance.offset - first - duration) / region.period())
                            .ceil()
                            .max(0.0) as u64;
                        let last = ((limit - first) / region.period()).ceil().max(0.0) as u64;
                        if last.saturating_sub(earliest) > MAX_EVENTS as u64 {
                            return Err("Arrangement MIDI repetition exceeds event capacity".into());
                        }
                        for cycle in earliest..last {
                            active(cancel)?;
                            let start = first + cycle as f64 * region.period();
                            add_note(
                                track,
                                instance,
                                note,
                                start,
                                start + duration,
                                gain,
                                &mut event_count,
                            )?;
                        }
                    }
                }
                if let Some(lanes) = &clip.lanes {
                    let lanes = lanes.prepare()?;
                    let at = region
                        .position(instance.offset, instance.repeating)
                        .unwrap_or(region.end)
                        * f64::from(lanes.ppqn);
                    let cycling = repeating && instance.offset >= intro;
                    let point = |key: u16| {
                        let lane = lanes
                            .state
                            .binary_search_by_key(&key, |l| l.key)
                            .ok()
                            .map(|i| &lanes.state[i])?;
                        let mut point = lane.before(at);
                        if cycling
                            && point.is_none_or(|p| {
                                (p.message.tick as f64) < region.loop_start * f64::from(lanes.ppqn)
                            })
                        {
                            point = lane
                                .before(region.loop_end * f64::from(lanes.ppqn))
                                .or(point);
                        }
                        point
                    };
                    let mut error = None;
                    midi_data::chase(point, |bytes, length, order| {
                        if error.is_none() {
                            error = add_control(
                                track,
                                instance.start,
                                bytes,
                                length,
                                ((instance.id as u128) << 64) + u128::from(order),
                                &mut event_count,
                            )
                            .err();
                        }
                    });
                    if let Some(error) = error {
                        return Err(error);
                    }
                    for message in &lanes.messages {
                        let position = message.tick as f64 / f64::from(lanes.ppqn);
                        if (region.start..region.start + intro).contains(&position) {
                            add_instance_control(
                                track,
                                instance,
                                position - region.start,
                                message,
                                &mut event_count,
                            )?;
                        }
                        if repeating && (region.loop_start..region.loop_end).contains(&position) {
                            let first = intro + position - region.loop_start;
                            let earliest = ((instance.offset - first) / region.period())
                                .ceil()
                                .max(0.0) as u64;
                            let last = ((limit - first) / region.period()).ceil().max(0.0) as u64;
                            if last.saturating_sub(earliest) > MAX_EVENTS as u64 {
                                return Err(
                                    "Arrangement controller repetition exceeds event capacity"
                                        .into(),
                                );
                            }
                            for cycle in earliest..last {
                                active(cancel)?;
                                add_instance_control(
                                    track,
                                    instance,
                                    first + cycle as f64 * region.period(),
                                    message,
                                    &mut event_count,
                                )?;
                            }
                        }
                    }
                }
            }
        }
        let mut bytes = std::mem::size_of::<Self>()
            + serde_json::to_vec(model.as_ref())
                .map_err(|e| e.to_string())?
                .len();
        for track in tracks.iter_mut().flatten() {
            active(cancel)?;
            track.activity = segments(
                model
                    .instances
                    .iter()
                    .filter(|i| i.track == track.reference && model.sources.iter().any(|s|s.id==i.source&&!s.clip.properties.disabled))
                    .map(|i| (i.start, i.start + i.duration)),
                MAX_OVERLAP,
                &mut bytes,
                cancel,
            )?;
            track.audio_segments = segments(
                track.audio.iter().map(|s| (s.start, s.end)),
                MAX_OVERLAP,
                &mut bytes,
                cancel,
            )?;
            track.note_segments = segments(
                track.notes.iter().map(|s| (s.start, s.end)),
                MAX_ACTIVE_NOTES,
                &mut bytes,
                cancel,
            )?;
            for segment in &mut track.note_segments {
                segment.active.sort_by(|a, b| {
                    track.notes[*a as usize]
                        .start
                        .total_cmp(&track.notes[*b as usize].start)
                        .then_with(|| a.cmp(b))
                });
            }
            track.events.sort_by(|a, b| {
                a.beat
                    .total_cmp(&b.beat)
                    .then_with(|| rank(a.kind).cmp(&rank(b.kind)))
                    .then_with(|| a.order.cmp(&b.order))
            });
            track.gates = track
                .events
                .iter()
                .copied()
                .filter(|e| !matches!(e.kind, Kind::Control { .. }))
                .collect();
            for window in track.gates.windows(513) {
                if window[512].beat - window[0].beat < 240.0 / 60.0 / 8000.0 {
                    return Err(
                        "Arrangement note density exceeds the bounded audio callback".into(),
                    );
                }
            }
            let mut controls = BTreeMap::<u16, Vec<(f64, midi_data::StatePoint)>>::new();
            let mut banks = [[None; 2]; 16];
            for (ordinal, event) in track.events.iter().enumerate() {
                if let Kind::Control { bytes, length } = event.kind {
                    let channel = usize::from(bytes[0] & 15);
                    if bytes[0] & 0xf0 == 0xb0 {
                        if bytes[1] == 0 {
                            banks[channel][0] = Some(bytes[2]);
                        }
                        if bytes[1] == 32 {
                            banks[channel][1] = Some(bytes[2]);
                        }
                    }
                    let key = control_key(bytes);
                    let message = crate::midi_file::Message {
                        tick: ordinal as u64,
                        order: 0,
                        bytes,
                        length,
                    };
                    controls.entry(key).or_default().push((
                        event.beat,
                        midi_data::StatePoint {
                            message,
                            banks: banks[channel],
                        },
                    ));
                }
            }
            track.controls = controls
                .into_iter()
                .map(|(key, points)| ControlLane { key, points })
                .collect();
            bytes = bytes.saturating_add(
                track.audio.capacity() * std::mem::size_of::<AudioSpan>()
                    + track.notes.capacity() * std::mem::size_of::<NoteSpan>()
                    + (track.events.capacity() + track.gates.capacity())
                        * std::mem::size_of::<Event>()
                    + track
                        .controls
                        .iter()
                        .map(|l| {
                            l.points.capacity()
                                * std::mem::size_of::<(f64, midi_data::StatePoint)>()
                        })
                        .sum::<usize>(),
            );
            if bytes > MAX_PLAN_BYTES {
                return Err("Arrangement prepared schedule exceeds 64 MiB".into());
            }
        }
        let end = model
            .instances
            .iter()
            .map(|i| i.start + i.duration)
            .fold(0.0_f64, f64::max);
        Ok(Arc::new(Self {
            model,
            sources,
            tracks,
            end,
            bytes,
        }))
    }
}
fn active(cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        Err("Arrangement preparation cancelled".into())
    } else {
        Ok(())
    }
}
fn rank(kind: Kind) -> u8 {
    match kind {
        Kind::Off(_) => 0,
        Kind::Control { .. } => 1,
        Kind::On(_) => 2,
    }
}
fn count(count: &mut usize) -> Result<(), String> {
    *count += 1;
    if *count > MAX_EVENTS {
        Err("Arrangement exceeds 262144 scheduled boundaries".into())
    } else {
        Ok(())
    }
}
fn add_note(
    track: &mut TrackPlan,
    instance: &Instance,
    note: &MidiNote,
    start: f64,
    end: f64,
    gain: f32,
    total: &mut usize,
) -> Result<(), String> {
    let a = start.max(instance.offset);
    let b = end.min(instance.offset + instance.duration);
    if b <= a || gain == 0.0 {
        return Ok(());
    }
    count(total)?;
    count(total)?;
    let id = midi_edit::NoteId::new();
    if !id.valid() {
        return Err("Arrangement note identity unavailable".into());
    }
    let index = track.notes.len() as u32;
    let span = NoteSpan {
        start: instance.start + a - instance.offset,
        end: instance.start + b - instance.offset,
        id,
        pitch: note.pitch,
        velocity: note.vel,
        channel: note.channel,
        release: note.release_vel,
        gain,
    };
    track.notes.push(span);
    track.events.push(Event {
        beat: span.start,
        order: u128::from(instance.id),
        kind: Kind::On(index),
    });
    track.events.push(Event {
        beat: span.end,
        order: u128::from(instance.id),
        kind: Kind::Off(index),
    });
    Ok(())
}
fn control_key(bytes: [u8; 3]) -> u16 {
    u16::from(bytes[0] & 15) * 256
        + match bytes[0] & 0xf0 {
            0xb0 => u16::from(bytes[1]),
            0xe0 => 128,
            0xd0 => 129,
            _ => 130,
        }
}
fn add_control(
    track: &mut TrackPlan,
    beat: f64,
    bytes: [u8; 3],
    length: u8,
    order: u128,
    total: &mut usize,
) -> Result<(), String> {
    count(total)?;
    track.events.push(Event {
        beat,
        order,
        kind: Kind::Control { bytes, length },
    });
    Ok(())
}
fn add_instance_control(
    track: &mut TrackPlan,
    instance: &Instance,
    position: f64,
    message: &crate::midi_file::Message,
    total: &mut usize,
) -> Result<(), String> {
    if (instance.offset..instance.offset + instance.duration).contains(&position) {
        add_control(
            track,
            instance.start + position - instance.offset,
            message.bytes,
            message.length,
            ((instance.id as u128) << 64) + (1_u128 << 32) + u128::from(message.order),
            total,
        )?;
    }
    Ok(())
}
fn segments(
    intervals: impl Iterator<Item = (f64, f64)>,
    limit: usize,
    bytes: &mut usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<Segment>, String> {
    let mut boundaries = intervals
        .enumerate()
        .flat_map(|(i, (a, b))| [(a, true, i as u32), (b, false, i as u32)])
        .collect::<Vec<_>>();
    boundaries.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut active_set = BTreeSet::new();
    let mut segments = Vec::new();
    let mut i = 0;
    while i < boundaries.len() {
        active(cancel)?;
        let beat = boundaries[i].0;
        while i < boundaries.len() && boundaries[i].0 == beat {
            let (_, on, index) = boundaries[i];
            if on {
                active_set.insert(index);
            } else {
                active_set.remove(&index);
            }
            i += 1;
        }
        if active_set.len() > limit {
            return Err(format!(
                "Arrangement exceeds {limit} simultaneous sources or notes on one track"
            ));
        }
        *bytes = bytes.saturating_add(std::mem::size_of::<Segment>() + active_set.len() * 4);
        if *bytes > MAX_PLAN_BYTES {
            return Err("Arrangement seek index exceeds 64 MiB".into());
        }
        segments.push(Segment {
            beat,
            active: active_set.iter().copied().collect(),
        });
    }
    Ok(segments)
}
