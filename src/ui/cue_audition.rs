use super::*;
use crate::engine::deck_controls::{Button, Control};

pub(super) struct Inputs {
    source: u64,
    pub(super) owners: [[Option<egui::ViewportId>; 6]; DECKS],
    keys: [Option<Key>; DECKS],
    pending: [Option<(egui::ViewportId, u64, u64)>; DECKS],
    media: [u64; DECKS],
    epoch: u64,
}

impl Inputs {
    /// Keep local Cue input independent of MIDI and remote API owners.
    /// Takes no arguments; returns empty, bounded hold ownership for both decks.
    pub(super) fn new() -> Self {
        Self { source: crate::engine::midi::next_source_id(), owners: [[None; 6]; DECKS], keys: [None; DECKS], pending: [None; DECKS], media: [0; DECKS], epoch: 0 }
    }
    fn held(&self, deck: usize) -> bool { self.owners[deck].iter().any(Option::is_some) }
}

impl App {
    /// Merge independent local Cue holds into one admitted renderer gate.
    /// Takes exact deck, input slot, pressed state and native window; changes only that local owner.
    pub(super) fn set_cue_input(&mut self, deck: u8, slot: usize, on: bool, viewport: egui::ViewportId) {
        let index = usize::from(deck);
        if index >= DECKS || slot >= 6 || self.cue_audition.source == 0 { return; }
        let before = self.cue_audition.held(index);
        if on && self.cue_audition.owners[index][slot].is_some()
            || !on && self.cue_audition.owners[index][slot] != Some(viewport) { return; }
        if on && !before && !self.submit(Command::DeckControl { source: self.cue_audition.source, deck, control: Control::Hold { button: Button::Cue, on: true } }) { return; }
        self.cue_audition.owners[index][slot] = on.then_some(viewport);
        if before && !self.cue_audition.held(index) {
            self.send(Command::DeckControl { source: self.cue_audition.source, deck, control: Control::Hold { button: Button::Cue, on: false } });
        }
    }

    /// Retire every local Cue input without touching independent controller owners.
    /// Takes this app; returns no value and requires a new input press afterward.
    pub(super) fn release_cue_inputs(&mut self) {
        for deck in 0..DECKS { self.release_deck_cue_inputs(deck); }
    }
    fn release_deck_cue_inputs(&mut self, deck: usize) {
        for slot in 0..6 {
            if let Some(viewport) = self.cue_audition.owners[deck][slot] { self.set_cue_input(deck as u8, slot, false, viewport); }
        }
        self.cue_audition.keys[deck] = None;
        self.cue_audition.pending[deck] = None;
    }

    /// Retire stale Cue gestures when their source, window or safety boundary changes.
    /// Takes the current native frame and blocking state; preserves holds owned by another window.
    pub(super) fn guard_cue_inputs(&mut self, ctx: &egui::Context, blocked: bool) {
        let epoch = self.engine.cmd.performance().input_epoch();
        if epoch != self.cue_audition.epoch {
            self.release_cue_inputs(); self.cue_audition.epoch = epoch;
        }
        for deck in 0..DECKS {
            let media = self.snap.decks.get(deck).map_or(0, |d| d.media_key);
            if media != self.cue_audition.media[deck] { self.release_deck_cue_inputs(deck); self.cue_audition.media[deck] = media; }
            if blocked || !ctx.input(|input| input.focused) {
                for slot in 0..6 { self.set_cue_input(deck as u8, slot, false, ctx.viewport_id()); }
                if self.cue_audition.owners[deck][5].is_none() { self.cue_audition.keys[deck] = None; }
            }
        }
    }

    /// Release a captured Cue shortcut regardless of current modifiers or selection.
    /// Takes its native window and physical logical key; returns no value.
    pub(super) fn release_cue_key(&mut self, viewport: egui::ViewportId, key: Key) {
        for deck in 0..DECKS {
            if self.cue_audition.keys[deck] == Some(key) && self.cue_audition.owners[deck][5] == Some(viewport) {
                self.set_cue_input(deck as u8, 5, false, viewport); self.cue_audition.keys[deck] = None;
            }
        }
    }
    /// Handle key release before text and dialog shortcut guards.
    /// Takes the current native context; retires only its previously captured Cue keys.
    pub(super) fn release_cue_keys(&mut self, ctx: &egui::Context) {
        let blocked = self.engine.safe_mode() || self.engine.cmd.performance().status().recovery
            || self.project.committing() || !self.project.dialog_is_closed() || keyboard::dialogs_block_input(ctx);
        self.guard_cue_inputs(ctx, blocked);
        for deck in 0..DECKS {
            if let Some(key) = self.cue_audition.keys[deck] {
                if !ctx.input(|input| input.key_down(key)) || !self.settings.profile().shortcuts_enabled { self.release_cue_key(ctx.viewport_id(), key); }
            }
        }
    }
    /// Capture a Cue shortcut's exact deck until that key releases.
    /// Takes its frame, key and admitted deck; returns no value and ignores existing holds.
    pub(super) fn press_cue_key(&mut self, viewport: egui::ViewportId, key: Key, deck: u8) {
        if usize::from(deck) >= DECKS || self.engine.safe_mode() || self.engine.cmd.performance().status().recovery { return; }
        self.set_cue_input(deck, 5, true, viewport);
        if self.cue_audition.owners[usize::from(deck)][5] == Some(viewport) { self.cue_audition.keys[usize::from(deck)] = Some(key); }
    }

    /// Render a source-owned hold-to-audition Cue control.
    /// Takes the native UI, exact deck and loaded state; mouse, keys and assistive actions share admitted edges.
    pub(super) fn deck_cue_audition(&mut self, ui: &mut Ui, deck: u8, loaded: bool) {
        let enabled = loaded && !self.engine.safe_mode() && !self.engine.cmd.performance().status().recovery;
        ui.push_id(("cue-audition", deck), |ui| {
            let response = ui.add_enabled(enabled, egui::Button::new("CUE"));
            let window = ui.ctx().viewport_id();
            let pass = ui.ctx().cumulative_pass_nr();
            if let Some((owner, media, armed)) = self.cue_audition.pending[usize::from(deck)] {
                if owner == window && armed < pass {
                    self.cue_audition.pending[usize::from(deck)] = None;
                    if enabled && media == self.cue_audition.media[usize::from(deck)] && ui.input(|input| input.focused) && !keyboard::dialogs_block_input(ui.ctx()) {
                        self.set_cue_input(deck, 2, true, window);
                    }
                }
            }
            let focused = response.has_focus() && response.enabled() && ui.input(|input| input.focused);
            let mouse = touch::register(ui, &response, touch::Target::Cue(deck), response.rect, enabled);
            if !mouse.down || mouse.released || !response.enabled() || !ui.input(|input| input.focused) { self.set_cue_input(deck, 0, false, window); }
            if mouse.pressed && mouse.starts_here(&response) && enabled && ui.input(|input| input.focused) {
                response.request_focus(); self.set_cue_input(deck, 0, true, window);
                if !mouse.down { self.set_cue_input(deck, 0, false, window); }
            }
            if !focused { self.set_cue_input(deck, 1, false, window); self.set_cue_input(deck, 3, false, window); }
            let events = ui.input(|input| input.events.iter().filter_map(|event| match event {
                egui::Event::Key { key: key @ (Key::Space | Key::Enter), pressed, repeat: false, modifiers, .. }
                    if !pressed || *modifiers == egui::Modifiers::NONE => Some((*key, *pressed)), _ => None,
            }).collect::<Vec<_>>());
            for (key, on) in events {
                let slot = if key == Key::Space { 1 } else { 3 };
                if !on || focused { self.set_cue_input(deck, slot, on, window); }
                if focused && on { keyboard::block_for_activation(ui.ctx()); }
            }
            let held = self.cue_audition.held(usize::from(deck)) || self.snap.decks.get(usize::from(deck)).is_some_and(|d| d.controls.cue_held);
            accessibility::button(ui, &response, "Hold Cue audition", Some(held));
            let action = accessibility::actions(ui, &response, &["Press Cue", "Release Cue", "Set/return Cue"]);
            let click = ui.input(|input| input.num_accesskit_action_requests(response.id, egui::accesskit::Action::Click) > 0);
            if action == Some(1) || click && self.cue_audition.owners[usize::from(deck)][2].is_some() {
                self.cue_audition.pending[usize::from(deck)] = None; self.set_cue_input(deck, 2, false, window);
            }
            else if enabled && (action == Some(0) || click) {
                if keyboard::dialogs_block_input(ui.ctx()) { self.cue_audition.pending[usize::from(deck)] = Some((window, self.cue_audition.media[usize::from(deck)], pass)); }
                else { self.set_cue_input(deck, 2, true, window); }
            }
            else if enabled && action == Some(2) { self.send(Command::DeckCue { deck }); }
            help::annotate(ui, &response, HelpControl::CueAudition);
            ui.ctx().accesskit_node_builder(response.id, |node| node.set_description("From pause, hold to audition and release to return. Press Play during the hold to continue. While playing, Cue stops and returns. Hold Space or Enter; moving focus releases those keys. Assistive Click toggles a visible hold; Press Cue and Release Cue are explicit actions."));
            if held { active_mark(ui.painter(), response.rect, self.theme.fg); }
        });
    }
}

#[cfg(test)]
mod tests;
