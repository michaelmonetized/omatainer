use super::*;
use crate::engine::deck_controls::{Button, Control};

pub(super) struct Inputs {
    source: u64,
    pub(super) owners: [[Option<egui::ViewportId>; 5]; DECKS],
    media: [u64; DECKS],
    epoch: u64,
    pending: [Option<(egui::ViewportId, u64, u64)>; DECKS],
}
impl Inputs {
    /// Prepare independent native censor holds before performance begins.
    /// Takes no arguments; returns fixed pointer, Space, assistive, Enter and touch ownership.
    pub(super) fn new() -> Self {
        Self {
            source: crate::engine::midi::next_source_id(),
            owners: [[None; 5]; DECKS],
            media: [0; DECKS],
            epoch: 0,
            pending: [None; DECKS],
        }
    }
    fn held(&self, deck: usize) -> bool {
        self.owners[deck].iter().any(Option::is_some)
    }
}

/// Describe the confirmed direction without changing the source waveform.
/// Takes a renderer snapshot; returns the active censor/reverse/scratch mode or forward transport.
pub(super) fn direction(snapshot: &crate::engine::DeckSnap) -> &'static str {
    if snapshot.controls.bleep {
        "← Censor"
    } else if snapshot.controls.reverse {
        "← Reverse"
    } else if snapshot.playback_rate < 0.0 {
        "← Scratch"
    } else {
        "Forward →"
    }
}

impl App {
    /// Admit one native censor edge while preserving every independent release.
    /// Takes exact deck, route, gate and original viewport; changes local ownership only after required renderer admission succeeds.
    pub(super) fn set_censor_input(
        &mut self,
        deck: u8,
        route: usize,
        on: bool,
        viewport: egui::ViewportId,
    ) {
        let index = usize::from(deck);
        if index >= DECKS || route >= 5 {
            return;
        }
        let owner = self.deck_direction.owners[index][route];
        if on && owner.is_some() || !on && owner != Some(viewport) {
            return;
        }
        let held = self.deck_direction.held(index);
        let final_release = !on
            && self.deck_direction.owners[index]
                .iter()
                .filter(|owner| owner.is_some())
                .count()
                == 1;
        if (on && !held || final_release)
            && !self.submit(Command::DeckControl {
                source: self.deck_direction.source,
                deck,
                control: Control::Hold {
                    button: Button::Bleep,
                    on,
                },
            })
        {
            return;
        }
        self.deck_direction.owners[index][route] = on.then_some(viewport);
    }
    /// Release original native censor owners at deliberate application boundaries.
    /// Takes this app; retries priority release admission without borrowing a controller owner.
    pub(super) fn release_censor_inputs(&mut self) {
        for deck in 0..DECKS {
            for route in 0..5 {
                if let Some(owner) = self.deck_direction.owners[deck][route] {
                    self.set_censor_input(deck as u8, route, false, owner);
                }
            }
            self.deck_direction.pending[deck] = None;
        }
    }
    /// Retire native censor holds when their source, safety epoch or viewport ends.
    /// Takes the native context and ordinary input-blocking state; retains holds owned by another active viewport.
    pub(super) fn guard_censor_inputs(&mut self, ctx: &egui::Context, blocked: bool) {
        let epoch = self.engine.cmd.performance().input_epoch();
        let changed = epoch != self.deck_direction.epoch;
        let viewport = ctx.viewport_id();
        for deck in 0..DECKS {
            let media = self.snap.decks[deck].media_key;
            let replaced = media != self.deck_direction.media[deck];
            for route in 0..5 {
                let Some(owner) = self.deck_direction.owners[deck][route] else {
                    continue;
                };
                let ended = owner == viewport
                    && (!ctx.input(|input| input.focused)
                        || match route {
                            0 => !ctx.input(|input| input.pointer.primary_down()),
                            1 => !ctx.input(|input| input.key_down(Key::Space)),
                            3 => !ctx.input(|input| input.key_down(Key::Enter)),
                            _ => false,
                        });
                if blocked || changed || replaced || ended {
                    self.set_censor_input(deck as u8, route, false, owner);
                }
            }
            if blocked || changed || replaced || !ctx.input(|input| input.focused) {
                self.deck_direction.pending[deck] = None;
            }
            self.deck_direction.media[deck] = media;
        }
        self.deck_direction.epoch = epoch;
    }
    /// Draw a native reverse latch and source-owned momentary censor.
    /// Takes deck, confirmed snapshot and theme; sends ordinary guarded controls and captures original mouse/key/assistive/touch edges.
    pub(super) fn deck_direction_controls(
        &mut self,
        ui: &mut Ui,
        deck: u8,
        snapshot: &crate::engine::DeckSnap,
    ) {
        let index = usize::from(deck);
        let enabled = snapshot.frames > 0.0
            && !self.engine.safe_mode()
            && !self.engine.cmd.performance().status().recovery
            && !self.project.committing();
        ui.push_id(("deck-direction",deck),|ui|{
            ui.horizontal(|ui|{
                let reverse=ui.add_enabled(enabled,egui::Button::new("REV").small().selected(snapshot.controls.reverse_latched));
                accessibility::button(ui,&reverse,"Reverse playback",Some(snapshot.controls.reverse_latched));
                help::annotate(ui,&reverse,HelpControl::DeckDirection);
                if reverse.clicked(){self.send(Command::DeckControl{source:self.deck_direction.source,deck,control:Control::Reverse{enabled:!snapshot.controls.reverse_latched}});}
                let response=ui.add_enabled(enabled,egui::Button::new("CENSOR").small().selected(snapshot.controls.bleep));
                let viewport=ui.ctx().viewport_id();let pass=ui.ctx().cumulative_pass_nr();
                if let Some((owner,media,armed))=self.deck_direction.pending[index] {if owner==viewport&&armed<pass {self.deck_direction.pending[index]=None;if enabled&&media==self.deck_direction.media[index]&&ui.input(|input|input.focused)&&!keyboard::dialogs_block_input(ui.ctx()){self.set_censor_input(deck,2,true,viewport);}}}
                let mouse=touch::register(ui,&response,touch::Target::Censor(deck),response.rect,enabled);
                let focused=response.has_focus()&&enabled&&ui.input(|input|input.focused);
                if !mouse.down||mouse.released||!enabled||!ui.input(|input|input.focused){self.set_censor_input(deck,0,false,viewport);}
                if mouse.pressed&&mouse.starts_here(&response)&&enabled&&ui.input(|input|input.focused){response.request_focus();self.set_censor_input(deck,0,true,viewport);if !mouse.down{self.set_censor_input(deck,0,false,viewport);}}
                if !focused{self.set_censor_input(deck,1,false,viewport);self.set_censor_input(deck,3,false,viewport);}
                let events=ui.input(|input|input.events.iter().filter_map(|event|match event{egui::Event::Key{key:key @ (Key::Space|Key::Enter),pressed,repeat:false,modifiers,..} if !pressed||*modifiers==egui::Modifiers::NONE=>Some((*key,*pressed)),_=>None}).collect::<Vec<_>>());
                for(key,on)in events {let route=if key==Key::Space{1}else{3};if !on||focused{self.set_censor_input(deck,route,on,viewport);}if focused&&on{keyboard::block_for_activation(ui.ctx());}}
                accessibility::button(ui,&response,"Hold Censor",Some(snapshot.controls.bleep));
                let action=accessibility::actions(ui,&response,&["Press Censor","Release Censor"]);
                let click=ui.input(|input|input.num_accesskit_action_requests(response.id,egui::accesskit::Action::Click)>0);
                if action==Some(1)||click&&self.deck_direction.owners[index][2].is_some(){self.deck_direction.pending[index]=None;self.set_censor_input(deck,2,false,viewport);}
                else if enabled&&(action==Some(0)||click){if keyboard::dialogs_block_input(ui.ctx()){self.deck_direction.pending[index]=Some((viewport,self.deck_direction.media[index],pass));}else{self.set_censor_input(deck,2,true,viewport);}}
                help::annotate(ui,&response,HelpControl::DeckDirection);
                ui.ctx().accesskit_node_builder(response.id,|node|node.set_description("Hold to reverse temporarily, then resume the advancing source timeline. Hold mouse, Space, Enter or touch; assistive Click toggles a hold. Press Censor and Release Censor are explicit actions. Losing focus, source or safety retires the original local hold."));
                let label=ui.add(egui::Label::new(RichText::new(direction(snapshot)).small()).truncate());
                ui.ctx().accesskit_node_builder(label.id,|node|{node.set_label(format!("Deck {}: Playback direction",(b'A'+deck)as char));node.set_value(direction(snapshot));node.set_description("Confirmed renderer direction; native latch and independent held controls remain separate.");});
            });
        });
    }
}

#[cfg(test)]
mod tests;
