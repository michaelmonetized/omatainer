//! One binding table supplies dispatch, in-app help and checked README rows.
use super::*;
use egui::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Transport,
    Play(u8),
    Cue(u8),
    Sync(u8),
    Scene(u8),
    Crossfader(u8),
    Load,
    Help,
    Midi,
    CloseFx,
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
        "Show / hide shortcut help",
        Action::Help,
    ),
    b(
        Key::F1,
        Modifiers::NONE,
        "F1",
        "Show / hide shortcut help",
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
];

pub(super) fn lookup(key: Key, modifiers: Modifiers, repeat: bool) -> Option<Action> {
    (!repeat)
        .then(|| {
            BINDINGS.iter().find(|binding| {
                binding.key == key
                    && modifiers.mac_cmd == binding.modifiers.mac_cmd
                    && if binding.modifiers.is_none() {
                        modifiers.is_none()
                    } else {
                        modifiers.matches_exact(binding.modifiers)
                    }
            })
        })
        .flatten()
        .map(|binding| binding.action)
}

pub(super) fn show_help(ui: &mut Ui) {
    egui::Grid::new("shortcut-help")
        .striped(true)
        .show(ui, |ui| {
            for binding in BINDINGS {
                ui.monospace(binding.label);
                ui.label(binding.description);
                ui.end_row();
            }
        });
}

impl App {
    pub(super) fn dispatch_shortcut(&mut self, action: Action) {
        match action {
            Action::Transport => self.send(Command::TogglePlay),
            Action::Play(deck) => self.send(Command::DeckPlay { deck }),
            Action::Cue(deck) => self.send(Command::DeckCue { deck }),
            Action::Sync(deck) => self.send(Command::DeckSync { deck }),
            Action::Scene(scene) => self.send(Command::LaunchScene { scene }),
            Action::Crossfader(deck) => self.send(Command::Xfader(deck as f32)),
            Action::Load => self.load_sel(self.load_target() as u8),
            Action::Help => self.keys_open = !self.keys_open,
            Action::Midi => self.midi_open = !self.midi_open,
            Action::CloseFx => self.send(Command::CloseFx),
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
