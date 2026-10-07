//! Worker-prepared MIDI file mapping and exact source-coordinate exports.
use super::{
    midi_data::{Conductor, Lanes},
    midi_edit::{Ack, Document, Region},
    *,
};
use crate::midi_file::{self as smf, MetaValue};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub track: usize,
    pub channel: Option<u8>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mapping {
    pub source: Source,
    pub destination: Option<(u8, u16)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TempoChoice {
    KeepSession,
    File(Option<usize>),
}
pub(crate) fn sources(file: &smf::File, split_channels: bool) -> Vec<Source> {
    file.tracks
        .iter()
        .enumerate()
        .flat_map(|(track, data)| {
            let channels: BTreeSet<_> = data
                .notes
                .iter()
                .map(|n| n.channel)
                .chain(data.messages.iter().map(|m| m.bytes[0] & 15))
                .collect();
            if !split_channels || channels.is_empty() {
                vec![Source {
                    track,
                    channel: None,
                }]
            } else {
                channels
                    .into_iter()
                    .map(|channel| Source {
                        track,
                        channel: Some(channel),
                    })
                    .collect()
            }
        })
        .collect()
}

pub(super) struct Target {
    pub baseline: Arc<Document>,
    pub replacement: Clip,
    pub spare_notes: Vec<MidiNote>,
    pub reserved_lane_bytes: usize,
}
impl Clone for Target {
    fn clone(&self) -> Self {
        let mut spare_notes = Vec::with_capacity(self.spare_notes.capacity());
        spare_notes.extend_from_slice(&self.spare_notes);
        Self {
            baseline: self.baseline.clone(),
            replacement: self.replacement.clone(),
            spare_notes,
            reserved_lane_bytes: self.reserved_lane_bytes,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Request {
    pub(super) targets: Vec<Target>,
    pub(super) epoch: u64,
    pub(super) session_namespace: Option<[u64; 2]>,
    pub(super) baseline_bpm: f32,
    pub(super) baseline_scene_timing: Option<super::scene::Timing>,
    pub(super) baseline_conductor: Option<Arc<Conductor>>,
    pub(super) conductor: Option<Arc<Conductor>>,
    pub(super) change_conductor: bool,
    pub(super) ack: Ack,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiImport")
            .field("targets", &self.targets.len())
            .field("epoch", &self.epoch)
            .finish()
    }
}
impl Request {
    /// Complete validation, merging and inverse scratch allocation happen on
    /// the worker. The renderer sees a private, immutable prepared request.
    pub fn prepare(
        captured: project::Captured,
        file: &smf::File,
        mappings: &[Mapping],
        split_channels: bool,
        merge: bool,
        tempo: TempoChoice,
        reviewed_omissions: bool,
        allow_rounding: bool,
    ) -> Result<(Self, Ack), String> {
        Self::prepare_with_cancel(
            captured,
            file,
            mappings,
            split_channels,
            merge,
            tempo,
            reviewed_omissions,
            allow_rounding,
            || false,
        )
    }
    pub fn prepare_with_cancel(
        mut captured: project::Captured,
        file: &smf::File,
        mappings: &[Mapping],
        split_channels: bool,
        merge: bool,
        tempo: TempoChoice,
        reviewed_omissions: bool,
        allow_rounding: bool,
        mut cancel: impl FnMut() -> bool,
    ) -> Result<(Self, Ack), String> {
        checkpoint(&mut cancel)?;
        let baseline_conductor = captured.state.conductor.clone();
        let baseline_bpm = captured.state.bpm;
        let baseline_scene_timing = captured.state.scene_timing;
        if !file.warnings.is_empty() && !reviewed_omissions {
            return Err(
                "Review and accept the listed unsupported MIDI data before importing".into(),
            );
        }
        let expected = sources(file, split_channels);
        if mappings.len() != expected.len()
            || mappings.iter().zip(&expected).any(|(m, s)| m.source != *s)
        {
            return Err("MIDI track/channel mapping no longer matches the inspected file".into());
        }
        let mut mapped = BTreeMap::<(u8, u16), smf::Track>::new();
        // Every source event receives a unique ordering range when tracks are
        // merged. Channel splitting attaches track metadata to its first row.
        for (row, mapping) in mappings.iter().enumerate() {
            checkpoint(&mut cancel)?;
            let Some(destination) = mapping.destination else {
                continue;
            };
            if destination.0 as usize >= captured.state.tracks.len() || destination.1 as usize >= captured.state.scene_fx.len()
                || captured.state.session.as_ref().is_some_and(|layout| layout.reference(session::Axis::Track, destination.0 as usize).is_none() || layout.reference(session::Axis::Scene,destination.1 as usize).is_none()) {
                return Err("Unknown or inactive MIDI destination".into());
            }
            let source = &file.tracks[mapping.source.track];
            let out = mapped.entry(destination).or_default();
            let offset = (mapping.source.track as u32)
                .checked_mul(smf::MAX_EVENTS as u32 + 1)
                .ok_or("MIDI source ordering overflow")?;
            max_order(source)
                .checked_add(offset)
                .ok_or("MIDI source order overflow")?;
            for note in source
                .notes
                .iter()
                .filter(|n| mapping.source.channel.is_none_or(|c| c == n.channel))
            {
                checkpoint(&mut cancel)?;
                let mut note = *note;
                note.start_order += offset;
                note.end_order += offset;
                out.notes.push(note);
            }
            for message in source
                .messages
                .iter()
                .filter(|m| mapping.source.channel.is_none_or(|c| c == m.bytes[0] & 15))
            {
                checkpoint(&mut cancel)?;
                let mut message = *message;
                message.order += offset;
                out.messages.push(message);
            }
            let first_row = row == 0 || mappings[row - 1].source.track != mapping.source.track;
            if first_row {
                out.meta.extend(source.meta.iter().cloned().map(|mut m| {
                    m.order += offset;
                    m
                }));
            }
            out.end_tick = out.end_tick.max(source.end_tick);
        }
        let conductor = match tempo {
            TempoChoice::KeepSession => captured.state.conductor.clone(),
            TempoChoice::File(track) => {
                if track.is_some_and(|i| i >= file.tracks.len()) {
                    return Err("Unknown authoritative conductor track".into());
                }
                let mut events = Vec::new();
                for (index, source) in file.tracks.iter().enumerate() {
                    if track.is_some_and(|t| t != index) {
                        continue;
                    }
                    for event in &source.meta {
                        checkpoint(&mut cancel)?;
                        if matches!(event.value, MetaValue::Tempo(_) | MetaValue::Meter { .. }) {
                            if events.len() >= 2 * super::midi_data::MAX_CONDUCTOR_POINTS {
                                return Err("File conductor exceeds 4096 tempo/meter points".into());
                            }
                            events.push(event.clone());
                        }
                    }
                }
                Some(Conductor::from_meta(file.ppqn, events.into_iter())?)
            }
        };
        let mut total_notes = captured
            .state
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.notes.len())
            .sum::<usize>();
        let mut total_bytes = captured
            .state
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.lanes.as_ref().map_or(0, |l| l.bytes()))
            .sum::<usize>()
            + conductor.as_ref().map_or(0, |c| c.bytes());
        let mut targets = Vec::with_capacity(mapped.len());
        for ((track, scene), mut incoming) in mapped {
            checkpoint(&mut cancel)?;
            let old = &captured.state.tracks[track as usize].clips[scene as usize];
            if old.kind == ClipKind::Audio || old.audio.is_some() {
                return Err(format!(
                    "Track {}, scene {} contains audio; choose another destination",
                    track + 1,
                    scene + 1
                ));
            }
            if merge && old.kind == ClipKind::Midi {
                let existing = export_clip(old, file.ppqn, true, allow_rounding, &mut cancel)?;
                let offset = max_order(&incoming)
                    .checked_add(1)
                    .ok_or("MIDI merge order overflow")?;
                max_order(&existing)
                    .checked_add(offset)
                    .ok_or("MIDI merge order overflow")?;
                incoming
                    .notes
                    .extend(existing.notes.into_iter().map(|mut n| {
                        n.start_order += offset;
                        n.end_order += offset;
                        n
                    }));
                incoming
                    .messages
                    .extend(existing.messages.into_iter().map(|mut m| {
                        m.order += offset;
                        m
                    }));
                incoming.meta.extend(existing.meta.into_iter().map(|mut m| {
                    m.order += offset;
                    m
                }));
                incoming.end_tick = incoming.end_tick.max(existing.end_tick);
            }
            // Refuse ambiguous same-channel/pitch overlap introduced by merging
            // tracks. Never rewrite note durations to make a file fit.
            smf::encode_with_cancel(
                &smf::File {
                    format: smf::Format::Single,
                    ppqn: file.ppqn,
                    tracks: vec![incoming.clone()],
                    warnings: vec![],
                },
                false,
                &mut cancel,
            )
            .map_err(|e| e.to_string())?;
            if incoming.notes.len() > project::MAX_NOTES_PER_CLIP {
                return Err(
                    "A mapped clip exceeds 8192 notes; split it across destinations".into(),
                );
            }
            let notes: Vec<_> = incoming
                .notes
                .iter()
                .map(|n| {
                    checkpoint(&mut cancel)?;
                    MidiNote::from_smf(n, file.ppqn)
                })
                .collect::<Result<_, _>>()?;
            let lanes = Lanes::new_with_cancel(
                file.ppqn,
                incoming.end_tick,
                incoming.messages,
                incoming.meta,
                &mut cancel,
            )?;
            total_notes = total_notes - old.notes.len() + notes.len();
            total_bytes = total_bytes - old.lanes.as_ref().map_or(0, |l| l.bytes()) + lanes.bytes();
            let end = (incoming.end_tick as f64 / f64::from(file.ppqn)).max(1.0 / 1024.0);
            let region = Region {
                start: 0.0,
                end,
                loop_start: 0.0,
                loop_end: end,
                loop_enabled: false,
            };
            if !region.allows(&notes) {
                return Err("Imported clip exceeds musical playback bounds".into());
            }
            let name = lanes
                .meta
                .iter()
                .find_map(|m| match &m.value {
                    MetaValue::Text { kind: 3, bytes } => std::str::from_utf8(bytes).ok(),
                    _ => None,
                })
                .filter(|n| n.len() <= 4096 && !n.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("MIDI import {}:{}", track + 1, scene + 1));
            let baseline = Arc::new(Document {
                track_identity: captured.state.session.as_ref().and_then(|layout| layout.reference(session::Axis::Track, track as usize)),
                scene_identity: captured.state.session.as_ref().and_then(|layout| layout.reference(session::Axis::Scene, scene as usize)),
                track,
                scene,
                epoch: captured.checkpoint.epoch,
                kind: old.kind,
                name: old.name.clone(),
                bars: old.bars,
                region: old.region,
                notes: old.notes.clone(),
                lanes: old.lanes.clone(),
            });
            let reserved_lane_bytes = old.lanes.as_ref().map_or(0, |l| l.bytes()) + lanes.bytes();
            targets.push(Target {
                baseline,
                replacement: Clip {
                    properties: old.properties,
                    audio_region: None, lanes: Some(lanes),
                    region: Some(region),
                    kind: ClipKind::Midi,
                    name,
                    bars: (end / 4.0) as f32,
                    notes,
                    gain: old.gain,
                    audio: None,
                },
                spare_notes: Vec::with_capacity(project::MAX_NOTES_PER_CLIP),
                reserved_lane_bytes,
            });
        }
        if total_notes > project::MAX_TOTAL_NOTES || total_bytes > super::midi_data::MAX_LANE_BYTES
        {
            return Err(
                "Import would exceed 65536 session notes or 16 MiB of MIDI metadata".into(),
            );
        }
        if targets.is_empty() && tempo == TempoChoice::KeepSession {
            return Err("Select at least one MIDI destination or apply a file conductor".into());
        }
        // Refuse a musical import that cannot be saved in the native project
        // codec. Leave room for the bounded GUI view envelope as well.
        for target in &targets {
            let saved = &mut captured.state.tracks[target.baseline.track as usize].clips
                [target.baseline.scene as usize];
            saved.name = target.replacement.name.clone();
            saved.kind = target.replacement.kind;
            saved.bars = target.replacement.bars;
            saved.region = target.replacement.region;
            saved.notes = target.replacement.notes.clone();
            saved.lanes = target.replacement.lanes.clone();
        }
        captured.state.conductor = conductor.clone();
        if tempo != TempoChoice::KeepSession { captured.state.scene_timing = None; }
        struct Size<'a, F> {
            bytes: usize,
            cancel: &'a mut F,
        }
        impl<F: FnMut() -> bool> std::io::Write for Size<'_, F> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if (self.cancel)() {
                    return Err(std::io::Error::other("MIDI operation cancelled"));
                }
                self.bytes += bytes.len();
                if self.bytes > crate::project_file::DEFAULT_METADATA_LIMIT - 128 * 1024 {
                    return Err(std::io::Error::other(
                        "Import cannot fit the native project's 64 MiB metadata limit",
                    ));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(
            Size {
                bytes: 0,
                cancel: &mut cancel,
            },
            &captured.state,
        )
        .map_err(|e| e.to_string())?;
        checkpoint(&mut cancel)?;
        let ack = Ack::new();
        Ok((
            Self {
                targets,
                epoch: captured.checkpoint.epoch,
                session_namespace: captured.state.session.as_ref().map(|s| s.namespace),
                baseline_bpm,
                baseline_scene_timing,
                baseline_conductor,
                conductor,
                change_conductor: tempo != TempoChoice::KeepSession,
                ack: ack.clone(),
            },
            ack,
        ))
    }
    /// Native timing uses the same atomic conductor inverse, publication and
    /// retirement path as a MIDI import, with no unrelated clip replacements.
    pub fn prepare_timing(
        mut captured: project::Captured,
        value: Option<Arc<Conductor>>,
        mut cancel: impl FnMut() -> bool,
    ) -> Result<(Self, Ack), String> {
        checkpoint(&mut cancel)?;
        let namespace = captured.state.session.as_ref().ok_or("Session identity is unavailable")?.namespace;
        let baseline_conductor = captured.state.conductor.clone();
        let baseline_bpm = captured.state.bpm;
        let baseline_scene_timing = captured.state.scene_timing;
        let conductor = value.map(|c| c.prepare()).transpose()?;
        captured.state.conductor = conductor.clone();
        captured.state.scene_timing = None;
        captured.state.validate(&captured.media)?;
        struct Size<'a, F> { bytes: usize, cancel: &'a mut F }
        impl<F: FnMut() -> bool> std::io::Write for Size<'_, F> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if (self.cancel)() { return Err(std::io::Error::other("Timing edit cancelled")); }
                self.bytes = self.bytes.checked_add(bytes.len()).ok_or_else(|| std::io::Error::other("Timing metadata overflow"))?;
                if self.bytes > crate::project_file::DEFAULT_METADATA_LIMIT - 128 * 1024 { return Err(std::io::Error::other("Timing edit exceeds the native project's 64 MiB metadata limit")); }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        serde_json::to_writer(Size { bytes: 0, cancel: &mut cancel }, &captured.state).map_err(|e| e.to_string())?;
        checkpoint(&mut cancel)?;
        let ack = Ack::new();
        Ok((Self {
            targets: Vec::new(), epoch: captured.checkpoint.epoch,
            session_namespace: Some(namespace), baseline_bpm, baseline_conductor, baseline_scene_timing,
            conductor, change_conductor: true, ack: ack.clone(),
        }, ack))
    }
    pub fn bytes(&self) -> usize {
        self.targets.capacity() * std::mem::size_of::<Target>()
            + self
                .targets
                .iter()
                .map(|t| {
                    t.baseline.bytes()
                        + t.replacement.name.capacity()
                        + (t.replacement.notes.capacity() + t.spare_notes.capacity())
                            * std::mem::size_of::<MidiNote>()
                        + t.reserved_lane_bytes
                })
                .sum::<usize>()
            + self.baseline_conductor.as_ref().map_or(0, |c| c.bytes())
            + self.conductor.as_ref().map_or(0, |c| c.bytes())
    }
}
impl RtEngine {
    pub(super) fn midi_import_current(&self, request: &Request) -> bool {
        if request.session_namespace.is_some_and(|namespace| namespace != self.session.namespace)
            || request.epoch != self.undo.checkpoint().epoch
            || request.change_conductor
                && (self.recording || self.count_in.is_some()
                    || self.has_held_project_notes()
                    || (self.conductor.is_none() && self.bpm != request.baseline_bpm)
                    || self.conductor != request.baseline_conductor
                    || self.scenes.timing != request.baseline_scene_timing)
        {
            return false;
        }
        let mut notes = self.midi_note_count();
        let mut bytes = self
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.lanes.as_ref().map_or(0, |l| l.bytes()))
            .sum::<usize>();
        for target in &request.targets {
            let base = &target.baseline;
            let Some(clip) = self
                .tracks
                .get(base.track as usize)
                .and_then(|t| t.clips.get(base.scene as usize))
            else {
                return false;
            };
            if self.recording_clip_held(base.track as usize, base.scene as usize)
                || clip.kind != base.kind
                || clip.kind == ClipKind::Audio
                || clip.audio.is_some()
                || clip.name != base.name
                || clip.bars != base.bars
                || clip.region != base.region
                || clip.notes != base.notes
                || clip.lanes != base.lanes
            {
                return false;
            }
            notes = notes - clip.notes.len() + target.replacement.notes.len();
            bytes = bytes - clip.lanes.as_ref().map_or(0, |l| l.bytes())
                + target.replacement.lanes.as_ref().map_or(0, |l| l.bytes());
        }
        bytes += if request.change_conductor {
            request.conductor.as_ref()
        } else {
            self.conductor.as_ref()
        }
        .map_or(0, |c| c.bytes());
        notes <= project::MAX_TOTAL_NOTES && bytes <= super::midi_data::MAX_LANE_BYTES
    }
}
fn max_order(track: &smf::Track) -> u32 {
    track
        .notes
        .iter()
        .flat_map(|n| [n.start_order, n.end_order])
        .chain(track.messages.iter().map(|m| m.order))
        .chain(track.meta.iter().map(|m| m.order))
        .max()
        .unwrap_or(0)
}
fn convert_tick(tick: u64, source: u16, target: u16, allow_rounding: bool) -> Result<u64, String> {
    let scaled = tick
        .checked_mul(u64::from(target))
        .ok_or("MIDI tick conversion overflow")?;
    if scaled % u64::from(source) != 0 && !allow_rounding {
        return Err(format!("PPQN {target} cannot represent a source tick at PPQN {source}; choose its original division or explicitly allow nearest-tick rounding"));
    }
    Ok((scaled + u64::from(source) / 2) / u64::from(source))
}
fn native_tick(beat: f64, ppqn: u16, projection: f32, allow_rounding: bool) -> Result<u64, String> {
    let rounded = (beat * f64::from(ppqn)).round();
    if !beat.is_finite() || !(0.0..=262144.0).contains(&beat) {
        return Err("Invalid native MIDI time".into());
    }
    if (rounded / f64::from(ppqn)) as f32 != projection && !allow_rounding {
        return Err(format!(
            "PPQN {ppqn} cannot represent an edited beat; explicitly allow nearest-tick rounding"
        ));
    }
    Ok(rounded as u64)
}
fn checkpoint(cancel: &mut impl FnMut() -> bool) -> Result<(), String> {
    if cancel() {
        Err("MIDI operation cancelled".into())
    } else {
        Ok(())
    }
}
fn export_clip(
    clip: &project::SavedClip,
    ppqn: u16,
    include_muted: bool,
    allow_rounding: bool,
    cancel: &mut impl FnMut() -> bool,
) -> Result<smf::Track, String> {
    checkpoint(cancel)?;
    let mut result = smf::Track::default();
    if let Some(lanes) = &clip.lanes {
        result.end_tick = convert_tick(lanes.end_tick, lanes.ppqn, ppqn, allow_rounding)?;
        result.messages = lanes
            .messages
            .iter()
            .map(|m| {
                checkpoint(cancel)?;
                Ok(smf::Message {
                    tick: convert_tick(m.tick, lanes.ppqn, ppqn, allow_rounding)?,
                    ..*m
                })
            })
            .collect::<Result<_, String>>()?;
        result.meta = lanes
            .meta
            .iter()
            .map(|m| {
                checkpoint(cancel)?;
                Ok(smf::Meta {
                    tick: convert_tick(m.tick, lanes.ppqn, ppqn, allow_rounding)?,
                    ..m.clone()
                })
            })
            .collect::<Result<_, String>>()?;
    } else {
        let end = clip.region.map_or(f64::from(clip.bars) * 4.0, |r| r.end);
        result.end_tick = native_tick(end, ppqn, end as f32, allow_rounding)?;
    }
    let mut next = result
        .messages
        .iter()
        .map(|m| m.order)
        .chain(result.meta.iter().map(|m| m.order))
        .chain(
            clip.notes
                .iter()
                .filter_map(|n| n.source_timing.map(|t| t.end_order)),
        )
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("MIDI order overflow")?;
    for note in clip.notes.iter().filter(|n| include_muted || !n.muted) {
        checkpoint(cancel)?;
        if note.vel == 0 {
            continue;
        }
        let (start_tick, duration_ticks, start_order, end_order) =
            if let Some(t) = note.source_timing {
                let start = convert_tick(t.start, t.ppqn, ppqn, allow_rounding)?;
                let end = convert_tick(t.start + t.duration, t.ppqn, ppqn, allow_rounding)?;
                (start, end - start, t.start_order, t.end_order)
            } else {
                let start = native_tick(note.source_start(), ppqn, note.start, allow_rounding)?;
                let end_beat = note.source_start() + note.source_duration();
                let end = native_tick(end_beat, ppqn, end_beat as f32, allow_rounding)?;
                let orders = (next, next.checked_add(1).ok_or("MIDI order overflow")?);
                next = next.checked_add(2).ok_or("MIDI order overflow")?;
                (start, end - start, orders.0, orders.1)
            };
        result.end_tick = result.end_tick.max(start_tick + duration_ticks);
        result.notes.push(smf::Note {
            channel: note.channel,
            pitch: note.pitch,
            velocity: note.vel,
            release_velocity: note.release_vel,
            start_tick,
            duration_ticks,
            start_order,
            end_order,
        });
    }
    Ok(result)
}

#[derive(Clone, Copy)]
pub(crate) struct ExportOptions {
    pub ppqn: u16,
    pub single_track: bool,
    pub include_muted: bool,
    pub allow_rounding: bool,
    pub session_conductor: bool,
}
pub(crate) fn export(
    state: &project::State,
    cells: &[(u8, u16)],
    options: ExportOptions,
) -> Result<smf::File, String> {
    export_with_cancel(state, cells, options, || false)
}
pub(crate) fn export_with_cancel(
    state: &project::State,
    cells: &[(u8, u16)],
    options: ExportOptions,
    mut cancel: impl FnMut() -> bool,
) -> Result<smf::File, String> {
    checkpoint(&mut cancel)?;
    if options.ppqn == 0 || options.ppqn > 32767 {
        return Err("Export PPQN must be 1–32767".into());
    }
    if cells.is_empty() || cells.len() > TRACKS * SCENES {
        return Err("Select 1–64 MIDI clips to export".into());
    }
    let mut seen = BTreeSet::new();
    let mut tracks = Vec::new();
    for &(track, scene) in cells {
        checkpoint(&mut cancel)?;
        if !seen.insert((track, scene)) {
            return Err("Duplicate export clip".into());
        }
        let clip = state
            .tracks
            .get(track as usize)
            .and_then(|t| t.clips.get(scene as usize))
            .ok_or("Unknown export clip")?;
        if state.session.as_ref().is_some_and(|layout| layout.reference(session::Axis::Track,track as usize).is_none() || layout.reference(session::Axis::Scene,scene as usize).is_none()) { return Err("Export target was deleted; select active clips".into()); }
        if clip.kind != ClipKind::Midi {
            return Err("Choose a MIDI clip for every export row".into());
        }
        let mut data = export_clip(
            clip,
            options.ppqn,
            options.include_muted,
            options.allow_rounding,
            &mut cancel,
        )?;
        if options.session_conductor {
            data.meta
                .retain(|m| !matches!(m.value, MetaValue::Tempo(_) | MetaValue::Meter { .. }));
        }
        tracks.push(data);
    }
    if options.session_conductor {
        let end_tick = tracks.iter().map(|t| t.end_tick).max().unwrap();
        let meta = if let Some(conductor) = &state.conductor {
            export_conductor(conductor, options.ppqn, end_tick, &mut cancel)?
                .into_iter()
                .map(|mut m| {
                    if !matches!(m.value, MetaValue::Tempo(_)) || !conductor.tempos.iter().any(|p| p.ramp) {
                        m.tick = convert_tick(m.tick, conductor.ppqn, options.ppqn, options.allow_rounding)?;
                    }
                    Ok(m)
                })
                .collect::<Result<Vec<_>, String>>()?
        } else {
            vec![
                smf::Meta {
                    tick: 0,
                    order: 0,
                    value: MetaValue::Tempo((60000000.0 / f64::from(state.bpm)).round() as u32),
                },
                smf::Meta {
                    tick: 0,
                    order: 1,
                    value: MetaValue::Meter {
                        numerator: 4,
                        denominator_power: 2,
                        clocks: 24,
                        thirty_seconds: 8,
                    },
                },
            ]
        };
        let end_tick = end_tick.max(meta.iter().map(|m| m.tick).max().unwrap_or(0));
        tracks.insert(
            0,
            smf::Track {
                end_tick,
                meta,
                ..Default::default()
            },
        );
    }
    if options.single_track {
        let mut merged = smf::Track::default();
        let mut offset = 0u32;
        for track in tracks {
            checkpoint(&mut cancel)?;
            let next = max_order(&track)
                .checked_add(1)
                .and_then(|v| offset.checked_add(v))
                .ok_or("MIDI ordering overflow")?;
            merged.end_tick = merged.end_tick.max(track.end_tick);
            merged.notes.extend(track.notes.into_iter().map(|mut n| {
                n.start_order += offset;
                n.end_order += offset;
                n
            }));
            merged
                .messages
                .extend(track.messages.into_iter().map(|mut m| {
                    m.order += offset;
                    m
                }));
            merged.meta.extend(track.meta.into_iter().map(|mut m| {
                m.order += offset;
                m
            }));
            offset = next;
        }
        tracks = vec![merged];
    }
    let file = smf::File {
        format: if options.single_track {
            smf::Format::Single
        } else {
            smf::Format::Parallel
        },
        ppqn: options.ppqn,
        tracks,
        warnings: vec![],
    };
    smf::encode_with_cancel(&file, false, &mut cancel).map_err(|e| e.to_string())?;
    Ok(file)
}

/// Encode exact constant tempos and tick-averaged ramp segments.
/// `map`, `ppqn` and `end_tick` select the conductor and exported time range;
/// `cancel` stops worker preparation. Returns bounded MIDI metadata or an error.
fn export_conductor(map: &Conductor, ppqn: u16, end_tick: u64, cancel: &mut impl FnMut() -> bool) -> Result<Vec<smf::Meta>, String> {
    if !map.tempos.iter().any(|point| point.ramp) { return Ok(map.meta()); }
    let ratio = f64::from(ppqn) / f64::from(map.ppqn);
    let samples: u64 = map.tempos.windows(2).filter(|pair| pair[0].ramp).map(|pair| {
        let start = (pair[0].tick as f64 * ratio).ceil() as u64;
        let end = (pair[1].tick as f64 * ratio).ceil() as u64;
        end.min(end_tick.saturating_add(1)).saturating_sub(start)
    }).sum();
    if samples > (smf::MAX_EVENTS / 2) as u64 { return Err("MIDI tempo ramp exceeds 131072 sampled ticks; reduce export PPQN or shorten the exported clips".into()); }
    let mut result: Vec<_> = map.meta().into_iter().filter(|m| matches!(m.value, MetaValue::Meter { .. })).collect();
    let mut previous = None;
    let mut tick = 0;
    while tick <= end_tick {
        checkpoint(cancel)?;
        let beat = tick as f64 / f64::from(ppqn);
        let index = map.tempos.partition_point(|point| point.tick as f64 <= beat * f64::from(map.ppqn)).saturating_sub(1);
        let point = &map.tempos[index];
        let micros = if point.ramp {
            ((map.seconds_at((tick + 1) as f64 / f64::from(ppqn)) - map.seconds_at(beat)) * f64::from(ppqn) * 1_000_000.0).round() as u32
        } else { point.micros };
        if previous != Some(micros) {
            if result.len() >= smf::MAX_EVENTS / 2 { return Err("MIDI tempo ramp exceeds 131072 events; reduce export PPQN or shorten the exported clips".into()); }
            result.push(smf::Meta { tick, order: result.len() as u32, value: MetaValue::Tempo(micros) });
            previous = Some(micros);
        }
        tick = if point.ramp { tick + 1 } else {
            map.tempos.get(index + 1).map_or(end_tick.saturating_add(1), |next| ((next.tick as f64 * f64::from(ppqn) / f64::from(map.ppqn)).ceil() as u64).max(tick + 1))
        };
    }
    for (order, event) in result.iter_mut().enumerate() { event.order = order as u32; }
    Ok(result)
}

#[cfg(test)]
mod tests;
