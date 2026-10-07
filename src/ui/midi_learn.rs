use super::*;
use crate::engine::midi::learn::{self, Config, Endpoint};
use crate::engine::midi::{Action, Binding, MsgKind, RelativeSpec};

pub(super) struct Panel {
    pub(super) presets: super::midi_presets::Panel,
    binding: Binding,
    endpoint: Option<Endpoint>,
    selected: Option<(u64, usize)>,
    message: String,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            presets: super::midi_presets::Panel::default(),
            binding: Binding {
                kind: MsgKind::Note,
                ch: 0,
                data: 0,
                action: Action::DeckPlay,
                deck: 0,
                extra: 0,
                relative: None,
                controls: None,
                pair_order: None,
            },
            endpoint: None,
            selected: None,
            message: String::new(),
        }
    }
}
fn label(action: Action) -> &'static str {
    match action {
        Action::DeckPlay => "Deck Play",
        Action::DeckCue => "Deck set/return Cue",
        Action::DeckCueHold => "Deck hold Cue audition",
        Action::DeckSync => "Deck Sync",
        Action::DeckJog => "Deck jog",
        Action::DeckJogTouch => "Deck platter touch",
        Action::DeckPitch => "Deck pitch",
        Action::DeckGain => "Deck gain",
        Action::DeckEqHi => "Deck high EQ",
        Action::DeckEqMid => "Deck mid EQ",
        Action::DeckEqLow => "Deck low EQ",
        Action::DeckFilter => "Deck filter",
        Action::DeckPfl => "Deck headphone cue",
        Action::DeckHotCue => "Deck hot cue",
        Action::DeckLoop4 => "Deck four-beat loop",
        Action::DeckLoopIn => "Deck loop in",
        Action::DeckLoopOut => "Deck loop out",
        Action::DeckSavedLoopRecall => "Deck activate saved loop",
        Action::DeckSavedLoopSave => "Deck save loop slot",
        Action::DeckSavedLoopDelete => "Deck delete saved loop",
        Action::DeckBeatJumpBack => "Deck beat jump backward",
        Action::DeckBeatJumpForward => "Deck beat jump forward",
        Action::DeckBeatJumpSmaller => "Deck beat jump smaller",
        Action::DeckBeatJumpLarger => "Deck beat jump larger",
        Action::DeckLoad => "Deck load",
        Action::DeckLoadLock => "Deck load lock",
        Action::DeckVinyl => "Deck vinyl mode",
        Action::Xfader => "Crossfader",
        Action::XfaderCurve => "Crossfader contour",
        Action::Master => "Master gain",
        Action::CueMix => "Headphone mix",
        Action::Browse => "Browse library",
        Action::BrowseCrates => "Browse crates",
        Action::CrateReturn => "Return to previous crate view",
        Action::SamplerSlotStop => "Stop sample slot",
        Action::Prepare => "Prepare selected track",
        Action::PrepareCrate => "Prepare filtered crate",
        Action::LoadA => "Load deck A",
        Action::LoadB => "Load deck B",
        Action::Scene => "Launch scene",
        Action::Clip => "Launch clip",
        Action::TrackFader => "Track gain",
        Action::TrackMute => "Track mute",
        Action::TrackSolo => "Track solo",
        Action::TrackArm => "Track record arm",
        Action::TrackPan => "Track pan",
        Action::TrackSendA => "Track reverb send",
        Action::TrackSendB => "Track echo send",
        Action::Play => "Session Play",
        Action::Stop => "Session Stop",
        Action::Record => "Session Record",
        Action::Tap => "Tap tempo",
        Action::Shift => "Controller Shift",
        Action::FxWet => "Master effect wet",
        Action::FxSelect => "Master effect select",
        Action::SongLocator => "Song named section",
        Action::SongPrevious => "Song previous section",
        Action::SongNext => "Song next section",
        Action::SongLoop => "Song loop toggle",
        Action::SongCancel => "Song cancel queued jump",
    }
}
fn description(mapping: &learn::Mapping) -> String {
    if learn::navigation(mapping.binding.action) {
        return format!("{} / {} · channel {} · {:?} {} → {}{}",mapping.endpoint.name,mapping.endpoint.id,mapping.binding.ch+1,mapping.binding.kind,mapping.binding.data,label(mapping.binding.action),if mapping.binding.action==Action::SongLocator {format!(" · locator ID {}",mapping.binding.extra)} else {String::new()});
    }
    format!(
        "{} / {} · channel {} · {:?} {} → {} · target {} / {}",
        mapping.endpoint.name,
        mapping.endpoint.id,
        mapping.binding.ch + 1,
        mapping.binding.kind,
        mapping.binding.data,
        label(mapping.binding.action),
        mapping.binding.deck + 1,
        mapping.binding.extra + 1
    )
}
impl App {
    fn apply_learn_config(&mut self, config: Config) {
        let handle = self.engine.cmd.midi_learn();
        match handle.configure(config) {
            Ok(_)=>self.midi_learn.message="Assignments applied for this run. Save MIDI assignments to retain them in the active profile.".into(),
            Err(error)=>self.midi_learn.message=error,
        }
    }
    /// Render capture, conflict review and persistent assignment controls.
    /// Takes the native MIDI panel; all hardware input stays on its dispatch worker.
    pub(super) fn midi_learn_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let handle = self.engine.cmd.midi_learn();
        let mut view = handle.view();
        if (view.armed || view.capture.is_some())
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            handle.cancel();
            view = handle.view();
        }
        let allowed = !self.engine.safe_mode()
            && !self.engine.cmd.performance().protected()
            && !self.project.committing();
        if !allowed && (view.armed || view.capture.is_some()) {
            handle.cancel();
        }
        ui.separator();
        ui.heading(tr!("MIDI learn"));
        ui.label(tr!("Learn one exact USB MIDI port and channel. Capture releases older held input, consumes one compatible gesture and leaves other controls available. Cancel restores existing assignments."));
        ui.label(crate::localization::format(
            "Active MIDI profile: {0}",
            &[self.settings.applied.active.clone()],
        ));
        ui.label(&view.message);
        ui.label(&self.midi_learn.message);
        if !allowed {
            ui.label(tr!("Leave performance protection or the current project operation before editing MIDI assignments."));
        }
        ui.add_enabled_ui(allowed, |ui| {
            let previous = self.midi_learn.binding.action;
            egui::ComboBox::from_label(tr!("MIDI action"))
                .selected_text(tr!(label(previous)))
                .show_ui(ui, |ui| {
                    for &action in learn::actions() {
                        let choice = ui.selectable_value(
                            &mut self.midi_learn.binding.action,
                            action,
                            tr!(label(action)),
                        );
                        if choice.clicked() { ui.close(); }
                        choice.help(ui, HelpControl::MidiLearn);
                    }
                })
                .response
                .help(ui, HelpControl::MidiLearn);
            if previous != self.midi_learn.binding.action {
                let binding = &mut self.midi_learn.binding;
                binding.kind = learn::kind(binding.action);
                binding.deck = 0;
                binding.extra = u16::from(binding.action == Action::SongLocator);
                binding.controls = None;
                binding.pair_order = None;
                binding.relative = (binding.kind == MsgKind::CcRel).then_some(RelativeSpec {
                    encoding: crate::engine::midi::RelativeEncoding::OffsetBinary,
                    scale: if matches!(binding.action, Action::Browse | Action::BrowseCrates) {
                        1.0
                    } else {
                        0.35
                    },
                });
            }
            let binding = &mut self.midi_learn.binding;
            if binding.action != Action::SamplerSlotStop && !learn::navigation(binding.action) {
            let mut deck = f32::from(binding.deck) + 1.0;
            preferences::float_control(
                ui,
                "MIDI target deck or track",
                &mut deck,
                1.0,
                if binding.action == Action::Clip {
                    128.0
                } else {
                    2.0
                },
                1.0,
                "",
                HelpControl::MidiLearn,
            );
            binding.deck = deck.round() as u8 - 1;
            }
            if binding.action == Action::SongLocator {
                let mut id = f32::from(binding.extra);
                preferences::float_control(ui, "MIDI target locator ID", &mut id, 1.0, 65535.0, 1.0, "", HelpControl::MidiLearn);
                binding.extra = id.round() as u16;
                ui.label("Use the stable ID shown in Arrangement sections. Moving or renaming a section preserves its mapping.");
            }
            let max = match binding.action {
                Action::SamplerSlotStop => 16.0,
                Action::DeckHotCue | Action::DeckSavedLoopRecall | Action::DeckSavedLoopSave | Action::DeckSavedLoopDelete => 8.0,
                Action::Scene | Action::Clip => 512.0,
                Action::TrackFader | Action::TrackMute | Action::TrackSolo | Action::TrackArm | Action::TrackPan | Action::TrackSendA | Action::TrackSendB => 127.0,
                Action::FxWet | Action::FxSelect => 3.0,
                _ => 1.0,
            };
            if max > 1.0 {
                let mut extra = f32::from(binding.extra) + 1.0;
                preferences::float_control(
                    ui,
                    if learn::saved_loop(binding.action) { "MIDI saved loop slot ID" } else if binding.action == Action::SamplerSlotStop { "MIDI target sample slot" } else { "MIDI target cue, scene, track or effect" },
                    &mut extra,
                    1.0,
                    max,
                    1.0,
                    "",
                    HelpControl::MidiLearn,
                );
                binding.extra = extra.round() as u16 - 1;
            }
            if crate::engine::midi::controls::continuous(binding.action) {
                let previous = binding.kind;
                ui.label("Message format");
                let response = egui::ComboBox::from_id_salt("MIDI message format")
                    .selected_text(match binding.kind { MsgKind::Cc14 => "14-bit CC pair", MsgKind::Pitch => "Pitch bend", MsgKind::CcRel => "Relative CC", _ => "Absolute CC" })
                    .show_ui(ui, |ui| {
                        for (kind, label) in [(MsgKind::Cc, "Absolute CC"), (MsgKind::Cc14, "14-bit CC pair"), (MsgKind::Pitch, "Pitch bend"), (MsgKind::CcRel, "Relative CC")] {
                            if ui.selectable_value(&mut binding.kind, kind, label).clicked() { ui.close(); }
                        }
                    });
                accessibility::button(ui, &response.response, "MIDI message format", None);
                if previous != binding.kind {
                    binding.relative = (binding.kind == MsgKind::CcRel).then_some(RelativeSpec { encoding: crate::engine::midi::RelativeEncoding::OffsetBinary, scale: 0.01 });
                    if binding.kind != MsgKind::Cc14 { binding.pair_order = None; }
                }
            }
            if binding.kind == MsgKind::Cc14 {
                let mut order = binding.pair_order.unwrap_or_default();
                let before = order;
                let response = egui::ComboBox::from_id_salt("CC pair order")
                    .selected_text(match order { crate::engine::midi::PairOrder::MsbFirst => "MSB first (MIDI standard)", crate::engine::midi::PairOrder::LsbFirst => "LSB first (paired)" })
                    .show_ui(ui, |ui| { for (value, label) in [(crate::engine::midi::PairOrder::MsbFirst, "MSB first (MIDI standard)"), (crate::engine::midi::PairOrder::LsbFirst, "LSB first (paired)")] { if ui.selectable_value(&mut order, value, label).clicked() { ui.close(); } } });
                accessibility::button(ui, &response.response, "CC pair order", None);
                if order != before { binding.pair_order = Some(order); }
            }
            if crate::engine::midi::controls::continuous(binding.action) || matches!(binding.action, Action::DeckJog | Action::Browse | Action::BrowseCrates) {
                let before = binding.controls.unwrap_or_default();
                let mut spec = before;
                let response = ui.checkbox(&mut spec.invert, "Invert MIDI direction");
                accessibility::button(ui, &response, "Invert MIDI direction", Some(spec.invert));
                if crate::engine::midi::controls::continuous(binding.action) {
                    let max = if matches!(binding.action, Action::DeckGain | Action::TrackFader | Action::Master) { 1.5 } else { 1.0 };
                    preferences::float_control(ui, "MIDI minimum", &mut spec.min, 0.0, max, 0.01, "", HelpControl::MidiLearn);
                    preferences::float_control(ui, "MIDI maximum", &mut spec.max, 0.0, max, 0.01, "", HelpControl::MidiLearn);
                    ui.label("Minimum must be below maximum. Bend center is 0.5. Standard CC pairs retain the coarse byte for fine updates; a new coarse byte clears the fine value. Capture waits for both bytes. Choose reverse order only when your controller sends it.");
                }
                if spec != before { binding.controls = Some(spec); }
            }
            if let Some(relative) = &mut binding.relative {
                let format = egui::ComboBox::from_id_salt("Relative encoder format")
                    .selected_text(format!("{:?}", relative.encoding))
                    .show_ui(ui, |ui| {
                        for (value, text) in [
                            (
                                crate::engine::midi::RelativeEncoding::OffsetBinary,
                                "Offset binary: 64 is stationary",
                            ),
                            (crate::engine::midi::RelativeEncoding::SignedBit, "Signed bit: 0/64 stationary; 1 forward, 65 backward"),
                            (
                                crate::engine::midi::RelativeEncoding::TwosComplement,
                                "Two's complement: 0 is stationary",
                            ),
                        ] {
                            let response = ui.selectable_value(&mut relative.encoding, value, tr!(text));
                            if response.clicked() { ui.close(); }
                            response.help(ui, HelpControl::MidiLearn);
                        }
                    })
                    ;
                accessibility::button(ui, &format.response, "Relative encoder format", None);
                format.response.help(ui, HelpControl::MidiLearn);
                if matches!(binding.action, Action::Browse | Action::BrowseCrates) {
                    relative.scale = 1.0;
                } else {
                    preferences::float_control(
                        ui,
                        "MIDI encoder sensitivity",
                        &mut relative.scale,
                        0.01,
                        4.0,
                        0.01,
                        "",
                        HelpControl::MidiLearn,
                    );
                }
            }
            egui::ComboBox::from_label(tr!("Capture MIDI input"))
                .selected_text(
                    self.midi_learn
                        .endpoint
                        .as_ref()
                        .map(|e| format!("{} / {}", e.name, e.id))
                        .unwrap_or_else(|| tr!("Any connected input").into()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.midi_learn.endpoint,
                        None,
                        tr!("Any connected input"),
                    )
                    .help(ui, HelpControl::MidiLearn);
                    for device in &view.devices {
                        ui.selectable_value(
                            &mut self.midi_learn.endpoint,
                            Some(device.endpoint.clone()),
                            format!("{} / {}", device.endpoint.name, device.endpoint.id),
                        )
                        .help(ui, HelpControl::MidiLearn);
                    }
                })
                .response
                .help(ui, HelpControl::MidiLearn);
            if ui.push_id("midi_learn_capture", |ui| {
                ui.button(tr!("Capture MIDI control")).help(ui, HelpControl::MidiLearn).clicked()
            }).inner
            {
                self.midi_learn.message = handle
                    .begin(self.midi_learn.binding, self.midi_learn.endpoint.clone())
                    .err()
                    .unwrap_or_default();
            }
        });
        if view.armed {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if let Some(capture) = &view.capture {
            ui.label(description(&capture.mapping));
            let value = if let Some(value) = capture.value { value } else if capture.bytes[0] & 0xf0 == 0xe0 {
                u16::from(capture.bytes[1]) | u16::from(capture.bytes[2]) << 7
            } else {
                u16::from(capture.bytes[2])
            };
            ui.label(crate::localization::format(
                "Captured MIDI value {0}",
                &[value.to_string()],
            ));
            for conflict in &capture.conflicts {
                ui.label(format!("Existing action: {}", label(conflict.action)));
            }
            ui.label(tr!("Adding preserves other addresses. Replace changes this address only, overriding any built-in action. Remove later restores its built-in action."));
            ui.add_enabled_ui(allowed, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (replace, text) in [
                        (false, "Add MIDI assignment"),
                        (true, "Replace MIDI assignment"),
                    ] {
                        if ui
                            .button(tr!(text))
                            .help(ui, HelpControl::MidiLearn)
                            .clicked()
                        {
                            match handle.assign(view.revision, replace) {
                                Ok(config) => self.apply_learn_config(config),
                                Err(error) => self.midi_learn.message = error,
                            };
                        }
                    }
                    if ui
                        .button(tr!("Test captured MIDI action"))
                        .help(ui, HelpControl::MidiLearn)
                        .clicked()
                    {
                        self.midi_learn.message = self
                            .engine
                            .midi
                            .preview_learn(&self.engine.cmd, capture)
                            .unwrap_or_else(|error| error);
                    }
                });
            });
        }
        if ui
            .button(tr!("Cancel MIDI learning"))
            .help(ui, HelpControl::MidiLearn)
            .clicked()
        {
            handle.cancel();
        }
        ui.label(tr!("Live learned assignments"));
        let previous = self.midi_learn.selected;
        egui::ScrollArea::vertical()
            .id_salt("learned-midi-assignments")
            .max_height(150.0)
            .show_rows(ui, 24.0, view.config.mappings.len(), |ui, range| {
                for index in range {
                    ui.selectable_value(
                        &mut self.midi_learn.selected,
                        Some((view.revision, index)),
                        description(&view.config.mappings[index]),
                    )
                    .help(ui, HelpControl::MidiLearn);
                }
            });
        if previous != self.midi_learn.selected {
            if let Some(mapping) = self.midi_learn.selected.and_then(|(revision, index)| {
                (revision == view.revision)
                    .then(|| view.config.mappings.get(index))
                    .flatten()
            }) {
                self.midi_learn.binding = mapping.binding;
                self.midi_learn.endpoint = Some(mapping.endpoint.clone());
            }
        }
        ui.add_enabled_ui(allowed, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.midi_learn.selected.is_some_and(|(revision, i)| {
                            revision == view.revision && i < view.config.mappings.len()
                        }),
                        egui::Button::new(tr!("Update selected MIDI action")),
                    )
                    .help(ui, HelpControl::MidiLearn)
                    .clicked()
                {
                    let (_, index) = self.midi_learn.selected.unwrap();
                    let mut config = view.config.clone();
                    let original = config.mappings[index].binding;
                    let target = self.midi_learn.binding;
                    let same_class = (original.kind == MsgKind::Note)
                        == (target.kind == MsgKind::Note)
                        && (original.kind == MsgKind::Pitch) == (target.kind == MsgKind::Pitch);
                    if !same_class {
                        self.midi_learn.message =
                            "Capture a compatible message to change this address's message family."
                                .into();
                        return;
                    }
                    config.mappings[index].binding = Binding {
                        kind: target.kind,
                        ch: original.ch,
                        data: original.data,
                        action: target.action,
                        deck: target.deck,
                        extra: target.extra,
                        relative: target.relative,
                        controls: target.controls,
                        pair_order: target.pair_order,
                    };
                    self.apply_learn_config(config);
                }
                if ui
                    .add_enabled(
                        self.midi_learn.selected.is_some_and(|(revision, i)| {
                            revision == view.revision && i < view.config.mappings.len()
                        }),
                        egui::Button::new(tr!("Remove selected MIDI assignment")),
                    )
                    .help(ui, HelpControl::MidiLearn)
                    .clicked()
                {
                    let mut config = view.config.clone();
                    config
                        .mappings
                        .remove(self.midi_learn.selected.take().unwrap().1);
                    self.apply_learn_config(config);
                }
                if ui
                    .add_enabled(
                        !self.settings.busy()
                            && !self.settings.blocked
                            && self.settings.worker.is_some()
                            && self.settings.draft == self.settings.applied,
                        egui::Button::new(tr!("Save MIDI assignments")),
                    )
                    .help(ui, HelpControl::MidiLearn)
                    .clicked()
                {
                    let mut preferences = self.settings.applied.clone();
                    let active = preferences.active.clone();
                    preferences.profiles.get_mut(&active).unwrap().midi_learn =
                        handle.view().config;
                    self.settings
                        .request(crate::preferences::worker::Job::Save {
                            preferences,
                            revision: self.settings.revision.clone(),
                        });
                    self.midi_learn.message =
                        "Saving active-profile MIDI assignments; wait for the preferences receipt."
                            .into();
                }
                if ui
                    .button(tr!("Restore saved MIDI assignments"))
                    .help(ui, HelpControl::MidiLearn)
                    .clicked()
                {
                    self.apply_learn_config(self.settings.profile().midi_learn.clone());
                }
            });
        });
        self.midi_presets_ui(ui, allowed);
        ui.label(&self.settings.message);
        if self.settings.draft != self.settings.applied {
            ui.label(tr!(
                "Finish or cancel preference edits before saving MIDI assignments."
            ));
        }
        let saved = &self.settings.profile().midi_learn;
        if saved != &view.config {
            ui.label(tr!(
                "Live MIDI assignments differ from the saved active profile."
            ));
        }
        for mapping in &view.config.mappings {
            if !view.devices.iter().any(|d| d.endpoint == mapping.endpoint) {
                ui.label(format!(
                    "Unavailable MIDI assignment: {}",
                    description(mapping)
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod encoder_tests;
