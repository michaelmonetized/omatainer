use super::*;

#[test]
fn track_template_removes_song_content_and_keeps_native_devices_and_required_audio() {
    let mut live = rt();
    live.tracks[2].poly.cutoff = 1555.0;
    live.tracks[2].gain = 0.22;
    live.tracks[2].poly.offline = Some(Arc::new(
        fx::OfflineDevice::new(
            "org.example.template-synth".into(),
            Some(fx::DeviceState {
                schema: 2,
                data: vec![0, 42, 255],
            }),
        )
        .unwrap(),
    ));
    let original = captured(&live);
    let before = serde_json::to_value(&original.state).unwrap();
    let (state, media) = original
        .state
        .track_configuration(&original.media, 2)
        .unwrap();
    assert_eq!(state.tracks.len(), 1);
    assert_eq!(state.scene_fx.len(), 1);
    assert_eq!(state.tracks[0].gain, 0.22);
    assert_eq!(state.tracks[0].synth.cutoff, 1555.0);
    assert_eq!(
        state.tracks[0].synth.offline,
        original.state.tracks[2].synth.offline
    );
    assert!(state.tracks[0]
        .clips
        .iter()
        .all(|clip| clip.kind == ClipKind::Empty && clip.notes.is_empty() && clip.audio.is_none()));
    assert!(state
        .decks
        .iter()
        .all(|deck| deck.audio.is_none() && deck.title.is_empty()));
    assert!(state
        .banks
        .iter()
        .all(|bank| bank.media.iter().all(Option::is_none)));
    assert!(media.len() <= 6);
    assert_ne!(
        state.session.as_ref().unwrap().namespace,
        original.state.session.as_ref().unwrap().namespace
    );
    assert_eq!(serde_json::to_value(&original.state).unwrap(), before);
    for (source, rebound) in original.state.tracks[2]
        .drums
        .iter()
        .zip(state.tracks[0].drums)
    {
        assert!(Arc::ptr_eq(&original.media[*source], &media[rebound]));
    }
    let reopened: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let prepared = Prepared::from_state(reopened, media, 48_000).unwrap();
    assert_eq!(
        prepared.rt.tracks[0].poly.offline,
        live.tracks[2].poly.offline
    );
    assert!(!prepared.rt.playing && prepared.rt.decks.iter().all(|deck| !deck.playing));
}

#[test]
fn applying_track_template_preserves_other_music_and_requires_exact_unique_bus_alias() {
    let original = captured(&rt());
    let before = serde_json::to_value(&original.state).unwrap();
    let (mut configuration, source_media) = original
        .state
        .track_configuration(&original.media, 0)
        .unwrap();
    configuration.tracks[0].name = "Reusable track".into();
    configuration.session.as_mut().unwrap().tracks[0].name = "Reusable track".into();
    configuration.tracks[0].gain = 0.23;
    let bus = original.state.session.as_ref().unwrap().scenes[0]
        .name
        .clone();
    for alias in ["unavailable alias", ""] {
        assert!(original
            .state
            .clone()
            .apply_track_configuration(
                original.media.clone(),
                &configuration,
                &source_media,
                2,
                alias
            )
            .is_err());
        assert_eq!(serde_json::to_value(&original.state).unwrap(), before);
    }
    let mut ambiguous = original.state.clone();
    ambiguous.session.as_mut().unwrap().scenes[1].name = bus.clone();
    assert!(ambiguous
        .apply_track_configuration(
            original.media.clone(),
            &configuration,
            &source_media,
            2,
            &bus
        )
        .is_err());
    let (state, media) = original
        .state
        .clone()
        .apply_track_configuration(
            original.media.clone(),
            &configuration,
            &source_media,
            2,
            &bus,
        )
        .unwrap();
    assert_eq!(state.tracks[2].gain, 0.23);
    assert_eq!(
        state.tracks[2].clips[0].notes,
        original.state.tracks[2].clips[0].notes
    );
    assert_eq!(
        state.session.as_ref().unwrap().namespace,
        original.state.session.as_ref().unwrap().namespace
    );
    assert_eq!(
        state.session.as_ref().unwrap().tracks[2].id,
        original.state.session.as_ref().unwrap().tracks[2].id
    );
    let after = serde_json::to_value(&state).unwrap();
    for index in [0, 1, 3] {
        assert_eq!(after["tracks"][index], before["tracks"][index]);
    }
    let mut prepared = Prepared::from_state(state, media, 48_000).unwrap();
    prepared.rt.apply(Command::LaunchScene { scene: 0 });
    let mut output = [0.0; 1024];
    prepared.rt.process(&mut output);
    assert!(output.iter().all(|sample| sample.is_finite()));
    assert!(output.iter().any(|sample| *sample != 0.0));
}
