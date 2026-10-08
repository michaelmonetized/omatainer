use super::*;
use crate::engine::musical_context::{Context, Origin, Resolved, Scale};
const TONICS: [&str; 12] = [
    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
];

pub(super) fn description(resolved: Resolved) -> String {
    match resolved.context {
        Some(context) => format!(
            "{} {} · {}",
            TONICS[usize::from(context.tonic)],
            context.scale.name(),
            match resolved.origin {
                Origin::Clip => "clip override",
                Origin::Song => "inherited song key",
                Origin::None => "unspecified",
            }
        ),
        None => "No saved key; absolute pitches are retained".into(),
    }
}
fn edit(ui: &mut Ui, label: &str, value: &mut Option<Context>) -> bool {
    let before = *value;
    ui.push_id(label, |ui| {
        ui.horizontal_wrapped(|ui| {
            let mut enabled = value.is_some();
            let response = ui.checkbox(&mut enabled, label);
            accessibility::button(ui, &response, label, Some(enabled));
            if response.changed() {
                *value = enabled.then_some(Context {
                    tonic: 0,
                    scale: Scale::Major,
                });
            }
            if let Some(context) = value {
                let tonic = egui::ComboBox::from_id_salt("tonic")
                    .selected_text(TONICS[usize::from(context.tonic)])
                    .show_ui(ui, |ui| {
                        for (tonic, name) in TONICS.iter().enumerate() {
                            ui.selectable_value(&mut context.tonic, tonic as u8, *name);
                        }
                    });
                tonic.response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::ComboBox,
                        true,
                        format!("{label} tonic"),
                    )
                });
                let scale = egui::ComboBox::from_id_salt("scale")
                    .selected_text(context.scale.name())
                    .show_ui(ui, |ui| {
                        for scale in Scale::ALL {
                            ui.selectable_value(&mut context.scale, scale, scale.name());
                        }
                    });
                scale.response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::ComboBox,
                        true,
                        format!("{label} scale"),
                    )
                });
            }
        });
    });
    before != *value
}

/// Edit the saved musical context without changing absolute notes.
/// Takes the native editor, captured owner, engine and current snapshot; stages clip context in its draft and sends explicit undoable song or instrument settings.
pub(super) fn show(
    ui: &mut Ui,
    draft: &mut Draft,
    engine: &Engine,
    snapshot: &crate::engine::Snapshot,
    editing: bool,
) {
    egui::CollapsingHeader::new("Saved key and scale").id_salt("midi-saved-scale").show(ui, |ui| {
        let text = description(draft.resolved_context());
        let response = ui.label(&text);
        accessibility::status(ui, &response, &text);
        ui.add_enabled_ui(editing && draft.tools.editing(), |ui| {
            draft.dirty |= edit(ui, "Override clip key", &mut draft.context);
            let mut song = snapshot.musical_context;
            if edit(ui, "Set song key", &mut song) {
                if let Err(error) = engine.send(Command::SongContext(song)) { ui.label(format!("Song key was not accepted: {error}")); }
            }
            let response = ui.checkbox(&mut draft.highlight_scale, "Highlight saved scale");
            accessibility::button(ui, &response, "Highlight saved scale", Some(draft.highlight_scale));
            let mut fold = draft.fold == 4;
            let response = ui.checkbox(&mut fold, "Fold to saved scale");
            accessibility::button(ui, &response, "Fold to saved scale", Some(fold));
            if response.changed() { draft.fold = if fold { 4 } else { 0 }; }
            let mut follow = snapshot.sampler_scale;
            let response = ui.checkbox(&mut follow, "Instrument pads follow saved scale");
            accessibility::button(ui, &response, "Instrument pads follow saved scale", Some(follow));
            if response.changed() {
                if let Err(error) = engine.send(Command::SamplerScale(follow)) { ui.label(format!("Instrument setting was not accepted: {error}")); }
            }
        });
        if snapshot.musical_context != draft.baseline.song_context { ui.label("Song key changed after this capture. Refresh the clip before applying; your draft is retained."); }
        if snapshot.active_scale.conflicted { ui.label("Playing clips have different keys. Each keeps its own pitches and scale; the song key stays unchanged."); }
        if snapshot.active_scale.unspecified { ui.label("Some playing material has no saved key."); }
        ui.label("Clip key changes commit with Apply MIDI edit. Song key and instrument participation each have their own Undo. Highlight and fold keep chromatic notes visible; changing a key never rewrites existing notes.");
    });
}

#[cfg(test)]
mod tests;
