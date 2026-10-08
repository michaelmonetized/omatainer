use super::*;
use crate::engine::{audio_clip::Fades, ClipKind};

/// Review independent placement fades and stable linked edits.
/// Takes native UI, the editable song, its pinned source preview and selected placement; edits metadata for the existing stopped-project Apply/Undo route without touching source PCM.
pub(super) fn controls(ui: &mut Ui, editor: &mut Editor, preview: &Preview, id: u64) {
    let Some(mut instance) = editor.model.instances.iter().find(|i| i.id == id).copied() else {
        return;
    };
    if !editor
        .model
        .sources
        .iter()
        .any(|s| s.id == instance.source && s.clip.kind == ClipKind::Audio)
    {
        return;
    }
    ui.separator();
    let mut own = instance.fades.is_some();
    if ui.checkbox(&mut own, "Use placement fades").changed() {
        instance.fades = own.then_some(Fades {
            automatic: true,
            ..Default::default()
        });
        if let Err(error) = editor
            .model
            .edit_fades(id, instance, &preview.captured.media)
        {
            editor.message = error;
        }
    }
    if let Some(mut fades) = instance.fades {
        if super::super::audio_fades::controls(ui, &mut fades, instance.duration) {
            instance.fades = Some(fades);
            if let Err(error) = editor
                .model
                .edit_fades(id, instance, &preview.captured.media)
            {
                editor.message = error;
            }
        }
    } else {
        ui.label("Uses the source clip's fades. Enable placement fades to edit this occurrence independently.");
    }
    if instance.fade_link != 0 {
        let count = editor
            .model
            .instances
            .iter()
            .filter(|i| i.fade_link == instance.fade_link)
            .count();
        ui.label(format!(
            "Fade link {} · {} tracks",
            instance.fade_link, count
        ));
    }
    let selector = egui::ComboBox::from_label("Fade link partner")
        .selected_text(editor.fade_link_partner.map_or_else(
            || "Choose aligned track".into(),
            |id| format!("Instance {id}"),
        ))
        .show_ui(ui, |ui| {
            for other in &editor.model.instances {
                if other.id != id
                    && other.track != instance.track
                    && editor
                        .model
                        .sources
                        .iter()
                        .any(|s| s.id == other.source && s.clip.kind == ClipKind::Audio)
                {
                    ui.selectable_value(
                        &mut editor.fade_link_partner,
                        Some(other.id),
                        format!("Link instance {} at beat {}", other.id, other.start),
                    );
                }
            }
        });
    ui.ctx()
        .accesskit_node_builder(selector.response.id, |node| {
            node.set_label("Fade link partner")
        });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                editor.fade_link_partner.is_some(),
                egui::Button::new("Link aligned fades"),
            )
            .clicked()
        {
            if let Err(error) = editor.model.link_fades(
                id,
                editor.fade_link_partner.unwrap(),
                &preview.captured.media,
            ) {
                editor.message = error;
            }
        }
        if ui
            .add_enabled(
                instance.fade_link != 0,
                egui::Button::new("Unlink fade group"),
            )
            .clicked()
        {
            editor.model.unlink_fades(id);
        }
    });
    let selector = egui::ComboBox::from_label("Crossfade partner")
        .selected_text(editor.fade_partner.or(instance.crossfade).map_or_else(
            || "Choose incoming placement".into(),
            |id| format!("Instance {id}"),
        ))
        .show_ui(ui, |ui| {
            for other in &editor.model.instances {
                if other.id != id
                    && other.track == instance.track
                    && other.start > instance.start
                    && editor
                        .model
                        .sources
                        .iter()
                        .any(|s| s.id == other.source && s.clip.kind == ClipKind::Audio)
                {
                    ui.selectable_value(
                        &mut editor.fade_partner,
                        Some(other.id),
                        format!(
                            "Crossfade into instance {} at beat {}",
                            other.id, other.start
                        ),
                    );
                }
            }
        });
    ui.ctx()
        .accesskit_node_builder(selector.response.id, |node| {
            node.set_label("Crossfade partner")
        });
    if editor.crossfade_length == 0.0 {
        editor.crossfade_length = 0.5;
    }
    number(
        ui,
        "Crossfade length beats",
        &mut editor.crossfade_length,
        0.000001,
        262144.0,
        0.01,
    );
    let curve =
        ui.add(egui::Slider::new(&mut editor.crossfade_curve, -1.0..=1.0).text("Crossfade curve"));
    if let Some(next) = accessibility::numeric(
        ui,
        &curve,
        "Crossfade curve",
        editor.crossfade_curve,
        -1.0,
        1.0,
        0.01,
        "",
    ) {
        editor.crossfade_curve = next;
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                editor.fade_partner.or(instance.crossfade).is_some(),
                egui::Button::new("Create or resize crossfade"),
            )
            .clicked()
        {
            if let Err(error) = editor.model.crossfade(
                id,
                editor.fade_partner.or(instance.crossfade).unwrap(),
                editor.crossfade_length,
                editor.crossfade_curve,
                &preview.captured.media,
            ) {
                editor.message = error;
            }
        }
        if ui.button("Unlink crossfade").clicked() {
            editor.model.unlink_crossfade(id);
        }
    });
    ui.label("Crossfades keep both outer ends and use unused audio on each side of the cut. Matching linked tracks change together. Apply song commits the complete edit with one Undo entry.");
}
