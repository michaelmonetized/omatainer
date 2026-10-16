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
        let mut attach_return = None;
        let mut close = false;
        let namespace = self
            .snap
            .session
            .as_ref()
            .map_or([0; 2], |session| session.namespace);
        egui::Window::new("Native plugins").id(egui::Id::new("plugin-browser")).open(&mut open).default_width(650.).vscroll(true).show(ctx,|ui|{
            keyboard_authoring::target(ui, ("native-plugins", namespace), |ui| {
            ui.label("Native Linux VST3, CLAP and LV2 instruments and effects are inspected in isolated workers. Select a class to review its ports and extension support. AU requires a macOS host. Windows and legacy VST2 bridges are not bundled; compatible native editions and aligned rendered stems remain the migration paths.");
            let label=ui.label("Plugin folders (one absolute path per line)");
            let edit=ui.add_enabled(!protected && !busy,egui::TextEdit::multiline(&mut self.plugins.roots).char_limit(65536).desired_rows(3).hint_text("/home/you/.vst3").id_source(keyboard_authoring::property(ui, "plugin-roots"))).labelled_by(label.id);
            edit.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,edit.enabled(),"Plugin folders"));
            accessibility::focus(ui, &edit);
            let roots = self.plugins.roots.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>();
            let roots_valid = roots.len() <= 64 && roots.iter().all(|root| {
                let path = std::path::Path::new(root);
                path.is_absolute() && path.as_os_str().len() <= 4096 && !path.components().any(|part| matches!(part, std::path::Component::ParentDir))
            });
            if !roots_valid { keyboard_authoring::error(ui, &edit, "Use at most 64 absolute plugin folders without parent-directory components; each path must fit 4096 bytes"); }
            ui.horizontal(|ui|{
                if keyboard_authoring::action(ui, "Reload plugin catalog", !protected && !busy).clicked(){self.plugins.reload();}
                if keyboard_authoring::action(ui, "Scan changed plugins", !protected && !busy && roots_valid).clicked(){self.plugins.scan(false);}
                if keyboard_authoring::action(ui, "Retry quarantined plugins", !protected && !busy && roots_valid).clicked(){self.plugins.scan(true);}
                if keyboard_authoring::action(ui, "Cancel plugin scan", busy).clicked(){self.plugins.cancel();}
            });
            ui.label(&self.plugins.message);
            if let Some((record,class)) = selected.and_then(|(r,c)| self.plugins.catalog.records.get(r).and_then(|record| record.classes.get(c).map(|class|(record,class)))) {
                if record.failure.is_none() && !self.plugins.catalog.blacklist.contains(&record.path) {
                    if let Some(binary) = &record.binary {
                        let instance = |instrument| crate::engine::audio::routing::plugins::Instance {id:0,name:class.info.name.char_indices().take_while(|(index,_)| *index < 80).map(|(_,c)|c).collect(),saved:crate::plugin_host::Saved {schema:1,binary:binary.clone(),class_id:class.info.uid.clone(),plugin_version:class.info.version.clone(),state_codec:crate::plugin_host::Saved::codec(&class.info.uid).into(),state:Vec::new()},inputs:class.layout.inputs.iter().map(|b| b.channel_count as u8).collect(),outputs:class.layout.outputs.iter().map(|b| b.channel_count as u8).collect(),midi_track:None,scene_track:None,return_bus:None,instrument,bypass:false,latency:class.latency.saturating_add(crate::plugin_host::BLOCK as u32 * crate::plugin_host::realtime::BRIDGE_BLOCKS as u32),parameters:Vec::new(),automation:Vec::new(),unavailable:None};
                        keyboard_authoring::target(ui, ("selected-class", &record.path, &class.info.uid), |ui| {
                        ui.horizontal(|ui| {
                            for (label,instrument) in [("Use as track instrument",true),("Append track effect",false)] {
                                if keyboard_authoring::action(ui, label, !protected && if instrument { class.info.category.contains("Instrument") && class.info.has_midi_input } else { !class.layout.inputs.is_empty() && !class.info.category.contains("Instrument") }).clicked() {
                                    attach=Some(instance(instrument));
                                }
                            }
                        });
                        let returns = self.snap.routing_mixer.iter().filter_map(|channel| if let crate::engine::audio::routing::mixer::Target::Return(id) = channel.target { Some((id,channel.name.text())) } else { None }).collect::<Vec<_>>();
                        let key=egui::Id::new("native-return-target");
                        let mut target=ui.data(|data|data.get_temp::<([u64;2],u64)>(key));
                        let namespace=self.snap.session.as_ref().map(|s|s.namespace).unwrap_or([0;2]);
                        if target.is_some_and(|(ns,id)|ns!=namespace || !returns.iter().any(|r|r.0==id)) {target=None;}
                        let response = egui::ComboBox::from_label("Return for effect").id_salt(keyboard_authoring::property(ui, "return-for-effect")).selected_text(target.and_then(|(_,id)|returns.iter().find(|r|r.0==id).map(|r|r.1)).unwrap_or("Choose return")).show_ui(ui,|ui|{ for (id,name) in &returns {let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("return", namespace, id)), |ui| ui.selectable_value(&mut target,Some((namespace,*id)),*name)); accessibility::focus(ui, &response);} }).response;
                        accessibility::focus(ui, &response);
                        ui.data_mut(|data|{if let Some(target)=target{data.insert_temp(key,target);}else{data.remove::<([u64;2],u64)>(key);}});
                        if keyboard_authoring::action(ui, "Append return effect", !protected && !busy && target.is_some() && !class.layout.inputs.is_empty() && !class.info.category.contains("Instrument")).clicked(){if let Some((namespace,id))=target{attach_return=Some((instance(false),namespace,id));}}
                        ui.label("The processor is added to an inspected routing draft. Return effects remain shared; track effects retain the active scene send. Review and apply while playback is stopped.");
                        });
                    }
                }
            }
            if protected{ui.label("Scanning is deferred while performance protection, safe mode or a project transaction is active.");}
            for (index,record) in self.plugins.catalog.records.iter().enumerate(){
                ui.push_id(&record.path,|ui|{keyboard_authoring::target(ui, ("plugin-record", &record.path), |ui| {ui.separator();let path_label = ui.label(record.path.display().to_string());let blocked=self.plugins.catalog.blacklist.contains(&record.path);
                    let name = if blocked {"Allow plugin"} else {"Blacklist plugin"};
                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "blacklist"), |ui| ui.add_enabled(!protected && !busy, egui::Button::new(name)));
                    accessibility::button(ui, &response, name, None);
                    ui.ctx().accesskit_node_builder(response.id, |node| node.push_described_by(path_label.id.value().into()));
                    if response.clicked(){blacklist=Some((record.path.clone(),!blocked));}
                    if let Some(error)=&record.failure{ui.colored_label(ui.visuals().warn_fg_color,format!("Quarantined: {error}"));}
                    for (class_index,class) in record.classes.iter().enumerate(){
                        let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("plugin-class", &class.info.uid)), |ui| ui.selectable_label(selected==Some((index,class_index)),format!("{} · {} · {}",class.info.name,class.info.vendor,class.info.version)));
                        accessibility::focus(ui, &response);
                        if response.clicked(){selected=Some((index,class_index));}
                        ui.label(format!("Class {} · {} · {} input buses / {} output buses · {} parameters · {} samples latency",class.info.uid,class.info.category,class.layout.inputs.len(),class.layout.outputs.len(),class.parameters.len(),class.latency));
                        if let Some(lv2)=&class.lv2 { ui.label(format!("LV2 required features: {}",lv2.required.join(", "))); if !lv2.unsupported.is_empty() { ui.label(format!("Unavailable optional features: {}",lv2.unsupported.join(", "))); } for port in &lv2.ports { ui.label(format!("{} · {} · {}",port.name,port.symbol,port.kind)); } }
                        if let Some(native)=&class.native { ui.label(format!("CLAP extensions: {}",native.extensions.join(", "))); if !native.unsupported_host_extensions.is_empty() { ui.label(format!("Unavailable host features: {}",native.unsupported_host_extensions.join(", "))); } for port in native.audio_inputs.iter().chain(&native.audio_outputs) { ui.label(format!("{} · {} channels",port.name,port.channels)); } for port in native.note_inputs.iter().chain(&native.note_outputs) {ui.label(format!("Note port {} · dialects {:#x}",port.name,port.dialects));} }
                        ui.label(if class.tail==u32::MAX{"Tail: unlimited".into()}else{format!("Tail: {} samples",class.tail)});
                    }
                });});
            }
            if keyboard_authoring::action(ui, "Close native plugins", true).clicked() { close = true; }
            if busy{ctx.request_repaint_after(std::time::Duration::from_millis(50));}
            });
        });
        if let Some((path, blocked)) = blacklist {
            self.plugins.blacklist(path, blocked);
        }
        if let Some((plugin,namespace,id))=attach_return {self.audio_routing.attach_return_plugin(&self.engine,plugin,namespace,id);}
        self.plugins.selected = selected;
        self.plugins.open = open && !close;
        if let Some(plugin)=attach { if let Some(track)=self.snap.session.as_ref().and_then(|s|s.tracks.get(self.snap.selected_track)).filter(|t|t.active) { self.audio_routing.attach_plugin(&self.engine,plugin,track.id); } else { self.plugins.message="Select a retained track before attaching a processor".into(); } }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod keyboard_tests;
