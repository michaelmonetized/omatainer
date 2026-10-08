use super::*;
use std::collections::BTreeMap;
fn node(name: &str, children: Vec<Element>) -> Element {
    Element {
        name: name.into(),
        attributes: BTreeMap::new(),
        children,
        text: String::new(),
    }
}
fn manual(name: &str, value: &str) -> Element {
    let mut child = node("Manual", vec![]);
    child.attributes.insert("Value".into(), value.into());
    node(name, vec![child])
}
fn contains_instrument(item: &Element) -> bool {
    matches!(
        item.name.as_str(),
        "Operator" | "OriginalSimpler" | "MultiSampler" | "Collision" | "DrumGroupDevice"
    ) || item.name.starts_with("Instrument")
        || item.children.iter().any(contains_instrument)
}
/// Preserve a device preset as one independently editable source track.
/// Takes an owned Live 10/11/12 preset tree; returns a structural native-import wrapper while leaving original device IDs, nested branches, macros, values and opaque state unchanged.
pub(super) fn normalize(mut root: Element) -> Result<Element, String> {
    if root.children.iter().any(|n| n.name == "LiveSet") {
        return Ok(root);
    }
    if root.children.len() != 1 {
        return Err("Device preset needs one unambiguous source device".into());
    }
    let original = &root.children[0];
    let device = if original.name == "GroupDevicePreset" {
        let container = original.one("Device")?;
        if container.children.len() != 1 {
            return Err("Rack preset repeats its root device".into());
        }
        container.children[0].clone()
    } else {
        original.clone()
    };
    if matches!(
        device.name.as_str(),
        "Device" | "Clip" | "Tracks" | "Scenes" | "MasterTrack" | "MainTrack" | "FolderConfigData"
    ) {
        return Err("Unsupported device preset root".into());
    }
    let midi = contains_instrument(&device)
        || device.name == "PluginDevice" && boolean(&device, &["IsInstrument"], false)?;
    let mut track = node(
        if midi { "MidiTrack" } else { "AudioTrack" },
        vec![node(
            "DeviceChain",
            vec![node("DeviceChain", vec![node("Devices", vec![device])])],
        )],
    );
    track.attributes.insert("Id".into(), "0".into());
    let set = node(
        "LiveSet",
        vec![
            node("Tracks", vec![track]),
            node("Scenes", vec![]),
            node(
                "MasterTrack",
                vec![node(
                    "DeviceChain",
                    vec![node(
                        "Mixer",
                        vec![manual("Tempo", "120"), manual("TimeSignature", "201")],
                    )],
                )],
            ),
        ],
    );
    root.children = vec![set];
    Ok(root)
}
