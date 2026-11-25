
#[cfg(test)]
mod adversarial_midi_audit {
    use super::*;
    fn observe(bytes: &[u8], map: &MidiMap) -> Vec<Command> {
        let (tx, rx) = crossbeam_channel::bounded(256);
        handle_msg(bytes, map, &tx, &Arc::new(Mutex::new(Vec::new())),
            &Arc::new(Mutex::new(None)), &Arc::new(Mutex::new([false; 4])), "audit");
        rx.try_iter().collect()
    }
    #[test]
    fn audit_one_byte_clock_and_transport_are_discarded() {
        for byte in [0xFA, 0xFC, 0xF8] {
            let cmds = observe(&[byte], &class_compliant());
            println!("MIDI realtime {byte:#04x}: {} commands", cmds.len());
            assert!(cmds.is_empty());
        }
    }
    #[test]
    fn audit_apc40_first_fader_fans_out_to_every_track() {
        let cmds = observe(&[0xB0, 7, 127], &akai_apc40());
        let tracks: Vec<u8> = cmds.iter().filter_map(|c| match c {
            Command::TrackGain {track, ..} => Some(*track), _ => None
        }).collect();
        println!("APC40 B0 07 7F changes tracks {tracks:?}");
        assert_eq!(tracks, vec![0,0,1,2,3,4,5,6,7]);
    }
    #[test]
    fn audit_relative_jog_decoder_has_no_reverse_value() {
        let deltas: Vec<f32> = (0..128).flat_map(|v| observe(&[0xB0, 0x21, v], &pioneer_ddj_fx()))
            .filter_map(|c| match c { Command::DeckJog {delta,..} => Some(delta), _ => None }).collect();
        println!("All 128 relative jog bytes: min={} max={} negative_count={}",
            deltas.iter().copied().fold(f32::INFINITY, f32::min),
            deltas.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            deltas.iter().filter(|v| **v < 0.0).count());
        assert_eq!(deltas.len(),128);
        assert!(deltas.iter().all(|d| *d >= 0.0));
    }
    #[test]
    fn audit_pioneer_load_reaches_empty_engine_arm() {
        let cmds = observe(&[0x90, 0x02, 127], &pioneer_ddj_fx());
        assert!(matches!(cmds.as_slice(), [Command::DeckLoadSelected {deck:0}]));
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let mut rt = crate::engine::RtEngine::new(48000.0, rx,
            Arc::new(Mutex::new(crate::engine::Snapshot::default())));
        let before = rt.decks[0].audio.as_ref().map(Arc::as_ptr);
        for c in cmds { rt.apply(c); }
        assert_eq!(before, rt.decks[0].audio.as_ref().map(Arc::as_ptr));
        println!("Pioneer load button emits DeckLoadSelected; engine keeps same audio allocation");
    }
    #[test]
    fn audit_learning_swallows_every_message_without_finishing() {
        let (tx, rx) = crossbeam_channel::bounded(32);
        let learn = Arc::new(Mutex::new(Some("master".to_string())));
        let log = Arc::new(Mutex::new(Vec::new()));
        let shift = Arc::new(Mutex::new([false;4]));
        for bytes in [[0xB0,7,64], [0x90,60,100], [0x80,60,0]] {
            handle_msg(&bytes, &class_compliant(), &tx, &log, &learn, &shift, "audit");
        }
        let cmds: Vec<_> = rx.try_iter().collect();
        assert_eq!(cmds.len(),3);
        assert!(cmds.iter().all(|c| matches!(c,Command::LearnCapture{..})));
        assert!(learn.lock().is_some());
        println!("Forced learn mode converts CC, NoteOn and NoteOff into 3 discarded LearnCapture commands; stays enabled");
    }
}
