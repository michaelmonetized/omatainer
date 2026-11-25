use super::*;
use crate::engine::{midi_interchange, project, Command, Engine};

fn fixture() -> (crate::engine::Engine, crate::engine::RtEngine, Content) {
    let (engine, rt) = Engine::headless_for_test(48_000, 256);
    let clip = crate::engine::mpe::tests::expressive_clip();
    let lanes = clip.lanes.unwrap();
    let content = Content {
        notes: clip.notes,
        ppqn: lanes.ppqn,
        end_tick: lanes.end_tick,
        messages: lanes.messages.clone(),
        meta: lanes.meta.clone(),
        labels: lanes.labels.clone(),
    };
    (engine, rt, content)
}

#[test]
fn shared_member_edit_splits_ownership_and_preserves_other_voice_and_reuse_state() {
    let (_engine, mut rt, mut content) = fixture();
    content.notes[1].channel = 2;
    let id = content.notes[0].id;
    let other = content.notes[1].id;
    let cancel = AtomicBool::new(false);
    let envelope = inspect(&content, id, &cancel).unwrap();
    assert_eq!(envelope.initial.pitch, 24.0);
    let point = envelope
        .points
        .iter()
        .find(|point| point.dimension == Dimension::Pressure)
        .unwrap();
    let edited = edit(
        &content,
        id,
        Edit {
            dimension: Dimension::Pressure,
            beat: 0.75,
            value: Some(44.0),
            original: Some(point.message),
        },
        &cancel,
    )
    .unwrap();
    assert_ne!(edited.channel, 2);
    assert_eq!(edited.content.notes[0].id, id);
    assert_eq!(edited.content.notes[1].id, other);
    assert_eq!(edited.content.notes[1].channel, 2);
    assert_eq!(edited.maximum_rounding_beats, 0.0);
    let lanes = prepared(&edited.content, &cancel).unwrap();
    let state = lanes
        .expression
        .as_ref()
        .unwrap()
        .before(336.0, &lanes.state);
    assert_eq!(state.member(edited.channel), Values::default());
    rt.tracks[2].clips[0] = crate::engine::mpe::tests::expressive_clip();
    rt.tracks[2].clips[0].notes = edited.content.notes;
    rt.tracks[2].clips[0].lanes = Some(lanes);
    rt.quant = 0.0;
    rt.bpm = 120.0;
    rt.apply(Command::Play);
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    let allocation = crate::engine::test_alloc::measure(|| {
        for _ in 0..340 {
            rt.process(&mut [0.0; 256]);
        }
    });
    assert_eq!(allocation, crate::engine::test_alloc::Counts::default());
    let edited_voice = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.clip_note == Some(id))
        .unwrap();
    let other_voice = rt.tracks[2]
        .poly
        .voices
        .iter()
        .find(|voice| voice.clip_note == Some(other))
        .unwrap();
    assert_eq!(edited_voice.expression_values.pressure, 44);
    assert_eq!(other_voice.expression_values.pressure, 91);
    assert_eq!(edited_voice.expression_values.pitch, 24.0);
    assert_eq!(other_voice.expression_values.pitch, 24.0);
}

#[test]
fn a_single_member_zone_can_edit_and_remove_owned_points_without_changing_reuse() {
    let (_engine, _rt, mut content) = fixture();
    content.notes.truncate(1);
    content.notes[0].channel = 1;
    content
        .messages
        .retain(|message| message.bytes[0] & 15 == 0);
    content.messages[2].bytes[2] = 1;
    let id = content.notes[0].id;
    let cancel = AtomicBool::new(false);
    let edited = edit(
        &content,
        id,
        Edit {
            dimension: Dimension::Pitch,
            beat: 0.0,
            value: Some(12.0),
            original: None,
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(edited.channel, 1);
    let initial = inspect(&edited.content, id, &cancel).unwrap();
    assert_eq!(initial.initial.pitch, 12.0);
    let edited = edit(
        &edited.content,
        id,
        Edit {
            dimension: Dimension::Timbre,
            beat: 0.5,
            value: Some(100.0),
            original: None,
        },
        &cancel,
    )
    .unwrap();
    let inspection = inspect(&edited.content, id, &cancel).unwrap();
    let point = inspection
        .points
        .iter()
        .find(|point| point.dimension == Dimension::Timbre)
        .unwrap();
    let removed = edit(
        &edited.content,
        id,
        Edit {
            dimension: Dimension::Timbre,
            beat: point.beat,
            value: None,
            original: Some(point.message),
        },
        &cancel,
    )
    .unwrap();
    assert!(inspect(&removed.content, id, &cancel)
        .unwrap()
        .points
        .iter()
        .all(|point| point.dimension != Dimension::Timbre));
    let lanes = prepared(&removed.content, &cancel).unwrap();
    assert_eq!(
        lanes
            .expression
            .as_ref()
            .unwrap()
            .before(336.0, &lanes.state)
            .member(1),
        Values::default()
    );
}

#[test]
fn edited_envelopes_keep_native_identities_and_export_through_the_actual_smf_path() {
    let (_engine, _rt, content) = fixture();
    let id = content.notes[0].id;
    let cancel = AtomicBool::new(false);
    let edited = edit(
        &content,
        id,
        Edit {
            dimension: Dimension::Pitch,
            beat: 0.75,
            value: Some(-12.0),
            original: None,
        },
        &cancel,
    )
    .unwrap();
    let (mut state, media) = project::State::empty().unwrap();
    let clip = &mut state.tracks[2].clips[0];
    clip.kind = crate::engine::ClipKind::Midi;
    clip.bars = 1.0;
    clip.name = "Expression".into();
    clip.notes = edited.content.notes.clone();
    clip.lanes = Some(prepared(&edited.content, &cancel).unwrap());
    state.validate(&media).unwrap();
    let bytes = serde_json::to_vec(&state).unwrap();
    let reopened: project::State = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reopened.tracks[2].clips[0].notes[0].id, id);
    assert_eq!(
        reopened.tracks[2].clips[0].lanes,
        state.tracks[2].clips[0].lanes
    );
    let file = midi_interchange::export(
        &state,
        &[(2, 0)],
        midi_interchange::ExportOptions {
            ppqn: 96,
            single_track: true,
            include_muted: true,
            allow_rounding: false,
            session_conductor: false,
        },
    )
    .unwrap();
    let encoded = crate::midi_file::encode(&file, false).unwrap();
    let decoded = crate::midi_file::decode(&encoded).unwrap();
    let original: Vec<_> = file.tracks[0]
        .messages
        .iter()
        .map(|message| (message.tick, message.bytes, message.length))
        .collect();
    let returned: Vec<_> = decoded.tracks[0]
        .messages
        .iter()
        .map(|message| (message.tick, message.bytes, message.length))
        .collect();
    assert_eq!(returned, original);
    assert_eq!(
        decoded.tracks[0]
            .notes
            .iter()
            .map(|note| (
                note.channel,
                note.pitch,
                note.start_tick,
                note.duration_ticks
            ))
            .collect::<Vec<_>>(),
        file.tracks[0]
            .notes
            .iter()
            .map(|note| (
                note.channel,
                note.pitch,
                note.start_tick,
                note.duration_ticks
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn invalid_or_cancelled_edits_preserve_source_and_exhausted_zones_refuse_voice_merges() {
    let (_engine, _rt, content) = fixture();
    let baseline = content.clone();
    let id = content.notes[0].id;
    let cancel = AtomicBool::new(false);
    for (beat, value) in [(2.0, 0.0), (0.0, f64::NAN), (0.0, 48.0)] {
        assert!(edit(
            &content,
            id,
            Edit {
                dimension: Dimension::Pitch,
                beat,
                value: Some(value),
                original: None
            },
            &cancel
        )
        .is_err());
    }
    cancel.store(true, Ordering::Release);
    assert!(inspect(&content, id, &cancel).is_err());
    assert!(edit(
        &content,
        id,
        Edit {
            dimension: Dimension::Pressure,
            beat: 0.5,
            value: Some(20.0),
            original: None
        },
        &cancel
    )
    .is_err());
    assert_eq!(content, baseline);
    cancel.store(false, Ordering::Release);
    let mut shared = content;
    shared.notes[0].channel = 1;
    shared.notes[1].channel = 1;
    shared.messages.retain(|message| message.bytes[0] & 15 == 0);
    shared.messages[2].bytes[2] = 1;
    let baseline = shared.clone();
    assert!(edit(
        &shared,
        id,
        Edit {
            dimension: Dimension::Pressure,
            beat: 0.5,
            value: Some(20.0),
            original: None
        },
        &cancel
    )
    .unwrap_err()
    .contains("occupied"));
    assert_eq!(shared, baseline);
}
