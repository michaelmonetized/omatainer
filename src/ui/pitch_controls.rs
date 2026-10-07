use super::*;
use crate::engine::deck_controls::{Button, Control};

#[derive(Clone, Copy)]
struct Hold {window:egui::ViewportId,widget:egui::Id,seen:u64}
pub(super) struct Inputs {
    source:u64,
    owners:[[[Option<Hold>;4];2];DECKS],
    media:[u64;DECKS],
    epoch:u64,
    pending:[[Option<(Hold,u64)>;2];DECKS],
}
impl Inputs {
    /// Prepare independent native momentary bend owners.
    /// Takes no arguments; returns bounded pointer, Space, Enter and assistive ownership for both directions on each deck.
    pub(super) fn new()->Self {Self {source:crate::engine::midi::next_source_id(),owners:[[[None;4];2];DECKS],media:[0;DECKS],epoch:0,pending:[[None;2];DECKS]}}
    fn held(&self,deck:usize,direction:usize)->bool {self.owners[deck][direction].iter().any(Option::is_some)}
}
impl App {
    /// Merge native gesture edges without releasing independent MIDI bend owners.
    /// Takes exact deck, direction, input route and optional native hold; admits the first press and last release through the existing reserved gate route.
    fn set_pitch_input(&mut self,deck:usize,direction:usize,route:usize,hold:Option<Hold>) {
        let old=self.pitch_inputs.owners[deck][direction][route];
        if hold.is_some() && old.is_some() {return;}
        let before=self.pitch_inputs.held(deck,direction);
        let button=if direction==0 {Button::BendDown} else {Button::BendUp};
        if hold.is_some() && !before && !self.submit(Command::DeckControl {source:self.pitch_inputs.source,deck:deck as u8,control:Control::Hold {button,on:true}}) {return;}
        self.pitch_inputs.owners[deck][direction][route]=hold;
        if before && !self.pitch_inputs.held(deck,direction) {self.send(Command::DeckControl {source:self.pitch_inputs.source,deck:deck as u8,control:Control::Hold {button,on:false}});}
    }
    /// Retire native bend gestures when their controls or performance context disappear.
    /// Takes the current native frame; handles releases globally even when a popup or deck panel was closed.
    pub(super) fn guard_pitch_inputs(&mut self,ctx:&egui::Context) {
        let epoch=self.engine.cmd.performance().input_epoch();
        let blocked=self.engine.safe_mode() || self.engine.cmd.performance().status().recovery
            || self.project.committing() || !self.project.dialog_is_closed() || keyboard::dialogs_block_input(ctx);
        let pass=ctx.cumulative_pass_nr();let window=ctx.viewport_id();
        for deck in 0..DECKS {
            let media=self.snap.decks[deck].media_key;
            for direction in 0..2 {for route in 0..4 {
                let Some(owner)=self.pitch_inputs.owners[deck][direction][route] else {continue;};
                let ended=owner.window==window && (!ctx.input(|i|i.focused) || blocked || owner.seen.saturating_add(1)<pass
                    || route==0 && !ctx.input(|i|i.pointer.primary_down())
                    || matches!(route,1|2) && (!ctx.memory(|m|m.has_focus(owner.widget)) || !ctx.input(|i|i.key_down(if route==1 {Key::Space} else {Key::Enter}))));
                if ended || epoch!=self.pitch_inputs.epoch || media!=self.pitch_inputs.media[deck] {self.set_pitch_input(deck,direction,route,None);}
            }}
            self.pitch_inputs.media[deck]=media;
        }
        self.pitch_inputs.epoch=epoch;
        if !ctx.input(|i|i.focused) || self.engine.safe_mode() || self.engine.cmd.performance().status().recovery {self.pitch_inputs.pending=[[None;2];DECKS];}
    }
    /// Close every native bend gate before retiring its panel.
    /// Takes the app; preserves controller and remote owners and requires a fresh press.
    pub(super) fn release_pitch_inputs(&mut self) {self.pitch_inputs.pending=[[None;2];DECKS];for deck in 0..DECKS {for direction in 0..2 {for route in 0..4 {self.set_pitch_input(deck,direction,route,None);}}}}
    /// Inspect base pitch in the existing Sync settings popup.
    /// Takes native UI, exact deck and one published snapshot; reports original/local/effective BPM and the physical pickup target separately.
    pub(super) fn deck_pitch_controls(&mut self,ui:&mut Ui,_deck:u8,snap:&crate::engine::DeckSnap) {
        ui.separator();
        let span=[8.0,16.0,50.0][usize::from(snap.pitch_range.min(2))];
        let effective=if snap.playing {snap.bpm*snap.playback_rate.abs()} else if snap.sync {snap.sync_target_bpm} else {snap.bpm*(1.0+(snap.pitch-0.5)*span*0.02)};
        ui.label(format!("Original {:.2} BPM · local grid {:.2} BPM",snap.source_bpm,snap.bpm));
        ui.label(format!("Effective {:.2} BPM · pitch range ±{span:.0}%",effective));
        let pickup=pickup_label(snap.pitch_pickup,span);
        ui.label(&pickup);
    }
    /// Hold a source-owned temporary bend on the native pitch column.
    /// Takes its native UI, exact deck and current snapshot; pointer, focused keyboard and assistive inputs share admitted press/release edges.
    pub(super) fn deck_bend_controls(&mut self,ui:&mut Ui,deck:u8,snap:&crate::engine::DeckSnap) {
        let enabled=snap.media_key!=0 && !self.engine.safe_mode() && !self.engine.cmd.performance().status().recovery;
        ui.horizontal(|ui| {for direction in 0..2 {
            let index=usize::from(deck);
            if let Some((owner,media))=self.pitch_inputs.pending[index][direction] {
                if owner.window==ui.ctx().viewport_id() && owner.seen<ui.ctx().cumulative_pass_nr() && !keyboard::dialogs_block_input(ui.ctx()) {self.pitch_inputs.pending[index][direction]=None;if enabled && media==snap.media_key {self.set_pitch_input(index,direction,3,Some(Hold {seen:ui.ctx().cumulative_pass_nr(),..owner}));}}
            }
            let held=self.pitch_inputs.held(index,direction);
            let response=ui.add_enabled(enabled,egui::Button::new(if direction==0 {"−"} else {"+"}).small().selected(held));
            let window=ui.ctx().viewport_id();let pass=ui.ctx().cumulative_pass_nr();
            let hold=Hold {window,widget:response.id,seen:pass};
            for route in 0..4 {if let Some(owner)=&mut self.pitch_inputs.owners[index][direction][route] {if owner.window==window {owner.seen=pass;}}}
            if response.is_pointer_button_down_on() && ui.input(|i|i.pointer.any_pressed()) && enabled {response.request_focus();self.set_pitch_input(index,direction,0,Some(hold));}
            if !ui.input(|i|i.pointer.primary_down()) {self.set_pitch_input(index,direction,0,None);}
            let focused=response.has_focus() && ui.input(|i|i.focused);
            let events=ui.input(|i|i.events.iter().filter_map(|e|match e {egui::Event::Key {key:key @ (Key::Space|Key::Enter),pressed,repeat:false,modifiers,..} if !pressed || *modifiers==egui::Modifiers::NONE=>Some((*key,*pressed)),_=>None}).collect::<Vec<_>>());
            for (key,on) in events {let route=if key==Key::Space {1} else {2};if !on || focused {self.set_pitch_input(index,direction,route,(on && enabled).then_some(hold));}if focused && on {keyboard::block_for_activation(ui.ctx());}}
            let name=if direction==0 {"Bend down"} else {"Bend up"};
            accessibility::button(ui,&response,name,Some(held));
            let actions=accessibility::actions(ui,&response,&["Press bend","Release bend"]);
            let click=ui.input(|i|i.num_accesskit_action_requests(response.id,egui::accesskit::Action::Click)>0);
            if actions==Some(1) || click && self.pitch_inputs.owners[index][direction][3].is_some() {self.pitch_inputs.pending[index][direction]=None;self.set_pitch_input(index,direction,3,None);}
            else if enabled && (actions==Some(0) || click) {
                if keyboard::dialogs_block_input(ui.ctx()) {self.pitch_inputs.pending[index][direction]=Some((hold,snap.media_key));}
                else {self.set_pitch_input(index,direction,3,Some(hold));}
            }
            help::annotate(ui,&response,HelpControl::Pitch);
            ui.ctx().accesskit_node_builder(response.id,|node|node.set_description("Hold mouse, Space or Enter for an 8 percent bend. Release returns to the base rate. Assistive Click toggles this visible hold; Press bend and Release bend are explicit actions. Closing the deck panel or losing focus releases native holds."));
        }});
    }
}
/// Describe one retained physical fader without conflating it with audible tempo.
/// Takes published pickup state and the chosen percent range; returns a visible and accessible target, direction and ownership message.
pub(super) fn pickup_label(status:crate::engine::pitch_pickup::Status,span:f32)->String {
    let span=f64::from(span);
    let target=(status.target*2.0-1.0)*span;
    match status.physical {
        None=>format!("Base pitch {target:+.2}% · absolute faders wait for pickup"),
        Some(_) if status.sync=>format!("Pickup {target:+.2}% · Sync on; choose Off before moving the base pitch"),
        Some(_) if status.acquired=>format!("Pitch fader acquired · base {target:+.2}%"),
        Some(value)=>format!("Pickup {target:+.2}% · move {} from {:+.2}%",if value<status.target {"up"} else {"down"},(value*2.0-1.0)*span),
    }
}
#[cfg(test)]
mod tests;
