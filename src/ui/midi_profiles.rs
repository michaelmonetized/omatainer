use super::*;
#[derive(Default)]
pub(super) struct Panel {
    selected: String,
    control: String,
    application: String,
    physical: String,
    message: String,
}
impl App {
    /// Review exact controller profiles and record one physical control check.
    /// Takes the MIDI panel; acquisition and evidence writes use a bounded owner, and mapping changes use the normal stopped connection transaction.
    pub(super) fn controller_profiles_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let Some(registry) = self.engine.midi.profiles().cloned() else {
            return;
        };
        let view = registry.view();
        if (self.snap.playing
            || self.snap.decks.iter().any(|d| d.playing)
            || self.engine.cmd.performance().protected())
            && view.guide.as_ref().is_some_and(|g| g.capturing)
        {
            registry.finish_check();
        }
        let p = &mut self.midi_learn.profiles;
        let stopped = !self.snap.playing
            && !self.snap.recording
            && self.snap.decks.iter().all(|d| !d.playing);
        let allowed = stopped
            && !self.engine.cmd.performance().protected()
            && !self.project.committing()
            && !self.engine.midi.connections_busy();
        ui.collapsing("Controller profiles and connection check",|ui|{
            if !view.ready{ui.label("Recovering private controller cache");ctx.request_repaint_after(std::time::Duration::from_millis(50));return;}
            ui.label("Profiles use USB identity and exact port roles. A sent LED message does not prove a button works.");
            ui.label(format!("Cached profiles: {} · previous profiles: {}",view.cached_generation.map_or_else(||"none".into(),|g|g.to_string()),view.previous_generation.map_or_else(||"none".into(),|g|if g==0{"bundled".into()}else{g.to_string()})));
            ui.label(&view.message);ui.label(&p.message);
            ui.horizontal(|ui|{
                if ui.add_enabled(!view.busy&&!self.engine.cmd.performance().protected(),egui::Button::new("Check profile updates")).clicked(){p.message=registry.acquire(self.engine.cmd.performance()).err().unwrap_or_default();}
                if view.busy&&ui.button("Cancel profile work").clicked(){registry.cancel();}
                if ui.add_enabled(allowed&&!view.busy&&view.cached_generation.is_some(),egui::Button::new("Apply cached profiles")).clicked(){p.message=self.engine.midi.apply_profiles(false,&self.snap).err().unwrap_or_default();}
                if ui.add_enabled(allowed&&!view.busy&&view.previous_generation.is_some(),egui::Button::new("Restore previous profiles")).clicked(){p.message=self.engine.midi.apply_profiles(true,&self.snap).err().unwrap_or_default();}
            });
            if view.busy{ctx.request_repaint_after(std::time::Duration::from_millis(50));}
            for d in &view.devices {
                let label=format!("{} · USB {:04x}:{:04x} · {}",d.name,d.device.vendor,d.device.product,d.device.topology);
                if ui.selectable_label(p.selected==d.id,label).clicked(){p.selected=d.id.clone();}
                ui.label(&d.reason);
            if d.local_preset_override{ui.label("Saved MPD232 preset retained over the factory layer");}
                if let Some(profile)=&d.profile{ui.label(format!("{} {} · {} · generation {}",profile.id,profile.version,profile.protocol,d.generation));}
                ui.label(format!("Input open: {} · output open: {} · initialization sent: {} · output failures: {}",d.input_open,d.output_open,d.initialization_sent,d.output_failures));
                ui.label(format!("USB release {:04x} · inquiry reply: {}",d.device.release,d.inquiry.as_deref().unwrap_or("pending; no device acknowledgment recorded")));
            }
            ui.label("Name one control. Capture consumes its messages so it cannot launch clips or change transport. Finish capture, then exercise it normally and enter what you saw.");
            ui.horizontal(|ui|{ui.label("Control");ui.text_edit_singleline(&mut p.control).widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Controller check name"));});
            if ui.add_enabled(allowed&&!view.busy&&!p.selected.is_empty()&&!view.guide.as_ref().is_some_and(|g|g.capturing),egui::Button::new("Capture this control")).clicked(){p.message=registry.begin_check(&p.selected,&p.control,&self.snap).err().unwrap_or_default();p.application.clear();p.physical.clear();}
            if let Some(check)=&view.guide {
                ui.label(format!("{} · received input: {} · worker packets: {}",check.control,check.input_received,check.worker_processed));
                if check.capturing{ctx.request_repaint_after(std::time::Duration::from_millis(50));if ui.button("Finish input capture").clicked(){registry.finish_check();}}
                for packet in &check.packets{ui.monospace(packet);}
                if !check.capturing {
                    ui.horizontal(|ui|{ui.label("App response observed");ui.text_edit_singleline(&mut p.application).widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Controller app observation"));});
                    ui.horizontal(|ui|{ui.label("Lights / hardware observed");ui.text_edit_singleline(&mut p.physical).widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Controller hardware observation"));});
                    if ui.button("Record observations").clicked(){p.message=registry.observations((!p.application.is_empty()).then_some(p.application.as_str()),(!p.physical.is_empty()).then_some(p.physical.as_str())).err().unwrap_or_else(||"Observations recorded; save evidence to retain them".into());}
                    if ui.add_enabled(!view.busy,egui::Button::new("Save controller evidence")).clicked(){p.message=registry.save_check(self.engine.cmd.performance()).err().unwrap_or_default();}
                    ui.label(format!("App observation: {} · physical observation: {}",check.application_observation.as_deref().unwrap_or("pending"),check.physical_observation.as_deref().unwrap_or("pending")));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
