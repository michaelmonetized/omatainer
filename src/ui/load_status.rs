use super::*;
use crate::engine::load_receipt::{Receipt, State};
use crate::engine::media_load::LoadToken;

pub(super) struct LoadState {
    pub approval: Option<crate::engine::performance::DeckApproval>,
    pub deck_generation: Option<u64>,
    pub selection: Option<Selection>,
    pub phase: Phase,
    pub token: Option<LoadToken>,
    pub receipt: Option<Receipt>,
    pub bpm: Option<Bpm>,
    pub metadata: Option<library_metadata::Patch>,
    pub warning: Option<&'static str>,
}

pub(super) enum Phase {
    Loading,
    Queued,
    Loaded,
    Failed(String),
    Superseded,
}

impl LoadState {
    pub fn new(selection: Option<Selection>, phase: Phase) -> Self {
        Self {
            approval: None,
            deck_generation: None,
            selection,
            phase,
            token: None,
            receipt: None,
            bpm: None,
            metadata: None,
            warning: None,
        }
    }

    fn text(&self, deck: usize) -> String {
        let name = self
            .selection
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or("no selection");
        let deck = (b'A' + deck as u8) as char;
        let mut text = match &self.phase {
            Phase::Queued if self.selection.is_none() => format!("queued unload → {deck} · waiting for audio engine"),
            Phase::Loaded if self.selection.is_none() => format!("unloaded → {deck}"),
            Phase::Loading => format!("loading {name} → {deck}"),
            Phase::Queued => format!("queued {name} → {deck} · waiting for audio engine"),
            Phase::Loaded => format!("loaded {name} → {deck}"),
            Phase::Failed(error) => format!("load failed on {deck}: {name}: {error}"),
            Phase::Superseded => format!("{name} → {deck} · replaced or unloaded"),
        };
        if let Some(bpm) = self.bpm {
            text.push_str(&format!(
                " · BPM {} ({})",
                bpm.value()
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "unknown".into()),
                bpm.label()
            ));
        }
        if let Some(warning) = self.warning {
            text.push_str(&format!(" · {warning}"));
        }
        text
    }
}

impl App {
    pub(super) fn set_load_state(&mut self, deck: u8, state: LoadState) {
        self.load_revision[usize::from(deck)]=self.load_revision[usize::from(deck)].wrapping_add(1);
        self.update_load_state(deck,state);
    }
    /// Publish progress for the same admitted media job.
    /// Takes its deck and updated state; preserves job ownership across decode completion.
    pub(super) fn update_load_state(&mut self,deck:u8,state:LoadState) {
        self.status = state.text(deck as usize);
        self.loads[deck as usize] = Some(state);
    }

    /// Called even when no decode finishes: application and replacement are
    /// renderer events, not decoder events or title comparisons.
    pub(super) fn poll_load_receipts(&mut self) {
        self.poll_play_history();
        for (deck, load) in self.loads.iter_mut().enumerate() {
            let Some(load) = load else { continue };
            // Decoder cancellation prevents future application; it cannot undo
            // media already claimed/applied. Renderer completion determines its outcome.
            let next = if load.token.as_ref().is_some_and(|token| !token.is_current())
                && !load.receipt.as_ref().is_some_and(|receipt| {
                    matches!(receipt.state(), State::Current | State::Applying)
                }) {
                if matches!(load.phase, Phase::Superseded) {
                    None
                } else {
                    Some(Phase::Superseded)
                }
            } else if let Some(receipt) = &load.receipt {
                match receipt.state() {
                    State::Pending | State::Applying | State::Current
                        if !self.engine.cmd.is_connected() =>
                    {
                        if matches!(load.phase, Phase::Failed(_)) {
                            None
                        } else {
                            Some(Phase::Failed(
                                "audio engine disconnected; media is not current".into(),
                            ))
                        }
                    }
                    State::Current if !matches!(load.phase, Phase::Loaded) => Some(Phase::Loaded),
                    State::Protected if !matches!(load.phase, Phase::Failed(_)) => Some(Phase::Failed(
                        "Deck protection refused this load at the renderer; current media is preserved. Wait for a quiet paused deck, or review a deliberate replacement.".into(),
                    )),
                    State::Unavailable if !matches!(load.phase, Phase::Failed(_)) => Some(
                        Phase::Failed("built-in media is unavailable; media was not loaded".into()),
                    ),
                    State::Superseded if !matches!(load.phase, Phase::Superseded) => {
                        Some(Phase::Superseded)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(phase) = next {
                load.phase = phase;
                self.status = load.text(deck);
            }
        }
        self.reconcile_loaded_metadata();
    }

    pub(super) fn load_status(&mut self, ctx: &egui::Context) {
        self.deck_load_confirmation(ctx);
        // Recheck immediately before rendering, including unloads/replacements
        // processed since the last regular frame poll.
        self.poll_load_receipts();
        let mut retry = None;
        let mut dismiss = None;
        egui::TopBottomPanel::bottom("load-status")
            .resizable(false)
            .frame(egui::Frame::new().fill(self.theme.bg).inner_margin(6.0))
            .show(ctx, |ui| {
                scale_status::show(ui, &self.snap);
                if !self.loads.iter().enumerate().any(|(deck, load)| {
                    load.as_ref()
                        .is_some_and(|load| load.text(deck) == self.status)
                }) {
                    ui.label(RichText::new(&self.status).color(self.theme.fg));
                }
                for (deck, load) in self.loads.iter().enumerate() {
                    let Some(load) = load else { continue };
                    ui.push_id(("deck-load-status", deck), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let failed = matches!(load.phase, Phase::Failed(_));
                            let color = if failed {
                                self.theme.red
                            } else {
                                self.theme.fg
                            };
                            ui.label(RichText::new(load.text(deck)).color(color));
                            if failed && load.selection.is_some() {
                                let response = ui.button(tr!("Retry"));
                                accessibility::button(ui, &response, &format!("Deck {}: Retry media load", (b'A' + deck as u8) as char), None);
                                help::annotate(ui, &response, help::Control::LoadRetry);
                                if response.clicked() { retry = Some((deck as u8, load.selection.clone())); }
                            }
                            if !matches!(load.phase, Phase::Loading | Phase::Queued) {
                                let response = ui.button(tr!("Dismiss"));
                                accessibility::button(ui, &response, &format!("Deck {}: Dismiss load status", (b'A' + deck as u8) as char), None);
                                help::annotate(ui, &response, help::Control::LoadDismiss);
                                if response.clicked() { dismiss = Some(deck); }
                            }
                        });
                        if let Some(Selection {
                            source: LibSource::File(path),
                            ..
                        }) = &load.selection
                        {
                            ui.label(
                                RichText::new(path.to_string_lossy())
                                    .small()
                                    .color(self.theme.fg),
                            );
                        }
                    });
                }
            });
        if let Some(deck) = dismiss {
            self.loads[deck] = None;
            self.status = "Q quant · pads compose · ctrl-gain = fx".into();
        }
        if let Some((deck, selection)) = retry {
            self.load_source(deck, selection.as_ref());
        }
    }
}
