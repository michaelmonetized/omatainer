use super::*;
impl App {
    pub(super) fn poll_plugins(&mut self) {
        if self.engine.cmd.performance().protected() || self.engine.safe_mode() {
            self.plugins.cancel();
        }
        self.plugins.poll();
    }
    pub(super) fn plugins_ui(&mut self, ctx: &egui::Context) {
        if !self.plugins.open {
            return;
        }
        let mut open = true;
        let protected = self.engine.cmd.performance().protected()
            || self.engine.safe_mode()
            || self.project.committing()
            || !self.project.dialog_is_closed();
        let mut blacklist = None;
        let busy = self.plugins.busy();
        let mut selected = self.plugins.selected;
        let mut attach = None;
        egui::Window::new("VST3 plugins").id(egui::Id::new("plugin-browser")).open(&mut open).default_width(650.).vscroll(true).show(ctx,|ui|{
            ui.label("Native Linux VST3 instruments and effects are inspected in isolated workers. AU, VST2 and incompatible OS/CPU bundles require an installed native replacement or a rendered stem.");
            let label=ui.label("Plugin folders (one absolute path per line)");
            let edit=ui.add_enabled(!protected && !busy,egui::TextEdit::multiline(&mut self.plugins.roots).char_limit(65536).desired_rows(3).hint_text("/home/you/.vst3").id_source("plugin-roots")).labelled_by(label.id);
            edit.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,edit.enabled(),"Plugin folders"));
            ui.horizontal(|ui|{
                if ui.add_enabled(!protected && !busy,egui::Button::new("Reload plugin catalog")).clicked(){self.plugins.reload();}
                if ui.add_enabled(!protected && !busy,egui::Button::new("Scan changed plugins")).clicked(){self.plugins.scan(false);}
                if ui.add_enabled(!protected && !busy,egui::Button::new("Retry quarantined plugins")).clicked(){self.plugins.scan(true);}
                if ui.add_enabled(busy,egui::Button::new("Cancel plugin scan")).clicked(){self.plugins.cancel();}
            });
            ui.label(&self.plugins.message);
            if let Some((record,class)) = selected.and_then(|(r,c)| self.plugins.catalog.records.get(r).and_then(|record| record.classes.get(c).map(|class|(record,class)))) {
                if record.failure.is_none() && !self.plugins.catalog.blacklist.contains(&record.path) {
                    if let Some(binary) = &record.binary {
                        ui.horizontal(|ui| {
                            for (label,instrument) in [("Use as track instrument",true),("Append track effect",false)] {
                                if ui.add_enabled(!protected && !busy && if instrument { class.info.category.contains("Instrument") && class.info.has_midi_input } else { !class.layout.inputs.is_empty() && !class.info.category.contains("Instrument") },egui::Button::new(label)).clicked() {
                                    attach=Some(crate::engine::audio::routing::plugins::Instance {id:0,name:class.info.name.char_indices().take_while(|(index,_)| *index < 80).map(|(_,c)|c).collect(),saved:crate::plugin_host::Saved {schema:1,binary:binary.clone(),class_id:class.info.uid.clone(),plugin_version:class.info.version.clone(),state_codec:"vst3-host-0.9-state".into(),state:Vec::new()},inputs:class.layout.inputs.iter().map(|b| b.channel_count as u8).collect(),outputs:class.layout.outputs.iter().map(|b| b.channel_count as u8).collect(),midi_track:None,scene_track:None,instrument,bypass:false,latency:class.latency.saturating_add(crate::plugin_host::BLOCK as u32 * crate::plugin_host::realtime::BRIDGE_BLOCKS as u32),parameters:Vec::new(),automation:Vec::new(),unavailable:None});
                                }
                            }
                        });
                        ui.label("The processor is added to an inspected routing draft for the selected track. Review and apply while playback is stopped.");
                    }
                }
            }
            if protected{ui.label("Scanning is deferred while performance protection, safe mode or a project transaction is active.");}
            for (index,record) in self.plugins.catalog.records.iter().enumerate(){
                ui.push_id(index,|ui|{ui.separator();ui.label(record.path.display().to_string());let blocked=self.plugins.catalog.blacklist.contains(&record.path);
                    if ui.add_enabled(!protected && !busy,egui::Button::new(if blocked{"Allow plugin"}else{"Blacklist plugin"})).clicked(){blacklist=Some((record.path.clone(),!blocked));}
                    if let Some(error)=&record.failure{ui.colored_label(ui.visuals().warn_fg_color,format!("Quarantined: {error}"));}
                    for (class_index,class) in record.classes.iter().enumerate(){
                        if ui.selectable_label(selected==Some((index,class_index)),format!("{} · {} · {}",class.info.name,class.info.vendor,class.info.version)).clicked(){selected=Some((index,class_index));}
                        ui.label(format!("Class {} · {} · {} input buses / {} output buses · {} parameters · {} samples latency",class.info.uid,class.info.category,class.layout.inputs.len(),class.layout.outputs.len(),class.parameters.len(),class.latency));
                        ui.label(if class.tail==u32::MAX{"Tail: unlimited".into()}else{format!("Tail: {} samples",class.tail)});
                    }
                });
            }
            if busy{ctx.request_repaint_after(std::time::Duration::from_millis(50));}
        });
        if let Some((path, blocked)) = blacklist {
            self.plugins.blacklist(path, blocked);
        }
        self.plugins.selected = selected;
        self.plugins.open = open;
        if let Some(plugin)=attach { if let Some(track)=self.snap.session.as_ref().and_then(|s|s.tracks.get(self.snap.selected_track)).filter(|t|t.active) { self.audio_routing.attach_plugin(&self.engine,plugin,track.id); } else { self.plugins.message="Select a retained track before attaching a processor".into(); } }
    }
}

#[cfg(test)]
mod tests;
