use super::*;
fn fixture()->(Engine,RtEngine) { Engine::headless_for_test(48000,256) }
fn tick(rt:&mut RtEngine) {rt.process(&mut [0.0;128]);}
fn send(engine:&Engine,rt:&mut RtEngine,c:Command){engine.send(c).unwrap();tick(rt);}
fn note(pitch:u8)->MidiNote {MidiNote {pitch,start:0.0,len:8.0,vel:100}}
fn notes(rt:&RtEngine,t:usize,s:usize)->Vec<u8>{rt.tracks[t].clips[s].notes.iter().map(|n|n.pitch).collect()}

#[test]
fn renderer_controls_undo_and_redo_without_stopping_unrelated_live_owners() {
    let (engine,mut rt)=fixture();
    let initial=Global::get(&rt);
    rt.selected_track=2;
    send(&engine,&mut rt,Command::LiveNoteOn {source:9,ch:2,note:67,vel:90});
    let commands=[Command::SetBpm(141.0),Command::Xfader(0.2),Command::Master(0.6),Command::CueMix(0.8),Command::FxWet {slot:2,value:0.4},Command::Quant(0.5),Command::Metronome,Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Analog))];
    for command in commands {send(&engine,&mut rt,command);}
    let edited=Global::get(&rt);
    rt.playing=true;
    for _ in 0..8 {send(&engine,&mut rt,Command::Undo);assert!(rt.playing);}
    assert_eq!(Global::get(&rt),initial);
    assert!(rt.tracks[2].poly.voices.iter().any(|v|v.input==Some(InputKey::Midi {source:9,ch:2,note:67}) && v.env.stage!=4));
    for _ in 0..8 {send(&engine,&mut rt,Command::Redo);}
    assert_eq!(Global::get(&rt),edited);
    send(&engine,&mut rt,Command::LiveNoteOff {source:9,ch:2,note:67});
    assert!(rt.tracks[2].poly.voices.iter().filter(|v|v.input==Some(InputKey::Midi {source:9,ch:2,note:67})).all(|v|v.env.stage==4 || v.env.stage==0));
}

#[test]
fn gestures_coalesce_keep_foreign_edits_separate_and_restore_saved_checkpoint() {
    let (engine,mut rt)=fixture();let clean=engine.undo.checkpoint();let id=engine.undo.gesture();
    for value in [0.2,0.4,0.6] {engine.send(Command::Gesture {id,command:Box::new(Command::Master(value))}).unwrap();}
    let counts=test_alloc::measure(||tick(&mut rt));assert_eq!((counts.allocations,counts.frees),(0,0));
    assert_eq!(engine.undo.view().cursor,1);assert_eq!(rt.master,0.6);
    let middle=engine.undo.checkpoint();
    send(&engine,&mut rt,Command::Gesture {id,command:Box::new(Command::Master(0.8))});
    assert_ne!(engine.undo.checkpoint(),middle);assert_eq!(engine.undo.view().cursor,1);
    send(&engine,&mut rt,Command::Undo);assert_eq!(engine.undo.checkpoint(),clean);
    send(&engine,&mut rt,Command::Redo);assert_eq!(rt.master,0.8);
    send(&engine,&mut rt,Command::TrackGain {track:1,value:0.1});
    send(&engine,&mut rt,Command::Gesture {id,command:Box::new(Command::Master(0.9))});
    assert_eq!(engine.undo.view().cursor,3);
    send(&engine,&mut rt,Command::Undo);assert_eq!(rt.master,0.8);assert_eq!(rt.tracks[1].gain,0.1);
    send(&engine,&mut rt,Command::Select {track:4,scene:7});
    assert_ne!(engine.undo.checkpoint().untracked,clean.untracked);
}

#[test]
fn note_replacement_owns_original_and_preserves_live_notes_while_playing() {
    let (engine,mut rt)=fixture();let t=2;let s=7;
    rt.tracks[t].clips[s]=Clip {kind:ClipKind::Midi,name:"original".into(),bars:2.0,notes:vec![note(60),note(64)],gain:0.7,audio:None};
    rt.apply(Command::LaunchClip {track:t as u8,scene:s as u8});
    rt.apply(Command::LiveNoteOn {source:44,ch:0,note:72,vel:100});
    send(&engine,&mut rt,Command::SetNotes {track:t as u8,scene:s as u8,notes:vec![note(61)]});
    assert_eq!(notes(&rt,t,s),[61]);
    let counts=test_alloc::measure(||{rt.apply(Command::Undo);rt.apply(Command::Redo);rt.apply(Command::Undo);});
    assert_eq!((counts.allocations,counts.frees),(0,0));assert_eq!(notes(&rt,t,s),[60,64]);
    assert_eq!(rt.tracks[t].clips[s].name,"original");assert_eq!(rt.tracks[t].clips[s].gain,0.7);
    assert!(rt.tracks[t].poly.voices.iter().any(|v|v.input==Some(InputKey::Midi {source:44,ch:0,note:72}) && v.env.stage!=4));
    assert!(rt.playing);assert_eq!(rt.tracks[t].playing.unwrap().scene,s as u8);
}

#[test]
fn media_undo_retains_exact_sample_and_receipt_without_crediting_playback() {
    let (engine,mut rt)=fixture();let old=rt.decks[0].audio.clone().unwrap();let receipt=rt.decks[0].load_receipt.clone().unwrap();
    rt.decks[0].pos=100.5;rt.decks[0].hotcues[4]=HotCue {set:true,pos:88.25};
    let next=rt.builtin[1].clone().unwrap();
    send(&engine,&mut rt,Command::DeckAudio {deck:0,audio:next.clone()});
    assert!(receipt.retained_by_history());assert_eq!(receipt.state(),load_receipt::State::Superseded);
    let played=receipt.last_play();
    let counts=test_alloc::measure(||rt.apply(Command::Undo));assert_eq!((counts.allocations,counts.frees),(0,0));
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(),&old));assert_eq!(rt.decks[0].pos,100.5);assert_eq!(rt.decks[0].hotcues[4].pos,88.25);
    assert_eq!(receipt.state(),load_receipt::State::Current);assert_eq!(receipt.last_play(),played);assert!(!rt.decks[0].playing);
    rt.apply(Command::Redo);assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(),&next));
}

#[test]
fn one_recording_take_groups_all_cells_and_undo_does_not_misdirect_releases() {
    let (engine,mut rt)=fixture();rt.playing=true;rt.recording=true;
    for t in 0..TRACKS {for s in 0..SCENES {rt.tracks[t].clips[s]=Clip::empty();}}
    for t in 0..TRACKS {for s in 0..SCENES {
        rt.selected_track=t;rt.selected_scene=s;
        rt.apply(Command::LiveNoteOn {source:1,ch:0,note:(t*8+s) as u8,vel:100});
    }}
    tick(&mut rt);assert_eq!(engine.undo.view().cursor,1);assert_eq!(engine.undo.view().items[0].unwrap().patches,64);
    rt.apply(Command::Undo);
    for track in &rt.tracks {for clip in &track.clips {assert_eq!(clip.kind,ClipKind::Empty);assert!(clip.notes.is_empty());}}
    // Off after undo cannot finalize a cancelled capture into another note list.
    for pitch in 0..64 {rt.apply(Command::LiveNoteOff {source:1,ch:0,note:pitch});}
    rt.apply(Command::Redo);
    for track in &rt.tracks {for clip in &track.clips {assert_eq!(clip.kind,ClipKind::Midi);assert_eq!(clip.notes.len(),1);}}
}

#[test]
fn oversized_payload_is_rejected_before_mutation_and_retired_off_callback() {
    let (engine,mut rt)=fixture();let original=notes(&rt,0,0);let mut many=Vec::with_capacity(NOTE_LIMIT+1);many.resize(NOTE_LIMIT+1,note(60));
    engine.send(Command::SetNotes {track:0,scene:0,notes:many}).unwrap();
    let counts=test_alloc::measure(||tick(&mut rt));assert_eq!((counts.allocations,counts.frees),(0,0));
    assert_eq!(notes(&rt,0,0),original);assert_eq!(engine.undo.view().failure,Some(Failure::Notes));assert_eq!(engine.undo.view().cursor,0);
    rt.undo.budget=1;
    rt.selected_track=2;rt.selected_scene=7;rt.playing=true;rt.recording=true;
    rt.apply(Command::LiveNoteOn {source:1,ch:0,note:60,vel:90});
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    assert!(rt.tracks[2].poly.voices.iter().any(|v|v.input==Some(InputKey::Midi {source:1,ch:0,note:60})));
    assert_eq!(rt.undo.failure,Some(Failure::Budget));
}

#[test]
fn effect_add_and_controls_replay_owned_processor_without_allocating() {
    let (engine,mut rt)=fixture();rt.fx_view=2;
    send(&engine,&mut rt,Command::FxAdd(5));
    send(&engine,&mut rt,Command::FxParam {slot:0,p:1,value:0.85});
    let old=rt.tracks[2].fx.slots[0].p[1];assert_eq!(old,0.85);
    let counts=test_alloc::measure(||{rt.apply(Command::Undo);rt.apply(Command::Undo);rt.apply(Command::Redo);rt.apply(Command::Redo);});
    assert_eq!((counts.allocations,counts.frees),(0,0));assert_eq!(rt.tracks[2].fx.slots[0].p[1],old);
}

#[test]
fn real_midi_worker_and_ipc_cue_both_enter_renderer_history() {
    use std::io::{BufRead,Write};use std::os::unix::net::UnixStream;
    let (engine,mut rt)=fixture();let original=rt.tracks[3].gain;
    engine.midi.receive_for_test(&engine.cmd,14,"APC40 mkII",&[0xb3,7,25]);
    tick(&mut rt);assert_eq!(rt.tracks[3].gain,25.0/127.0);
    rt.apply(Command::Undo);assert_eq!(rt.tracks[3].gain,original);
    let (mut client,server)=UnixStream::pair().unwrap();client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let cmd=engine.cmd.clone();let snap=engine.snap.clone();let worker=std::thread::spawn(move||crate::handle_client(server,cmd,snap));
    rt.decks[0].pos=1123.75;rt.decks[0].cue_pos=12.0;
    client.write_all(b"{\"op\":\"deckCue\",\"deck\":0}\n").unwrap();
    let mut reply=String::new();std::io::BufReader::new(&client).read_line(&mut reply).unwrap();assert!(reply.contains("accepted"));
    tick(&mut rt);assert_eq!(rt.decks[0].cue_pos,1123.75);
    rt.apply(Command::Undo);assert_eq!(rt.decks[0].cue_pos,12.0);
    drop(client);worker.join().unwrap().unwrap();
}

#[test]
fn held_recording_changes_invalidate_saved_states_before_later_mixer_undo() {
    let (engine,mut rt)=fixture();rt.selected_track=2;rt.selected_scene=7;rt.recording=true;rt.playing=true;
    send(&engine,&mut rt,Command::LiveNoteOn {source:4,ch:0,note:62,vel:100});
    let saved=engine.undo.checkpoint();
    send(&engine,&mut rt,Command::Master(0.4));tick(&mut rt);
    rt.apply(Command::Undo);
    assert_ne!(engine.undo.checkpoint(),saved,"the held note kept growing while the mixer edit was undone");
    rt.apply(Command::Undo);
    assert_eq!(engine.undo.checkpoint().state,0);assert!(rt.tracks[2].clips[7].notes.is_empty());
}

#[test]
fn oldest_whole_transactions_evict_redo_branches_retire_and_pcm_is_deduplicated() {
    let (engine,mut rt)=fixture();
    for i in 0..MAX_ENTRIES+10 {rt.apply(Command::Master(i as f32/1000.0));}
    assert_eq!(rt.undo.entries.len(),MAX_ENTRIES);assert_eq!(rt.undo.cursor,MAX_ENTRIES);
    for _ in 0..2 {rt.apply(Command::Undo);}rt.apply(Command::Xfader(0.1));assert_eq!(rt.undo.entries.len(),MAX_ENTRIES-1);
    let sample=rt.decks[0].audio.clone().unwrap();
    for _ in 0..8 {rt.apply(Command::DeckAudio {deck:0,audio:sample.clone()});}
    assert!(rt.undo.bytes<sample_bytes(&sample)+9*TEXT_LIMIT,"repeated refs must not multiply PCM charge: {}",rt.undo.bytes);
    tick(&mut rt);assert!(engine.undo.view().fixed_bytes>0);
    eprintln!("undo fixed storage {} bytes; live {} bytes",engine.undo.view().fixed_bytes,rt.undo.bytes);
}

#[test]
fn effect_gesture_targets_do_not_alias_across_large_racks_and_capacity_is_atomic() {
    let (engine,mut rt)=fixture();
    rt.tracks[0].fx.slots=(0..128).map(|_|fx::FxSlot::new(fx::FxId::Balance,rt.sr)).collect();
    rt.tracks[1].fx.slots.push(fx::FxSlot::new(fx::FxId::Balance,rt.sr));
    let id=engine.undo.gesture();rt.fx_view=0;
    send(&engine,&mut rt,Command::Gesture {id,command:Box::new(Command::FxMix {slot:64,value:0.1})});
    rt.fx_view=1;send(&engine,&mut rt,Command::Gesture {id,command:Box::new(Command::FxMix {slot:0,value:0.8})});
    rt.apply(Command::Undo);assert_eq!(rt.tracks[1].fx.slots[0].mix,0.5);assert_eq!(rt.tracks[0].fx.slots[64].mix,0.1);
    rt.apply(Command::Undo);assert_eq!(rt.tracks[0].fx.slots[64].mix,0.5);
    rt.fx_view=0;let before=rt.undo.checkpoint();rt.apply(Command::FxAdd(0));assert_eq!(rt.tracks[0].fx.slots.len(),128);assert_eq!(rt.undo.checkpoint(),before);assert_eq!(rt.undo.failure,Some(Failure::Effects));
}

#[test]
fn reserved_processor_storage_matches_real_primitives_at_all_supported_rates() {
    for sr in [8000.0,44100.0,48000.0,96000.0,384000.0] {for &id in fx::FxId::all() {
        assert_eq!(fx::FxSlot::required_storage(id,sr),fx::FxSlot::new(id,sr).storage_bytes(),"{id:?} at {sr}");
    }}
}
