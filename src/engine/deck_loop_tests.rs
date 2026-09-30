use super::*;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn sample(name: &str, frames: usize) -> Arc<Sample> {
    Arc::new(Sample {
        name: name.into(),
        sr: 48_000,
        ch: 1,
        data: (0..frames).map(|i| (i as f32 * 0.1).sin() * 0.25).collect(),
        peaks: Vec::new(),
        bpm: 120.0,
        path: String::new(),
    })
}

fn late_loop(rt: &mut RtEngine, deck: u8, audio: Arc<Sample>) {
    rt.apply(Command::DeckAudio { deck, audio });
    rt.apply(Command::DeckSeek { deck, frac: 0.75 });
    rt.apply(Command::DeckLoop { deck, beats: 1.0 });
    assert!(rt.decks[deck as usize].loop_start > 4_800.0);
    assert!(rt.decks[deck as usize].loop_len > 4_800.0);
    rt.publish();
    assert!(rt.snap.lock().decks[deck as usize].loop_on);
}

fn assert_reset(rt: &mut RtEngine, deck: usize, frames: usize, title: &str) {
    let d = &rt.decks[deck];
    assert!(!d.loop_on);
    assert_eq!(d.loop_start, 0.0);
    assert_eq!(d.loop_len, 0.0);
    assert_eq!(d.pos, 0.0);
    assert!(!d.playing);
    rt.publish();
    let snap = rt.snap.lock();
    let d = &snap.decks[deck];
    assert!(!d.loop_on);
    assert_eq!(d.pos, 0.0);
    assert_eq!(d.frames, frames as f64);
    assert_eq!(d.title, title);
    assert!(!d.playing);
}

#[test]
fn replacing_late_loop_with_short_media_starts_at_zero_and_reaches_the_end() {
    let mut rt = engine();
    let long = sample("Long", 192_000);
    let short = sample("Short", 4_800);
    for deck in 0..DECKS {
        for builtin in [false, true] {
            late_loop(&mut rt, deck as u8, long.clone());
            // Replacement affects only the chosen deck.
            let other = 1 - deck;
            rt.decks[other].loop_on = true;
            rt.decks[other].loop_start = 48.0;
            rt.decks[other].loop_len = 128.0;
            if builtin {
                rt.builtin[0] = Some(short.clone());
                rt.apply(Command::LoadBuiltin {
                    deck: deck as u8,
                    stem: 0,
                });
            } else {
                rt.apply(Command::DeckAudio {
                    deck: deck as u8,
                    audio: short.clone(),
                });
            }
            assert_reset(&mut rt, deck, short.frames(), "Short");
            assert!(rt.decks[other].loop_on);
            assert_eq!(rt.decks[other].loop_start, 48.0);
            assert_eq!(rt.decks[other].loop_len, 128.0);
            for _ in 0..128 {
                rt.render_deck(deck);
                assert_eq!(rt.decks[deck].pos, 0.0, "a stopped new deck must not jump");
            }
            rt.apply(Command::DeckPlay { deck: deck as u8 });
            let mut energy = 0.0;
            let mut previous = 0.0;
            for frame in 1..=128 {
                let (l, r) = rt.render_deck(deck);
                energy += l * l + r * r;
                let position = rt.decks[deck].pos;
                assert!(position > previous && position <= frame as f64);
                previous = position;
            }
            assert!(energy > 0.1, "new media must advance and produce audio");
            // Allow for the normal playback-rate ramp after a stopped deck.
            for _ in 128..short.frames() + 64 {
                rt.render_deck(deck);
            }
            assert!(
                !rt.decks[deck].playing,
                "the old loop must not restart new media"
            );
            assert_reset(&mut rt, deck, short.frames(), "Short");
        }
    }
}

#[test]
fn unload_then_reload_clears_active_and_remembered_loop_bounds() {
    let mut rt = engine();
    let long = sample("Long", 192_000);
    let short = sample("Short", 4_800);
    for deck in 0..DECKS {
        for active in [false, true] {
            late_loop(&mut rt, deck as u8, long.clone());
            if !active {
                rt.apply(Command::DeckLoop {
                    deck: deck as u8,
                    beats: 1.0,
                });
                assert!(!rt.decks[deck].loop_on);
                assert!(rt.decks[deck].loop_start > 0.0);
            }
            rt.apply(Command::DeckUnload { deck: deck as u8 });
            assert_reset(&mut rt, deck, 0, "");
            assert!(rt.decks[deck].audio.is_none());
            for _ in 0..128 {
                rt.render_deck(deck);
                assert_eq!(rt.decks[deck].pos, 0.0);
            }
            rt.apply(Command::DeckAudio {
                deck: deck as u8,
                audio: short.clone(),
            });
            assert_reset(&mut rt, deck, short.frames(), "Short");
        }
    }
}

#[test]
fn same_media_reload_clears_inactive_or_partial_loops() {
    let mut rt = engine();
    let long = sample("Long", 192_000);
    for deck in 0..DECKS {
        late_loop(&mut rt, deck as u8, long.clone());
        rt.apply(Command::DeckLoop {
            deck: deck as u8,
            beats: 1.0,
        });
        rt.apply(Command::DeckAudio {
            deck: deck as u8,
            audio: long.clone(),
        });
        assert_reset(&mut rt, deck, long.frames(), "Long");
        rt.apply(Command::DeckSeek {
            deck: deck as u8,
            frac: 0.75,
        });
        rt.apply(Command::DeckLoopIn { deck: deck as u8 });
        assert!(rt.decks[deck].loop_start > 0.0);
        assert!(!rt.decks[deck].loop_on);
        rt.apply(Command::DeckAudio {
            deck: deck as u8,
            audio: long.clone(),
        });
        assert_reset(&mut rt, deck, long.frames(), "Long");
    }
}
