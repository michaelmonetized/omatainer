use super::*;

fn fixture(two_tones: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    for deck in 0..DECKS {
        rt.apply(Command::DeckUnload { deck: deck as u8 });
    }
    for track in &mut rt.tracks {
        track.fx.slots.clear();
    }
    rt.tracks[1].pan = -1.0;
    rt.tracks[2].pan = 1.0;
    rt.tracks[1].poly.note_on(57, 0.8);
    if two_tones {
        rt.tracks[2].poly.note_on(81, 0.7);
    }
    rt
}

#[test]
fn all_bypassed_scene_devices_preserve_independent_panned_tones_exactly() {
    let mut dry = fixture(true);
    let mut bypass = fixture(true);
    bypass.apply(Command::OpenFxScene(0));
    for (slot, _) in fx::FxId::all().iter().enumerate() {
        bypass.apply(Command::FxAdd(slot as u8));
        bypass.apply(Command::FxToggle(slot));
    }
    let mut expected = vec![0.0; 16_384];
    let mut actual = vec![0.0; 16_384];
    dry.process(&mut expected);
    bypass.process(&mut actual);
    assert_eq!(actual, expected);
    assert!(actual
        .chunks_exact(2)
        .any(|frame| (frame[0] - frame[1]).abs() > 0.01));
    for channel in 0..2 {
        assert!(actual
            .chunks_exact(2)
            .any(|frame| frame[channel].abs() > 0.01));
    }
}

#[test]
fn enabled_scene_processors_do_not_feed_a_panned_source_into_the_empty_channel() {
    for id in fx::FxId::all() {
        let mut rt = fixture(false);
        let mut slot = fx::FxSlot::new(*id, rt.sr);
        slot.mix = 1.0;
        rt.scene_fx.slots.push(slot);
        let mut output = vec![0.0; 48_000 * 2];
        rt.process(&mut output);
        assert!(
            output.chunks_exact(2).any(|frame| frame[0].abs() > 1e-6),
            "{id:?}"
        );
        assert!(
            output.chunks_exact(2).all(|frame| frame[1] == 0.0),
            "{id:?}"
        );
    }
}

#[test]
fn explicitly_selected_zero_width_is_mono_and_center_width_preserves_stereo() {
    let mut mono = fixture(true);
    let mut slot = fx::FxSlot::new(fx::FxId::Spread, mono.sr);
    slot.p[0] = 0.0;
    mono.scene_fx.slots.push(slot);
    let mut output = vec![0.0; 4096];
    mono.process(&mut output);
    assert!(output.iter().any(|x| x.abs() > 0.01));
    assert!(output.chunks_exact(2).all(|frame| frame[0] == frame[1]));

    let mut center = fixture(true);
    center
        .scene_fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Spread, center.sr));
    let mut reference = fixture(true);
    let mut expected = vec![0.0; output.len()];
    center.process(&mut output);
    reference.process(&mut expected);
    assert_eq!(output, expected);
}
