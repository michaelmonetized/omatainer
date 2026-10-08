use super::*;
use crate::{
    engine::audio::routing::{
        model::{ChannelMap, Connection, Group, Source as TapSource, Tap},
        plugins::{Automation, Instance, Parameter},
    },
    plugin_host::{self, Class, Request, Response, Saved},
};
use std::time::Duration;

fn element<'a>(root: &'a Element, device: &Device) -> Result<&'a Element, String> {
    let tracks = root.one("LiveSet")?.one("Tracks")?;
    let track = tracks
        .children
        .iter()
        .find(|t| integer(t.attr("Id")).ok() == Some(device.track))
        .ok_or("Original device track is absent")?;
    let mut node =
        at(track, &["DeviceChain", "DeviceChain"])?.ok_or("Original device chain is absent")?;
    for segment in device.path.split('/').skip(1) {
        let (name, id) = segment
            .rsplit_once('[')
            .ok_or("Invalid retained device path")?;
        let id = id.strip_suffix(']').ok_or("Invalid retained device path")?;
        node = node
            .children
            .iter()
            .enumerate()
            .find(|(index, n)| {
                n.name == name
                    && if n.attr("Id").is_empty() {
                        index.to_string() == id
                    } else {
                        n.attr("Id") == id
                    }
            })
            .map(|(_, n)| n)
            .ok_or("Original device path cannot be resolved")?;
    }
    Ok(node)
}
fn hex(node: &Element) -> Result<Vec<u8>, String> {
    if !node.children.is_empty() {
        return Err("Plugin state must contain a single hexadecimal stream".into());
    }
    let mut result = Vec::new();
    let mut high = None;
    for byte in node.text.bytes().filter(|b| !b.is_ascii_whitespace()) {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err("Plugin state contains invalid hexadecimal bytes".into()),
        };
        if let Some(previous) = high.take() {
            if result.len() >= plugin_host::MAX_STATE {
                return Err("Imported plugin state exceeds 8 MiB".into());
            }
            result.push(previous * 16 + nibble);
        } else {
            high = Some(nibble);
        }
    }
    if high.is_some() {
        return Err("Imported plugin state has a truncated hexadecimal byte".into());
    }
    Ok(result)
}
fn state(node: &Element) -> Result<Vec<u8>, String> {
    let mut presets = Vec::new();
    walk(node, "Vst3Preset", &mut presets);
    if presets.len() != 1 {
        return Err("VST3 device needs one unambiguous original preset state".into());
    }
    let component = hex(presets[0].one("ProcessorState")?)?;
    if component.is_empty() {
        return Err("Original processor state is empty; choose an explicit replacement or render instead of assuming default sound".into());
    }
    let controller = child(presets[0], "ControllerState")?.map(hex).transpose()?;
    if component
        .len()
        .saturating_add(controller.as_ref().map_or(0, Vec::len))
        .saturating_add(28)
        > plugin_host::MAX_STATE
    {
        return Err("Combined component/controller state exceeds the 8 MiB project limit".into());
    }
    vst3_host::plugin::encode_project_state(component, controller).map_err(|e| e.to_string())
}
fn map(source: usize, destination: usize) -> Vec<ChannelMap> {
    if destination == 1 && source >= 2 {
        vec![
            ChannelMap {
                source: 0,
                destination: 0,
                gain: 0.5,
            },
            ChannelMap {
                source: 1,
                destination: 0,
                gain: 0.5,
            },
        ]
    } else {
        (0..destination.min(2))
            .map(|n| ChannelMap {
                source: if source == 1 { 0 } else { n as u8 },
                destination: n as u8,
                gain: 1.,
            })
            .collect()
    }
}
fn link(from: Group, to: Group, width: usize, channels: usize) -> Connection {
    Connection {
        source: TapSource {
            group: from,
            tap: Tap::PostMixer,
        },
        destination: to,
        map: map(width, channels),
    }
}

/// Relink one retained VST3 device without touching the current session.
/// Takes a native draft, source/device indexes, scanned class identity and explicit version review; returns the complete updated draft after isolated state restoration and graph validation.
pub(crate) fn relink(
    mut draft: Imported,
    source_index: usize,
    device_index: usize,
    binary: plugin_host::BinaryIdentity,
    class: Class,
    reviewed_version: bool,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    active(cancel)?;
    class.validate()?;
    let migration = draft
        .state
        .migration
        .as_ref()
        .ok_or("No imported device provenance")?;
    let source = migration
        .sources
        .get(source_index)
        .ok_or("Imported source changed")?;
    let device = source
        .devices
        .get(device_index)
        .ok_or("Imported device changed")?
        .clone();
    if device.format.as_deref() != Some("VST3")
        || device
            .class_id
            .as_ref()
            .is_none_or(|id| !id.eq_ignore_ascii_case(&class.info.uid))
    {
        return Err("Relink requires the original stable VST3 class identity; AU/VST2 and name-only matches are unsupported".into());
    }
    if !reviewed_version && device.version.as_deref() != Some(class.info.version.as_str()) {
        return Err("Source plugin version is absent or different. Review installed-version and cross-platform state compatibility before trying this relink".into());
    }
    let root = presets::normalize(
        interchange_xml::parse_text(source.xml.as_bytes(), &|| !cancel.load(Ordering::Acquire))?)?;
    let node = element(&root, &device)?;
    if device.path.split('/').count() != 3 {
        return Err("Nested rack/branch processors retain their state; resolve the rack through an explicit compatible routing replacement or aligned render".into());
    }
    let binding = source
        .tracks
        .iter()
        .find(|t| t.source_id == device.track)
        .ok_or("Source track was not selected for this import")?;
    let layout = draft
        .state
        .session
        .as_ref()
        .ok_or("Native session identities unavailable")?;
    let slot = layout
        .tracks
        .iter()
        .position(|t| t.id == binding.native.id && layout.namespace == binding.native.namespace)
        .ok_or("Imported track is no longer present")?;
    let track_id = binding.native.id;
    let instrument = class.info.category.contains("Instrument");
    if instrument && (!class.info.has_midi_input || binding.role != "MidiTrack")
        || !instrument && class.layout.inputs.is_empty()
    {
        return Err(
            "Scanned processor is incompatible with this instrument/audio-effect role".into(),
        );
    }
    let graph = draft
        .state
        .routing
        .as_ref()
        .ok_or("Imported routing is absent")?;
    if device.resolution.is_none()
        && graph.plugins.len() >= engine::audio::routing::plugins::MAX_PLUGINS
    {
        return Err("Native graph already has 32 processors".into());
    }
    let saved = Saved {
        schema: 1,
        binary,
        class_id: class.info.uid.clone(),
        plugin_version: class.info.version.clone(),
        state_codec: "vst3-host-0.9-state".into(),
        state: state(node)?,
    };
    let executable = {
        #[cfg(test)]
        {
            std::env::var_os("OMATAINER_TEST_BIN")
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(std::env::current_exe)
        }
        #[cfg(not(test))]
        {
            std::env::current_exe()
        }
    }
    .map_err(|e| e.to_string())?;
    let mut process = plugin_host::process::Process::start(&executable)?;
    let loaded = match process.exchange(
        &Request::Load {
            saved: saved.clone(),
            rate: 48000,
        },
        cancel,
        Duration::from_secs(10),
    )? {
        Response::Loaded { class } => class,
        _ => return Err("State restoration worker returned an unexpected response".into()),
    };
    loaded.validate()?;
    let (checkpoint, observed) =
        match process.exchange(&Request::State, cancel, Duration::from_secs(10))? {
            Response::State { saved, parameters } => (saved, parameters),
            _ => return Err("State restoration worker did not return a checkpoint".into()),
        };
    drop(process);
    active(cancel)?;
    checkpoint.validate()?;
    if checkpoint.binary != saved.binary
        || checkpoint.class_id != saved.class_id
        || checkpoint.plugin_version != saved.plugin_version
    {
        return Err("State checkpoint changed the qualified plugin identity".into());
    }
    let (parameters, automation) = parameters(&root, node, &device, &loaded)?;
    let mut graph = (**graph).clone();
    let id = if let Some(resolution) = &device.resolution {
        resolution.native_id
    } else {
        let id = graph.next_id;
        graph.next_id = id.checked_add(1).ok_or("Graph identities exhausted")?;
        id
    };
    let processor = Instance {
        id,
        name: format!("Imported VST3 {id}"),
        saved: checkpoint.clone(),
        inputs: loaded
            .layout
            .inputs
            .iter()
            .map(|b| b.channel_count as u8)
            .collect(),
        outputs: loaded
            .layout
            .outputs
            .iter()
            .map(|b| b.channel_count as u8)
            .collect(),
        midi_track: Some(track_id),
        scene_track: None,
        instrument,
        bypass: !device.enabled,
        latency: loaded.latency.saturating_add(
            plugin_host::BLOCK as u32 * plugin_host::realtime::BRIDGE_BLOCKS as u32,
        ),
        parameters: if parameters.is_empty() {
            observed
                .into_iter()
                .map(|(id, value)| Parameter { id, value })
                .collect()
        } else {
            parameters
        },
        automation,
        unavailable: None,
    };
    let resolution=Resolution{native_id:id,binary_sha256:checkpoint.binary.sha256.clone(),installed_version:checkpoint.plugin_version.clone(),state_sha256:digest(&checkpoint.state),fidelity:if instrument{"Original VST3 class and opaque state restored; compare source playback before claiming identical sound"}else{"Original VST3 class and opaque state restored; audio-track effects follow the native mixer. Source pre-mixer FX and sends require render comparison"}.into()};
    let source = source.clone();
    let mut devices = source.devices.clone();
    devices[device_index].resolution = Some(resolution.clone());
    let ids: BTreeSet<_> = devices
        .iter()
        .filter(|d| d.track == device.track)
        .filter_map(|d| d.resolution.as_ref().map(|r| r.native_id))
        .collect();
    let mut outgoing: Vec<_> = graph
        .connections
        .iter()
        .filter(|r| {
            (r.source.group == Group::Track(track_id) && r.source.tap == Tap::PostMixer
                || matches!(r.source.group,Group::Plugin(p) if ids.contains(&p)))
                && !matches!(r.destination,Group::Plugin(p) if ids.contains(&p))
                && r.destination != Group::Track(track_id)
        })
        .cloned()
        .collect();
    graph.connections.retain(|r| {
        !matches!(r.source.group,Group::Plugin(p) if ids.contains(&p))
            && !matches!(r.destination,Group::Plugin(p) if ids.contains(&p))
            && !(r.source.group == Group::Track(track_id) && r.source.tap == Tap::PostMixer)
    });
    graph.plugins.retain(|p| p.id != id);
    graph.plugins.push(processor);
    let ordered: Vec<_> = devices
        .iter()
        .filter(|d| d.track == device.track)
        .filter_map(|d| {
            d.resolution
                .as_ref()
                .and_then(|r| graph.plugins.iter().find(|p| p.id == r.native_id).cloned())
        })
        .collect();
    let instruments: Vec<_> = ordered.iter().filter(|p| p.instrument).collect();
    if instruments.len() > 1 {
        return Err("A source track needs one unambiguous native instrument".into());
    }
    let has_instrument = !instruments.is_empty();
    if has_instrument && ordered.first().is_some_and(|p| !p.instrument) {
        return Err("Source audio processing before the instrument cannot be reordered automatically; use an explicit compatible replacement or aligned render".into());
    }
    let mut previous = instruments
        .first()
        .map(|p| (Group::Plugin(p.id), p.output_width()))
        .unwrap_or((Group::Track(track_id), 2));
    for effect in ordered.iter().filter(|p| !p.instrument) {
        graph.connections.push(link(
            previous.0,
            Group::Plugin(effect.id),
            previous.1,
            effect.input_width(),
        ));
        previous = (Group::Plugin(effect.id), effect.output_width());
    }
    if has_instrument {
        graph
            .connections
            .push(link(previous.0, Group::Track(track_id), previous.1, 2));
        draft.state.tracks[slot].synth.offline = None;
    }
    for route in &mut outgoing {
        route.source.group = if has_instrument {
            Group::Track(track_id)
        } else {
            previous.0
        };
    }
    graph.connections.extend(outgoing);
    graph.version = 3;
    graph.latency.get_or_insert_with(Default::default);
    graph.order(layout)?;
    let migration = Arc::make_mut(draft.state.migration.as_mut().unwrap());
    let source = &mut migration.sources[source_index];
    source.devices[device_index].resolution = Some(resolution.clone());
    source.difference(
        format!("track:{}/{}", device.track, device.path),
        "Resolved VST3",
        resolution.fidelity,
    )?;
    draft.state.routing = Some(Arc::new(graph));
    draft.state.validate(&draft.media)?;
    crate::project_file::validate_metadata(
        &crate::project_file::Bundle {
            state: &draft.state,
            media: draft.media.clone(),
        },
        &Default::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    Ok(draft)
}

fn parameters(
    root: &Element,
    node: &Element,
    device: &Device,
    class: &Class,
) -> Result<(Vec<Parameter>, Vec<Automation>), String> {
    let mut parameters = Vec::new();
    let mut targets = std::collections::BTreeMap::new();
    let mut records = Vec::new();
    walk(node, "PluginFloatParameter", &mut records);
    for record in records {
        let id = value(record, &["ParameterId"])?;
        if id.is_empty() {
            continue;
        }
        let id: u32 = id.parse().map_err(|_| "Invalid source VST3 parameter ID")?;
        let parameter = class
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or("Installed plugin is missing a source parameter identity")?;
        let manual = number(record, &["ParameterValue", "Manual"], parameter.value)?;
        if !parameter.is_read_only {
            parameters.push(Parameter { id, value: manual });
        }
        if let Some(target) = at(record, &["ParameterValue", "AutomationTarget"])? {
            if !parameter.can_automate || parameter.is_read_only {
                return Err(
                    "Source parameter target is not writable/automatable in the installed plugin"
                        .into(),
                );
            }
            if targets.insert(target.attr("Id"), id).is_some() {
                return Err("Duplicate source plugin automation target".into());
            }
        }
    }
    let track = root
        .one("LiveSet")?
        .one("Tracks")?
        .children
        .iter()
        .find(|t| integer(t.attr("Id")).ok() == Some(device.track))
        .ok_or("Source automation track disappeared")?;
    let mut automation = Vec::new();
    if let Some(envelopes) = at(track, &["AutomationEnvelopes", "Envelopes"])? {
        for envelope in &envelopes.children {
            let target = value(envelope, &["EnvelopeTarget", "PointeeId"])?;
            let Some(&id) = targets.get(target) else {
                continue;
            };
            let Some(events) = at(envelope, &["Automation", "Events"])? else {
                continue;
            };
            let mut points: Vec<[f64; 2]> = Vec::new();
            let mut previous = f64::NEG_INFINITY;
            for event in &events.children {
                if event.name != "FloatEvent"
                    || [
                        "CurveControl1X",
                        "CurveControl1Y",
                        "CurveControl2X",
                        "CurveControl2Y",
                    ]
                    .iter()
                    .any(|key| !event.attr(key).is_empty())
                {
                    return Err("Source plugin envelope requires a linear FloatEvent lane; curved/typed automation remains unresolved until render or explicit target replacement".into());
                }
                let coordinate = |name| -> Result<f64, String> {
                    let value: f64 = event
                        .attr(name)
                        .parse()
                        .map_err(|_| "Invalid source plugin automation coordinate")?;
                    if !value.is_finite() {
                        return Err("Nonfinite source plugin automation".into());
                    }
                    Ok(value)
                };
                let time = coordinate("Time")?;
                if time <= previous || time < -63072000. {
                    return Err("Source plugin automation is unordered or has an invalid pre-roll coordinate".into());
                }
                previous = time;
                let point = [time.max(0.), coordinate("Value")?];
                if points.last().is_some_and(|p| p[0] == point[0]) {
                    points.pop();
                }
                points.push(point);
                if points.len() > 16384 {
                    return Err("Source plugin automation exceeds native event storage".into());
                }
            }
            if !points.is_empty() {
                automation.push(Automation { id, points });
            }
        }
    }
    Ok((parameters, automation))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn state_hex_preserves_component_controller_and_refuses_truncation() {
        let xml=br#"<PluginDevice><Vst3Preset><ProcessorState>00 01 ff</ProcessorState><ControllerState>42 00</ControllerState></Vst3Preset></PluginDevice>"#;
        let root = interchange_xml::parse_text(xml, &|| true).unwrap();
        let encoded = state(&root).unwrap();
        assert_eq!(&encoded[..16], b"VST3HOST_STATE\0\0");
        assert_eq!(&encoded[20..24], 3u32.to_le_bytes());
        assert_eq!(&encoded[24..28], 2u32.to_le_bytes());
        assert_eq!(&encoded[28..], [0, 1, 255, 66, 0]);
        for text in ["<PluginDevice><Vst3Preset><ProcessorState>000</ProcessorState></Vst3Preset></PluginDevice>","<PluginDevice><Vst3Preset><ProcessorState>GG</ProcessorState></Vst3Preset></PluginDevice>","<PluginDevice><Vst3Preset><ProcessorState/></Vst3Preset></PluginDevice>"] {assert!(state(&interchange_xml::parse_text(text.as_bytes(),&||true).unwrap()).is_err());}
    }

    pub(crate) fn contract_set(instrument: &Class, effect: &Class) -> String {
        let component = |gain: f64, delay: f64| {
            [gain, delay, 0.]
                .into_iter()
                .flat_map(f64::to_le_bytes)
                .map(|b| format!("{b:02X}"))
                .collect::<String>()
        };
        let device = |id, class: &Class, gain, delay, param| {
            format!(
                r#"<PluginDevice Id="{id}"><On><Manual Value="true"/></On><ParameterList><PluginFloatParameter Id="0"><ParameterId Value="0"/><ParameterValue><Manual Value="{gain}"/><AutomationTarget Id="{param}"/></ParameterValue></PluginFloatParameter></ParameterList><PluginDesc><Vst3PluginInfo><Name Value="{}"/><Uid Value="{}"/><Version Value="{}"/><Preset><Vst3Preset><ProcessorState>{}</ProcessorState></Vst3Preset></Preset></Vst3PluginInfo></PluginDesc></PluginDevice>"#,
                class.info.name,
                class.info.uid,
                class.info.version,
                component(gain, delay)
            )
        };
        let mut text = crate::ableton::tests::document(11);
        let start = text.find("<InstrumentGroupDevice").unwrap();
        let end = text.find("</InstrumentGroupDevice>").unwrap() + "</InstrumentGroupDevice>".len();
        text.replace_range(
            start..end,
            &format!(
                "{}{}",
                device(7, instrument, 0.25, 64., 300),
                device(8, effect, 0.5, 0., 301)
            ),
        );
        text.replacen("<MidiTrack Id=\"4\">",r#"<MidiTrack Id="4"><AutomationEnvelopes><Envelopes><AutomationEnvelope Id="1"><EnvelopeTarget><PointeeId Value="301"/></EnvelopeTarget><Automation><Events><FloatEvent Time="-63072000" Value="0.5"/><FloatEvent Time="0" Value="0.5"/><FloatEvent Time="4" Value="0.25"/></Events></Automation></AutomationEnvelope></Envelopes></AutomationEnvelopes>"#,1)
    }
    #[test]
    #[ignore = "requires an original compiled SDK contract fixture and a separately qualified native worker; structural ALS fixture is independently authored, not a Live-created Set"]
    fn native_vst3_state_chain_automation_merge_and_reopen_restore_measured_audio() {
        let (instrument, instrument_saved) = engine::audio::routing::plugin_tests::fixture(true);
        let (effect, effect_saved) = engine::audio::routing::plugin_tests::fixture(false);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/validation/ableton-native-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("Original contract.als");
        let text = contract_set(&instrument, &effect);
        std::fs::write(&source, &text).unwrap();
        let cancel = AtomicBool::new(false);
        let draft = load(&source, &Default::default(), &cancel).unwrap();
        assert_eq!(
            draft.state.migration.as_ref().unwrap().sources[0]
                .devices
                .len(),
            2
        );
        let mut wrong = instrument.clone();
        wrong.info.uid = "00000000000000000000000000000001".into();
        assert!(relink(
            draft.clone(),
            0,
            0,
            instrument_saved.binary.clone(),
            wrong,
            true,
            &cancel
        )
        .is_err());
        let restored = relink(
            draft,
            0,
            0,
            instrument_saved.binary.clone(),
            instrument.clone(),
            false,
            &cancel,
        )
        .unwrap();
        let restored = relink(
            restored,
            0,
            1,
            effect_saved.binary.clone(),
            effect.clone(),
            false,
            &cancel,
        )
        .unwrap();
        let graph = restored.state.routing.as_ref().unwrap();
        assert_eq!(graph.plugins.len(), 2);
        assert_eq!(graph.plugins[0].latency, 832);
        assert_eq!(
            graph.plugins[1].automation[0].points,
            [[0., 0.5], [4., 0.25]]
        );
        let track = restored.state.session.as_ref().unwrap().tracks[0].id;
        let a = graph.plugins.iter().find(|p| p.instrument).unwrap().id;
        let b = graph.plugins.iter().find(|p| !p.instrument).unwrap().id;
        assert!(graph
            .connections
            .iter()
            .any(|r| r.source.group == Group::Plugin(a) && r.destination == Group::Plugin(b)));
        assert!(graph
            .connections
            .iter()
            .any(|r| r.source.group == Group::Plugin(b) && r.destination == Group::Track(track)));
        let path = root.join("Restored.omatainer");
        crate::project_file::save(
            &path,
            &crate::project_file::Bundle {
                state: restored.state.clone(),
                media: restored.media.clone(),
            },
            crate::project_file::Overwrite::Never,
            &Default::default(),
            &cancel,
        )
        .unwrap();
        let reopened =
            crate::project_file::load::<project::State>(&path, &Default::default(), &cancel)
                .unwrap();
        assert_eq!(
            reopened.state.migration.as_ref().unwrap().sources[0].xml,
            text
        );
        assert!(reopened.state.migration.as_ref().unwrap().sources[0]
            .devices
            .iter()
            .all(|d| d.resolution.is_some()));
        let mut rt = project::Prepared::from_state(reopened.state, reopened.media, 48000)
            .unwrap()
            .into_offline();
        rt.apply(engine::Command::LiveNoteOn {
            source: 61,
            ch: 0,
            note: 69,
            vel: 127,
        });
        let mut audio = [0.; 512];
        for _ in 0..32 {
            rt.process(&mut audio);
        }
        let expected = 0.0125f32;
        assert!(
            audio.iter().all(|v| (*v - expected).abs() < 1e-6),
            "Wrong restored instrument/FX amplitude: {} vs {expected}",
            audio[0]
        );
        rt.apply(engine::Command::LiveNoteOff {
            source: 61,
            ch: 0,
            note: 69,
        });
        for _ in 0..32 {
            rt.process(&mut audio);
        }
        assert!(audio.iter().all(|v| v.abs() < 1e-6));
        drop(rt);
        let (engine, mut rt) = engine::Engine::headless_for_test(48000, 256);
        let initial = rt.tracks.len();
        let layout = restored.state.session.as_ref().unwrap();
        let selection = session::ImportSelection {
            tracks: layout.tracks.iter().map(|t| t.id).collect(),
            scenes: layout.scenes.iter().map(|s| s.id).collect(),
            clips: true,
            devices: true,
            keep_timing: true,
        };
        let captured = crate::ableton::tests::capture(&engine, &mut rt);
        let (request, ack) = session::Request::import(
            captured,
            &restored.state,
            &restored.media,
            &selection,
            48000,
        )
        .unwrap();
        engine.send(engine::Command::session_edit(request)).unwrap();
        assert_eq!(
            engine::test_alloc::measure(|| rt.process(&mut [])),
            Default::default()
        );
        assert_eq!(ack.state(), engine::midi_edit::Outcome::Applied);
        let record = &rt.migration.as_ref().unwrap().sources[0];
        for device in &record.devices {
            assert!(rt
                .routing
                .as_ref()
                .unwrap()
                .model
                .plugins
                .iter()
                .any(|p| p.id == device.resolution.as_ref().unwrap().native_id
                    && p.midi_track == Some(rt.session.tracks[initial].id)));
        }
        engine.send(engine::Command::Undo).unwrap();
        assert_eq!(
            engine::test_alloc::measure(|| rt.process(&mut [])),
            Default::default()
        );
        assert!(rt.migration.is_none());
        engine.send(engine::Command::Redo).unwrap();
        assert_eq!(
            engine::test_alloc::measure(|| rt.process(&mut [])),
            Default::default()
        );
        assert!(rt.migration.is_some());
        assert_eq!(std::fs::read_to_string(source).unwrap(), text);
        std::fs::remove_dir_all(root).unwrap();
    }
}
