use super::*;
use crate::engine::musical_context::{Context, Scale};

fn phrase() -> Content {
    crate::engine::midi_edit::initialize().unwrap();
    let notes = [(0, 60, 0, 1, 4), (1, 61, 960, 5, 8)]
        .into_iter()
        .map(|(channel, pitch, start_tick, start_order, end_order)| {
            MidiNote::from_smf(
                &crate::midi_file::Note {
                    channel,
                    pitch,
                    velocity: 91,
                    release_velocity: 37,
                    start_tick,
                    duration_ticks: 960,
                    start_order,
                    end_order,
                },
                960,
            )
            .unwrap()
        })
        .collect();
    Content {
        notes,
        ppqn: 960,
        end_tick: 3840,
        messages: vec![
            Message {
                tick: 480,
                order: 2,
                bytes: [0xa0, 60, 77],
                length: 3,
            },
            Message {
                tick: 720,
                order: 3,
                bytes: [0xb0, 1, 54],
                length: 3,
            },
        ],
        meta: vec![Meta {
            tick: 0,
            order: 0,
            value: crate::midi_file::MetaValue::Key {
                sharps: -2,
                minor: true,
            },
        }],
        labels: vec![Label {
            channel: 0,
            control: crate::engine::midi_data::ControlKind::Cc { controller: 1 },
            name: "Modulation retained".into(),
        }],
    }
}
fn parameters(kind: Kind, scale: Scale) -> Parameters {
    Parameters {
        kind,
        context: Some(Context { tonic: 0, scale }),
        degrees: 2,
        ..Default::default()
    }
}

#[test]
fn three_non_major_scale_transpositions_preserve_absolute_exceptions_source_ticks_and_owned_pressure(
) {
    let original = phrase();
    let selected = original.notes.iter().map(|n| n.id).collect();
    for (scale, pitches) in [
        (Scale::Dorian, [63, 61]),
        (Scale::Phrygian, [63, 65]),
        (Scale::HarmonicMinor, [63, 61]),
    ] {
        let next = prepare(
            &original,
            &selected,
            &parameters(Kind::ScaleTranspose, scale),
            &AtomicBool::new(false),
        )
        .unwrap()
        .content;
        assert_eq!(
            next.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            pitches
        );
        for (old, new) in original.notes.iter().zip(&next.notes) {
            assert_eq!(new.id, old.id);
            assert_eq!(new.source_timing, old.source_timing);
            assert_eq!(new.release_vel, 37);
            assert_eq!(new.channel, old.channel);
            assert_eq!(new.vel, old.vel);
        }
        assert_eq!(
            next.messages[0],
            Message {
                bytes: [0xa0, 63, 77],
                ..original.messages[0]
            }
        );
        assert_eq!(next.messages[1], original.messages[1]);
        assert_eq!(next.meta, original.meta);
        assert_eq!(next.labels, original.labels);
        assert_eq!(next.end_tick, original.end_tick);
    }
    let only_exception = BTreeSet::from([original.notes[1].id]);
    let mut explicit = parameters(Kind::ScaleTranspose, Scale::Dorian);
    explicit.degrees = 0;
    explicit.include_chromatic = true;
    let next = prepare(
        &original,
        &only_exception,
        &explicit,
        &AtomicBool::new(false),
    )
    .unwrap()
    .content;
    assert_eq!(next.notes[1].pitch, 60);
    assert_eq!(next.notes[0], original.notes[0]);
}

#[test]
fn harmony_retains_original_voices_and_metadata_and_adds_distinct_exact_timed_owned_pressure() {
    let original = phrase();
    let selected = original.notes.iter().map(|n| n.id).collect();
    let prepared = prepare(
        &original,
        &selected,
        &parameters(Kind::Harmony, Scale::Dorian),
        &AtomicBool::new(false),
    )
    .unwrap();
    let next = prepared.content;
    assert_eq!(&next.notes[..2], original.notes.as_slice());
    assert_eq!(next.notes.len(), 3);
    assert_eq!(next.notes[2].pitch, 63);
    assert_ne!(next.notes[2].id, original.notes[0].id);
    assert_ne!(next.notes[2].id, original.notes[1].id);
    let copied = next.notes[2].source_timing.unwrap();
    let base = original.notes[0].source_timing.unwrap();
    assert_eq!(
        (copied.ppqn, copied.start, copied.duration),
        (base.ppqn, base.start, base.duration)
    );
    assert!(copied.start_order > 8);
    assert!(copied.end_order > copied.start_order);
    assert!(next.messages.contains(&original.messages[0]));
    assert!(next.messages.contains(&original.messages[1]));
    let pressure = next
        .messages
        .iter()
        .find(|m| m.bytes == [0xa0, 63, 77])
        .unwrap();
    assert_eq!(pressure.tick, 480);
    assert!(pressure.order > copied.end_order);
    assert_eq!(next.meta, original.meta);
    assert_eq!(next.labels, original.labels);
    assert_eq!(prepared.summary.expression_events, 1);
    validate_wire(&next, &AtomicBool::new(false)).unwrap();
}

#[test]
fn harmony_refuses_mpe_aliases_cancelled_work_out_of_range_pitches_and_ambiguous_new_pressure_without_partial_content(
) {
    let original = phrase();
    let selected = original.notes.iter().map(|n| n.id).collect();
    assert!(prepare(
        &original,
        &selected,
        &parameters(Kind::Harmony, Scale::Dorian),
        &AtomicBool::new(true)
    )
    .is_err());
    let mut mpe = parameters(Kind::Harmony, Scale::Phrygian);
    mpe.expression = Expression::Lower(1);
    assert!(prepare(&original, &selected, &mpe, &AtomicBool::new(false))
        .unwrap_err()
        .contains("MPE"));
    let mut bounds = parameters(Kind::ScaleTranspose, Scale::Dorian);
    bounds.degrees = 128;
    assert!(prepare(&original, &selected, &bounds, &AtomicBool::new(false)).is_err());
    let mut alias = original.clone();
    let mut overlap = alias.notes[0].clone();
    overlap.id = NoteId::new();
    overlap.pitch = 63;
    overlap.source_timing.as_mut().unwrap().start_order = 9;
    overlap.source_timing.as_mut().unwrap().end_order = 10;
    alias.messages.push(Message {
        tick: 480,
        order: 11,
        bytes: [0xa0, 63, 77],
        length: 3,
    });
    alias.notes.push(overlap);
    let only_first = BTreeSet::from([alias.notes[0].id]);
    assert!(prepare(
        &alias,
        &only_first,
        &parameters(Kind::Harmony, Scale::Dorian),
        &AtomicBool::new(false)
    )
    .is_err());
}
