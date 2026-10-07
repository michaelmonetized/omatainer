use super::*;
use crate::engine::surface_controls::{
    fx::{Control, Placement, Timing},
    Input,
};

impl App {
    /// Draw both independently assigned DJ FX units.
    /// Takes the native UI context; submits exact bank edits from confirmed renderer state and leaves window closure separate from bypass.
    pub(super) fn dj_fx_ui(&mut self, ctx: &egui::Context) {
        if !self.dj_fx_open {
            return;
        }
        let mut open = true;
        egui::Window::new("DJ FX").id(egui::Id::new("dj-fx-window")).open(&mut open).default_width(780.0).show(ctx, |ui| {
            accessibility::scope(ui, "DJ FX", |ui| {
                ui.label("Each unit can process Deck A, Deck B, a selected sampler destination, and Master. Assigning several sources applies the unit separately to each; assigning a deck and Master applies it twice along that path.");
                let scroll=egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-180.0).max(120.0)).show(ui, |ui| {
                    for bank in 0..2 {
                        let settings = self.snap.surfaces.fx[bank];
                        let scope=format!("Unit {}", (b'A'+bank as u8) as char);
                        ui.push_id(bank, |ui| accessibility::scope(ui, &scope, |ui| {
                            ui.heading(&scope);
                            let send=|control| self.send(Command::Surface(Input::DjFx { bank: bank as u8, control }));
                            ui.horizontal_wrapped(|ui| {
                                for deck in 0..2 {
                                    let mut assigned=settings.assigned[deck];
                                    let label=format!("Deck {}", (b'A'+deck as u8) as char);
                                    let response=ui.checkbox(&mut assigned,&label);
                                    accessibility::button(ui,&response,&label,Some(settings.assigned[deck]));
                                    help::annotate(ui,&response,HelpControl::DjFx);
                                    if response.changed() { send(Control::Deck {deck:deck as u8,enabled:assigned}); }
                                }
                                let mut master=settings.master;
                                let response=ui.checkbox(&mut master,"Master");
                                accessibility::button(ui,&response,"Master",Some(settings.master));
                                help::annotate(ui,&response,HelpControl::DjFx);
                                if response.changed() { send(Control::Master(master)); }
                                let mut sampler=settings.sampler.is_some();
                                let target=self.snap.session.as_ref().and_then(|layout|layout.reference(crate::engine::session::Axis::Track,self.snap.selected_track));
                                let response=ui.add_enabled(settings.sampler.is_some()||target.is_some()&&!settings.on.iter().any(|on|*on)&&!settings.tails[2],egui::Checkbox::new(&mut sampler,"Sampler destination"));
                                accessibility::button(ui,&response,"Sampler destination",Some(settings.sampler.is_some()));
                                help::annotate(ui,&response,HelpControl::DjFx);
                                if response.changed() { send(Control::Sampler {target:if sampler {target}else{None}}); }
                            });
                            let name=settings.sampler.and_then(|reference|self.snap.session.as_ref().and_then(|layout|layout.tracks.iter().enumerate().find_map(|(index,track)|layout.resolves(crate::engine::session::Axis::Track,index,reference).then(||format!("Track {} · {}",index+1,track.name))))).unwrap_or_else(||"Unassigned".into());
                            let sampler=ui.add_enabled_ui(!settings.on.iter().any(|on|*on)&&!settings.tails[2],|ui|egui::ComboBox::from_id_salt("sampler-destination").selected_text(format!("Sampler destination · {name}")).show_ui(ui,|ui| {
                                if let Some(layout)=&self.snap.session {
                                    for (index,track) in layout.tracks.iter().enumerate().filter(|(_,track)|track.active) {
                                        let response=ui.button(format!("Track {} · {}",index+1,track.name));
                                        accessibility::button(ui,&response,&format!("Sampler destination track {}",index+1),None);
                                        if response.clicked() { send(Control::Sampler {target:layout.reference(crate::engine::session::Axis::Track,index)});ui.close(); }
                                    }
                                }
                            })).inner.response;
                            accessibility::button(ui,&sampler,"Choose sampler destination",None);
                            help::annotate(ui,&sampler,HelpControl::DjFx);
                            ui.horizontal_wrapped(|ui| {
                                let can_move=!settings.on.iter().any(|on|*on)&&!settings.tails.iter().any(|tail|*tail);
                                for (placement,label) in [(Placement::PreFader,"Pre fader"),(Placement::PostFader,"Post fader")] {
                                    let response=ui.add_enabled(can_move,egui::Button::selectable(settings.placement==placement,label));
                                    accessibility::button(ui,&response,label,Some(settings.placement==placement));help::annotate(ui,&response,HelpControl::DjFx);
                                    if response.clicked(){send(Control::Placement(placement));}
                                }
                                for (timing,label) in [(Timing::Beat,"Beat timing"),(Timing::Manual,"Manual timing")] {
                                    let response=ui.selectable_label(settings.timing==timing,label);
                                    accessibility::button(ui,&response,label,Some(settings.timing==timing));help::annotate(ui,&response,HelpControl::DjFx);
                                    if response.clicked(){send(Control::Timing(timing));}
                                }
                            });
                            match settings.timing {
                                Timing::Beat=>ui.horizontal_wrapped(|ui| {
                                    for (beats,label) in [(-4,"3/64 beat"),(-3,"3/32 beat"),(-2,"3/16 beat"),(-1,"3/8 beat"),(0,"3/4 beat"),(1,"1½ beats"),(2,"3 beats"),(3,"6 beats")] {
                                        let response=ui.selectable_label(settings.beats==beats,label);
                                        accessibility::button(ui,&response,label,Some(settings.beats==beats));help::annotate(ui,&response,HelpControl::DjFx);
                                        if response.clicked(){send(Control::Beats(beats));}
                                    }
                                }).response,
                                Timing::Manual=>ui.horizontal(|ui| {
                                    let mut milliseconds=settings.manual_ms;
                                    let response=ui.add(egui::Slider::new(&mut milliseconds,1.0..=1998.0).text("Echo time ms"));
                                    let alternate=accessibility::numeric(ui,&response,"Echo time ms",settings.manual_ms,1.0,1998.0,1.0,"ms");help::annotate(ui,&response,HelpControl::DjFx);
                                    if let Some(value)=alternate {milliseconds=value;}
                                    if response.changed()||alternate.is_some(){send(Control::ManualMs(milliseconds));}
                                }).response,
                            };
                            let rate=self.engine.sr().max(8000) as f32;
                            let spb=f64::from(rate)*60.0/f64::from(self.snap.bpm.max(1.0));
                            let time=format!("Echo {:.3} ms · independent stereo · {}",1000.0*settings.frames(spb,rate)/rate,if settings.tails.iter().any(|tail|*tail){"processing / tails"}else{"dry / settled"});
                            let response=ui.label(&time);accessibility::status(ui,&response,&time);
                            ui.label("Deck placement is around the crossfader; sampler placement is around sampler volume; Master placement is around master volume. Turn all slots off and let tails settle before changing placement or sampler destination. Disengaging a source stops new wet input and keeps its original tails. Echo time cannot exceed two seconds; the value above is the applied duration.");
                            for slot in 0..3 {
                                accessibility::scope(ui,&format!("{scope} slot {}",slot+1),|ui|ui.horizontal_wrapped(|ui| {
                                    let mut on=settings.on[slot];
                                    let response=ui.checkbox(&mut on,"Enabled");accessibility::button(ui,&response,"Enabled",Some(settings.on[slot]));help::annotate(ui,&response,HelpControl::DjFx);
                                    if response.changed(){send(Control::Enabled {slot:slot as u8,enabled:on});}
                                    for kind in [crate::engine::FxKind::Echo,crate::engine::FxKind::Reverb,crate::engine::FxKind::Filter] {
                                        let response=ui.selectable_label(settings.kinds[slot]==kind,kind.name());accessibility::button(ui,&response,kind.name(),Some(settings.kinds[slot]==kind));help::annotate(ui,&response,HelpControl::DjFx);
                                        if response.clicked(){send(Control::Kind {slot:slot as u8,kind});}
                                    }
                                    for (parameter,label,original) in [(false,"Wet",settings.wet[slot]),(true,"Feedback / cutoff",settings.parameter[slot])] {
                                        let mut value=original;let response=ui.add(egui::Slider::new(&mut value,0.0..=1.0).text(label));
                                        let alternate=accessibility::numeric(ui,&response,label,original,0.0,1.0,0.01,"");help::annotate(ui,&response,HelpControl::DjFx);
                                        if let Some(next)=alternate {value=next;}
                                        if response.changed()||alternate.is_some(){self.send(Command::Surface(Input::FxValue {bank:bank as u8,slot:slot as u8,parameter,value}));}
                                    }
                                }));
                            }
                            ui.separator();
                        }));
                    }
                });
                accessibility::scrollbars(ui,"DJ FX units",&scroll);
            });
        });
        self.dj_fx_open = open;
    }
}

#[cfg(test)]
mod tests;
