use super::*;
use crate::audio_delivery::batch::{Item, Kind};
use crate::engine::audio::routing::selection::{Policy, Selection};

pub(super) fn controls(ui: &mut egui::Ui, panel: &mut Panel, plan: &Plan) -> Option<Operation> {
    let source_valid = !keyboard_authoring::invalid(ui)
        && panel.request.frames().is_ok()
        && panel.config.validate().is_ok();
    let mut operation = None;
    let response = ui.collapsing("Aligned stems & versions", |ui| {
        keyboard_authoring::target(ui, "batch", |ui| {
            ui.label("One captured revision. All passes share the reviewed range, origin, sample rate, output order, repeats and tail. The final folder appears only when every pass succeeds.");
            ui.label("Default group stems include shared scene effects. Track stems stop after their own devices, before shared scene effects. Shared returns are separate passes. Dry uses pre-FX taps; mastered solo includes the master and limiter and cannot promise an exact sum. Sidechain feeders keep running.");
            if keyboard_authoring::action(ui, "Use aligned group & return defaults", true).clicked() {
                let mut recipe = plan.batch.clone();
                recipe.source_scene = plan.scene_refs.iter().find(|(slot, _)| *slot == panel.request.scene).map(|(_, reference)| *reference);
                panel.batch = Some(recipe);
                panel.batch_normalize = false;
                panel.batch_dither = false;
            }
            let Some(recipe) = &mut panel.batch else { return; };
            let mut remove = None;
            egui::ScrollArea::vertical().id_salt("batch-pass-list").max_height(180.).show(ui, |ui| {
                for (index, item) in recipe.items.iter_mut().enumerate() {
                    ui.push_id(item.id, |ui| {
                        keyboard_authoring::target(ui, ("pass", item.id), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                text(ui, "pass-name", &format!("Pass {} name", item.id), &mut item.name, false);
                                ui.label(format!("{:?} · {:?} · {} contributions", item.kind, item.selection.policy, item.selection.targets.len()));
                                let response = keyboard_authoring::action(ui, &format!("Remove pass {}", item.id), true);
                                if response.clicked() { remove = Some(index); }
                            });
                        });
                    });
                }
            });
            if let Some(index) = remove { recipe.items.remove(index); }
            ui.separator();
            ui.label("Add a stem or explicit version from selected contributions");
            ui.horizontal(|ui| {
                text(ui, "new-pass-name", "New pass name", &mut panel.version_name, false);
                for (kind, name) in [(Kind::Stem, "Stem"), (Kind::Version, "Version")] {
                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("new-kind", kind as u8)), |ui| ui.selectable_value(&mut panel.version_kind, kind, name));
                    accessibility::focus(ui, &response);
                }
            });
            ui.horizontal(|ui| {
                for (policy, name) in [(Policy::PostDevice, "Post-device contributions"), (Policy::Dry, "Dry pre-FX"), (Policy::MasteredSolo, "Mastered solo")] {
                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("contribution-policy", policy as u8)), |ui| ui.selectable_value(&mut panel.version_policy, policy, name));
                    accessibility::focus(ui, &response);
                }
            });
            egui::ScrollArea::vertical().id_salt("batch-target-list").max_height(140.).show(ui, |ui| {
                for (target, name) in &plan.catalog {
                    let mut selected = panel.version_targets.contains(target);
                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("contribution", plan.namespace, format!("{target:?}"))), |ui| ui.checkbox(&mut selected, name));
                    accessibility::focus(ui, &response);
                    if response.changed() {
                        if selected { panel.version_targets.push(*target); }
                        else { panel.version_targets.retain(|saved| saved != target); }
                    }
                }
            });
            ui.label("Versions contain exactly these saved contribution IDs. Select the vocal track/group explicitly for vocal/instrumental versions; delivery does not separate a mixed recording.");
            if keyboard_authoring::action(ui, "Add selected contribution pass", !panel.version_name.is_empty() && !panel.version_targets.is_empty()).on_disabled_hover_text("Name the pass and choose at least one contribution").clicked() {
                if panel.version_name.is_empty() || panel.version_targets.is_empty() {
                    panel.message = "Name the pass and choose at least one contribution".into();
                } else if let Some(next) = recipe.next_id.checked_add(1) {
                    recipe.items.push(Item { id: recipe.next_id, name: panel.version_name.clone(), kind: panel.version_kind, selection: Selection { namespace: plan.namespace, targets: panel.version_targets.clone(), policy: panel.version_policy } });
                    recipe.next_id = next;
                    panel.version_targets.clear();
                    panel.version_name.clear();
                }
            }
            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "batch-normalize"), |ui| ui.checkbox(&mut panel.batch_normalize, "Normalize each batch pass separately to −1 dBFS sample peak"));
            accessibility::focus(ui, &response);
            if !panel.request.options.format.dither() { panel.batch_dither = false; }
            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "batch-dither"), |ui| ui.add_enabled(panel.request.options.format.dither(), egui::Checkbox::new(&mut panel.batch_dither, "Dither each integer batch pass")));
            accessibility::focus(ui, &response);
            ui.label("Uses the common export format/range/metadata above. Float PCM without independent normalization or dither preserves linear summation; lossy encoding, separate normalization and nonlinear shared/master processing do not.");
            text(ui, "recipe-path", "Batch recipe JSON path", &mut panel.recipe_path, false);
            if keyboard_authoring::action(ui, "Save batch recipe to new file", source_valid).clicked() {
                let mut saved = recipe.clone();
                saved.export = panel.request.clone(); saved.export.options.normalize = panel.batch_normalize; saved.export.options.dither = panel.batch_dither; saved.config = panel.config.clone();
                operation = Some(Operation::SaveRecipe(saved, PathBuf::from(&panel.recipe_path)));
            }
            if keyboard_authoring::action(ui, "Reopen batch recipe", true).clicked() { operation = Some(Operation::LoadRecipe(PathBuf::from(&panel.recipe_path))); }
            ui.label(format!("{} reviewed passes · shared destination: {}", recipe.items.len(), panel.path));
            if keyboard_authoring::action(ui, "Render aligned batch", source_valid).clicked() {
                let mut saved = recipe.clone();
                saved.export = panel.request.clone(); saved.export.options.normalize = panel.batch_normalize; saved.export.options.dither = panel.batch_dither; saved.config = panel.config.clone();
                operation = Some(Operation::Batch { plan: plan.clone(), recipe: saved, path: PathBuf::from(&panel.path) });
            }
        });
    });
    accessibility::focus(ui, &response.header_response);
    operation
}
