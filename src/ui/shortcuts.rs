//! One binding table supplies dispatch, in-app help and checked README rows.
use super::*;
use egui::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Transport,
    Play(u8),
    Cue(u8),
    Sync(u8),
    Scene(u16),
    Crossfader(u8),
    BeatJump(bool),
    BeatJumpScale(bool),
    Load,
    Help,
    Midi,
    CloseFx,
    Undo,
    Redo,
}

impl Action {
    /// Name the keyboard context that owns this action outside text entry.
    /// Takes an action; returns its discoverable editor, navigation or performance context.
    pub(super) fn context(self) -> &'static str {
        match self {
            Self::Undo | Self::Redo | Self::CloseFx => "Editor",
            Self::Help | Self::Midi => "Navigation",
            _ => "Performance",
        }
    }
}

pub(super) struct Binding {
    pub key: Key,
    pub modifiers: Modifiers,
    pub label: &'static str,
    pub description: &'static str,
    pub action: Action,
}

const fn b(
    key: Key,
    modifiers: Modifiers,
    label: &'static str,
    description: &'static str,
    action: Action,
) -> Binding {
    Binding {
        key,
        modifiers,
        label,
        description,
        action,
    }
}

pub(super) const BINDINGS: &[Binding] = &[
    b(
        Key::Space,
        Modifiers::NONE,
        "Space",
        "Play / stop session",
        Action::Transport,
    ),
    b(
        Key::Num1,
        Modifiers::NONE,
        "1",
        "Launch scene 1",
        Action::Scene(0),
    ),
    b(
        Key::Num2,
        Modifiers::NONE,
        "2",
        "Launch scene 2",
        Action::Scene(1),
    ),
    b(
        Key::Num3,
        Modifiers::NONE,
        "3",
        "Launch scene 3",
        Action::Scene(2),
    ),
    b(
        Key::Num4,
        Modifiers::NONE,
        "4",
        "Launch scene 4",
        Action::Scene(3),
    ),
    b(
        Key::Num5,
        Modifiers::NONE,
        "5",
        "Launch scene 5",
        Action::Scene(4),
    ),
    b(
        Key::Num6,
        Modifiers::NONE,
        "6",
        "Launch scene 6",
        Action::Scene(5),
    ),
    b(
        Key::Num7,
        Modifiers::NONE,
        "7",
        "Launch scene 7",
        Action::Scene(6),
    ),
    b(
        Key::Num8,
        Modifiers::NONE,
        "8",
        "Launch scene 8",
        Action::Scene(7),
    ),
    b(
        Key::Q,
        Modifiers::NONE,
        "Q",
        "Play / pause deck A",
        Action::Play(0),
    ),
    b(Key::A, Modifiers::NONE, "A", "Cue deck A", Action::Cue(0)),
    b(
        Key::W,
        Modifiers::NONE,
        "W",
        "Toggle deck A tempo sync",
        Action::Sync(0),
    ),
    b(
        Key::P,
        Modifiers::NONE,
        "P",
        "Play / pause deck B",
        Action::Play(1),
    ),
    b(Key::L, Modifiers::NONE, "L", "Cue deck B", Action::Cue(1)),
    b(
        Key::O,
        Modifiers::NONE,
        "O",
        "Toggle deck B tempo sync",
        Action::Sync(1),
    ),
    b(
        Key::OpenBracket,
        Modifiers::NONE,
        "[",
        "Crossfader fully to A",
        Action::Crossfader(0),
    ),
    b(
        Key::CloseBracket,
        Modifiers::NONE,
        "]",
        "Crossfader fully to B",
        Action::Crossfader(1),
    ),
    b(
        Key::F,
        Modifiers::NONE,
        "F",
        "Load selected crate item onto selected deck",
        Action::Load,
    ),
    b(
        Key::Slash,
        Modifiers::SHIFT,
        "? (Shift+/)",
        "Show / hide contextual help and lessons",
        Action::Help,
    ),
    b(Key::OpenBracket, Modifiers::SHIFT, "Shift+[", "Beat jump backward on selected deck", Action::BeatJump(false)),
    b(Key::CloseBracket, Modifiers::SHIFT, "Shift+]", "Beat jump forward on selected deck", Action::BeatJump(true)),
    b(Key::OpenBracket, Modifiers::ALT, "Alt+[", "Smaller beat jump on selected deck", Action::BeatJumpScale(false)),
    b(Key::CloseBracket, Modifiers::ALT, "Alt+]", "Larger beat jump on selected deck", Action::BeatJumpScale(true)),
    b(
        Key::F1,
        Modifiers::NONE,
        "F1",
        "Show / hide contextual help and lessons",
        Action::Help,
    ),
    b(
        Key::M,
        Modifiers::CTRL,
        "Ctrl+M",
        "Show / hide MIDI window",
        Action::Midi,
    ),
    b(
        Key::Escape,
        Modifiers::NONE,
        "Escape",
        "Close effect chain",
        Action::CloseFx,
    ),
    b(
        Key::Z,
        Modifiers::CTRL,
        "Ctrl+Z",
        "Undo last creative edit",
        Action::Undo,
    ),
    b(
        Key::Z,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        "Ctrl+Shift+Z",
        "Redo next creative edit",
        Action::Redo,
    ),
    b(
        Key::Y,
        Modifiers::CTRL,
        "Ctrl+Y",
        "Redo next creative edit",
        Action::Redo,
    ),
];

impl Binding {
    pub(super) fn id(&self) -> &'static str {
        match self.action {
            Action::Transport => "transport", Action::Play(0) => "play_a", Action::Play(_) => "play_b",
            Action::Cue(0) => "cue_a", Action::Cue(_) => "cue_b", Action::Sync(0) => "sync_a", Action::Sync(_) => "sync_b",
            Action::Scene(0) => "scene_1", Action::Scene(1) => "scene_2", Action::Scene(2) => "scene_3", Action::Scene(3) => "scene_4",
            Action::Scene(4) => "scene_5", Action::Scene(5) => "scene_6", Action::Scene(6) => "scene_7", Action::Scene(_) => "scene_8",
            Action::Crossfader(0) => "crossfader_a", Action::Crossfader(_) => "crossfader_b", Action::Load => "load",
            Action::BeatJump(false) => "beat_jump_back", Action::BeatJump(true) => "beat_jump_forward",
            Action::BeatJumpScale(false) => "beat_jump_smaller", Action::BeatJumpScale(true) => "beat_jump_larger",
            Action::Undo => "undo", Action::Redo if self.key == Key::Z => "redo_shift_z", Action::Redo => "redo_y",
            Action::Help if self.key == Key::F1 => "help_f1", Action::Help => "help_question", Action::Midi => "midi", Action::CloseFx => "close_fx",
        }
    }
    pub(super) fn effective(&self, profile: &crate::preferences::Profile) -> Option<crate::preferences::Shortcut> {
        profile.shortcuts.get(self.id()).cloned().unwrap_or_else(|| Some(crate::preferences::Shortcut {
            key: self.key.name().into(), ctrl: self.modifiers.ctrl, shift: self.modifiers.shift, alt: self.modifiers.alt,
        }))
    }
}

pub(super) fn validate(profile: &crate::preferences::Profile) -> Result<(), String> {
    for id in profile.shortcuts.keys() {
        if !BINDINGS.iter().any(|binding| binding.id() == id) { return Err(format!("Unknown shortcut action {id}; preferences were preserved")); }
    }
    let mut seen = std::collections::BTreeMap::new();
    for binding in BINDINGS {
        if let Some(value) = binding.effective(profile) {
            // Canonicalize aliases accepted by egui before checking conflicts.
            let key = Key::from_name(&value.key).ok_or_else(|| format!("Unknown key {}", value.key))?;
            let identity = (key.name(), value.ctrl, value.shift, value.alt);
            if let Some(other) = seen.insert(identity, binding.description) {
                return Err(format!("{} and {other} both use {}", binding.description, value.label()));
            }
        }
    }
    Ok(())
}

/// Preserve old shortcut owners when introducing beat jump defaults.
/// Takes a legacy profile; disables each new default chord already owned by an existing action.
pub(super) fn migrate_beat_jump(profile: &mut crate::preferences::Profile) {
    for binding in BINDINGS.iter().filter(|b| matches!(b.action, Action::BeatJump(_) | Action::BeatJumpScale(_))) {
        let occupied = BINDINGS.iter().filter(|b| !matches!(b.action, Action::BeatJump(_) | Action::BeatJumpScale(_)))
            .filter_map(|b| b.effective(profile)).any(|value| Key::from_name(&value.key) == Some(binding.key)
                && value.ctrl == binding.modifiers.ctrl && value.shift == binding.modifiers.shift && value.alt == binding.modifiers.alt);
        if occupied { profile.shortcuts.insert(binding.id().into(), None); }
    }
}

#[cfg(test)]
pub(super) fn lookup(key: Key, modifiers: Modifiers, repeat: bool) -> Option<Action> {
    lookup_with(&crate::preferences::Profile::defaults(std::path::Path::new("/tmp")), key, modifiers, repeat)
}
pub(super) fn lookup_with(profile: &crate::preferences::Profile, key: Key, modifiers: Modifiers, repeat: bool) -> Option<Action> {
    if repeat || !profile.shortcuts_enabled { return None; }
    BINDINGS.iter().find_map(|binding| {
        let value = binding.effective(profile)?;
        let expected = value.modifiers();
        (Key::from_name(&value.key) == Some(key) && !modifiers.mac_cmd && (!modifiers.command || modifiers.ctrl)
            && modifiers.ctrl == expected.ctrl && modifiers.shift == expected.shift && modifiers.alt == expected.alt)
            .then_some(binding.action)
    })
}

pub(super) fn action_label(profile: &crate::preferences::Profile, action: Action) -> String {
    if !profile.shortcuts_enabled { return String::new(); }
    BINDINGS.iter().filter(|binding| binding.action == action)
        .find_map(|binding| binding.effective(profile).map(|value| value.label()))
        .unwrap_or_default()
}

pub(super) fn show_help_with(ui: &mut Ui, profile: &crate::preferences::Profile) {
    ui.label(tr!("Ctrl+, opens Preferences and profiles. This setup shortcut is always available outside text editing and dialogs."));
    ui.label(tr!("Tab / Shift+Tab traverses controls. Enter or Space activates the focused control."));
    ui.label(tr!("Numeric controls: arrows adjust, Shift is fine adjustment, Home/End choose limits, F2 enters a value."));
    ui.label(tr!("Shift+F10 opens alternate actions, including cue deletion, looping, solo, compose arming and clip gain."));
    ui.label(tr!("Pads: hold Space or Enter; release or move focus to stop. Assistive click toggles a hold; Press/Release actions are also available."));
    ui.label(tr!("Crate: Up/Down, Page Up/Down, Home/End browse all filtered rows; F2 enters a row number, Enter loads the selected deck."));
    egui::Grid::new("shortcut-help")
        .striped(true)
        .show(ui, |ui| {
            for binding in BINDINGS {
                ui.monospace(if profile.shortcuts_enabled { if profile.shortcuts.contains_key(binding.id()) { binding.effective(profile).map(|s| s.label()).unwrap_or_else(|| "Disabled".into()) } else { binding.label.into() } } else { "Disabled".into() });
                ui.label(binding.description);
                ui.end_row();
            }
        });
}

impl App {
    pub(super) fn dispatch_shortcut(&mut self, action: Action) {
        if self.engine.safe_mode() && !matches!(action,Action::Help|Action::Midi) {
            self.status="Safe mode keeps engine controls offline. Use Project Open/recovery/Save, or Restart normally.".into();
            return;
        }
        match action {
            Action::Transport => self.send(Command::TogglePlay),
            Action::Play(deck) => self.send(Command::DeckPlay { deck }),
            Action::Cue(deck) => self.send(Command::DeckCue { deck }),
            Action::Sync(deck) => self.send(Command::DeckSync { deck }),
            Action::Scene(position) => {
                let scene=self.snap.session.as_ref().and_then(|layout|layout.scene_order.get(usize::from(position))).copied();
                if let Some(scene)=scene { self.send(Command::LaunchScene { scene }); }
            },
            Action::Crossfader(deck) => self.send(Command::Xfader(deck as f32)),
            Action::BeatJump(forward) => self.send(Command::DeckControl { source: 0, deck: self.load_target() as u8,
                control: crate::engine::deck_controls::Control::BeatJump { forward } }),
            Action::BeatJumpScale(up) => self.send(Command::DeckControl { source: 0, deck: self.load_target() as u8,
                control: crate::engine::deck_controls::Control::BeatJumpScale { up } }),
            Action::Load => self.load_sel(self.load_target() as u8),
            Action::Help => self.keys_open = !self.keys_open,
            Action::Midi => self.midi_open = !self.midi_open,
            Action::CloseFx => self.send(Command::CloseFx),
            Action::Undo => self.history_action(false),
            Action::Redo => self.history_action(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_table_has_unique_chords_and_supplies_readme_and_help() {
        for (index, binding) in BINDINGS.iter().enumerate() {
            assert!(!BINDINGS[..index]
                .iter()
                .any(|other| other.key == binding.key && other.modifiers == binding.modifiers));
            assert_eq!(
                lookup(binding.key, binding.modifiers, false),
                Some(binding.action)
            );
            assert_eq!(lookup(binding.key, binding.modifiers, true), None);
        }
        let rows = BINDINGS
            .iter()
            .map(|binding| format!("| `{}` | {} |\n", binding.label, binding.description))
            .collect::<String>();
        let expected = format!("<!-- app-shortcuts:start -->\n| Keys | Action |\n| --- | --- |\n{rows}<!-- app-shortcuts:end -->");
        assert!(include_str!("../../README.md").contains(&expected));
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, show_help);
        });
        for binding in BINDINGS {
            test_support::label_center(&output, binding.label);
            test_support::label_center(&output, binding.description);
        }
    }
}

#[cfg(test)]
pub(super) fn show_help(ui: &mut Ui) { show_help_with(ui, &crate::preferences::Profile::defaults(std::path::Path::new("/tmp"))); }
