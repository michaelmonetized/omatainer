use super::*;
use crate::engine::midi::{ControlSpec, MidiMap, UnmappedNotes};
use crate::ui::library_annotations::tests::{Files, Gui};
use egui::accesskit::{Action as AccessAction, ActionData};

#[test]
fn native_encoder_format_pair_capture_preview_assignment_and_saved_preset_reopen_use_real_handlers()
{
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    gui.app.midi_open = true;
    gui.app.settings =
        preferences::Settings::with_worker_for_test(files.0.join("preferences.json"));
    gui.app.midi_learn.binding = Binding {
        kind: MsgKind::Cc,
        ch: 0,
        data: 0,
        action: Action::Master,
        deck: 0,
        extra: 0,
        relative: None,
        controls: None,
        pair_order: None,
    };
    let mut input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        506,
        MidiMap {
            name: "Native encoder fixture".into(),
            matchers: vec![],
            bindings: vec![],
            unmapped_notes: UnmappedNotes::Ignore,
        },
        "Fixture 14-bit encoder",
        "fixture:native:14",
    );
    gui.frame(vec![]);
    gui.click("MIDI message format");
    gui.click("14-bit CC pair");
    gui.click("CC pair order");
    gui.click("LSB first (paired)");
    gui.click("Invert MIDI direction");
    gui.action(
        "MIDI minimum",
        AccessAction::SetValue,
        Some(ActionData::NumericValue(0.2)),
    );
    gui.action(
        "MIDI maximum",
        AccessAction::SetValue,
        Some(ActionData::NumericValue(0.8)),
    );
    gui.click("Capture MIDI control");
    let before = gui.rt.master;
    input.push(&[0xb3, 39, 127]);
    gui.frame(vec![]);
    assert!(gui.app.engine.cmd.midi_learn().view().capture.is_none());
    assert_eq!(gui.rt.master, before);
    input.push(&[0xb3, 7, 127]);
    gui.frame(vec![]);
    let capture = gui.app.engine.cmd.midi_learn().view().capture.unwrap();
    assert_eq!(capture.mapping.binding.data, 7);
    assert_eq!(
        capture.mapping.binding.pair_order,
        Some(crate::engine::midi::PairOrder::LsbFirst)
    );
    assert_eq!(capture.value, Some(16383));
    assert_eq!(
        capture.mapping.binding.controls,
        Some(ControlSpec {
            invert: true,
            min: 0.2,
            max: 0.8
        })
    );
    gui.click("Test captured MIDI action");
    gui.frame(vec![]);
    assert!((gui.rt.master - 0.2).abs() < 1e-6);
    gui.click("Add MIDI assignment");
    input.push(&[0xb3, 39, 0]);
    input.push(&[0xb3, 7, 0]);
    gui.frame(vec![]);
    assert!((gui.rt.master - 0.8).abs() < 1e-6);
    let view = gui.app.engine.cmd.midi_learn().view();
    let preset = crate::engine::midi::presets::Preset::capture(
        "Encoder stage".into(),
        "".into(),
        &capture.mapping.endpoint,
        &view.config,
    )
    .unwrap();
    let path = files.0.join("encoders.omatmidi");
    std::fs::write(&path, serde_json::to_vec(&preset).unwrap()).unwrap();
    let imported =
        crate::engine::midi::presets::Preset::decode(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(imported, preset);
    gui.click("MIDI message format");
    gui.click("Relative CC");
    gui.click("Relative encoder format");
    gui.click("Signed bit: 0/64 stationary; 1 forward, 65 backward");
    assert_eq!(
        gui.app.midi_learn.binding.relative.unwrap().encoding,
        crate::engine::midi::RelativeEncoding::SignedBit
    );
    gui.click("Save MIDI assignments");
    gui.wait(|gui| {
        !gui.app.settings.busy() && gui.app.settings.profile().midi_learn == view.config
    });
    gui.click("MIDI mapping presets");
    gui.click("Preset destination port");
    gui.click("Fixture 14-bit encoder / fixture:native:14");
    gui.text("MIDI preset name", "Encoder stage");
    gui.click("Save port as preset");
    gui.wait(|gui| !gui.app.settings.busy() && gui.app.settings.profile().midi_presets.len() == 1);
    assert_eq!(gui.app.settings.profile().midi_presets[0], preset);
    let export = files.0.join("native-encoder.json");
    gui.text("MIDI preset import or export path", &export.to_string_lossy());
    gui.click("Export MIDI preset");
    gui.wait(|gui| !gui.app.settings.busy() && export.exists());
    assert_eq!(
        crate::engine::midi::presets::Preset::decode(&std::fs::read(&export).unwrap()).unwrap(),
        preset
    );
    gui.click("Import MIDI preset");
    gui.wait(|gui| {
        !gui.app.settings.busy()
            && gui.visible_text().any(|text| text.starts_with("Import review: Encoder stage"))
    });
    gui.text("MIDI preset name", "Imported encoder");
    gui.click("Save imported preset to bank");
    gui.wait(|gui| !gui.app.settings.busy() && gui.app.settings.profile().midi_presets.len() == 2);
    let mut imported = preset.clone();
    imported.name = "Imported encoder".into();
    assert_eq!(gui.app.settings.profile().midi_presets, vec![preset.clone(), imported.clone()]);
    let reopened = crate::preferences::storage::load(
        &files.0.join("preferences.json"),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reopened.preferences.current().unwrap().midi_presets,
        vec![preset, imported]
    );
    assert_eq!(
        reopened.preferences.current().unwrap().midi_learn,
        view.config
    );
}
