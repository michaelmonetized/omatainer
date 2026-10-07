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
    }
}

#[cfg(test)]
mod tests;
