use super::*;
use crate::engine::{arrangement, song_navigation};

pub(super) fn convert(
    set: &Element,
    mut source: Source,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    engine::midi_edit::initialize()?;
    let tracks = set.one("Tracks")?;
    if tracks.children.is_empty()
        || tracks.children.len() > session::MAX_TRACKS
        || tracks.children.iter().any(|t| {
            !["MidiTrack", "AudioTrack", "GroupTrack", "ReturnTrack"].contains(&t.name.as_str())
        })
    {
        return Err("Live Set needs 1–128 recognized tracks".into());
    }
    let scenes = set.one("Scenes")?;
    let scene_count = scenes.children.len().max(1);
    if scene_count > session::MAX_SCENES || scenes.children.iter().any(|s| s.name != "Scene") {
        return Err("Live Set exceeds 512 scenes or contains an unknown scene record".into());
    }
    let (mut state, mut media) = project::State::empty()?;
    let blank = state.tracks[0].clone();
    state.tracks.resize(tracks.children.len(), blank.clone());
    state.tracks.truncate(tracks.children.len());
    state.scene_fx = vec![vec![]; scene_count];
    for (index, node) in tracks.children.iter().enumerate() {
        let name = value(node, &["Name", "UserName"])?;
        let name = if name.is_empty() {
            value(node, &["Name", "EffectiveName"])?
        } else {
            name
        };
        let track = &mut state.tracks[index];
        track.name = if name.is_empty() {
            format!("{} {}", node.name, index + 1)
        } else {
            name.into()
        };
        track.clips = vec![blank.clips[0].clone(); scene_count];
        track.gain = number(node, &["DeviceChain", "Mixer", "Volume", "Manual"], 1.)? as f32;
        track.pan = number(node, &["DeviceChain", "Mixer", "Pan", "Manual"], 0.)? as f32;
        track.mute = !boolean(node, &["DeviceChain", "Mixer", "Speaker", "Manual"], true)?;
        track.kind = if node.name == "MidiTrack" { 1 } else { 0 };
        track.input_monitor = Some(engine::input_monitor::Mode::Off);
        if track.kind == 1 {
            track.synth.offline = Some(Arc::new(engine::fx::OfflineDevice::new(
                format!("ableton-track-{}", node.attr("Id")),
                None,
            )?));
        }
    }
    let mut layout =
        session::Layout::fresh(state.tracks.iter().map(|t| t.name.clone()), scene_count);
    let mut scene_ids = BTreeSet::new();
    for (index, node) in scenes.children.iter().enumerate() {
        let id = integer(node.attr("Id"))?;
        if id < 0 || !scene_ids.insert(id) {
            return Err("Live Set repeats or has an invalid scene identity".into());
        }
        let name = value(node, &["Name"])?;
        if !name.is_empty() {
            layout.scenes[index].name = name.into();
        }
        color(
            node,
            &mut layout.scenes[index].color,
            &mut source,
            &format!("scene:{id}"),
        )?;
        if boolean(node, &["IsTempoEnabled"], false)?
            || boolean(node, &["IsTimeSignatureEnabled"], false)?
        {
            source.difference(format!("scene:{id}"),"Scene timing","Live scene tempo/meter changes are retained for review; the imported conductor owns transport timing")?;
        }
        if boolean(node, &["FollowAction", "FollowActionEnabled"], false)? {
            source.difference(format!("scene:{id}"),"Follow actions","Source follow-action choices and chance are retained; automatic follow playback is not matched")?;
        }
    }
    timing::convert(set, &mut state, &mut source)?;
    let snapshot = crate::media_location::Snapshot::discover().map_err(|e| e.to_string())?;
    let mut pcm = 0;
    let mut song = arrangement::Model::default();
    song.enabled = true;
    let mut ids = BTreeSet::new();
    for (index, node) in tracks.children.iter().enumerate() {
        active(cancel)?;
        let id = integer(node.attr("Id"))?;
        if id < 0 || !ids.insert(id) {
            return Err("Live Set repeats or has an invalid track identity".into());
        }
        let parent = number(node, &["TrackGroupId"], -1.)?;
        if parent.fract() != 0. {
            return Err("Invalid group track reference".into());
        }
        let reference = layout
            .reference(session::Axis::Track, index)
            .ok_or("Cannot bind imported track")?;
        source.tracks.push(Track {
            source_id: id,
            native: reference,
            role: node.name.clone(),
            parent: parent as i64,
            color_index: number(node, &["Color"], -1.)? as i32,
        });
        color(
            node,
            &mut layout.tracks[index].color,
            &mut source,
            &format!("track:{id}"),
        )?;
        dependencies::inventory(node, id, &mut source, cancel)?;
        if let Some(chain) = child(node, "DeviceChain")? {
            for kind in ["AudioInputRouting", "MidiInputRouting", "MidiOutputRouting"] {
                if let Some(route) = child(chain, kind)? {
                    let target = value(route, &["Target"])?;
                    if !target.is_empty() && !target.ends_with("/None") {
                        source.difference(format!("track:{id}"),"External routing",format!("{kind}: {target}; external/source endpoints require explicit native routing"))?;
                    }
                }
            }
        }
        if let Some(slots) = at(node, &["DeviceChain", "MainSequencer", "ClipSlotList"])? {
            let mut occupied = BTreeSet::new();
            for slot in &slots.children {
                if slot.name != "ClipSlot" {
                    return Err("Unknown session clip slot record".into());
                }
                let scene_id = integer(slot.attr("Id"))?;
                let scene = scenes
                    .children
                    .iter()
                    .position(|s| integer(s.attr("Id")).ok() == Some(scene_id))
                    .ok_or("Session clip slot references an absent scene")?;
                if !occupied.insert(scene) {
                    return Err("Repeated session clip slot".into());
                }
                if let Some(container) = at(slot, &["ClipSlot", "Value"])? {
                    if container.children.len() > 1 {
                        return Err("Session slot contains multiple clips".into());
                    }
                    if let Some(clip) = container.children.first() {
                        state.tracks[index].clips[scene] = clips::convert(
                            clip,
                            id,
                            state.bpm,
                            &mut source,
                            options,
                            &snapshot,
                            &mut media,
                            &mut pcm,
                            cancel,
                        )?;
                    }
                }
            }
        }
        if let Some(events) = at(
            node,
            &[
                "DeviceChain",
                "MainSequencer",
                "ClipTimeable",
                "ArrangerAutomation",
                "Events",
            ],
        )? {
            for clip in &events.children {
                if song.sources.len() >= arrangement::MAX_SOURCES
                    || song.instances.len() >= arrangement::MAX_INSTANCES
                {
                    return Err(
                        "Live arrangement exceeds 256 clip sources or 4096 placements".into(),
                    );
                }
                let saved = clips::convert(
                    clip,
                    id,
                    state.bpm,
                    &mut source,
                    options,
                    &snapshot,
                    &mut media,
                    &mut pcm,
                    cancel,
                )?;
                let start = number(clip, &["CurrentStart"], number(clip, &["Time"], 0.)?)?;
                let end = number(clip, &["CurrentEnd"], start + f64::from(saved.bars) * 4.)?;
                let offset = number(clip, &["Loop", "StartRelative"], 0.)?.max(0.);
                let source_id = song.identity()?;
                let instance_id = song.identity()?;
                song.sources.push(arrangement::Source {
                    audio_clock: None,
                    id: source_id,
                    clip: saved,
                });
                song.instances.push(arrangement::Instance {
                    id: instance_id,
                    source: source_id,
                    track: reference,
                    start,
                    offset,
                    duration: end - start,
                    repeating: boolean(clip, &["Loop", "LoopOn"], true)?,
                    gain: 1.,
                    fades: None,
                    fade_link: 0,
                    crossfade: None,
                });
            }
        }
        if let Some(envelopes) = at(node, &["AutomationEnvelopes", "Envelopes"])? {
            if !envelopes.children.is_empty() {
                source.difference(format!("track:{id}"),"Automation","Source envelopes and stable target IDs retained; unresolved native targets require review")?;
            }
        }
    }
    for track in &source.tracks {
        if track.parent != -1 && !ids.contains(&track.parent) {
            return Err("Live track group references an absent track".into());
        }
    }
    for track in &source.tracks {
        let mut parent = track.parent;
        let mut ancestry = BTreeSet::from([track.source_id]);
        while parent != -1 {
            if !ancestry.insert(parent) {
                return Err("Live track groups contain a cycle".into());
            }
            parent = source
                .tracks
                .iter()
                .find(|t| t.source_id == parent)
                .ok_or("Missing group ancestor")?
                .parent;
        }
    }
    let metadata = source.tracks.clone();
    state.routing = Some(Arc::new(routing::convert(
        set,
        tracks,
        &metadata,
        &layout,
        &mut state.tracks,
        &mut source,
    )?));
    state.session = Some(layout);
    if !song.sources.is_empty() {
        state.arrangement = Some(Arc::new(song));
    }
    navigation(set, &mut state)?;
    source.difference("project","Device fidelity","Ableton stock engines, racks, MIDI effects, AU and unsupported plugins remain retained dependencies. MIDI tracks are silent until an instrument or an aligned render is explicitly resolved")?;
    state.migration = Some(Arc::new(Migration {
        schema: 1,
        sources: vec![source],
    }));
    state.validate(&media)?;
    crate::project_file::validate_metadata(
        &crate::project_file::Bundle {
            state: &state,
            media: media.clone(),
        },
        &Default::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    Ok(Imported {
        state,
        media,
        reviewed_native: None,
    })
}
fn color(
    node: &Element,
    target: &mut Option<[u8; 3]>,
    source: &mut Source,
    item: &str,
) -> Result<(), String> {
    let rgb = value(node, &["ColorRgb"])?;
    if !rgb.is_empty() {
        let n: u32 = rgb.parse().map_err(|_| "Invalid source RGB color")?;
        if n > 0xffffff {
            return Err("Invalid source RGB color".into());
        }
        *target = Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]);
    } else {
        let index = number(node, &["Color"], number(node, &["ColorIndex"], -1.)?)?;
        if index >= 0. {
            source.difference(item,"Color",format!("Live palette index {index} retained in source; RGB palette conversion is not yet qualified"))?;
        }
    }
    Ok(())
}
fn navigation(set: &Element, state: &mut project::State) -> Result<(), String> {
    let mut model = song_navigation::Model::default();
    if let Some(locators) = at(set, &["Locators", "Locators"])? {
        for locator in locators.named("Locator") {
            let name = value(locator, &["Name"])?;
            model.add(
                if name.is_empty() {
                    format!("Locator {}", locator.attr("Id"))
                } else {
                    name.into()
                },
                number(locator, &["Time"], 0.)?,
            )?;
        }
    }
    let looping = boolean(set, &["Transport", "LoopOn"], false)?;
    if let Some(transport) = child(set, "Transport")? {
        let start = number(transport, &["LoopStart"], 0.)?;
        let length = number(transport, &["LoopLength"], 0.)?;
        if length > 0. {
            model.loop_region = Some(song_navigation::metadata::Loop {
                start,
                end: start + length,
            });
        }
    }
    if !model.locators.is_empty() || model.loop_region.is_some() {
        let next_id = model.next_id;
        state.navigation = Some(song_navigation::Saved {
            model: Arc::new(model),
            looping,
            next_id,
        });
    }
    Ok(())
}
