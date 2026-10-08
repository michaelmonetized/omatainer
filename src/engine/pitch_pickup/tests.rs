use super::*;
use crate::engine::{Engine,test_alloc};
use crate::engine::midi::{Action,ControlSpec};
fn capture(engine:&Engine,rt:&mut RtEngine)->crate::engine::project::Captured {
    let handle=engine.project.clone();let worker=std::thread::spawn(move||handle.capture(&std::sync::atomic::AtomicBool::new(false)).unwrap());
    let end=std::time::Instant::now()+std::time::Duration::from_secs(15);
    while !worker.is_finished() {rt.process(&mut []);assert!(std::time::Instant::now()<end);std::thread::sleep(std::time::Duration::from_millis(1));}
    worker.join().unwrap()
}
fn input(kind:MsgKind,deck:u8,value:f32)->Input {Input {source:2191,context:[0;3],channel:3,binding:Binding {kind,ch:3,data:7,action:Action::DeckPitch,deck,extra:0,relative:None,controls:None,pair_order:None},value}}
fn send(rt:&mut RtEngine,input:Input) {assert_eq!(test_alloc::measure(||rt.apply(Command::MidiPitch(input))),Default::default());}
#[test]
fn absolute_crossings_acquire_without_a_tempo_jump_and_follow_only_after_pickup() {
    for kind in [MsgKind::Cc,MsgKind::Cc14,MsgKind::Pitch] {for (first,cross,next) in [(0.1,0.8,0.81),(0.9,0.2,0.19)] {
        let (_engine,mut rt)=Engine::headless_for_test(8000,256);let pos=rt.decks[0].pos;let other=rt.decks[1].pitch;
        send(&mut rt,input(kind,0,first));assert_eq!(rt.decks[0].pitch,0.5);assert!(!rt.pitch_pickup_status(0).acquired);
        send(&mut rt,input(kind,0,cross));assert_eq!(rt.decks[0].pitch,0.5);assert!(rt.pitch_pickup_status(0).acquired);
        send(&mut rt,input(kind,0,next));assert_eq!(rt.decks[0].pitch,next);assert_eq!(rt.decks[1].pitch,other);
        assert_eq!(rt.decks[0].pos,pos);assert!(!rt.decks[0].playing);
    }}
}
#[test]
fn physical_layer_range_media_sync_and_other_owners_require_new_crossings() {
    use crate::engine::deck_sync::Mode;
    let (_engine,mut rt)=Engine::headless_for_test(8000,256);
    send(&mut rt,input(MsgKind::Cc,0,0.5));send(&mut rt,input(MsgKind::Cc,0,0.7));assert_eq!(rt.decks[0].pitch,0.7);
    send(&mut rt,input(MsgKind::Cc,1,0.9));assert_eq!(rt.decks[1].pitch,0.5);assert!(rt.pitch_pickup_status(0).physical.is_none());
    send(&mut rt,input(MsgKind::Cc,0,0.9));assert_eq!(rt.decks[0].pitch,0.7);assert!(!rt.pitch_pickup_status(0).acquired);
    send(&mut rt,input(MsgKind::Cc,0,0.6));assert_eq!(rt.decks[0].pitch,0.7);assert!(rt.pitch_pickup_status(0).acquired);
    rt.apply(Command::DeckPitchRange {deck:0});send(&mut rt,input(MsgKind::Cc,0,0.2));assert_eq!(rt.decks[0].pitch,0.7);
    rt.apply(Command::DeckSyncMode {deck:0,mode:Mode::Tempo});send(&mut rt,input(MsgKind::Cc,0,0.9));send(&mut rt,input(MsgKind::Cc,0,0.2));assert_eq!(rt.decks[0].pitch,0.7);assert!(rt.decks[0].sync);
    rt.apply(Command::DeckSyncMode {deck:0,mode:Mode::Off});send(&mut rt,input(MsgKind::Cc,0,0.9));assert_eq!(rt.decks[0].pitch,0.7);assert!(!rt.pitch_pickup_status(0).acquired);
    send(&mut rt,input(MsgKind::Cc,0,0.6));send(&mut rt,input(MsgKind::Cc,0,0.61));assert_eq!(rt.decks[0].pitch,0.61);
    send(&mut rt,Input {context:[1,2,3],value:0.9,..input(MsgKind::Cc,0,0.9)});assert_eq!(rt.decks[0].pitch,0.61);assert!(!rt.pitch_pickup_status(0).acquired);
    rt.apply(Command::DeckPitch {deck:0,value:0.3});send(&mut rt,input(MsgKind::Cc,0,0.7));assert_eq!(rt.decks[0].pitch,0.3);
    let other=Input {source:2192,..input(MsgKind::Cc,0,0.3)};send(&mut rt,other);send(&mut rt,Input {value:0.4,..other});assert_eq!(rt.decks[0].pitch,0.4);
    send(&mut rt,input(MsgKind::Cc,0,0.7));assert_eq!(rt.decks[0].pitch,0.4);
    let audio=rt.decks[0].audio.clone().unwrap();rt.apply(Command::DeckAudio {deck:0,audio});let before=rt.decks[0].pitch;
    send(&mut rt,input(MsgKind::Cc,0,0.9));assert_eq!(rt.decks[0].pitch,before);assert!(!rt.pitch_pickup_status(0).acquired);
    let (_engine,mut rt)=Engine::headless_for_test(8000,256);
    for action in 0..4 {
        send(&mut rt,input(MsgKind::Cc,0,0.5));send(&mut rt,input(MsgKind::Cc,0,0.6));
        match action {
            0=>{rt.apply(Command::DeckPitch {deck:0,value:0.7});rt.apply(Command::DeckPitch {deck:0,value:0.6});},
            1=>for _ in 0..3 {rt.apply(Command::DeckPitchRange {deck:0});},
            2=>{rt.apply(Command::DeckSyncMode {deck:0,mode:Mode::Tempo});rt.apply(Command::DeckSyncMode {deck:0,mode:Mode::Off});},
            _=>{rt.apply(Command::DeckPitch {deck:0,value:0.7});rt.apply(Command::Undo);},
        }
        assert_eq!(rt.decks[0].pitch,0.6);
        send(&mut rt,input(MsgKind::Cc,0,0.9));assert_eq!(rt.decks[0].pitch,0.6);assert!(!rt.pitch_pickup_status(0).acquired,"round-trip edit {action}");
        rt.apply(Command::DeckPitch {deck:0,value:0.5});
    }
}
#[test]
fn mapping_limits_inversion_wire_channels_and_bounded_owner_reuse_never_bypass_pickup() {
    let (_engine,mut rt)=Engine::headless_for_test(8000,256);
    let mut b=input(MsgKind::Cc14,0,0.2);b.binding.controls=Some(ControlSpec {min:0.2,max:0.8,invert:true});
    send(&mut rt,b);assert_eq!(rt.decks[0].pitch,0.5);send(&mut rt,Input {value:0.8,..b});assert_eq!(rt.decks[0].pitch,0.5);send(&mut rt,Input {value:0.79,..b});assert_eq!(rt.decks[0].pitch,0.79);
    send(&mut rt,Input {channel:4,binding:Binding {ch:4,..b.binding},value:0.2,..b});assert_eq!(rt.decks[0].pitch,0.79);
    for source in 2200..2400 {send(&mut rt,Input {source,value:0.2,..b});assert_eq!(rt.decks[0].pitch,0.79);}
    send(&mut rt,Input {value:0.2,..b});assert_eq!(rt.decks[0].pitch,0.79);assert!(!rt.pitch_pickup_status(0).acquired);
    for bad in [Input {source:0,..b},Input {channel:16,..b},Input {value:f32::NAN,..b},Input {value:1.1,..b},Input {binding:Binding {deck:2,..b.binding},..b}] {assert!(!bad.valid());let before=rt.decks[0].pitch;rt.apply(Command::MidiPitch(bad));assert_eq!(rt.decks[0].pitch,before);}
}
#[test]
fn input_acquisition_is_transient_and_native_pitch_undo_restores_the_base_without_retargeting_the_fader() {
    let (engine,mut rt)=Engine::headless_for_test(8000,256);
    send(&mut rt,input(MsgKind::Pitch,0,0.5));rt.clear_undo_for_test();send(&mut rt,input(MsgKind::Pitch,0,0.75));
    rt.apply(Command::Undo);assert_eq!(rt.decks[0].pitch,0.5);assert!(!rt.pitch_pickup_status(0).acquired);
    send(&mut rt,input(MsgKind::Pitch,0,0.9));assert_eq!(rt.decks[0].pitch,0.5);
    send(&mut rt,input(MsgKind::Pitch,0,0.4));assert!(rt.pitch_pickup_status(0).acquired);assert_eq!(rt.decks[0].pitch,0.5);
    let captured=capture(&engine,&mut rt);assert_eq!(captured.state.decks[0].pitch,0.5);
    let path=std::env::temp_dir().join(format!("omat-pitch-pickup-{}.omat",std::process::id()));
    let limits=crate::project_file::Limits::default();let cancel=std::sync::atomic::AtomicBool::new(false);
    crate::project_file::save(&path,&crate::project_file::Bundle {state:captured.state,media:captured.media},crate::project_file::Overwrite::Never,&limits,&cancel).unwrap();
    let disk=crate::project_file::load::<crate::engine::project::State>(&path,&limits,&cancel).unwrap();std::fs::remove_file(path).unwrap();
    let reopened=crate::engine::project::Prepared::from_state(disk.state,disk.media,44100).unwrap();
    assert_eq!(reopened.rt.decks[0].pitch,0.5);assert!(!reopened.rt.decks[0].playing);assert!(reopened.rt.pitch_pickup_status(0).physical.is_none());
}

#[test]
fn absolute_pickup_and_momentary_bend_match_independent_base_rate_pcm_at_three_output_rates() {
    use crate::engine::deck_controls::{Control,Button};
    for rate in [8000,44100,48000] {
        let (_actual_engine,actual)=Engine::headless_for_test(rate,256);let mut actual=Box::new(actual);
        let (_reference_engine,reference)=Engine::headless_for_test(rate,256);let mut reference=Box::new(reference);
        for rt in [&mut actual,&mut reference] {rt.metronome=false;rt.master=1.0;rt.xfader=0.0;rt.apply(Command::DeckPlay {deck:0});}
        let mut a=vec![0.0;rate as usize/10*2];let mut b=vec![0.0;a.len()];let mut energy=0.0_f64;
        for (value,target) in [(0.1,0.5),(0.9,0.5),(0.8,0.8)] {
            send(&mut actual,input(MsgKind::Pitch,0,value));reference.apply(Command::DeckPitch {deck:0,value:target});
            assert_eq!(test_alloc::measure(||actual.process(&mut a)),Default::default());reference.process(&mut b);
            assert_eq!(a,b);assert_eq!(actual.decks[0].pos,reference.decks[0].pos);energy+=a.iter().map(|s|f64::from(*s).powi(2)).sum::<f64>();
        }
        for rt in [&mut actual,&mut reference] {rt.apply(Command::DeckPitch {deck:0,value:0.5});rt.apply(Command::DeckPitchRange {deck:0});}
        for (button,on,pitch) in [(Button::BendUp,true,0.75),(Button::BendUp,false,0.5),(Button::BendDown,true,0.25),(Button::BendDown,false,0.5)] {
            actual.apply(Command::DeckControl {source:2199,deck:0,control:Control::Hold {button,on}});
            reference.apply(Command::DeckPitch {deck:0,value:pitch});
            assert_eq!(test_alloc::measure(||actual.process(&mut a)),Default::default());reference.process(&mut b);
            assert!(a==b,"bend {button:?} on={on}, rate={rate}: max PCM difference {}",a.iter().zip(&b).map(|(a,b)|(a-b).abs()).fold(0.0_f32,f32::max));assert_eq!(actual.decks[0].pos,reference.decks[0].pos);assert_eq!(actual.decks[0].pitch,0.5);
            energy+=a.iter().map(|s|f64::from(*s).powi(2)).sum::<f64>();
        }
        assert!(energy>0.01);println!("PITCH_PICKUP_PCM_RECEIPT {{\"output_rate\":{rate},\"compared_frames\":{},\"max_sample_error\":0,\"source_position_error\":0,\"energy\":{energy},\"physical_devices_opened\":false}}",a.len()/2*7);
    }
}
