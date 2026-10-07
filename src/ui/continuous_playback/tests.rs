use super::*;
use crate::ui::library_annotations::tests::{Files,Gui};
use crate::library::crates::Edit;
use std::time::Duration;

fn files_and_crate()->(Files,CrateId,Vec<LibSource>) {
    let files=Files::new();let mut store=crate::library::Store::open(files.0.join("catalog.json")).unwrap();let mut members=Vec::new();let mut sources=Vec::new();
    for (name,kind) in [("Short A.wav",0),("Missing.wav",1),("Corrupt.wav",2),("Short B.wav",0)] {
        let path=files.0.join(name);
        if kind==0 {std::fs::write(&path,crate::engine::media_analysis::tests::wav(512,8000,1,false)).unwrap();}
        if kind==2 {std::fs::write(&path,b"this is not an audio file").unwrap();}
        let source=LibSource::File(path.clone());store.catalog.upsert(source.clone(),FileFingerprint::read(&path),crate::library::Metadata {title:path.file_stem().unwrap().to_string_lossy().into_owned(),artist:String::new(),bpm:Bpm::hint(120.0),key:"C".into(),duration:Some(0.064),last_play:None}).unwrap();members.push(store.catalog.track(&source).unwrap().id.clone());sources.push(source);
    }
    let id=CrateId("1234567890abcdef1234567890abcdef".into());
    store.catalog.crates.apply(0,&Edit::Create {id:id.clone(),name:"Continuous fixture".into(),parent:None,before:None},|_|true).unwrap();
    store.catalog.crates.apply(1,&Edit::AddMembers {id:id.clone(),members,before:None},|_|true).unwrap();store.save().unwrap();drop(store);(files,id,sources)
}
fn gui(files:&Files,id:&CrateId)->Box<Gui> {
    let mut gui=Box::new(Gui::new(files));gui.app.library_annotations.open=false;gui.app.continuous_playback.decks[0].crate_id=Some(id.clone());
    gui.click("continuous…");gui.frame(vec![]);gui.frame(vec![]);gui
}
fn run(gui:&mut Gui,mut complete:impl FnMut(&Gui)->bool)->f64 {
    let until=Instant::now()+Duration::from_secs(10);let mut energy=0.0;
    while !complete(gui) {gui.frame(vec![]);assert!(gui.audio.iter().all(|sample|sample.is_finite()));energy+=gui.audio.iter().map(|sample|f64::from(*sample).powi(2)).sum::<f64>();assert!(Instant::now()<until,"continuous playback: {}",gui.app.continuous_playback.decks[0].message);std::thread::sleep(Duration::from_millis(1));}energy
}
#[test]
fn actual_native_enable_runs_real_short_files_skips_missing_and_corrupt_sources_then_stops_at_the_end() {
    let (files,id,sources)=files_and_crate();let mut gui=gui(&files,&id);gui.rt.apply(Command::Xfader(0.0));gui.rt.publish_for_test();
    assert!(gui.visible_text().any(|text|text=="Next: Short A"));gui.click("Deck A: Enable continuous playback");assert!(gui.app.continuous_playback.decks[0].run.is_some());
    let energy=run(&mut gui,|gui|gui.app.continuous_playback.decks[0].run.is_none());assert!(energy>0.001);
    assert_eq!(gui.app.continuous_playback.decks[0].message,"Ordered crate finished");assert_eq!(gui.app.continuous_playback.decks[0].skipped.len(),2);assert!(gui.app.continuous_playback.decks[0].skipped[0].contains("Missing"));assert!(gui.app.continuous_playback.decks[0].skipped[1].contains("Corrupt"));
    assert_eq!(gui.app.loads[0].as_ref().unwrap().selection.as_ref().unwrap().source,sources[3]);assert!(!gui.rt.decks[0].playing);assert_eq!(gui.app.snap.xfader,0.0);
    println!("CONTINUOUS_FILES_RECEIPT {}",serde_json::json!({"real_readable_files":2,"missing_files":1,"corrupt_files":1,"skipped_reports":2,"natural_ends":gui.app.snap.decks[0].natural_end,"finite_policy_complete":true,"energy":energy,"physical_devices_opened":false}));
}
fn reference()->(crate::engine::Engine,Box<crate::engine::RtEngine>) {
    let (engine,rt)=crate::engine::Engine::headless_for_test(48000,256);(engine,Box::new(rt))
}
#[test]
fn actual_repeat_and_disable_preserve_the_other_decks_pcm_against_an_independent_renderer() {
    let (files,id,_sources)=files_and_crate();let mut gui=gui(&files,&id);
    let (reference_engine,mut reference)=reference();
    assert_eq!(gui.rt.decks[1].audio.as_ref().unwrap().data,reference.decks[1].audio.as_ref().unwrap().data);
    for rt in [&mut gui.rt,reference.as_mut()] {rt.apply(Command::Xfader(1.0));rt.publish_for_test();}
    for _ in 0..64 {gui.frame(vec![]);reference.process(&mut [0.0;256]);}
    for rt in [&mut gui.rt,reference.as_mut()] {rt.apply(Command::DeckPlay {deck:1});rt.apply(Command::DeckLoop {deck:1,beats:4.0});rt.publish_for_test();}
    let original=gui.rt.decks[1].audio.clone().unwrap();let gain=gui.rt.decks[1].gain;let pitch=gui.rt.decks[1].pitch;
    gui.app.continuous_playback.decks[0].repeat=true;gui.app.enable_continuous_playback(0);let mut compared=0;let until=Instant::now()+Duration::from_secs(10);
    while gui.app.snap.decks[0].natural_end<4 {
        gui.frame(vec![]);let mut expected=[0.0;256];reference.process(&mut expected);assert!(gui.audio==expected,"independent PCM differs by {}",gui.audio.iter().zip(&expected).map(|(a,b)|(a-b).abs()).fold(0.0_f32,f32::max));compared+=128;
        assert!(Arc::ptr_eq(gui.rt.decks[1].audio.as_ref().unwrap(),&original));assert!(gui.rt.decks[1].playing);assert_eq!(gui.rt.decks[1].gain,gain);assert_eq!(gui.rt.decks[1].pitch,pitch);
        assert!(Instant::now()<until,"repeat: {}",gui.app.continuous_playback.decks[0].message);std::thread::sleep(Duration::from_millis(1));
    }
    assert!(gui.app.continuous_playback.decks[0].run.is_some());gui.app.disable_continuous_playback(0,"Continuous playback disabled");assert!(gui.app.continuous_playback.decks[0].run.is_none());
    for _ in 0..64 {gui.frame(vec![]);let mut expected=[0.0;256];reference.process(&mut expected);assert_eq!(gui.audio,expected);compared+=128;}
    assert!(reference_engine.cmd.is_connected());println!("CONTINUOUS_OTHER_DECK_RECEIPT {}",serde_json::json!({"compared_frames":compared,"max_sample_error":0,"other_audio_preserved":true,"other_gain_and_pitch_preserved":true,"repeat_ends":4,"disabled":true,"physical_devices_opened":false}));
}
#[test]
fn actual_repeat_with_all_members_unavailable_exhausts_one_pass_without_starting_audio() {
    let (files,id,sources)=files_and_crate();let mut gui=gui(&files,&id);
    for source in [sources[0].clone(),sources[3].clone()] {if let LibSource::File(path)=source {std::fs::remove_file(path).unwrap();}}
    gui.click("Deck A: Repeat ordered crate");gui.click("Deck A: Enable continuous playback");
    run(&mut gui,|gui|gui.app.continuous_playback.decks[0].run.is_none());assert_eq!(gui.app.continuous_playback.decks[0].skipped.len(),4);assert_eq!(gui.app.continuous_playback.decks[0].message,"No available tracks remain in this crate pass");assert!(!gui.rt.decks[0].playing);
}

#[test]
fn native_disable_and_manual_replacement_cancel_their_owned_blocked_real_decode_without_starting_a_late_result() {
    for manual in [false,true] {
        let (files,id,sources)=files_and_crate();let mut gui=gui(&files,&id);
        let (entered,received)=std::sync::mpsc::channel();let (release,resume)=std::sync::mpsc::channel();
        struct Release(Option<std::sync::mpsc::Sender<()>>);impl Drop for Release {fn drop(&mut self){if let Some(sender)=self.0.take(){let _=sender.send(());}}}
        let guard=Release(Some(release));
        gui.app.loader=Some(Loader::with_decoder_for_show(move|path,token| {entered.send(path.to_path_buf()).unwrap();let _=resume.recv();crate::engine::decode::decode_audio_with_cancel(path,||!token.is_current())},gui.app.engine.cmd.performance().clone()).unwrap());
        let original=gui.rt.decks[0].audio.clone().unwrap();gui.click("Deck A: Enable continuous playback");
        assert_eq!(received.recv_timeout(Duration::from_secs(3)).unwrap(),match &sources[0] {LibSource::File(path)=>path.clone(),_=>unreachable!()});
        if manual {gui.app.load_source(0,Some(&Selection {source:sources[3].clone(),title:"Manual short B".into(),fingerprint:match &sources[3]{LibSource::File(path)=>FileFingerprint::read(path),_=>None}}));}
        else {gui.click("Deck A: Enable continuous playback");}
        assert!(gui.app.continuous_playback.decks[0].run.is_none());guard.0.as_ref().unwrap().send(()).unwrap();
        if manual {assert_eq!(received.recv_timeout(Duration::from_secs(3)).unwrap(),match &sources[3] {LibSource::File(path)=>path.clone(),_=>unreachable!()});guard.0.as_ref().unwrap().send(()).unwrap();gui.wait(|gui|gui.app.loads[0].as_ref().is_some_and(|load|matches!(load.phase,load_status::Phase::Loaded)));assert_eq!(gui.app.loads[0].as_ref().unwrap().selection.as_ref().unwrap().source,sources[3]);}
        else {gui.wait(|gui|gui.app.loads[0].as_ref().is_some_and(|load|matches!(load.phase,load_status::Phase::Superseded)));assert!(Arc::ptr_eq(gui.rt.decks[0].audio.as_ref().unwrap(),&original));}
        for _ in 0..32 {gui.frame(vec![]);}assert!(!gui.rt.decks[0].playing);assert!(gui.app.continuous_playback.decks[0].run.is_none());
    }
}
#[test]
fn native_disable_before_queued_application_preserves_the_old_audio_and_refuses_automatic_start() {
    let (files,id,_)=files_and_crate();let mut gui=gui(&files,&id);let original=gui.rt.decks[0].audio.clone().unwrap();
    gui.app.enable_continuous_playback(0);gui.app.poll_continuous_playback();let until=Instant::now()+Duration::from_secs(3);
    while gui.app.loads[0].as_ref().is_some_and(|load|matches!(load.phase,load_status::Phase::Loading)) {gui.app.poll_loads();assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
    assert!(matches!(gui.app.loads[0].as_ref().unwrap().phase,load_status::Phase::Queued));let receipt=gui.app.loads[0].as_ref().unwrap().receipt.clone().unwrap();assert_eq!(receipt.state(),crate::engine::load_receipt::State::Pending);
    gui.click("Deck A: Enable continuous playback");assert!(gui.app.continuous_playback.decks[0].run.is_none());assert_eq!(receipt.state(),crate::engine::load_receipt::State::Superseded);assert!(Arc::ptr_eq(gui.rt.decks[0].audio.as_ref().unwrap(),&original));assert!(!gui.rt.decks[0].playing);
}
#[test]
fn actual_loaded_crate_member_resumes_or_finishes_before_advancing_to_its_next_member() {
    for playing in [false,true] {
        let (files,id,sources)=files_and_crate();let mut gui=gui(&files,&id);
        gui.app.load_source(0,Some(&Selection {source:sources[0].clone(),title:"Short A".into(),fingerprint:match &sources[0]{LibSource::File(path)=>FileFingerprint::read(path),_=>None}}));
        gui.wait(|gui|gui.app.loads[0].as_ref().is_some_and(|load|matches!(load.phase,load_status::Phase::Loaded)));
        let current=gui.rt.decks[0].audio.clone().unwrap();let key=gui.app.snap.decks[0].media_key;let ended=gui.app.snap.decks[0].natural_end;
        if playing {gui.rt.apply(Command::DeckPlay {deck:0});gui.rt.publish_for_test();}
        gui.click("Deck A: Enable continuous playback");assert!(Arc::ptr_eq(gui.rt.decks[0].audio.as_ref().unwrap(),&current));assert_eq!(gui.app.snap.decks[0].media_key,key);assert!(gui.rt.decks[0].playing);
        run(&mut gui,|gui|gui.app.continuous_playback.decks[0].run.is_none());assert_eq!(gui.app.snap.decks[0].natural_end,ended+2);assert_eq!(gui.app.loads[0].as_ref().unwrap().selection.as_ref().unwrap().source,sources[3]);assert_eq!(gui.app.continuous_playback.decks[0].skipped.len(),2);
    }
}
