use super::{cbind, nbind, rbind, Action, MidiMap, RelativeEncoding, RelativeSpec, UnmappedNotes};
use crate::engine::{Command, CommandPort};
mod spindle;

pub(super) fn pioneer_sp1() -> MidiMap {
    let mut bindings = Vec::new();
    for deck in 0..4 {
        let target = deck % 2;
        bindings.push(nbind(deck, 0x58, Action::DeckSync, target, 0));
        bindings.push(nbind(deck, 0x55, Action::DeckLoop4, target, 0));
        bindings.push(nbind(6, 0x46 + deck, Action::DeckLoad, target, 0));
        for pad in 0..8 {
            bindings.push(nbind(7 + deck, pad, Action::DeckHotCue, target, pad));
        }
        bindings.push(nbind(7 + deck, 0x60, Action::DeckLoopIn, target, 0));
        bindings.push(nbind(7 + deck, 0x61, Action::DeckLoopOut, target, 0));
    }
    bindings.push(nbind(6, 0x65, Action::CrateReturn, 0, 0));
    bindings.push(nbind(6, 0x67, Action::Prepare, 0, 0));
    bindings.push(nbind(6, 0x68, Action::PrepareCrate, 0, 0));
    let browse = RelativeSpec {
        encoding: RelativeEncoding::TwosComplement,
        scale: 1.0,
    };
    bindings.push(rbind(6, 0x40, Action::Browse, 0, 0, browse));
    bindings.push(rbind(6, 0x64, Action::BrowseCrates, 0, 0, browse));
    for channel in 4..6 {
        for slot in 0..3 {
            bindings.push(nbind(channel, 0x47 + slot, Action::FxSelect, 0, slot));
        }
    }
    MidiMap {
        name: "Pioneer DDJ-SP1".into(),
        matchers: vec!["ddj-sp1".into(), "ddj sp1".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ns7_front_contour_dispatches_independently_of_crossfader_position() {
        let map = numark_ns7();
        let (cmd, received) = CommandPort::channel(32);
        let log = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let shift = std::sync::Arc::new(parking_lot::Mutex::new([false; 4]));
        for message in [[0xb0, 7, 64], [0xb0, 0x55, 0], [0xb0, 0x55, 127], [0xb1, 0x55, 64]] {
            super::super::handle_channel(&message, 42, &map, &cmd, &log, &shift, "NS7", false);
        }
        let commands = received.try_iter().collect::<Vec<_>>();
        assert!(matches!(commands.as_slice(), [Command::Xfader(position), Command::XfaderCurve(0.0), Command::XfaderCurve(1.0)] if *position == 64.0 / 127.0));
    }

    #[test]
    fn ns7_trim_cannot_open_a_closed_fader_or_change_the_other_deck() {
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(32);
        for message in [
            [0xb0, 0x08, 0],
            [0xb0, 0x0c, 127],
            [0xb0, 0x0d, 127],
            [0xb0, 0x11, 64],
            [0xb0, 0x08, 127],
        ] {
            decoder.input(&map, &message, &cmd);
        }
        let values = received
            .try_iter()
            .map(|command| match command {
                Command::DeckGain { deck, value } => ("trim", deck, value),
                Command::Monitor(crate::engine::monitor::Control::Fader { deck, value }) => {
                    ("fader", deck, value)
                }
                _ => panic!("unexpected command"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            [
                ("fader", 0, 0.0),
                ("trim", 0, 1.5),
                ("fader", 1, 1.0),
                ("trim", 1, 1.0),
                ("fader", 0, 1.0)
            ]
        );
        decoder.reset();
        decoder.input(&map, &[0xb0, 0x08, 127], &cmd);
        assert!(matches!(
            received.try_iter().next(),
            Some(Command::Monitor(crate::engine::monitor::Control::Fader {
                deck: 0,
                value: 1.0
            }))
        ));
    }

    #[test]
    fn ns7_wheels_wrap_both_directions_and_reset_without_moving() {
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(64);
        let at = std::time::Instant::now();
        for (i, message) in [
            [0xb0, 0, 126],
            [0xb0, 0, 127],
            [0xb0, 0, 0],
            [0xb0, 0, 127],
            [0xb0, 2, 2],
            [0xb0, 2, 1],
        ].into_iter().enumerate() {
            assert!(decoder.input_at(&map, &message, &cmd, 7, at + std::time::Duration::from_millis(i as u64 * 5)));
        }
        let commands = received.try_iter().collect::<Vec<_>>();
        assert!(
            matches!(commands.as_slice(), [Command::DeckSpindle {deck:0,motion:a,..}, Command::DeckSpindle {deck:0,motion:b,..}, Command::DeckSpindle {deck:0,motion:c,..}, Command::DeckSpindle {deck:0,motion:d,..}, Command::DeckSpindle {deck:1,motion:e,..}, Command::DeckSpindle {deck:1,motion:f,..}] if a.ticks==0 && b.ticks==1 && c.ticks==2 && d.ticks==1 && e.ticks==0 && f.ticks == -1 && f.rate < 0.0), "{commands:?}"
        );
        decoder.reset();
        decoder.input(&map, &[0xb0, 0, 20], &cmd);
        assert!(matches!(received.try_iter().next(),Some(Command::DeckSpindle {motion,..}) if motion.ticks == 0 && motion.rate == 0.0));
    }

    #[test]
    fn ns7_spindle_accumulation_preserves_ticks_and_bounds_command_traffic() {
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(64);
        let at = std::time::Instant::now();
        for i in 0..=100 { decoder.input_at(&map, &[0xb0,0,(i * 2 % 128) as u8], &cmd, 42, at + std::time::Duration::from_millis(i)); }
        let motions = received.try_iter().map(|c| match c { Command::DeckSpindle {source:42,deck:0,motion} => motion, _=>panic!("spindle must use its own clock") }).collect::<Vec<_>>();
        assert_eq!(motions.len(),26);
        assert_eq!(motions.last().unwrap().ticks,200);
        assert_eq!(motions.last().unwrap().rate,1.0);
        cmd.release_midi_source(41);
        assert!(received.try_iter().next().is_none());
        cmd.release_midi_source(42);
        assert!(matches!(received.try_iter().next(),Some(Command::DeckSpindleRelease {source:42,deck:0})));
    }

    #[test]
    #[ignore = "requires a timestamped capture from the connected original NS7"]
    fn ns7_captured_free_rotation_preserves_audio_continuity() {
        use crate::engine::spindle::Playback;
        use std::time::{Duration, Instant};
        let path = std::env::var("OMATAINER_NS7_SPINDLE_CAPTURE").unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let mut events = text.lines().filter_map(|line| {
            let fields = line.split_whitespace().map(|v| v.parse::<f64>().unwrap()).collect::<Vec<_>>();
            (fields.len() == 5 && fields[0] >= 52.0 && fields[0] < 59.0 && fields[1] == 10.0 && fields[3] == 0.0)
                .then(|| (fields[0] - 52.0, fields[4] as u8))
        }).peekable();
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (commands, received) = CommandPort::channel(256);
        let at = Instant::now();
        let mut spindle = None;
        let (mut previous, mut peak, mut minimum, mut maximum, mut count) = (None::<f64>, 0.0_f64, f32::MAX, f32::MIN, 0);
        for frame in 0..7 * 44100 {
            let seconds = frame as f64 / 44100.0;
            let now = at + Duration::from_secs_f64(seconds);
            while let Some(&(seconds, value)) = events.peek() {
                if at + Duration::from_secs_f64(seconds) > now { break; }
                decoder.input_at(&map, &[0xb0, 0, value], &commands, 42, at + Duration::from_secs_f64(seconds));
                events.next();
            }
            if frame % 128 == 0 {
                for command in received.try_iter() {
                    if let Command::DeckSpindle { motion, .. } = command {
                        if let Some(spindle) = &mut spindle { Playback::update(spindle, motion); }
                        else { spindle = Some(Playback::new(42, motion, 10.0)); }
                        count += 1;
                    }
                }
            }
            let Some(spindle) = &mut spindle else { continue; };
            if frame % 128 == 0 { spindle.begin(now); }
            let (position, rate) = spindle.next(44100.0);
            let sample = (std::f64::consts::TAU * 1000.0 * position).sin();
            if frame > 44100 {
                if let Some(previous) = previous { peak = peak.max((sample - previous).abs()); }
                minimum = minimum.min(rate); maximum = maximum.max(rate);
            }
            previous = Some(sample);
        }
        eprintln!("captured NS7: {count} motion updates; signed speed {minimum}..{maximum}; 1 kHz peak adjacent sample difference {peak}");
        assert!(count > 1000);
        assert!(minimum > 0.85 && maximum < 1.05, "free rotation must remain forward and stable");
        assert!(peak < 0.16, "input arrivals must not splice the audio waveform");
    }

    #[test]
    fn sp1_knobs_require_a_complete_pair_and_shifted_pads_delete_the_right_deck() {
        let map = pioneer_sp1();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(32);
        decoder.input(&map, &[0xb4, 0x22, 127], &cmd);
        decoder.input(&map, &[0xb5, 4, 64], &cmd);
        assert!(received.try_iter().next().is_none());
        decoder.input(&map, &[0xb5, 0x24, 1], &cmd);
        assert!(
            matches!(received.try_iter().next(), Some(Command::FxWet {slot:1,value}) if value == 8193.0/16383.0)
        );
        decoder.input(&map, &[0x9a, 15, 127], &cmd);
        assert!(matches!(
            received.try_iter().next(),
            Some(Command::DeckHotCue {
                deck: 1,
                pad: 7,
                del: true
            })
        ));
        decoder.input(&map, &[0xb4, 2, 127], &cmd);
        decoder.reset();
        decoder.input(&map, &[0xb4, 0x22, 127], &cmd);
        assert!(received.try_iter().next().is_none());
    }
}

pub(super) fn numark_ns7() -> MidiMap {
    let mut bindings = Vec::new();
    for deck in 0..2 {
        let offset = deck * 0x21;
        bindings.push(nbind(0, 0x11 + offset, Action::DeckPlay, deck, 0));
        bindings.push(nbind(0, 0x10 + offset, Action::DeckCue, deck, 0));
        bindings.push(nbind(0, 0x0f + offset, Action::DeckSync, deck, 0));
        bindings.push(nbind(0, 0x21 + offset, Action::DeckVinyl, deck, 0));
        bindings.push(nbind(0, 0x12 + offset, Action::Shift, deck, 0));
        bindings.push(nbind(0, 0x0c + deck * 2, Action::DeckLoad, deck, 0));
        bindings.push(nbind(0, 0x28 + offset, Action::DeckLoopIn, deck, 0));
        bindings.push(nbind(0, 0x29 + offset, Action::DeckLoopOut, deck, 0));
        bindings.push(nbind(0, 0x24 + offset, Action::DeckLoop4, deck, 0));
        for pad in 0..5 {
            bindings.push(nbind(0, 0x13 + offset + pad, Action::DeckHotCue, deck, pad));
        }
        bindings.push(cbind(0, 4 + deck, Action::DeckPitch, deck, 0));
        let mixer = deck * 5;
        bindings.push(cbind(0, 0x0c + mixer, Action::DeckGain, deck, 0));
        bindings.push(cbind(0, 0x08 + mixer, Action::DeckGain, deck, 0));
        bindings.push(cbind(0, 0x0b + mixer, Action::DeckEqHi, deck, 0));
        bindings.push(cbind(0, 0x0a + mixer, Action::DeckEqMid, deck, 0));
        bindings.push(cbind(0, 0x09 + mixer, Action::DeckEqLow, deck, 0));
    }
    bindings.push(cbind(0, 7, Action::Xfader, 0, 0));
    bindings.push(cbind(0, 0x55, Action::XfaderCurve, 0, 0));
    bindings.push(cbind(0, 0x40, Action::Master, 0, 0));
    MidiMap {
        name: "Numark NS7 (original)".into(),
        matchers: vec!["ns7".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

pub(super) struct Decoder {
    spindles: [spindle::Spindle; 2],
    fx: [[Option<u8>; 3]; 2],
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            spindles: std::array::from_fn(|_| spindle::Spindle::default()),
            fx: [[None; 3]; 2],
        }
    }
}

impl Decoder {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Decode controller state into engine commands.
    /// Takes the factory profile, a complete wire message and command port; returns whether the message was consumed.
    pub(super) fn input(&mut self, map: &MidiMap, message: &[u8; 3], cmd: &CommandPort) -> bool {
        self.input_at(map, message, cmd, 0, std::time::Instant::now())
    }

    pub(super) fn idle(&mut self, source: u64, cmd: &CommandPort) {
        for (deck, spindle) in self.spindles.iter_mut().enumerate() { spindle.flush(source, deck as u8, std::time::Instant::now(), cmd); }
    }

    /// Decode a wire message with its original callback time and owner.
    /// Takes profile, message, command port, source and time; returns whether it was consumed.
    pub(super) fn input_at(&mut self, map: &MidiMap, message: &[u8; 3], cmd: &CommandPort, source: u64, at: std::time::Instant) -> bool {
        let [status, control, value] = *message;
        if map.name == "Numark NS7 (original)" && matches!(status, 0x90 | 0x80) && control == 0 {
            let _ = cmd.send(Command::Monitor(crate::engine::monitor::Control::Master(
                status == 0x80 || value == 0,
            )));
            return true;
        }
        if map.name == "Numark NS7 (original)" && status == 0xb0 && matches!(control, 0x12 | 0x42) {
            let value = f32::from(value) / 127.0;
            let _ = cmd.send(Command::Monitor(if control == 0x12 {
                crate::engine::monitor::Control::Mix(value)
            } else {
                crate::engine::monitor::Control::Volume(value)
            }));
            return true;
        }
        if map.name == "Numark NS7 (original)"
            && status == 0xb0
            && matches!(control, 0x08 | 0x0d | 0x0c | 0x11)
        {
            let deck = usize::from(matches!(control, 0x0d | 0x11));
            if matches!(control, 0x08 | 0x0d) {
                let _ = cmd.send(Command::Monitor(crate::engine::monitor::Control::Fader {
                    deck: deck as u8,
                    value: f32::from(value) / 127.0,
                }));
            } else {
                let _ = cmd.send(Command::DeckGain {
                    deck: deck as u8,
                    value: (f32::from(value) / 64.0).min(1.5),
                });
            }
            return true;
        }
        if map.name == "Numark NS7 (original)" && status == 0xb0 && matches!(control, 0 | 2) {
            let deck = usize::from(control / 2);
            self.spindles[deck].position(value, at);
            self.spindles[deck].flush(source, deck as u8, at, cmd);
            return true;
        }
        if map.name == "Numark NS7 (original)" && matches!(status, 0xe0 | 0xe2) {
            return true;
        }
        if map.name == "Pioneer DDJ-SP1" {
            if (0x97..=0x9a).contains(&status) && (8..16).contains(&control) {
                if value > 0 {
                    let _ = cmd.send(Command::DeckHotCue {
                        deck: (status - 0x97) % 2,
                        pad: control - 8,
                        del: true,
                    });
                }
                return true;
            }
            if matches!(status, 0xb4 | 0xb5) {
                let bank = usize::from(status - 0xb4);
                if matches!(control, 2 | 4 | 6) {
                    self.fx[bank][usize::from(control / 2 - 1)] = Some(value);
                    return true;
                }
                if matches!(control, 0x22 | 0x24 | 0x26) {
                    let slot = usize::from((control - 0x20) / 2 - 1);
                    if let Some(msb) = self.fx[bank][slot].take() {
                        let value = f32::from((u16::from(msb) << 7) | u16::from(value)) / 16383.0;
                        let _ = cmd.send(Command::FxWet {
                            slot: slot as u8,
                            value,
                        });
                    }
                    return true;
                }
            }
        }
        false
    }
}
