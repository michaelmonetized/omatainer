use super::*;

fn arp(mix: f32) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.tracks[2].fx.slots.clear();
    rt.tracks[2].clips[0].bars = 1.0;
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: [60, 64, 67]
            .into_iter()
            .map(|pitch| MidiNote {
                pitch,
                start: 0.0,
                len: 4.0,
                vel: 100,
            })
            .collect(),
    });
    rt.apply(Command::OpenFxTrack(2));
    rt.apply(Command::FxAdd(7));
    let before = (rt.tracks[2].fx.slots[0].mix, rt.tracks[2].fx.slots[0].p);
    rt.apply(Command::FxMix {
        slot: 0,
        value: mix,
    });
    for p in 0..4 {
        rt.apply(Command::FxParam {
            slot: 0,
            p,
            value: 0.9,
        });
    }
    assert_eq!(
        (rt.tracks[2].fx.slots[0].mix, rt.tracks[2].fx.slots[0].p),
        before
    );
    // Legacy stored mix values do not affect this upstream event processor.
    rt.tracks[2].fx.slots[0].mix = mix;
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    rt
}

#[test]
fn arp_has_working_on_off_note_processing_and_no_meaningless_mix_or_parameter_edits() {
    let mut dry = arp(0.0);
    let mut wet = arp(1.0);
    for beat in [0.001, 0.251, 0.501, 0.751] {
        for rt in [&mut dry, &mut wet] {
            rt.beat = beat;
            rt.render_track(2, false);
        }
        assert_eq!(dry.tracks[2].arp_note, wet.tracks[2].arp_note);
        assert_eq!(
            dry.tracks[2].arp_note,
            Some([60, 64, 67, 60][(beat * 4.0) as usize])
        );
    }
    for rt in [&mut dry, &mut wet] {
        rt.apply(Command::FxToggle(0));
        rt.beat = 1.001;
        rt.render_track(2, false);
        let mut held: Vec<_> = rt.tracks[2]
            .poly
            .voices
            .iter()
            .filter(|v| matches!(v.env.stage, 1..=3))
            .map(|v| v.note())
            .collect();
        held.sort();
        assert_eq!(held, vec![60, 64, 67]);
    }
}
