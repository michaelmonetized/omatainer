use super::*;
use crate::engine::{Engine,load_receipt::{Media,Receipt},test_alloc};

fn loaded()->(Engine,RtEngine) {
    let (engine,mut rt)=Engine::headless_for_test(48000,64);
    rt.apply(Command::DeckLoadRequested {deck:0,media:Media::Builtin(1),receipt:Receipt::new()});rt.publish_for_test();(engine,rt)
}
fn arm(engine:&Engine,lease:Arc<Lease>)->Request {Request::new(0,&engine.snapshot().decks[0],engine.cmd.performance().safety_epoch(),lease).unwrap()}

#[test]
fn natural_end_counts_once_for_its_source_but_not_preview_seek_loop_or_pause() {
    let (engine,mut rt)=loaded();let media=engine.snapshot().decks[0].media_key;let frames=rt.decks[0].audio.as_ref().unwrap().frames() as f64;
    rt.decks[0].pos=frames-8.0;rt.apply(Command::DeckPreview {deck:0,expected:media,on:true});rt.decks[0].pos=frames-8.0;
    for _ in 0..32 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,0);
    rt.apply(Command::DeckPreview {deck:0,expected:media,on:false});rt.apply(Command::DeckSeek {deck:0,frac:1.0});for _ in 0..32 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,0);
    rt.decks[0].pos=frames-8.0;rt.decks[0].loop_on=true;rt.decks[0].loop_start=frames-128.0;rt.decks[0].loop_len=128.0;rt.apply(Command::DeckPlay {deck:0});
    for _ in 0..256 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,0);assert!(rt.decks[0].playing);
    rt.apply(Command::DeckPlay {deck:0});for _ in 0..32 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,0);
    rt.decks[0].loop_on=false;rt.decks[0].pos=frames-8.0;rt.apply(Command::DeckPlay {deck:0});
    for _ in 0..64 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,1);assert_eq!(rt.decks[0].end_media_key,media);assert!(!rt.decks[0].playing);
    for _ in 0..64 {rt.render_deck(0);}assert_eq!(rt.decks[0].natural_end,1);
}
#[test]
fn revoked_source_changed_and_round_trip_manual_transport_requests_cannot_start_or_toggle_audio() {
    for scenario in 0..3 {
        let (engine,mut rt)=loaded();let lease=Lease::new();let request=arm(&engine,lease.clone());engine.send(Command::DeckContinue(request.clone())).unwrap();
        match scenario {0=>lease.disable(),1=>rt.apply(Command::DeckLoadRequested {deck:0,media:Media::Builtin(0),receipt:Receipt::new()}),_=>{rt.apply(Command::DeckPlay {deck:0});rt.apply(Command::DeckPlay {deck:0});}}
        assert_eq!(test_alloc::measure(||rt.process(&mut [0.0;256])),Default::default());assert!(!rt.decks[0].playing);assert_eq!(request.outcome(),Outcome::Refused);
    }
    let (engine,mut rt)=loaded();let request=arm(&engine,Lease::new());engine.send(Command::DeckContinue(request.clone())).unwrap();rt.apply(Command::DeckPlay {deck:0});rt.process(&mut [0.0;256]);assert!(rt.decks[0].playing);assert_eq!(request.outcome(),Outcome::Refused);
}
#[test]
fn start_ack_preserves_an_end_inside_its_first_block_and_duplicate_queue_entries_are_idempotent_without_heap_work() {
    let (engine,mut rt)=loaded();rt.decks[0].pos=rt.decks[0].audio.as_ref().unwrap().frames() as f64-8.0;rt.publish_for_test();
    let request=arm(&engine,Lease::new());let before=engine.snapshot().decks[0].natural_end;
    for _ in 0..2 {engine.send(Command::DeckContinue(request.clone())).unwrap();}
    let mut out=[0.0;256];assert_eq!(test_alloc::measure(||rt.process(&mut out)),Default::default());assert!(out.iter().all(|sample|sample.is_finite()));
    assert_eq!(request.outcome(),Outcome::Started {end:before,transport:1});assert_eq!(rt.decks[0].natural_end,before+1);assert!(!rt.decks[0].playing);
    println!("CONTINUOUS_START_RECEIPT {}",serde_json::json!({"captured_end":before,"published_end":rt.decks[0].natural_end,"duplicate_requests":2,"callback_allocations":0,"callback_frees":0,"physical_devices_opened":false}));
}
#[test]
fn safety_epoch_changes_refuse_a_stale_start_even_after_input_release() {
    let (engine,mut rt)=loaded();let request=arm(&engine,Lease::new());let before=engine.cmd.performance().safety_epoch();
    rt.apply(Command::SafetyStop(crate::engine::performance::Safety::Stop));rt.apply(Command::RecoverPerformance);assert_ne!(engine.cmd.performance().safety_epoch(),before);
    assert_eq!(test_alloc::measure(||request.apply(&mut rt)),Default::default());assert_eq!(request.outcome(),Outcome::Refused);assert!(!rt.decks[0].playing);
}
#[test]
fn manual_hotcue_loop_jump_strip_and_reverse_round_trips_revoke_an_armed_start() {
    use crate::engine::deck_controls::{Control,Button};
    for control in [Control::Hold {button:Button::Reverse,on:true},Control::Hold {button:Button::Bleep,on:true},Control::Hold {button:Button::Cue,on:true},Control::Hold {button:Button::HotCue(0),on:true},Control::Hold {button:Button::Roll(0),on:true},Control::Hold {button:Button::Slice(0),on:true},Control::BeatJump {forward:true},Control::Strip {value:0.5},Control::TrackStart,Control::LoopToggle] {
        let (engine,mut rt)=loaded();let before=rt.decks[0].transport_generation;let request=arm(&engine,Lease::new());
        rt.apply(Command::DeckControl {source:200,deck:0,control});
        if let Control::Hold {button,..}=control {rt.apply(Command::DeckControl {source:200,deck:0,control:Control::Hold {button,on:false}});}
        assert!(rt.decks[0].transport_generation!=before,"{control:?}");
        assert_eq!(test_alloc::measure(||rt.apply(Command::DeckContinue(request.clone()))),Default::default());assert_eq!(request.outcome(),Outcome::Refused,"{control:?}");assert!(!rt.decks[0].playing);
    }
    for command in [Command::DeckHotCue {deck:0,pad:0,del:false},Command::DeckLoop {deck:0,beats:4.0}] {
        let (engine,mut rt)=loaded();let request=arm(&engine,Lease::new());rt.apply(command);
        rt.apply(Command::DeckContinue(request.clone()));assert_eq!(request.outcome(),Outcome::Refused);assert!(!rt.decks[0].playing);
    }
}
#[test]
fn outer_safety_refusal_and_unapplied_command_retirement_acknowledge_without_heap_work() {
    let (engine,mut rt)=loaded();let request=arm(&engine,Lease::new());
    rt.apply(Command::SafetyStop(crate::engine::performance::Safety::Stop));
    assert_eq!(test_alloc::measure(||rt.apply(Command::DeckContinue(request.clone()))),Default::default());assert_eq!(request.outcome(),Outcome::Refused);assert!(!rt.decks[0].playing);
    let (engine,mut rt)=loaded();let request=arm(&engine,Lease::new());
    assert_eq!(test_alloc::measure(||rt.undo.retire_command(Command::DeckContinue(request.clone()))),Default::default());assert_eq!(request.outcome(),Outcome::Refused);
}
#[test]
fn disabled_run_dropped_before_rendering_retires_the_final_request_on_the_worker() {
    let (engine,mut rt)=loaded();let lease=Lease::new();let request=arm(&engine,lease.clone());engine.send(Command::DeckContinue(request)).unwrap();
    lease.disable();drop(lease);
    assert_eq!(test_alloc::measure(||rt.process(&mut [0.0;256])),Default::default());assert!(!rt.decks[0].playing);
}
