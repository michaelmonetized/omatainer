use super::*;
fn note(pitch: u8, start: f32, properties: Properties) -> MidiNote {
    MidiNote { id: NoteId::new(), channel: 0, release_vel: 64, source_timing: None,
        muted: false, pitch, start, len: 0.25, vel: 90, variation: Some(properties) }
}
#[test]
fn saved_seed_replays_fifty_thousand_passes_with_bounded_velocity_and_observed_chance() {
    let note=note(60,0.0,Properties { chance: 3700,velocity:Some(Velocity {minimum:41,maximum:79}),..Default::default() });
    let plan=Plan::prepare(&[note],None,&AtomicBool::new(false)).unwrap().unwrap();
    let first:Vec<_>=(0..50_000).map(|cycle|plan.velocity(0,72,cycle,90)).collect();
    let second:Vec<_>=(0..50_000).map(|cycle|plan.velocity(0,72,cycle,90)).collect();
    assert_eq!(first,second);
    let accepted=first.iter().flatten().count();
    assert!((18_000..=19_000).contains(&accepted),"Observed {accepted}/50000 rather than 37%");
    assert!(first.iter().flatten().all(|&velocity|(41..=79).contains(&velocity)));
    assert_eq!(*first.iter().flatten().min().unwrap(),41);
    assert_eq!(*first.iter().flatten().max().unwrap(),79);
    assert!((0..50_000).filter(|&cycle|plan.velocity(0,73,cycle,90)!=first[cycle as usize]).count()>20_000);
    assert!(plan.velocity(0,72,-1,90).is_none());
    assert_eq!(plan.velocity(7,72,0,91),None);
}
#[test]
fn linked_groups_share_the_gate_and_exclusive_weights_never_sound_together() {
    let linked=Group {identity:NoteId::new(),kind:GroupKind::Linked};
    let exclusive=Group {identity:NoteId::new(),kind:GroupKind::Exclusive};
    let notes=vec![
        note(60,0.0,Properties {chance:4200,group:Some(linked),..Default::default()}),
        note(64,0.0,Properties {chance:4200,group:Some(linked),..Default::default()}),
        note(67,1.0,Properties {chance:2500,group:Some(exclusive),..Default::default()}),
        note(69,1.0,Properties {chance:5000,group:Some(exclusive),..Default::default()}),
    ];
    let plan=Plan::prepare(&notes,None,&AtomicBool::new(false)).unwrap().unwrap();
    let mut counts=[0;3];
    for cycle in 0..50_000 {
        assert_eq!(plan.velocity(0,3,cycle,90).is_some(),plan.velocity(1,3,cycle,90).is_some());
        let a=plan.velocity(2,3,cycle,90).is_some();let b=plan.velocity(3,3,cycle,90).is_some();
        assert!(!(a&&b));counts[if a {0}else if b {1}else {2}]+=1;
    }
    assert!((12_000..=13_000).contains(&counts[0]));
    assert!((24_500..=25_500).contains(&counts[1]));
    assert!((12_000..=13_000).contains(&counts[2]));
    let mut reordered=notes.clone();reordered.reverse();
    let reordered_plan=Plan::prepare(&reordered,None,&AtomicBool::new(false)).unwrap().unwrap();
    for cycle in 0..1000 { for index in 0..4 { assert_eq!(plan.velocity(index,3,cycle,90),reordered_plan.velocity(3-index,3,cycle,90)); } }
}
#[test]
fn incoherent_groups_invalid_ranges_mpe_channels_and_cancel_refuse_before_publication() {
    let group=Group {identity:NoteId::new(),kind:GroupKind::Linked};
    let a=note(60,0.0,Properties {chance:5000,group:Some(group),..Default::default()});
    let mut b=note(62,0.0,a.variation.unwrap());
    assert!(validate(&[a.clone(),b.clone()]).is_ok());
    b.start=1.0;assert!(validate(&[a.clone(),b.clone()]).is_err());
    b.start=0.0;b.variation.as_mut().unwrap().chance=3000;assert!(validate(&[a.clone(),b.clone()]).is_err());
    let mut notes=vec![a,b];for note in &mut notes {let p=note.variation.as_mut().unwrap();p.group.as_mut().unwrap().kind=GroupKind::Exclusive;p.chance=6000;}
    assert!(validate(&notes).is_err());
    notes[0].variation=Some(Properties {velocity:Some(Velocity {minimum:80,maximum:20}),..Default::default()});
    assert!(validate(&notes[..1]).is_err());
    notes[0].variation=Some(Properties {expression:Expression::Lower(15),..Default::default()});
    assert!(validate(&notes[..1]).is_err());notes[0].channel=2;assert!(validate(&notes[..1]).is_ok());
    assert!(Plan::prepare(&notes[..1],None,&AtomicBool::new(true)).is_err());
    notes[0].variation=None;assert!(Plan::prepare(&notes[..1],None,&AtomicBool::new(false)).unwrap().is_none());
}
#[test]
fn source_owned_pressure_is_retained_and_ambiguous_voices_refuse() {
    let a=note(60,0.0,Properties {chance:5000,..Default::default()});
    let lanes=super::super::midi_data::Lanes::new(960,960,vec![crate::midi_file::Message {tick:120,order:3,bytes:[0xa0,60,87],length:3},crate::midi_file::Message {tick:120,order:4,bytes:[0xb0,7,90],length:3}],vec![]).unwrap();
    let plan=Plan::prepare(&[a.clone()],Some(&lanes),&AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(plan.expression_owner(0),Some(0));assert_eq!(plan.expression_owner(1),None);
    let mut b=a.clone();b.id=NoteId::new();
    assert!(Plan::prepare(&[a,b],Some(&lanes),&AtomicBool::new(false)).is_err());
}

#[test]
fn independent_notes_and_probability_groups_have_separate_decision_domains() {
    let mut first = note(60, 0.0, Properties { chance: 5000, ..Default::default() });
    let group = Group { identity: first.id, kind: GroupKind::Linked };
    let second = note(64, 0.0, Properties { chance: 5000, group: Some(group), ..Default::default() });
    let plan = Plan::prepare(&[first.clone(), second.clone()], None, &AtomicBool::new(false)).unwrap().unwrap();
    assert!((0..50_000).filter(|&cycle| plan.velocity(0, 42, cycle, 90).is_some() != plan.velocity(1, 42, cycle, 90).is_some()).count() > 20_000);
    first.variation.as_mut().unwrap().group = Some(group);
    let linked = Plan::prepare(&[first, second], None, &AtomicBool::new(false)).unwrap().unwrap();
    for cycle in 0..50_000 { assert_eq!(linked.velocity(0, 42, cycle, 90).is_some(), linked.velocity(1, 42, cycle, 90).is_some()); }
}
#[test]
fn copied_groups_retain_internal_choices_and_leave_original_owners_unchanged() {
    let group = Group { identity: NoteId::new(), kind: GroupKind::Exclusive };
    let original = vec![note(60, 0.0, Properties { chance: 3000, group: Some(group), ..Default::default() }), note(64, 0.0, Properties { chance: 6000, group: Some(group), ..Default::default() })];
    let before = original.clone();
    let mut copies = original.clone();
    for note in &mut copies { note.id = NoteId::new(); }
    remap_copied_groups(&mut copies).unwrap();
    assert_eq!(original, before);
    let copied = copies[0].variation.unwrap().group.unwrap();
    assert_ne!(copied.identity, group.identity);
    assert_eq!(copied, copies[1].variation.unwrap().group.unwrap());
    for (old, new) in original.iter().zip(&copies) { assert_ne!(old.id, new.id); assert_eq!(old.variation.unwrap().chance, new.variation.unwrap().chance); }
    let plan = Plan::prepare(&copies, None, &AtomicBool::new(false)).unwrap().unwrap();
    for cycle in 0..50_000 { assert!(!(plan.velocity(0, 9, cycle, 90).is_some() && plan.velocity(1, 9, cycle, 90).is_some())); }
}
#[test]
fn native_schema_keeps_full_width_seed_and_refuses_future_note_fields_in_older_projects() {
    use crate::engine::project::State;
    let state = State::blank();
    let mut legacy = serde_json::to_value(&state).unwrap();
    legacy["version"] = 33.into();
    assert!(legacy.get("note_seed").is_none());
    let decoded: State = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(decoded.note_seed, DEFAULT_SEED);
    legacy["note_seed"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<State>(legacy).is_err());
    let mut state = state;
    state.note_seed = u64::MAX;
    let bytes = serde_json::to_vec(&state).unwrap();
    let decoded: State = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.version, 34);
    assert_eq!(decoded.note_seed, u64::MAX);
    let clip = serde_json::json!({"notes":[{"variation":null}]});
    assert!(has_future_note_fields(&clip));
    let raw = serde_json::json!({"version":33,"tracks":[{"clips":[clip.clone()]}]});
    assert!(reject_legacy_fields(&raw).is_err());
    let raw = serde_json::json!({"version":33,"arrangement":{"sources":[{"clip":clip}]}});
    assert!(reject_legacy_fields(&raw).is_err());
    eprintln!("MIDI_NOTE_VARIATION_METADATA {{\"native_schema\":34,\"full_u64_seed\":true,\"legacy_null_refusal\":true,\"arrangement_field_refusal\":true,\"physical_devices_opened\":false}}");
}
fn captured(engine: &crate::engine::Engine, rt: &mut crate::engine::RtEngine) -> crate::engine::project::Captured {
    let handle = engine.project.clone();
    let task = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !task.is_finished() { rt.process(&mut []); assert!(std::time::Instant::now() < deadline, "Project capture did not finish"); std::thread::sleep(std::time::Duration::from_millis(1)); }
    task.join().unwrap().unwrap()
}
#[test]
fn actual_seed_admission_cancellation_stale_refusal_and_history_keep_pcm_and_notes() {
    use crate::engine::{arrangement::edit::Request, midi_edit::Outcome, test_alloc, Command, Engine};
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let original_notes: Vec<Vec<Vec<MidiNote>>> = rt.tracks.iter().map(|track|track.clips.iter().map(|clip|clip.notes.clone()).collect()).collect();
    let source = rt.tracks[0].drum_samples[0].clone();
    let before = captured(&engine, &mut rt);
    assert!(Request::prepare_seed(before, 2, &AtomicBool::new(true)).is_err());
    assert_eq!(rt.note_seed, DEFAULT_SEED);
    let (request, ack) = Request::prepare_seed(captured(&engine, &mut rt), 2, &AtomicBool::new(false)).unwrap();
    assert!(ack.cancel());
    assert_eq!(test_alloc::measure(||rt.apply(Command::ArrangementEdit(request))), Default::default());
    assert_eq!(ack.state(), Outcome::Cancelled);
    assert_eq!(rt.note_seed, DEFAULT_SEED);
    let (request, ack) = Request::prepare_seed(captured(&engine, &mut rt), 2, &AtomicBool::new(false)).unwrap();
    engine.send(Command::Master(0.37)).unwrap();
    rt.process(&mut []);
    assert_eq!(test_alloc::measure(||rt.apply(Command::ArrangementEdit(request))), Default::default());
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(rt.note_seed, DEFAULT_SEED);
    let (request, ack) = Request::prepare_seed(captured(&engine, &mut rt), 2, &AtomicBool::new(false)).unwrap();
    assert_eq!(test_alloc::measure(||rt.apply(Command::ArrangementEdit(request))), Default::default());
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.note_seed, 2);
    assert!(rt.tracks.iter().all(|track|track.note_seed == 2));
    assert_eq!(rt.master, 0.37);
    for command in [Command::Undo, Command::Redo] { engine.send(command).unwrap(); assert_eq!(test_alloc::measure(||rt.process(&mut [])), Default::default()); }
    assert_eq!(rt.note_seed, 2);
    assert!(Arc::ptr_eq(&source, &rt.tracks[0].drum_samples[0]));
    assert_eq!(original_notes, rt.tracks.iter().map(|track|track.clips.iter().map(|clip|clip.notes.clone()).collect()).collect::<Vec<Vec<Vec<MidiNote>>>>());
    eprintln!("MIDI_NOTE_VARIATION_ADMISSION {{\"actual_cancellation\":true,\"stale_refusal\":true,\"exact_history\":true,\"pcm_and_notes_retained\":true,\"callback_allocations\":0,\"callback_frees\":0,\"physical_devices_opened\":false}}");
}
