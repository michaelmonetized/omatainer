use super::*;
use crate::{engine::key_shift as transpose, musical_key::Key};

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    targets: [Option<Key>; DECKS],
    messages: [String; DECKS],
}

#[cfg(test)]
mod tests;

impl App {
    /// Resolve musical-key provenance from the source actually loaded.
    /// Takes its deck; returns a known exact-version key and provenance, or an explicit unknown result.
    fn loaded_musical_key(&self, deck: usize) -> (Option<Key>, String) {
        let Some(load) = self.loads[deck].as_ref().filter(|load| {
            load.receipt.as_ref().is_some_and(|receipt| {
                receipt.state() == crate::engine::load_receipt::State::Current
                    && receipt.history_key() == self.snap.decks[deck].media_key
            })
        }) else {
            return (
                None,
                "Source key unknown: load a catalog track with a saved or analyzed key".into(),
            );
        };
        let Some(selection) = &load.selection else {
            return (None, "Source key unknown".into());
        };
        let Some(track) = self
            .library_metadata
            .catalog
            .track_for_version(&selection.source, selection.fingerprint)
        else {
            return (
                None,
                "Source key unknown: this source has no catalog key".into(),
            );
        };
        let Some(version) = track
            .versions
            .iter()
            .find(|version| version.fingerprint == selection.fingerprint)
        else {
            return (
                None,
                "Source key unknown: the loaded version is no longer in this catalog".into(),
            );
        };
        let (label, provenance) =
            crate::musical_key::effective(Some(version), "", track.locks.metadata);
        let key = Key::parse(&label);
        (
            key,
            key.map_or_else(
                || format!("Source key unknown · {provenance}"),
                |key| {
                    format!(
                        "Source {} · {} · {provenance}",
                        key.conventional(),
                        key.harmonic()
                    )
                },
            ),
        )
    }

    /// Submit an explicit source-qualified semitone edit.
    /// Takes deck, supported offset and match-lock intent; returns admission and publishes a refusal without changing another deck.
    fn request_key_shift(&mut self, deck: usize, semitones: i8, enable_lock: bool) -> bool {
        let request = match transpose::Request::new(
            deck as u8,
            &self.snap.decks[deck],
            semitones,
            enable_lock,
        ) {
            Ok(request) => request,
            Err(error) => {
                self.key_shift.messages[deck] = error.into();
                return false;
            }
        };
        if self.submit(Command::DeckKeyShift(request)) {
            self.key_shift.messages[deck] = if enable_lock {
                "Target key requested · key lock enabled in the same edit".into()
            } else {
                "Key shift requested; the confirmed setting is shown above".into()
            };
            true
        } else {
            self.key_shift.messages[deck] = "Key shift was refused by command admission".into();
            false
        }
    }

    /// Match one chosen key without retargeting browser selection.
    /// Takes its deck and exact target; returns admission for the closest same-mode shift with key lock, or a visible unknown/mode refusal.
    fn match_chosen_key(&mut self, deck: usize, target: Key) -> bool {
        let Some(source) = self.loaded_musical_key(deck).0 else {
            self.key_shift.messages[deck] =
                "Analyze or correct the loaded source key before matching".into();
            return false;
        };
        match transpose::matching(source, target) {
            Ok(semitones) => self.request_key_shift(deck, semitones, true),
            Err(error) => {
                self.key_shift.messages[deck] = error.into();
                false
            }
        }
    }

    /// Configure independent key shift in the native app.
    /// Takes GUI context; exposes confirmed offsets, reset, source and chosen keys, supported tempo and actual processing/bypass state.
    pub(super) fn key_shift_ui(&mut self, ctx: &egui::Context) {
        if !self.key_shift.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut actions = Vec::new();
        let mut matches = Vec::new();
        egui::Window::new("Deck key shift").id(egui::Id::new("deck-key-shift")).open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("Shift up or down by six semitones without moving the playhead. Reset restores the original-key offset.");
            ui.label("Match chosen key also enables key lock. With lock off, the tempo fader still bends the shifted key. Transposing preserves major or minor.");
            for deck in 0..DECKS {
                ui.push_id(deck, |ui| {
                    let label = (b'A' + deck as u8) as char;
                    let source = self.snap.decks[deck].clone();
                    let (key, provenance) = self.loaded_musical_key(deck);
                    ui.separator(); ui.heading(format!("Deck {label}")); ui.label(provenance);
                    ui.label(format!("Confirmed offset: {:+} semitones", source.key_shift));
                    let state = match source.key_shift_mode {
                        crate::engine::keylock::Mode::Off => "Original-key offset",
                        crate::engine::keylock::Mode::NoMedia => "Armed: no source",
                        crate::engine::keylock::Mode::Stopped => "Armed: deck stopped",
                        crate::engine::keylock::Mode::Unity | crate::engine::keylock::Mode::Locked => "Key shift active",
                        crate::engine::keylock::Mode::ScratchBypass => "Scratch bypass: pitch follows the vinyl",
                        crate::engine::keylock::Mode::UnsupportedRate => "Tempo or reverse bypass: requested shift is not being heard",
                    };
                    ui.label(state);
                    if let Some(result) = key.and_then(|key| transpose::shifted(key, source.key_shift)) {
                        ui.label(format!("Requested result: {} · {}", result.conventional(), result.harmonic()));
                    }
                    let (low, high) = transpose::tempo_range(source.keylock, source.key_shift);
                    ui.label(format!("Shift range ±6 semitones · supported forward tempo {:.1}–{:.1}% · scratch and reverse bypass", low * 100.0, high * 100.0));
                    ui.horizontal(|ui| {
                        for (text, name, value) in [("−", "Key shift down", source.key_shift - 1), ("+", "Key shift up", source.key_shift + 1), ("Reset", "Reset key shift", 0)] {
                            let response = ui.add_enabled(source.frames > 0.0 && (-transpose::MAX_SEMITONES..=transpose::MAX_SEMITONES).contains(&value), egui::Button::new(text));
                            accessibility::button(ui, &response, &format!("Deck {label}: {name}"), None); help::annotate(ui, &response, HelpControl::KeyShift);
                            if response.clicked() { actions.push((deck, value, false)); }
                        }
                    });
                    let target = &mut self.key_shift.targets[deck];
                    if target.is_none() { *target = key; }
                    let response = egui::ComboBox::from_id_salt("target-key").selected_text(target.map_or_else(|| "Choose target key".into(), |key| format!("{} · {}", key.conventional(), key.harmonic()))).show_ui(ui, |ui| {
                        for minor in [false, true] { for tonic in 0..12 { let key = Key { tonic, minor }; ui.selectable_value(target, Some(key), format!("{} · {}", key.conventional(), key.harmonic())); } }
                    }).response;
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, format!("Deck {label}: Target key"))); accessibility::focus(ui, &response); help::annotate(ui, &response, HelpControl::KeyShift);
                    let selected = *target;
                    let response = ui.add_enabled(selected.is_some(), egui::Button::new("Match chosen key"));
                    accessibility::button(ui, &response, &format!("Deck {label}: Match chosen key"), None); help::annotate(ui, &response, HelpControl::KeyShift);
                    if response.clicked() { if let Some(target) = selected { matches.push((deck,target)); } }
                    ui.label(&self.key_shift.messages[deck]);
                });
            }
        });
        self.key_shift.open = open;
        for (deck, value, lock) in actions {
            self.request_key_shift(deck, value, lock);
        }
        for (deck, target) in matches {
            self.match_chosen_key(deck, target);
        }
    }
}
