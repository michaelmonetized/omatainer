#![allow(dead_code)]
mod engine { include!(concat!(env!("OMATAINER_AUDIT_REPO"), "/src/engine/mod.rs")); }
use engine::*;
use std::sync::Arc;
use parking_lot::Mutex;
fn fresh() -> RtEngine {
 let (_,rx)=crossbeam_channel::bounded(256);
 let mut rt=RtEngine::new(48000., rx, Arc::new(Mutex::new(Snapshot::default())));
 rt.apply(Command::DeckUnload{deck:0}); rt.apply(Command::DeckUnload{deck:1});
 rt
}
fn render(rt:&mut RtEngine,n:usize) { rt.process(&mut vec![0.;n*2]); }
fn active(rt:&RtEngine,t:usize)->Vec<(u8,u8)> { rt.tracks[t].poly.voices.iter().filter(|v|v.env.active()).map(|v|(v.note,v.env.stage)).collect() }
fn main() {
 let mut rt=fresh(); rt.beat=0.25; rt.playing=true;
 rt.tracks[1].clips[2]=Clip{kind:ClipKind::Midi,name:"probe".into(),bars:1.,notes:vec![MidiNote{pitch:60,start:0.,len:3.9,vel:100}],gain:1.,audio:None};
 rt.apply(Command::FireClip{track:1,scene:2,looping:false});
 let scheduled=rt.tracks[1].playing.unwrap().start_beat;
 render(&mut rt,1);
 println!("QUANT first frame: beat={:.6}, scheduled={}, active={:?}",rt.beat,scheduled,active(&rt,1));
 assert!(rt.beat<scheduled && !active(&rt,1).is_empty());
 render(&mut rt,20000);
 println!("QUANT one-shot after scheduled start: beat={:.6}, playing_clip={:?}, active={:?}",rt.beat,rt.tracks[1].playing,active(&rt,1));
 assert!(rt.tracks[1].playing.is_none());
 let mut rt=fresh(); rt.apply(Command::Select{track:1,scene:2}); rt.apply(Command::LiveNoteOn{ch:0,note:60,vel:100}); render(&mut rt,128);
 rt.apply(Command::Select{track:2,scene:2}); rt.apply(Command::LiveNoteOff{ch:0,note:60}); render(&mut rt,96000);
 println!("NOTE OFF after changing selection: old_track={:?}, new_track={:?}",active(&rt,1),active(&rt,2));
 assert!(active(&rt,1).contains(&(60,3)));
 let mut rt=fresh(); rt.tracks[1].clips[2]=Clip{kind:ClipKind::Midi,name:"held".into(),bars:1.,notes:vec![MidiNote{pitch:64,start:0.,len:3.,vel:100}],gain:1.,audio:None};
 rt.apply(Command::LaunchClip{track:1,scene:2}); render(&mut rt,1000); rt.apply(Command::ToggleScene{scene:2}); render(&mut rt,96000);
 println!("SCENE OFF after 2 s: playing_clip={:?}, active={:?}",rt.tracks[1].playing,active(&rt,1));
 assert!(rt.tracks[1].playing.is_none() && active(&rt,1).contains(&(64,3)));
 let mut rt=fresh(); render(&mut rt,5500); let snap=rt.snap.clone(); let held=snap.lock();
 let (tx,rx)=std::sync::mpsc::channel(); let (done_tx,done_rx)=std::sync::mpsc::channel();
 let thread=std::thread::spawn(move||{tx.send(()).unwrap(); let t=std::time::Instant::now(); render(&mut rt,512); done_tx.send(t.elapsed()).unwrap();});
 rx.recv().unwrap(); std::thread::sleep(std::time::Duration::from_millis(80));
 let blocked=done_rx.try_recv().is_err(); drop(held); let elapsed=done_rx.recv().unwrap(); thread.join().unwrap();
 println!("SNAPSHOT lock: render blocked={}, total_ms={:.3}, simulated_block_budget_ms=10.667",blocked,elapsed.as_secs_f64()*1000.); assert!(blocked);
}
