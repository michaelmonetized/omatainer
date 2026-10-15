use super::*;
use crate::audio_delivery::batch::{Item, Kind, Recipe};
use crate::engine::audio::routing::selection::{Policy, Selection, Target};

pub(super) fn controls(ui: &mut egui::Ui, panel: &mut Panel, plan: &Plan) -> Option<Operation> {
    let mut operation = None;
    ui.collapsing("Aligned stems & versions",|ui|{
        ui.label("One captured revision. All passes share the reviewed range, origin, sample rate, output order, repeats and tail. The final folder appears only when every pass succeeds.");
        ui.label("Default group stems include shared scene effects. Track stems stop after their own devices, before shared scene effects. Shared returns are separate passes. Dry uses pre-FX taps; mastered solo includes the master and limiter and cannot promise an exact sum. Sidechain feeders keep running.");
        if ui.button("Use aligned group & return defaults").clicked(){let mut recipe=plan.batch.clone();recipe.source_scene=plan.scene_refs.iter().find(|(slot,_)|*slot==panel.request.scene).map(|(_,r)|*r);panel.batch=Some(recipe);panel.batch_normalize=false;panel.batch_dither=false;}
        let Some(recipe)=&mut panel.batch else {return};
        let mut remove=None;
        egui::ScrollArea::vertical().id_salt("batch-pass-list").max_height(180.).show(ui,|ui|{for (index,item) in recipe.items.iter_mut().enumerate(){ui.push_id(item.id,|ui|{ui.horizontal(|ui|{let label=ui.label(format!("Pass {} name",item.id));ui.text_edit_singleline(&mut item.name).labelled_by(label.id);ui.label(format!("{:?} · {:?} · {} contributions",item.kind,item.selection.policy,item.selection.targets.len()));if ui.button("Remove pass").clicked(){remove=Some(index);}});});}});
        if let Some(index)=remove {recipe.items.remove(index);}
        ui.separator();ui.label("Add a stem or explicit version from selected contributions");
        ui.horizontal(|ui|{let label=ui.label("New pass name");ui.text_edit_singleline(&mut panel.version_name).labelled_by(label.id);ui.selectable_value(&mut panel.version_kind,Kind::Stem,"Stem");ui.selectable_value(&mut panel.version_kind,Kind::Version,"Version");});
        ui.horizontal(|ui|{for (policy,label)in[(Policy::PostDevice,"Post-device contributions"),(Policy::Dry,"Dry pre-FX"),(Policy::MasteredSolo,"Mastered solo")]{ui.selectable_value(&mut panel.version_policy,policy,label);}});
        egui::ScrollArea::vertical().id_salt("batch-target-list").max_height(140.).show(ui,|ui|{for(target,name)in &plan.catalog{let mut selected=panel.version_targets.contains(target);if ui.checkbox(&mut selected,name).changed(){if selected{panel.version_targets.push(*target);}else{panel.version_targets.retain(|t|t!=target);}}}});
        ui.label("Versions contain exactly these saved contribution IDs. Select the vocal track/group explicitly for vocal/instrumental versions; delivery does not separate a mixed recording.");
        if ui.button("Add selected contribution pass").clicked(){
            if panel.version_name.is_empty()||panel.version_targets.is_empty(){panel.message="Name the pass and choose at least one contribution".into();}
            else if let Some(next)=recipe.next_id.checked_add(1){recipe.items.push(Item{id:recipe.next_id,name:panel.version_name.clone(),kind:panel.version_kind,selection:Selection{namespace:plan.namespace,targets:panel.version_targets.clone(),policy:panel.version_policy}});recipe.next_id=next;panel.version_targets.clear();panel.version_name.clear();}
        }
        ui.checkbox(&mut panel.batch_normalize,"Normalize each batch pass separately to −1 dBFS sample peak");
        if !panel.request.options.format.dither(){panel.batch_dither=false;}
        ui.add_enabled(panel.request.options.format.dither(),egui::Checkbox::new(&mut panel.batch_dither,"Dither each integer batch pass"));
        ui.label("Uses the common export format/range/metadata above. Float PCM without independent normalization or dither preserves linear summation; lossy encoding, separate normalization and nonlinear shared/master processing do not.");
        ui.horizontal(|ui|{let label=ui.label("Batch recipe JSON path");ui.text_edit_singleline(&mut panel.recipe_path).labelled_by(label.id);});
        if ui.button("Save batch recipe to new file").clicked(){let mut saved=recipe.clone();saved.export=panel.request.clone();saved.export.options.normalize=panel.batch_normalize;saved.export.options.dither=panel.batch_dither;saved.config=panel.config.clone();operation=Some(Operation::SaveRecipe(saved,PathBuf::from(&panel.recipe_path)));}
        if ui.button("Reopen batch recipe").clicked(){operation=Some(Operation::LoadRecipe(PathBuf::from(&panel.recipe_path)));}
        ui.label(format!("{} reviewed passes · shared destination: {}",recipe.items.len(),panel.path));
        if ui.button("Render aligned batch").clicked(){let mut saved=recipe.clone();saved.export=panel.request.clone();saved.export.options.normalize=panel.batch_normalize;saved.export.options.dither=panel.batch_dither;saved.config=panel.config.clone();operation=Some(Operation::Batch{plan:plan.clone(),recipe:saved,path:PathBuf::from(&panel.path)});}
    });
    operation
}
