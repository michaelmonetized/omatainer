use super::*;

const EPS: f64 = 1e-9;
const TRACK: usize = 1;

#[derive(Clone, Copy, Debug)]
enum Instrument {
    Synth,
    Drums,
    Arp,
}

fn engine(sr: u32) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn clip(pitch: u8, length: f32) -> Clip {
    Clip {
        region: None,
        kind: ClipKind::Midi,
        name: "quantized probe".into(),
        bars: 0.25,
        notes: vec![MidiNote {
            id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch,
            start: 0.0,
            len: length,
            vel: 100,
        }],
        gain: 1.0,
        audio: None,
    }
}

fn prepare(rt: &mut RtEngine, bpm: f32, quant: f32, instrument: Instrument) {
    rt.apply(Command::Stop);
    rt.beat = quant as f64 * 0.2;
    rt.bpm = bpm;
    rt.quant = quant;
    rt.playing = true;
    let track = &mut rt.tracks[TRACK];
    track.poly = Poly::new(rt.sr, SynthInstrument::Keys, 16);
    track.eq = ThreeBand::new(rt.sr);
    track.eq_right = ThreeBand::new(rt.sr);
    track.drum_pos.fill(None);
    track.fx.slots.clear();
    track.kind = if matches!(instrument, Instrument::Drums) {
        0
    } else {
        2
    };
    track.clips = std::array::from_fn(|_| Clip::empty());
    let pitch = if matches!(instrument, Instrument::Drums) {
        36
    } else {
        60
    };
    track.clips[0] = clip(
        pitch,
        if matches!(instrument, Instrument::Arp) {
            1.25
        } else {
            0.125
        },
    );
    if matches!(instrument, Instrument::Arp) {
        track.fx.slots.push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    }
}

fn render_one(rt: &mut RtEngine) -> (f32, f32, bool) {
    rt.beat += rt.bpm as f64 / (rt.sr as f64 * 60.0);
    rt.render_track(TRACK, false)
}

fn count_onsets(rt: &RtEngine, instrument: Instrument, drum_events: &mut u64) -> u64 {
    if matches!(instrument, Instrument::Drums) {
        // Each newly started one-shot advances from zero to one source frame;
        // a repeated boundary would occupy another slot and be counted again.
        *drum_events += rt.tracks[TRACK]
            .drum_pos
            .iter()
            .filter(|slot| slot.is_some_and(|voice| (voice.position - 1.0).abs() < EPS))
            .count() as u64;
        *drum_events
    } else {
        rt.tracks[TRACK].poly.note_on_events
    }
}

#[test]
fn quantized_sample_boundaries_cover_tempos_grids_instruments_and_complete_one_shots() {
    for sr in [44_100, 48_000] {
        let mut rt = engine(sr);
        for bpm in [60.0, 123.0, 180.0] {
            for quant in [0.25, 0.5, 1.0, 4.0] {
                for instrument in [Instrument::Synth, Instrument::Drums, Instrument::Arp] {
                    for looping in [false, true] {
                        prepare(&mut rt, bpm, quant, instrument);
                        rt.apply(Command::FireClip {
                            track: TRACK as u8,
                            scene: 0,
                            looping,
                        });
                        let start = rt.tracks[TRACK].playing.unwrap().start_beat;
                        assert!((start - quant as f64).abs() < EPS);
                        let increment = bpm as f64 / (sr as f64 * 60.0);
                        let mut drum_events = 0;
                        let initial_beat = rt.beat;
                        let mut pending_frames = 0;
                        while rt.beat + increment <= start + EPS {
                            assert_eq!(render_one(&mut rt), (0.0, 0.0, false));
                            assert_eq!(count_onsets(&rt, instrument, &mut drum_events), 0);
                            assert!(rt.tracks[TRACK].playing.is_some());
                            assert!(rt.tracks[TRACK].playing.unwrap().last_beat < 0.0);
                            pending_frames += 1;
                        }
                        let distance = (start - initial_beat) / increment;
                        assert_eq!(pending_frames, (distance + 1e-7).floor() as usize);
                        rt.publish_for_test();
                        let pending = rt.snap.lock().tracks[TRACK].clone();
                        assert!(pending.clip_pending);
                        assert_eq!(pending.clip_progress, 0.0);
                        render_one(&mut rt);
                        assert_eq!(
                            count_onsets(&rt, instrument, &mut drum_events),
                            1,
                            "first boundary: sr={sr}, bpm={bpm}, quant={quant}, instrument={instrument:?}"
                        );
                        assert!(rt.tracks[TRACK].playing.is_some());
                        rt.publish_for_test();
                        assert!(!rt.snap.lock().tracks[TRACK].clip_pending);
                        // The initial event must not retrigger on the next sample.
                        render_one(&mut rt);
                        assert_eq!(count_onsets(&rt, instrument, &mut drum_events), 1);
                        let end = start + 1.0;
                        while rt.beat + increment <= end + EPS {
                            render_one(&mut rt);
                            count_onsets(&rt, instrument, &mut drum_events);
                            assert!(
                                rt.tracks[TRACK].playing.is_some(),
                                "one-shot ended early: sr={sr}, bpm={bpm}, quant={quant}"
                            );
                        }
                        render_one(&mut rt);
                        let events = count_onsets(&rt, instrument, &mut drum_events);
                        let per_loop = if matches!(instrument, Instrument::Arp) {
                            4
                        } else {
                            1
                        };
                        if looping {
                            assert!(rt.tracks[TRACK].playing.is_some());
                            assert_eq!(
                                events,
                                per_loop + 1,
                                "loop boundary {instrument:?}, sr={sr}, bpm={bpm}, quant={quant}"
                            );
                            render_one(&mut rt);
                            assert_eq!(
                                count_onsets(&rt, instrument, &mut drum_events),
                                per_loop + 1
                            );
                        } else {
                            assert!(rt.tracks[TRACK].playing.is_none());
                            rt.publish_for_test();
                            let stopped = rt.snap.lock().tracks[TRACK].clone();
                            assert!(!stopped.clip_pending);
                            assert_eq!(stopped.playing_scene, -1);
                            assert_eq!(stopped.clip_progress, 0.0);
                            assert_eq!(
                                events, per_loop,
                                "one-shot must not replay beat zero at its end"
                            );
                            assert!(
                                rt.tracks[TRACK]
                                    .poly
                                    .voices
                                    .iter()
                                    .all(|voice| !matches!(voice.env.stage, 1..=3))
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn exact_launch_grid_tolerates_clock_rounding_without_waiting_an_extra_grid() {
    let mut rt = engine(48_000);
    for quant in [0.25, 0.5, 1.0, 4.0] {
        for offset in [-EPS * 0.5, 0.0, EPS * 0.5] {
            prepare(&mut rt, 123.0, quant, Instrument::Synth);
            let boundary = quant as f64 * 3.0;
            rt.beat = boundary + offset;
            rt.apply(Command::LaunchClip {
                track: TRACK as u8,
                scene: 0,
            });
            assert!((rt.tracks[TRACK].playing.unwrap().start_beat - boundary).abs() < EPS);
            render_one(&mut rt);
            assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
            render_one(&mut rt);
            assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
        }
    }
}

#[test]
fn scene_batches_share_one_start_even_when_transport_was_stopped_off_grid() {
    let mut rt = engine(48_000);
    for running in [false, true] {
        for command in [
            Command::LaunchScene { scene: 0 },
            Command::RestartScene { scene: 0 },
            Command::AddScene { scene: 0 },
        ] {
            rt.apply(Command::Stop);
            rt.quant = 1.0;
            rt.beat = 0.375;
            rt.playing = running;
            for track in &mut rt.tracks {
                track.clips[0] = clip(60, 0.125);
            }
            rt.apply(command);
            let expected = if running { 1.0 } else { 0.375 };
            assert!(rt.tracks.iter().all(|track| {
                track
                    .playing
                    .is_some_and(|clip| clip.start_beat == expected)
            }));
        }
    }
}

#[test]
fn pending_replacement_and_cancellation_do_not_emit_the_old_clip() {
    let mut rt = engine(48_000);
    for cancel in [false, true] {
        prepare(&mut rt, 120.0, 1.0, Instrument::Synth);
        rt.apply(Command::LaunchClip {
            track: TRACK as u8,
            scene: 0,
        });
        rt.tracks[TRACK].clips[1] = clip(67, 0.125);
        rt.apply(Command::LaunchClip {
            track: TRACK as u8,
            scene: 1,
        });
        if cancel {
            rt.apply(Command::StopTrack { track: TRACK as u8 });
        }
        while rt.beat <= 1.0 + EPS {
            render_one(&mut rt);
        }
        assert_eq!(rt.tracks[TRACK].poly.note_on_events, u64::from(!cancel));
        assert!(
            rt.tracks[TRACK]
                .poly
                .voices
                .iter()
                .all(|voice| voice.note() != 60 || !voice.env.active())
        );
        if !cancel {
            assert!(
                rt.tracks[TRACK]
                    .poly
                    .voices
                    .iter()
                    .any(|voice| voice.note() == 67 && voice.env.active())
            );
        }
    }
}

#[test]
fn retrigger_waits_for_its_new_grid_and_gets_a_complete_one_shot() {
    let mut rt = engine(48_000);
    prepare(&mut rt, 120.0, 1.0, Instrument::Synth);
    rt.quant = 0.0;
    rt.apply(Command::LaunchClip {
        track: TRACK as u8,
        scene: 0,
    });
    render_one(&mut rt);
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
    rt.quant = 1.0;
    rt.apply(Command::FireClip {
        track: TRACK as u8,
        scene: 0,
        looping: false,
    });
    while rt.beat + 1.0 / 24_000.0 <= 1.0 + EPS {
        render_one(&mut rt);
    }
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
    render_one(&mut rt);
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 2);
    while rt.beat + 1.0 / 24_000.0 <= 2.0 + EPS {
        render_one(&mut rt);
        assert!(rt.tracks[TRACK].playing.is_some());
    }
    render_one(&mut rt);
    assert!(rt.tracks[TRACK].playing.is_none());
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 2);
}

#[test]
fn callback_output_stays_silent_until_the_scheduled_first_sample() {
    let mut rt = engine(48_000);
    // Isolate the incoming clip from independent deck sources and tails.
    for deck in 0..DECKS {
        rt.apply(Command::DeckUnload { deck: deck as u8 });
    }
    prepare(&mut rt, 123.0, 0.25, Instrument::Synth);
    rt.apply(Command::FireClip {
        track: TRACK as u8,
        scene: 0,
        looping: false,
    });
    let start = rt.tracks[TRACK].playing.unwrap().start_beat;
    let increment = rt.bpm as f64 / rt.sr as f64 / 60.0;
    let first_frame = ((start - rt.beat) / increment).floor() as usize;
    let mut sample = [0.0; 2];
    for frame in 0..first_frame {
        sample.fill(f32::NAN);
        rt.process(&mut sample);
        assert_eq!(sample, [0.0; 2], "audio before launch at frame {frame}");
        assert_eq!(rt.tracks[TRACK].poly.note_on_events, 0);
    }
    rt.process(&mut sample);
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
    let mut audible = false;
    for _ in 0..128 {
        rt.process(&mut sample);
        audible |= sample.iter().any(|x| x.abs() > 1e-5);
    }
    assert!(
        audible,
        "scheduled clip failed to reach the output callback"
    );
    assert_eq!(rt.tracks[TRACK].poly.note_on_events, 1);
}
