use super::*;
use crate::engine::{
    midi_data::TickTiming,
    musical_context::{Context, Scale},
};
fn empty() -> Content {
    crate::engine::midi_edit::initialize().unwrap();
    Content {
        notes: Vec::new(),
        ppqn: 960,
        end_tick: 16 * 960,
        messages: Vec::new(),
        meta: Vec::new(),
        labels: Vec::new(),
    }
}
fn parameters(kind: Kind) -> Parameters {
    Parameters {
        kind,
        context: Some(Context {
            tonic: 0,
            scale: Scale::Dorian,
        }),
        ..Default::default()
    }
}
fn source() -> Content {
    let mut content = empty();
    content.notes = [60, 64, 67]
        .into_iter()
        .enumerate()
        .map(|(index, pitch)| MidiNote {
            id: NoteId::new(),
            pitch,
            channel: 0,
            start: 1.0,
            len: 1.0,
            vel: 90,
            release_vel: 31,
            muted: false,
            source_timing: Some(TickTiming {
                ppqn: 960,
                start: 960,
                duration: 960,
                start_order: index as u32 * 3 + 1,
                end_order: index as u32 * 3 + 3,
            }),
            variation: None,
        })
        .collect();
    content
}
fn selection(content: &Content) -> BTreeSet<NoteId> {
    content.notes.iter().map(|note| note.id).collect()
}
fn run(content: &Content, params: &Parameters) -> Prepared {
    super::super::prepare(
        content,
        &selection(content),
        params,
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn musical(content: &Content) -> Vec<(u8, u8, u64, u64, u8, u8)> {
    content
        .notes
        .iter()
        .map(|note| {
            (
                note.channel,
                note.pitch,
                note.source_start().to_bits(),
                note.source_duration().to_bits(),
                note.vel,
                note.release_vel,
            )
        })
        .collect()
}
#[test]
fn eight_editable_chords_keep_saved_scale_ordered_voices_inversions_and_minimal_movement() {
    let original = empty();
    let params = parameters(Kind::Chords);
    let first = run(&original, &params);
    assert_eq!(first.content.notes.len(), 24);
    for (index, chord) in first.content.notes.chunks(3).enumerate() {
        assert!(chord.windows(2).all(|pair| pair[0].pitch < pair[1].pitch));
        assert!(chord
            .iter()
            .all(|note| params.context.unwrap().contains(note.pitch)));
        assert!(chord.iter().all(|note| note.source_start() == index as f64
            && note.vel == params.velocity[1]
            && note.source_duration() > 0.0));
    }
    assert_eq!(original.notes.len(), 0);
    for voicing in [Voicing::Closed, Voicing::Open, Voicing::DropTwo] {
        let mut changed = params.clone();
        changed.composition.voicing = voicing;
        changed.composition.register = [36, 108];
        changed.composition.chords[1].inversion = 1;
        let candidate = run(&original, &changed);
        assert_eq!(candidate.content.notes.len(), 24);
        assert!(candidate
            .content
            .notes
            .iter()
            .all(|note| (36..=108).contains(&note.pitch)));
        assert_ne!(musical(&first.content), musical(&candidate.content));
    }
    let options = vec![vec![48, 60, 72], vec![52, 64, 76], vec![55, 67, 79]];
    let previous = [62, 65, 69];
    let chosen = closest_voicing(&options, &previous, &AtomicBool::new(false)).unwrap();
    assert_eq!(chosen, [60, 64, 67]);
    let chosen_cost: u32 = chosen
        .iter()
        .zip(previous)
        .map(|(a, b)| u32::from(a.abs_diff(b)))
        .sum();
    for &a in &options[0] {
        for &b in &options[1] {
            for &c in &options[2] {
                if a < b && b < c {
                    let cost: u32 = [a, b, c]
                        .iter()
                        .zip(previous)
                        .map(|(a, b)| u32::from(a.abs_diff(b)))
                        .sum();
                    assert!(chosen_cost <= cost);
                }
            }
        }
    }
}
#[test]
fn borrowed_chords_require_permission_and_register_or_tick_failures_keep_source_unchanged() {
    let original = source();
    let before = musical(&original);
    let mut params = parameters(Kind::Chords);
    params.composition.replace = true;
    params.composition.chords[0].alteration = 1;
    assert!(super::super::prepare(
        &original,
        &selection(&original),
        &params,
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("permission"));
    params.include_chromatic = true;
    let borrowed = run(&original, &params);
    assert!(borrowed
        .content
        .notes
        .iter()
        .any(|note| !params.context.unwrap().contains(note.pitch)));
    params.composition.register = [127, 127];
    assert!(super::super::prepare(
        &original,
        &selection(&original),
        &params,
        &AtomicBool::new(false)
    )
    .is_err());
    assert_eq!(musical(&original), before);
    let mut coarse = empty();
    coarse.ppqn = 1;
    coarse.end_tick = 16;
    let mut melody = parameters(Kind::Melody);
    melody.grid = 1.0 / 1024.0;
    melody.composition.length = 1.0;
    assert!(
        super::super::prepare(&coarse, &BTreeSet::new(), &melody, &AtomicBool::new(false))
            .unwrap_err()
            .contains("collapse")
    );
}
#[test]
fn ten_seeded_contours_are_repeatable_bounded_and_density_or_duration_never_leaks_notes() {
    let original = empty();
    let mut params = parameters(Kind::Melody);
    params.grid = 0.125;
    params.composition.register = [55, 79];
    params.composition.density = 0.58;
    params.composition.pitch_variation = 0.4;
    params.composition.gate = [0.3, 0.95];
    params.velocity = [31, 92];
    let mut alternatives = BTreeSet::new();
    for seed in 0..10 {
        params.seed = seed;
        let candidate = run(&original, &params);
        let again = run(&original, &params);
        assert_eq!(musical(&candidate.content), musical(&again.content));
        assert!(!candidate.content.notes.is_empty());
        assert!(candidate.content.notes.len() <= 64);
        assert!(candidate
            .content
            .notes
            .iter()
            .all(|note| (55..=79).contains(&note.pitch)
                && params.context.unwrap().contains(note.pitch)
                && (31..=92).contains(&note.vel)
                && note.source_duration() > 0.0
                && note.source_duration() <= params.grid
                && note.variation.is_none()));
        alternatives.insert(musical(&candidate.content));
    }
    assert_eq!(alternatives.len(), 10);
    params.composition.density = 0.0;
    assert!(run(&original, &params).content.notes.is_empty());
    assert_eq!(run(&original, &params).summary.transformed.first, 0.0);
    params.composition.density = 1.0;
    let full = run(&original, &params);
    assert_eq!(full.content.notes.len(), 64);
    assert!(full
        .content
        .notes
        .windows(2)
        .all(|pair| pair[0].source_start() + pair[0].source_duration() <= pair[1].source_start()));
    assert!(original.notes.is_empty());
}
#[test]
fn strum_flam_glissando_and_legato_keep_individual_notes_editable_and_release_pairing_valid() {
    let original = source();
    let mut params = parameters(Kind::Articulate);
    params.composition.replace = true;
    params.grid = 0.125;
    params.composition.gate = [0.7, 0.8];
    let strum = run(&original, &params);
    assert_eq!(strum.content.notes.len(), 3);
    assert_eq!(
        strum
            .content
            .notes
            .iter()
            .map(|note| note.source_start())
            .collect::<Vec<_>>(),
        [1.0, 1.125, 1.25]
    );
    assert!(strum
        .content
        .notes
        .iter()
        .all(|note| !selection(&original).contains(&note.id)));
    params.composition.descending = true;
    let descending = run(&original, &params);
    assert_eq!(
        descending
            .content
            .notes
            .iter()
            .map(|note| note.pitch)
            .collect::<Vec<_>>(),
        [67, 64, 60]
    );
    params.composition.descending = false;
    for (mode, count) in [
        (Articulation::Flam, 6),
        (Articulation::Glissando, 24),
        (Articulation::Repeat, 24),
        (Articulation::Arpeggio, 8),
        (Articulation::Grace, 6),
        (Articulation::Legato, 3),
    ] {
        params.composition.articulation = mode;
        params.composition.interval = 7;
        let generated = run(&original, &params);
        assert_eq!(generated.content.notes.len(), count);
        assert!(generated
            .content
            .notes
            .iter()
            .all(|note| note.source_duration() > 0.0 && note.source_timing.unwrap().duration > 0));
        validate_wire(&generated.content, &AtomicBool::new(false)).unwrap();
        let mut edited = generated.content.clone();
        edited.notes[0].vel = 17;
        assert_eq!(edited.notes[0].vel, 17);
        assert_ne!(musical(&edited), musical(&generated.content));
    }
    assert_eq!(original.notes.len(), 3);
}
#[test]
fn copied_note_expression_follows_strum_and_repetition_without_stealing_unrelated_automation() {
    let mut original = source();
    original.messages = original
        .notes
        .iter()
        .enumerate()
        .map(|(index, note)| Message {
            tick: 1200,
            order: index as u32 * 3 + 2,
            bytes: [0xa0, note.pitch, 75],
            length: 3,
        })
        .collect();
    original.messages.push(Message {
        tick: 1100,
        order: 20,
        bytes: [0xb0, 1, 93],
        length: 3,
    });
    let mut params = parameters(Kind::Articulate);
    params.composition.replace = true;
    params.grid = 0.125;
    params.composition.gate = [0.8, 0.8];
    let strummed = run(&original, &params);
    assert_eq!(strummed.summary.expression_events, 3);
    assert!(strummed
        .content
        .messages
        .contains(original.messages.last().unwrap()));
    let flags = vec![true; strummed.content.notes.len()];
    let ownership = owners(
        &strummed.content,
        &flags,
        Expression::PolyPressure,
        &AtomicBool::new(false),
    )
    .unwrap();
    for (index, message) in strummed.content.messages.iter().enumerate() {
        if message.bytes[0] & 0xf0 == 0xa0 {
            let note = &strummed.content.notes[ownership[index].unwrap()];
            assert_eq!(message.bytes[1], note.pitch);
            let timing = note.source_timing.unwrap();
            assert!(message.tick >= timing.start && message.tick < timing.start + timing.duration);
        }
    }
    params.composition.articulation = Articulation::Flam;
    let flammed = run(&original, &params);
    assert_eq!(flammed.summary.expression_events, 6);
    validate_wire(&flammed.content, &AtomicBool::new(false)).unwrap();
    let mut stolen = empty();
    stolen.messages.push(Message {
        tick: 100,
        order: 1,
        bytes: [0xa0, 60, 80],
        length: 3,
    });
    let mut chords = parameters(Kind::Chords);
    chords.composition.register = [60, 84];
    assert!(
        super::super::prepare(&stolen, &BTreeSet::new(), &chords, &AtomicBool::new(false))
            .unwrap_err()
            .contains("take expression")
    );
}
#[test]
fn member_channel_expression_reuses_only_released_voices_and_keeps_manager_controls() {
    let mut original = source();
    original.notes.truncate(1);
    original.notes[0].channel = 1;
    original.messages = vec![
        Message {
            tick: 1200,
            order: 2,
            bytes: [0xd1, 88, 0],
            length: 2,
        },
        Message {
            tick: 1400,
            order: 4,
            bytes: [0xb1, 74, 63],
            length: 3,
        },
        Message {
            tick: 1250,
            order: 9,
            bytes: [0xe0, 0, 64],
            length: 3,
        },
    ];
    original.notes[0].source_timing.as_mut().unwrap().end_order = 5;
    let mut params = parameters(Kind::Articulate);
    params.expression = Expression::Lower(3);
    params.composition.replace = true;
    params.composition.articulation = Articulation::Repeat;
    params.grid = 0.25;
    let repeated = run(&original, &params);
    assert_eq!(repeated.content.notes.len(), 4);
    assert_eq!(repeated.summary.expression_events, 8);
    assert!(repeated
        .content
        .messages
        .contains(original.messages.last().unwrap()));
    let flags = vec![true; 4];
    let owned = owners(
        &repeated.content,
        &flags,
        params.expression,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(owned.iter().filter(|owner| owner.is_some()).count(), 8);
}
#[test]
fn cancellation_excessive_work_and_existing_chance_groups_refuse_the_complete_candidate() {
    let original = source();
    let before = original.clone();
    let mut params = parameters(Kind::Articulate);
    params.composition.replace = true;
    assert!(super::super::prepare(
        &original,
        &selection(&original),
        &params,
        &AtomicBool::new(true)
    )
    .unwrap_err()
    .contains("cancelled"));
    let mut melody = parameters(Kind::Melody);
    melody.grid = 1.0 / 1024.0;
    melody.composition.length = 8192.0;
    assert!(
        super::super::prepare(&empty(), &BTreeSet::new(), &melody, &AtomicBool::new(false))
            .unwrap_err()
            .contains("65536")
    );
    let mut choices = original.clone();
    choices.notes[0].variation = Some(crate::engine::note_variation::Properties::default());
    assert!(super::super::prepare(
        &choices,
        &selection(&choices),
        &params,
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("probability groups"));
    params.composition.replace = false;
    assert!(super::super::prepare(
        &original,
        &selection(&original),
        &params,
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("overlap"));
    assert_eq!(original, before);
}
