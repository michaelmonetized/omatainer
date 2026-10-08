use super::*;
use crate::engine::midi_data::{ControlKind, TickTiming};
use crate::midi_file::{MetaValue, Note};
fn note(
    channel: u8,
    pitch: u8,
    start: u64,
    duration: u64,
    orders: (u32, u32),
    vel: u8,
) -> MidiNote {
    super::super::midi_edit::initialize().unwrap();
    MidiNote::from_smf(
        &Note {
            channel,
            pitch,
            velocity: vel,
            release_velocity: 17,
            start_tick: start,
            duration_ticks: duration,
            start_order: orders.0,
            end_order: orders.1,
        },
        960,
    )
    .unwrap()
}
fn source() -> Content {
    let m = |tick, order, bytes: [u8; 3]| Message {
        tick,
        order,
        bytes,
        length: if matches!(bytes[0] & 0xf0, 0xc0 | 0xd0) {
            2
        } else {
            3
        },
    };
    Content {
        notes: vec![
            note(1, 60, 48, 432, (1, 4), 80),
            note(2, 67, 96, 576, (5, 9), 100),
            note(3, 72, 800, 100, (12, 13), 90),
        ],
        ppqn: 960,
        end_tick: 960,
        messages: vec![
            m(48, 0, [0xe1, 0, 64]),
            m(264, 2, [0xd1, 75, 0]),
            m(400, 3, [0xe1, 8, 70]),
            m(300, 6, [0xb2, 74, 87]),
            m(384, 7, [0xa2, 67, 66]),
            m(240, 10, [0xe0, 0, 64]),
            m(300, 11, [0xb1, 1, 23]),
            m(850, 14, [0xd3, 22, 0]),
        ],
        meta: vec![Meta {
            tick: 0,
            order: 15,
            value: MetaValue::Text {
                kind: 3,
                bytes: b"Expression motif".to_vec(),
            },
        }],
        labels: vec![Label {
            channel: 1,
            control: ControlKind::Pressure,
            name: "Finger pressure".into(),
        }],
    }
}
fn selected(source: &Content) -> BTreeSet<NoteId> {
    source.notes[..2].iter().map(|n| n.id).collect()
}
fn run(source: &Content, params: &Parameters) -> Prepared {
    prepare(source, &selected(source), params, &AtomicBool::new(false)).unwrap()
}
fn event(content: &Content, order: u32) -> Message {
    *content.messages.iter().find(|m| m.order == order).unwrap()
}
fn same_instrument(before: &MidiNote, after: &MidiNote) {
    assert_eq!(
        (before.id, before.channel, before.release_vel, before.muted),
        (after.id, after.channel, after.release_vel, after.muted)
    );
}
#[test]
fn partial_quantize_start_and_end_targets_retime_only_owned_expression_at_source_precision() {
    let source = source();
    let p = Parameters {
        strength: 0.5,
        expression: Expression::Lower(15),
        ..Parameters::default()
    };
    let result = run(&source, &p);
    assert_eq!(result.content.notes[0].source_timing.unwrap().start, 24);
    assert_eq!(result.content.notes[0].source_timing.unwrap().duration, 432);
    assert_eq!(result.content.notes[1].source_timing.unwrap().start, 48);
    assert_eq!(result.content.notes[1].source_timing.unwrap().duration, 576);
    for (order, tick) in [(0, 24), (2, 240), (3, 376), (6, 252), (7, 336)] {
        assert_eq!(event(&result.content, order).tick, tick);
        assert_eq!(
            event(&result.content, order).bytes,
            event(&source, order).bytes
        );
    }
    for order in [10, 11, 14] {
        assert_eq!(event(&result.content, order), event(&source, order));
    }
    assert_eq!(result.content.notes[2], source.notes[2]);
    assert_eq!(result.content.meta, source.meta);
    assert_eq!(result.content.labels, source.labels);
    assert_eq!(result.summary.expression_events, 5);
    assert_eq!(result.summary.original.maximum_overlap, 2);
    assert_eq!(result.summary.transformed.maximum_overlap, 2);
    assert_eq!(result.summary.original.total_gap, 0.0);
    assert!(result.summary.maximum_rounding_beats < 1e-12);
    assert!((result.summary.mean_start_shift + 0.0375).abs() < 1e-12);
    for (old, new) in source.notes.iter().zip(&result.content.notes) {
        same_instrument(old, new);
    }
    let ends = run(
        &source,
        &Parameters {
            starts: false,
            ends: true,
            grid: 1.0 / 3.0,
            ..p
        },
    );
    assert_eq!(ends.content.notes[0].source_timing.unwrap().start, 48);
    assert_eq!(ends.content.notes[0].source_timing.unwrap().duration, 512);
    assert_eq!(ends.content.notes[1].source_timing.unwrap().start, 96);
    assert_eq!(ends.content.notes[1].source_timing.unwrap().duration, 560);
    assert_eq!(event(&ends.content, 3).tick, 465);
    assert!(ends.summary.maximum_rounding_beats <= 0.5 / 960.0 + 1e-12);
    let both = run(
        &source,
        &Parameters {
            starts: true,
            ends: true,
            strength: 1.0,
            grid: 0.25,
            ..Parameters::default()
        },
    );
    assert_eq!(
        both.content.notes[0].source_timing.unwrap(),
        TickTiming {
            ppqn: 960,
            start: 0,
            duration: 480,
            start_order: 1,
            end_order: 4
        }
    );
    assert_eq!(both.content.notes[1].source_timing.unwrap().duration, 720);
}
#[test]
fn each_recombination_dimension_is_independent_and_seeded_order_is_repeatable() {
    let source = source();
    for property in [
        Property::Pitch,
        Property::Position,
        Property::Length,
        Property::Velocity,
    ] {
        let p = Parameters {
            kind: Kind::Recombine,
            property,
            rotation: 1,
            expression: Expression::Lower(15),
            ..Parameters::default()
        };
        let result = run(&source, &p);
        let back = run(&result.content, &p);
        for i in 0..2 {
            let old = &source.notes[i];
            let new = &result.content.notes[i];
            let donor = &source.notes[1 - i];
            same_instrument(old, new);
            assert_eq!(
                new.pitch,
                if property == Property::Pitch {
                    donor.pitch
                } else {
                    old.pitch
                }
            );
            assert_eq!(
                new.vel,
                if property == Property::Velocity {
                    donor.vel
                } else {
                    old.vel
                }
            );
            assert_eq!(
                new.source_start(),
                if property == Property::Position {
                    donor.source_start()
                } else {
                    old.source_start()
                }
            );
            assert_eq!(
                new.source_duration(),
                if property == Property::Length {
                    donor.source_duration()
                } else {
                    old.source_duration()
                }
            );
            assert_eq!(back.content.notes[i], source.notes[i]);
        }
        assert_eq!(result.content.notes[2], source.notes[2]);
        assert_eq!(event(&result.content, 10), event(&source, 10));
        if property == Property::Pitch {
            assert_eq!(event(&result.content, 7).bytes, [0xa2, 60, 66]);
        }
    }
    let p = Parameters {
        kind: Kind::Recombine,
        shuffle: true,
        seed: 529,
        property: Property::Velocity,
        ..Parameters::default()
    };
    let a = run(&source, &p);
    let b = run(&source, &p);
    assert_eq!(a.content, b.content);
    assert_eq!(a.summary, b.summary);
    let all = source.notes.iter().map(|n| n.id).collect();
    let mut permutations = BTreeSet::new();
    for seed in 0..32 {
        let r = prepare(
            &source,
            &all,
            &Parameters { seed, ..p.clone() },
            &AtomicBool::new(false),
        )
        .unwrap();
        permutations.insert(r.content.notes.iter().map(|n| n.vel).collect::<Vec<_>>());
    }
    assert!(permutations.len() > 1);
    assert!(permutations.len() <= 6);
}
#[test]
fn stretch_reverse_and_integrated_speed_curve_preserve_expression_and_round_trip() {
    let source = source();
    let stretch = Parameters {
        kind: Kind::Stretch,
        stretch: 1.5,
        expression: Expression::Lower(15),
        ..Parameters::default()
    };
    let result = run(&source, &stretch);
    assert_eq!(result.content.notes[0].source_timing.unwrap().start, 48);
    assert_eq!(result.content.notes[0].source_timing.unwrap().duration, 648);
    assert_eq!(result.content.notes[1].source_timing.unwrap().start, 120);
    assert_eq!(result.content.notes[1].source_timing.unwrap().duration, 864);
    assert_eq!(event(&result.content, 2).tick, 372);
    assert_eq!(event(&result.content, 6).tick, 426);
    assert_eq!(result.content.notes[2], source.notes[2]);
    assert_eq!(result.content.end_tick, 984);
    let reverse = Parameters {
        kind: Kind::Reverse,
        expression: Expression::Lower(15),
        ..Parameters::default()
    };
    let reversed = run(&source, &reverse);
    assert_eq!(reversed.content.notes[0].source_timing.unwrap().start, 240);
    assert_eq!(reversed.content.notes[1].source_timing.unwrap().start, 48);
    assert_eq!(event(&reversed.content, 0).tick, 672);
    assert_eq!(event(&reversed.content, 2).tick, 456);
    assert_eq!(event(&reversed.content, 6).tick, 420);
    let back = run(&reversed.content, &reverse);
    assert_eq!(back.content.notes, source.notes);
    for m in &source.messages {
        assert_eq!(event(&back.content, m.order), *m);
    }
    let warp = Parameters {
        kind: Kind::Warp,
        ends: true,
        warp_time: 0.5,
        warp_speed: [1.0, 2.0, 1.0],
        expression: Expression::Lower(15),
        ..Parameters::default()
    };
    let warped = run(&source, &warp);
    assert!((warped.summary.original.end - warped.summary.transformed.end).abs() < 1e-12);
    assert_eq!(warped.content.notes[0].source_timing.unwrap().start, 48);
    let expected = 48.0 + 624.0 * ((1.0_f64 + 2.0 * 48.0 / 624.0).ln() / 2.0) / (2.0_f64.ln());
    assert!((warped.content.notes[1].source_timing.unwrap().start as f64 - expected).abs() <= 0.5);
    let expected = 48.0 + 624.0 * ((1.0_f64 + 2.0 * 252.0 / 624.0).ln() / 2.0) / (2.0_f64.ln());
    assert!((event(&warped.content, 6).tick as f64 - expected).abs() <= 0.5);
    assert_eq!(warped.content.notes[2], source.notes[2]);
    assert!(warped.summary.maximum_rounding_beats <= 0.5 / 960.0 + 1e-12);
    let uniform = run(
        &source,
        &Parameters {
            warp_speed: [2.0; 3],
            preserve_range: false,
            ..warp
        },
    );
    assert_eq!(
        uniform.content.notes[0].source_timing.unwrap().duration,
        216
    );
    assert_eq!(uniform.content.notes[1].source_timing.unwrap().start, 72);
}
#[test]
fn drawn_and_cyclic_velocity_curves_change_only_selected_velocity() {
    let mut source = source();
    source.notes.insert(1, note(4, 65, 72, 120, (16, 17), 90));
    let all = source.notes[..3].iter().map(|n| n.id).collect();
    let p = Parameters {
        kind: Kind::Velocity,
        velocity: [20, 120],
        ..Parameters::default()
    };
    let r = prepare(&source, &all, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        r.content.notes[..3]
            .iter()
            .map(|n| n.vel)
            .collect::<Vec<_>>(),
        [20, 70, 120]
    );
    assert_eq!(r.content.messages, source.messages);
    assert_eq!(r.content.notes[3], source.notes[3]);
    for (old, new) in source.notes.iter().zip(&r.content.notes) {
        let mut n = new.clone();
        n.vel = old.vel;
        assert_eq!(n, *old);
    }
    let p = Parameters {
        curve: std::array::from_fn(|i| if i < CURVE_POINTS / 2 { 0.0 } else { 1.0 }),
        cycles: 2,
        phase: 0.25,
        strength: 0.5,
        ..p
    };
    let r = prepare(&source, &all, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        r.content.notes[..3]
            .iter()
            .map(|n| n.vel)
            .collect::<Vec<_>>(),
        [50, 55, 60]
    );
    assert_eq!(r.summary.expression_events, 0);
}
#[test]
fn ambiguity_foreign_expression_and_fifo_pairing_refuse_without_changing_input() {
    let mut source = source();
    source.notes[1].channel = 1;
    source
        .messages
        .iter_mut()
        .filter(|m| m.bytes[0] & 15 == 2)
        .for_each(|m| m.bytes[0] = (m.bytes[0] & 0xf0) | 1);
    let original = source.clone();
    let p = Parameters {
        kind: Kind::Stretch,
        stretch: 1.1,
        expression: Expression::Lower(15),
        ..Parameters::default()
    };
    assert!(
        prepare(&source, &selected(&source), &p, &AtomicBool::new(false))
            .unwrap_err()
            .contains("multiple possible")
    );
    assert_eq!(source, original);
    let mut source = super::tests::source();
    source.messages.push(Message {
        tick: 480,
        order: 20,
        bytes: [0xe1, 0, 64],
        length: 3,
    });
    let original = source.clone();
    assert!(prepare(
        &source,
        &selected(&source),
        &Parameters {
            kind: Kind::Reverse,
            ..p.clone()
        },
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("unowned"));
    assert_eq!(source, original);
    let source = super::tests::source();
    let velocity = run(
        &source,
        &Parameters {
            kind: Kind::Velocity,
            ..p
        },
    );
    assert_eq!(velocity.content.messages, source.messages);
    let mut source = super::tests::source();
    source.notes[0].channel = 1;
    source.notes[1].channel = 1;
    source.notes[0].pitch = 60;
    source.notes[1].pitch = 60;
    source.messages.clear();
    let original = source.clone();
    assert!(prepare(
        &source,
        &selected(&source),
        &Parameters {
            kind: Kind::Recombine,
            property: Property::Position,
            rotation: 1,
            ..Parameters::default()
        },
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("pairing"));
    assert_eq!(source, original);
}
#[test]
fn late_source_ticks_cancellation_and_invalid_metadata_keep_original_content() {
    let mut source = source();
    let late = Note {
        channel: 1,
        pitch: 60,
        velocity: 80,
        release_velocity: 5,
        start_tick: 32767 * 262144 - 100,
        duration_ticks: 10,
        start_order: 21,
        end_order: 22,
    };
    let original = MidiNote::from_smf(&late, 32767).unwrap();
    source.notes = vec![original.clone()];
    source.ppqn = 32767;
    source.end_tick = 32767 * 262144;
    source.messages.clear();
    source.meta.clear();
    source.labels.clear();
    let selection = source.notes.iter().map(|n| n.id).collect();
    let p = Parameters {
        kind: Kind::Stretch,
        stretch: 2.0,
        ..Parameters::default()
    };
    let result = prepare(&source, &selection, &p, &AtomicBool::new(false)).unwrap();
    let ticks = result.content.notes[0].source_timing.unwrap();
    assert_eq!(ticks.start, late.start_tick);
    assert_eq!(ticks.duration, 20);
    assert_eq!(result.content.notes[0].id, original.id);
    assert!(result.content.notes[0].interchange_valid());
    assert!(prepare(&source, &selection, &p, &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    for p in [
        Parameters {
            strength: f64::NAN,
            ..p.clone()
        },
        Parameters {
            expression: Expression::Upper(16),
            ..p.clone()
        },
        Parameters {
            warp_speed: [0.0; 3],
            ..p.clone()
        },
        Parameters {
            velocity: [120, 20],
            ..p.clone()
        },
        Parameters {
            rotation: i32::MIN,
            ..p.clone()
        },
    ] {
        assert!(prepare(&source, &selection, &p, &AtomicBool::new(false)).is_err());
    }
    assert_eq!(source.notes[0], original);
}

#[test]
fn complete_eight_thousand_note_expression_phrase_uses_the_bounded_worker_path() {
    let count = 8192usize;
    let mut notes = Vec::with_capacity(count);
    let mut messages = Vec::with_capacity(count * 28);
    for i in 0..count {
        let order = i as u32 * 30;
        notes.push(note(1, 60, i as u64 * 960, 900, (order, order + 29), 80));
        for j in 0..28 {
            messages.push(Message {
                tick: i as u64 * 960 + j * 32,
                order: order + j as u32 + 1,
                bytes: [0xd1, (j * 4) as u8, 0],
                length: 2,
            });
        }
    }
    let source = Content {
        notes,
        ppqn: 960,
        end_tick: count as u64 * 960,
        messages,
        meta: vec![],
        labels: vec![],
    };
    let selected = source.notes.iter().map(|n| n.id).collect();
    let before = source.clone();
    let cancel = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let result = prepare(
        &source,
        &selected,
        &Parameters {
            kind: Kind::Stretch,
            stretch: 1.5,
            expression: Expression::Lower(1),
            ..Parameters::default()
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(result.summary.transformed.notes, 8192);
    assert_eq!(result.summary.expression_events, 229376);
    assert_eq!(result.summary.original.maximum_overlap, 1);
    assert_eq!(result.summary.transformed.maximum_overlap, 1);
    assert_eq!(result.content.notes[1].source_timing.unwrap().start, 1440);
    assert_eq!(
        result.content.notes[1].source_timing.unwrap().duration,
        1350
    );
    assert_eq!(event(&result.content, 44).tick, 2064);
    assert_eq!(source, before);
    println!("MIDI_TRANSFORM_CAPACITY {{\"selected_notes\":8192,\"linked_expression_events\":229376,\"worker_preparation_ms\":{},\"physical_devices_opened\":false}}",started.elapsed().as_millis());
}
