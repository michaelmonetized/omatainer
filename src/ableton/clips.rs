use super::*;

pub(super) fn convert(
    node: &Element,
    track: i64,
    bpm: f32,
    source: &mut Source,
    options: &Options,
    snapshot: &crate::media_location::Snapshot,
    media: &mut Vec<Arc<Sample>>,
    pcm: &mut u64,
    cancel: &AtomicBool,
) -> Result<project::SavedClip, String> {
    active(cancel)?;
    if !["MidiClip", "AudioClip"].contains(&node.name.as_str()) {
        return Err(format!("Unsupported clip record {}", node.name));
    }
    let identity = node.attr("Id");
    if identity.is_empty() {
        return Err("Live clip has no source identity".into());
    }
    let item = format!("track:{track}/{}:{identity}", node.name);
    let mut clip = project::SavedClip::empty();
    clip.kind = if node.name == "MidiClip" {
        engine::ClipKind::Midi
    } else {
        engine::ClipKind::Audio
    };
    clip.name = value(node, &["Name"])?.into();
    clip.properties.disabled = boolean(node, &["Disabled"], false)?;
    let start = number(node, &["Loop", "LoopStart"], 0.)?;
    let end = number(node, &["Loop", "LoopEnd"], 4.)?;
    let looping = boolean(node, &["Loop", "LoopOn"], true)?;
    if start < 0. || end <= start || end > 262144. {
        return Err("Live clip has an invalid loop region".into());
    }
    clip.bars = (end / 4.) as f32;
    clip.gain = number(node, &["SampleVolume"], 1.)? as f32;
    clip.properties.launch.mode = match number(node, &["LaunchMode"], 0.)? {
        0. => engine::clip_launch::Mode::Trigger,
        1. => engine::clip_launch::Mode::Gate,
        2. => engine::clip_launch::Mode::Toggle,
        3. => engine::clip_launch::Mode::Repeat,
        _ => return Err("Unknown Live clip launch mode".into()),
    };
    clip.properties.launch.legato = boolean(node, &["Legato"], false)?;
    let quant = number(node, &["LaunchQuantisation"], 0.)?;
    if quant != 0. {
        source.difference(&item,"Launch quantization",format!("Live launch grid ID {quant} retained; current native clip uses global quantization"))?;
    }
    if number(node, &["Color"], number(node, &["ColorIndex"], -1.)?)? >= 0. {
        source.difference(
            &item,
            "Color",
            "Live clip palette index retained; RGB conversion needs palette qualification",
        )?;
    }
    if boolean(node, &["FollowAction", "FollowActionEnabled"], false)? {
        source.difference(
            &item,
            "Follow actions",
            "Source follow-action timing, choices and chances retained; not automatically played",
        )?;
    }
    if clip.kind == engine::ClipKind::Midi {
        let mut keys = Vec::new();
        if let Some(notes) = child(node, "Notes")? {
            walk(notes, "KeyTrack", &mut keys);
        }
        let mut note_ids = BTreeSet::new();
        let mut differences = BTreeSet::new();
        for key in keys {
            let pitch = number(key, &["MidiKey"], -1.)?;
            if !(0.0..=127.).contains(&pitch) || pitch.fract() != 0. {
                return Err("Invalid Live MIDI key".into());
            }
            let mut notes = Vec::new();
            walk(key, "MidiNoteEvent", &mut notes);
            for note in notes {
                active(cancel)?;
                if clip.notes.len() >= project::MAX_NOTES_PER_CLIP {
                    return Err("Live clip exceeds 8192 MIDI notes".into());
                }
                let id = note.attr("NoteId");
                if !id.is_empty() && !note_ids.insert(id) {
                    return Err("Live MIDI clip repeats a note identity".into());
                }
                let coordinate = |key: &str, default: f64| -> Result<f64, String> {
                    let raw = note.attr(key);
                    let value = if raw.is_empty() {
                        default
                    } else {
                        raw.parse().map_err(|_| "Invalid Live MIDI note value")?
                    };
                    if !value.is_finite() {
                        return Err("Nonfinite Live MIDI note".into());
                    }
                    Ok(value)
                };
                let time = coordinate("Time", 0.)?;
                let duration = coordinate("Duration", 0.)?;
                let velocity = coordinate("Velocity", 100.)?;
                let release = coordinate("OffVelocity", 64.)?;
                if time < 0.
                    || duration <= 0.
                    || time + duration > 262144.
                    || !(0.0..=127.).contains(&velocity)
                    || !(0.0..=127.).contains(&release)
                {
                    return Err("Live MIDI note exceeds native timing or velocity bounds".into());
                }
                let enabled = match note.attr("IsEnabled") {
                    "" | "true" => true,
                    "false" => false,
                    _ => return Err("Invalid MIDI note enable flag".into()),
                };
                let timing = engine::midi_data::TickTiming {
                    ppqn: 32767,
                    start: (time * 32767.).round() as u64,
                    duration: (duration * 32767.).round() as u64,
                    start_order: 1,
                    end_order: 2,
                };
                let exact = timing.valid()
                    && (timing.start_beats() - time).abs() < 1e-10
                    && (timing.duration_beats() - duration).abs() < 1e-10;
                if !exact
                    && ((time as f32 as f64 - time).abs() > 1e-10
                        || (duration as f32 as f64 - duration).abs() > 1e-10)
                {
                    differences.insert(
                        "Source fractional timing retained; native positions use float precision",
                    );
                }
                if velocity.fract() != 0. || release.fract() != 0. {
                    differences.insert("Fractional source velocities retained; native channel MIDI rounds to 7-bit values");
                }
                if coordinate("Probability", 1.)? != 1.
                    || coordinate("VelocityDeviation", 0.)? != 0.
                {
                    differences.insert("Note probability and velocity deviation retained; deterministic native note playback does not apply them");
                }
                clip.notes.push(engine::MidiNote { variation: None,
                    id: engine::midi_edit::NoteId::new(),
                    pitch: pitch as u8,
                    start: time as f32,
                    len: duration as f32,
                    vel: velocity.round() as u8,
                    channel: 0,
                    release_vel: release.round() as u8,
                    source_timing: exact.then_some(timing),
                    muted: !enabled,
                });
            }
        }
        for detail in differences {
            source.difference(&item, "MIDI notes", detail)?;
        }
        let offset = number(node, &["Loop", "StartRelative"], 0.)?;
        let initial = start + offset;
        if initial > start {
            source.difference(
                &item,
                "Clip offset",
                "StartRelative offset retained; native Session region starts at the loop start",
            )?;
        }
        clip.region = Some(engine::midi_edit::Region {
            start: start.min(initial).max(0.),
            end,
            loop_start: start,
            loop_end: end,
            loop_enabled: looping,
        });
        for name in [
            "NoteProbability",
            "PerNoteEventStore",
            "ExpressionLists",
            "NoteExpressionLists",
        ] {
            let mut nodes = Vec::new();
            walk(node, name, &mut nodes);
            if nodes.iter().any(|n| !n.children.is_empty()) {
                source.difference(
                    &item,
                    "Expressive MIDI",
                    format!(
                        "{name} retained; typed per-note expression is not mapped to channel MIDI"
                    ),
                )?;
            }
        }
    } else {
        let file = at(node, &["SampleRef", "FileRef"])?.ok_or("Audio clip has no FileRef")?;
        clip.audio = dependencies::asset(
            file,
            item.clone(),
            source,
            options,
            snapshot,
            media,
            pcm,
            cancel,
        )?;
        let warped = boolean(node, &["IsWarped"], false)?;
        if !warped {
            clip.bars = ((end - start) * f64::from(bpm) / 240.) as f32;
        }
        if clip.audio.is_none() {
            clip.properties.disabled = true;
        }
        if warped {
            source.difference(&item,"Warping",format!("Live warp mode {} and markers retained; source audio is embedded without claiming identical warping. Clip disabled until explicit render/relink",value(node,&["WarpMode"])?))?;
            clip.properties.disabled = true;
        } else if let Some(index) = clip.audio {
            let sample = &media[index];
            let mut region =
                engine::audio_clip::Region::full(sample, bpm).map_err(str::to_owned)?;
            let first = number(node, &["Loop", "StartRelative"], 0.)? + start;
            let finish = end;
            let start_frame = (first * f64::from(sample.sr)).round();
            let end_frame = (finish * f64::from(sample.sr)).round();
            if start_frame >= 0. && start_frame < end_frame && end_frame <= sample.frames() as f64 {
                region.start = start_frame as u64;
                region.end = end_frame as u64;
                region.loop_start = region.start;
                region.loop_end = region.end;
                region.loop_enabled = looping;
            } else {
                source.difference(&item,"Audio bounds","Live audio coordinates do not fit the source; full source retained and clip disabled pending render/relink")?;
                clip.properties.disabled = true;
            }
            region.transpose = number(node, &["PitchCoarse"], 0.)? as f32
                + number(node, &["PitchFine"], 0.)? as f32 / 100.;
            region.reverse = boolean(node, &["Reverse"], false)?;
            let plan = region.prepare(sample).map_err(str::to_owned)?;
            clip.bars = (plan.duration_beats / 4.) as f32;
            clip.audio_region = Some(region);
            source.difference(&item,"Audio tempo","Unwarped audio seconds map exactly at the initial source tempo; later native tempo changes resample this region. Preserve source playback with an aligned render if the source changes tempo")?;
        }
    }
    if let Some(envelopes) = at(node, &["Envelopes", "Envelopes"])? {
        if !envelopes.children.is_empty() {
            source.difference(&item,"Clip envelopes","Source envelopes and target IDs retained; unresolved modulation and automation require target review")?;
        }
    }
    clip.validate(project::STATE_VERSION, media)?;
    Ok(clip)
}
