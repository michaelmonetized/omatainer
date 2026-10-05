use super::super::{cbind, nbind, Action, MidiMap, UnmappedNotes};
use crate::engine::{Command, CommandPort};

/// Apply standard Machine Control messages from the programmable MPD transport.
/// Takes a complete packet and command port; returns whether a supported MMC command was consumed.
pub(crate) fn transport(bytes: &[u8], cmd: &CommandPort) -> bool {
    if bytes.len() != 6 || bytes[0] != 0xf0 || bytes[1] != 0x7f || bytes[3] != 6 || bytes[5] != 0xf7
    {
        return false;
    }
    let command = match bytes[4] {
        1 => Command::Stop,
        2 | 3 => Command::Play,
        6 => Command::Surface(crate::engine::surface_controls::Input::Recording(true)),
        7 => Command::Surface(crate::engine::surface_controls::Input::Recording(false)),
        _ => return false,
    };
    let _ = cmd.send(command);
    true
}

/// Read a saved MPD232 preset without changing the hardware.
/// Takes no arguments; returns the configured map, or no map when no preset has been qualified.
pub(crate) fn configured() -> anyhow::Result<Option<MidiMap>> {
    let path = crate::startup::Paths::environment()?
        .preferences
        .parent()
        .unwrap()
        .join("midi/mpd232.syx");
    match std::fs::read(path) {
        Ok(bytes) => parse(&bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Map the physical controls from an original editor-format preset dump.
/// Takes a complete SysEx preset; returns a validated map preserving the preset's musical pad notes.
pub(crate) fn parse(bytes: &[u8]) -> anyhow::Result<MidiMap> {
    anyhow::ensure!(
        bytes.len() == 3483
            && bytes[..5] == [0xf0, 0x47, 0, 0x36, 0x10]
            && bytes.last() == Some(&0xf7)
            && bytes[5] == 0x1b
            && bytes[6] == 0x13
            && bytes[7..3482].iter().all(|&byte| byte < 128),
        "Invalid MPD232 preset dump"
    );
    let name = std::str::from_utf8(&bytes[8..16])?.trim_end_matches(['\0', ' ']);
    anyhow::ensure!(!name.is_empty(), "MPD232 preset has no name");
    let mut bindings = Vec::with_capacity(72);
    for bank in 0..3 {
        for index in 0..8 {
            let knob = &bytes[734 + (bank * 8 + index) * 9..][..9];
            let fader = &bytes[950 + (bank * 8 + index) * 6..][..6];
            let switch = &bytes[1094 + (bank * 8 + index) * 13..][..13];
            anyhow::ensure!(
                knob[0] == 0 && fader[0] == 0 && matches!(switch[0], 0 | 1),
                "MPD232 preset uses unsupported control encoding in bank {} control {}",
                bank + 1,
                index + 1
            );
            let channel = |byte: u8| -> anyhow::Result<u8> {
                anyhow::ensure!(
                    (1..=16).contains(&byte),
                    "MPD232 mixer control must use USB A channel 1-16"
                );
                Ok(byte - 1)
            };
            anyhow::ensure!(
                knob[3] == 0 && knob[4] == 127 && fader[3] == 0 && fader[4] == 127,
                "MPD232 mixer controls require their full 0-127 range"
            );
            bindings.push(cbind(
                channel(knob[1])?,
                knob[2],
                [Action::TrackPan, Action::TrackSendA, Action::TrackSendB][bank],
                0,
                index as u8,
            ));
            bindings.push(cbind(
                channel(fader[1])?,
                fader[2],
                Action::TrackFader,
                0,
                index as u8,
            ));
            let action = [Action::TrackMute, Action::TrackSolo, Action::TrackArm][bank];
            bindings.push(if switch[0] == 1 {
                nbind(channel(switch[1])?, switch[2], action, 0, index as u8)
            } else {
                cbind(channel(switch[1])?, switch[2], action, 0, index as u8)
            });
        }
    }
    for binding in &bindings {
        if binding.kind != super::super::MsgKind::Note {
            continue;
        }
        for pad in bytes[30..734].chunks_exact(11) {
            anyhow::ensure!(
                pad[0] != 0 || pad[1] != binding.ch + 1 || pad[2] != binding.data,
                "MPD232 switch assignment collides with a musical pad"
            );
        }
    }
    let map = MidiMap {
        name: format!("Akai MPD232 ({name})"),
        matchers: vec!["mpd232".into(), "mpd 232".into(), "mpd-232".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Live,
    };
    map.validate()?;
    Ok(map)
}
