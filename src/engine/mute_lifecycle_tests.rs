use super::*;

fn fixture(drums: bool, arp: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.quant = 0.0;
    rt.bpm = 120.0;
    let track = &mut rt.tracks[1];
    track.kind = if drums { 0 } else { 3 };
    track.poly = Poly::new(rt.sr, SynthInstrument::Pad, 16);
    track.clips[0] = Clip {
        properties: Default::default(),
        audio_region: None, lanes: None,
        region: None,
        name: "Mute lifecycle".into(),
        kind: ClipKind::Midi,
        bars: 8.0,
        notes: vec![MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: if drums { 46 } else { 60 },
            start: 0.0,
            len: 0.4,
            vel: 100,
        }],
        gain: 1.0,
        audio: None,
    };
    if arp {
        track.fx.slots.push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    }
    rt.apply(Command::LaunchClip { track: 1, scene: 0 });
    rt
}

fn hide(rt: &mut RtEngine, solo: bool) {
    rt.apply(if solo {
        Command::Solo { track: 2 }
    } else {
        Command::Mute { track: 1 }
    });
}

fn frame(rt: &mut RtEngine) -> (f32, f32) {
    rt.beat += rt.bpm as f64 / (60.0 * rt.sr as f64);
    let any_solo = rt.tracks.iter().any(|track| track.solo);
    let (l, r, _) = rt.render_track(1, any_solo);
    (l, r)
}

fn render(rt: &mut RtEngine, seconds: f32, must_be_silent: bool) -> f32 {
    let mut peak = 0.0f32;
    for _ in 0..(seconds * rt.sr) as usize {
        let (l, r) = frame(rt);
        if must_be_silent {
            assert_eq!((l, r), (0.0, 0.0));
        }
        peak = peak.max(l.abs()).max(r.abs());
    }
    peak
}

#[test]
fn muted_and_solo_excluded_notes_release_on_schedule_and_do_not_resurrect() {
    for solo in [false, true] {
        for arp in [false, true] {
            let mut rt = fixture(false, arp);
            assert!(render(&mut rt, 0.05, false) > 0.001);
            assert!(rt.tracks[1]
                .poly
                .voices
                .iter()
                .any(|v| (1..=3).contains(&v.env.stage)));
            hide(&mut rt, solo);
            render(&mut rt, 0.3, true);
            assert!(rt.tracks[1]
                .poly
                .voices
                .iter()
                .all(|v| matches!(v.env.stage, 0 | 4)));
            render(&mut rt, 1.0, true);
            assert!(rt.tracks[1].poly.voices.iter().all(|v| !v.env.active()));
            assert!(rt.tracks[1].arp_note.is_none());
            hide(&mut rt, solo);
            assert!(render(&mut rt, 0.1, false) < 0.000001);
        }
    }
}

#[test]
fn drum_tails_and_hits_started_while_hidden_advance_instead_of_freezing() {
    for solo in [false, true] {
        for hide_before_start in [false, true] {
            let mut rt = fixture(true, false);
            if hide_before_start {
                hide(&mut rt, solo);
            }
            render(&mut rt, 0.01, hide_before_start);
            let positions = rt.tracks[1].drum_pos;
            assert!(positions.iter().any(Option::is_some));
            if !hide_before_start {
                hide(&mut rt, solo);
            }
            render(&mut rt, 0.01, true);
            assert_ne!(rt.tracks[1].drum_pos, positions);
            render(&mut rt, 2.0, true);
            assert!(rt.tracks[1].drum_pos.iter().all(Option::is_none));
            hide(&mut rt, solo);
            assert!(render(&mut rt, 0.1, false) < 0.000001);
        }
    }
}

#[test]
fn unmuted_output_matches_an_always_running_reference_including_fx_tails() {
    for drums in [false, true] {
        for solo in [false, true] {
            let mut actual = fixture(drums, false);
            let mut reference = fixture(drums, false);
            for rt in [&mut actual, &mut reference] {
                let mut delay = fx::FxSlot::new(fx::FxId::Delay, rt.sr);
                delay.mix = 0.75;
                delay.p[1] = 0.45;
                rt.tracks[1].fx.slots.push(delay);
            }
            for _ in 0..2400 {
                assert_eq!(frame(&mut actual), frame(&mut reference));
            }
            hide(&mut actual, solo);
            for _ in 0..12_000 {
                assert_eq!(frame(&mut actual), (0.0, 0.0));
                frame(&mut reference);
            }
            hide(&mut actual, solo);
            let mut energy = 0.0;
            for _ in 0..24_000 {
                let output = frame(&mut actual);
                assert_eq!(output, frame(&mut reference));
                energy += output.0 * output.0 + output.1 * output.1;
            }
            assert!(energy > 0.001, "the reference must exercise audible tails");
        }
    }
}
