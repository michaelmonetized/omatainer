//! Validate the wire addresses that dispatch actually matches, rather than
//! treating decoder flavor (absolute/relative CC) as a separate MIDI address.
use super::{Action, Binding, MidiMap, MsgKind};

fn message_class(kind: MsgKind) -> u8 {
    match kind {
        MsgKind::Note => 0x90,
        MsgKind::Cc | MsgKind::CcRel => 0xb0,
        MsgKind::Pitch => 0xe0,
    }
}

fn overlaps(first: &Binding, second: &Binding) -> bool {
    message_class(first.kind) == message_class(second.kind)
        && (first.ch == second.ch || first.ch == 0xff || second.ch == 0xff)
        && (first.kind == MsgKind::Pitch || first.data == second.data)
}

impl MidiMap {
    pub fn validate(&self) -> anyhow::Result<()> {
        for (index, binding) in self.bindings.iter().enumerate() {
            anyhow::ensure!(
                binding.ch < 16 || binding.ch == 0xff,
                "MIDI profile {:?}: binding {index} has invalid channel {}",
                self.name,
                binding.ch
            );
            anyhow::ensure!(
                binding.data < 128,
                "MIDI profile {:?}: binding {index} has invalid data byte {}",
                self.name,
                binding.data
            );
            anyhow::ensure!(
                match (binding.kind, binding.relative) {
                    (MsgKind::CcRel, Some(spec)) => spec.is_valid(),
                    (MsgKind::CcRel, None) => false,
                    (_, None) => true,
                    (_, Some(_)) => false,
                },
                "MIDI profile {:?}: binding {index} has invalid relative encoding/scale metadata",
                self.name
            );
            anyhow::ensure!(
                !matches!(binding.action,Action::Browse | Action::BrowseCrates) || (binding.kind == MsgKind::CcRel
                    && binding.relative.is_some_and(|spec| spec.scale == 1.0)),
                "MIDI profile {:?}: binding {index} Browse requires an explicit relative encoding and one row per wire step",
                self.name
            );
            anyhow::ensure!(match binding.action {
                Action::Scene => binding.extra < crate::engine::session::MAX_SCENES as u16,
                Action::Clip => usize::from(binding.deck) < crate::engine::session::MAX_TRACKS && binding.extra < crate::engine::session::MAX_SCENES as u16,
                Action::TrackFader | Action::TrackMute => binding.extra < crate::engine::session::MAX_TRACKS as u16,
                Action::DeckHotCue => usize::from(binding.extra) < crate::engine::HOTCUES,
                Action::FxWet | Action::FxSelect => binding.extra < 3,
                _ => true,
            }, "MIDI profile {:?}: binding {index} target is outside its resource limit", self.name);
            for (previous, other) in self.bindings[..index].iter().enumerate() {
                anyhow::ensure!(
                    !overlaps(other, binding),
                    "MIDI profile {:?}: bindings {previous} and {index} overlap: {other:?} and {binding:?}",
                    self.name
                );
            }
        }
        Ok(())
    }
}
