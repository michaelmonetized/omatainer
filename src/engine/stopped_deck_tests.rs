use super::*;

fn fixture(sr: u32, deck: u8, keylock: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::DeckAudio {
        deck,
        audio: Arc::new(Sample {
            name: "nonzero at every pause position".into(),
            sr,
            ch: 2,
            data: vec![0.25; 16_384],
            peaks: vec![],
            bpm: 120.0,
            path: String::new(),
        }),
    });
    if keylock {
        rt.apply(Command::DeckKeylock { deck });
    }
    rt
}

fn assert_fades_to_silence(rt: &mut RtEngine, deck: usize) {
    let previous = rt.decks[deck].last_output;
    let frames = (rt.sr as u32).div_ceil(500);
    assert!(previous.iter().any(|x| x.abs() > 0.01));
    let mut paused_position = None;
    for frame in 0..frames + 512 {
        let (l, r) = rt.render_deck(deck);
        if frame == 0 {
            paused_position = Some(rt.decks[deck].pos);
        }
        assert_eq!(rt.decks[deck].pos, paused_position.unwrap());
        if frame < frames {
            let gain = 1.0 - frame as f32 / (frames - 1) as f32;
            assert!((l - previous[0] * gain).abs() < 1e-6);
            assert!((r - previous[1] * gain).abs() < 1e-6);
        } else {
            assert_eq!((l, r), (0.0, 0.0));
        }
    }
    assert_eq!(rt.decks[deck].last_output, [0.0; 2]);
}

#[test]
fn untouched_stopped_decks_and_default_session_are_exactly_silent() {
    for deck in 0..DECKS {
        for keylock in [false, true] {
            let mut rt = fixture(48_000, deck as u8, keylock);
            rt.apply(Command::DeckSeek {
                deck: deck as u8,
                frac: 0.5,
            });
            let position = rt.decks[deck].pos;
            for _ in 0..4096 {
                assert_eq!(rt.render_deck(deck), (0.0, 0.0));
            }
            assert_eq!(rt.decks[deck].pos, position);
            assert_eq!(rt.deck_grain(deck, rt.sr as f64), (0.0, 0.0));
        }
    }
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let mut output = [1.0; 2048];
    for _ in 0..16 {
        rt.process(&mut output);
        assert!(output.iter().all(|sample| *sample == 0.0));
    }
}

#[test]
fn pause_cue_unload_replacement_and_eof_retire_output_with_bounded_fade() {
    for sr in [44_100, 48_000] {
        for keylock in [false, true] {
            for deck in 0..DECKS {
                for action in 0..5 {
                    let mut rt = fixture(sr, deck as u8, keylock);
                    rt.apply(Command::DeckPlay { deck: deck as u8 });
                    for _ in 0..256 {
                        rt.render_deck(deck);
                    }
                    assert!(rt.decks[deck].pos > 100.0);
                    match action {
                        0 => rt.apply(Command::DeckPlay { deck: deck as u8 }),
                        1 => rt.apply(Command::DeckCue { deck: deck as u8 }),
                        2 => rt.apply(Command::DeckUnload { deck: deck as u8 }),
                        3 => rt.apply(Command::DeckAudio {
                            deck: deck as u8,
                            audio: rt.decks[deck].audio.clone().unwrap(),
                        }),
                        _ => rt.apply(Command::DeckSeek {
                            deck: deck as u8,
                            frac: 1.0,
                        }),
                    }
                    assert_fades_to_silence(&mut rt, deck);
                    assert!(!rt.decks[deck].playing);
                    if rt.decks[deck].audio.is_some() {
                        // Seeking to EOF also places the cue there. Choose a
                        // playable location explicitly before testing restart.
                        if action == 4 {
                            rt.apply(Command::DeckSeek {
                                deck: deck as u8,
                                frac: 0.25,
                            });
                        }
                        rt.apply(Command::DeckPlay { deck: deck as u8 });
                        let start = rt.decks[deck].pos;
                        for _ in 0..128 {
                            rt.render_deck(deck);
                        }
                        assert!(rt.decks[deck].pos > start, "sr={sr} lock={keylock} deck={deck} action={action} start={start} end={} playing={} cue={}", rt.decks[deck].pos, rt.decks[deck].playing, rt.decks[deck].cue_pos);
                        assert!(rt.decks[deck].last_output[0] > 0.1);
                    }
                }
            }
        }
    }
}

#[test]
fn deliberate_scratch_contact_can_play_a_stopped_deck_until_release() {
    for keylock in [false, true] {
        let mut rt = fixture(48_000, 0, keylock);
        rt.apply(Command::DeckTouch { deck: 0, on: true });
        rt.apply(Command::DeckJog {
            deck: 0,
            delta: 0.1,
        });
        let start = rt.decks[0].pos;
        for _ in 0..128 {
            rt.render_deck(0);
        }
        assert!(!rt.decks[0].playing);
        assert!(rt.decks[0].pos > start);
        assert!(rt.decks[0].last_output[0] > 0.1);
        rt.apply(Command::DeckTouch { deck: 0, on: false });
        assert_fades_to_silence(&mut rt, 0);
    }
}

#[test]
fn downstream_master_effects_keep_decaying_tails_after_source_becomes_silent() {
    for keylock in [false, true] {
        for effect in 0..2 {
            let mut rt = fixture(48_000, 0, keylock);
            rt.fx_wet[effect] = 1.0;
            rt.apply(Command::DeckPlay { deck: 0 });
            rt.process(&mut [0.0; 512]);
            rt.apply(Command::DeckPlay { deck: 0 });
            let mut output = vec![0.0; 48_000 * 2 * 4];
            rt.process(&mut output);
            assert!(output.iter().all(|x| x.is_finite()));
            assert_eq!(rt.decks[0].last_output, [0.0; 2]);
            let tail_peak = output[96 * 2..48_000 * 2]
                .iter()
                .fold(0.0_f32, |peak, x| peak.max(x.abs()));
            let late_peak = output[48_000 * 2 * 3..]
                .iter()
                .fold(0.0_f32, |peak, x| peak.max(x.abs()));
            assert!(tail_peak > 0.01, "effect={effect}: downstream tail was cut");
            assert!(
                late_peak < tail_peak * 0.01,
                "effect={effect}: tail failed to decay: early={tail_peak} late={late_peak}"
            );
            for _ in 0..512 {
                assert_eq!(rt.render_deck(0), (0.0, 0.0));
            }
        }
    }
}
