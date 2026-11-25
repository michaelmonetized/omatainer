#![allow(dead_code)]
mod engine { include!(concat!(env!("OMATAINER_AUDIT_REPO"), "/src/engine/mod.rs")); }
use engine::{Command, RtEngine, Snapshot};
use engine::dsp::Sample;
use engine::fx::{FxId, FxSlot, FxChain};
use std::sync::{Arc,atomic::{AtomicBool,AtomicUsize, Ordering}};
use std::alloc::{GlobalAlloc,Layout,System};
use std::time::{Instant,Duration};
use parking_lot::Mutex;

static COUNT: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
struct Counted;
unsafe impl GlobalAlloc for Counted {
 unsafe fn alloc(&self,l:Layout)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(l.size(),Ordering::Relaxed); } System.alloc(l) }
 unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(l.size(),Ordering::Relaxed); } System.alloc_zeroed(l) }
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(n,Ordering::Relaxed); } System.realloc(p,l,n) }
 unsafe fn dealloc(&self,p:*mut u8,l:Layout) { System.dealloc(p,l) }
}
#[global_allocator] static A: Counted = Counted;
fn make()->RtEngine { let (_tx,rx)=crossbeam_channel::bounded(256); RtEngine::new(48000.,rx,Arc::new(Mutex::new(Snapshot::default()))) }
fn sample(v:Vec<f32>,ch:u16)->Arc<Sample> { Arc::new(Sample{name:"audit".into(),sr:48000,ch,data:v,peaks:vec![],bpm:120.,path:String::new()}) }
fn measure(name:&str, rt:&mut RtEngine, frames:usize) {
 let mut out=vec![0.;frames*2];
 ALLOCS.store(0,Ordering::Relaxed);BYTES.store(0,Ordering::Relaxed);COUNT.store(true,Ordering::Relaxed);
 let t=Instant::now();rt.process(&mut out);let elapsed=t.elapsed();COUNT.store(false,Ordering::Relaxed);
 println!("{name}: frames={frames} allocations={} allocated_bytes={} cpu_wall_ms={:.3}",ALLOCS.load(Ordering::Relaxed),BYTES.load(Ordering::Relaxed),elapsed.as_secs_f64()*1000.);
}
fn main() {
 let mut rt=make();let mut idle=[0.;2048];rt.process(&mut idle);println!("default_stopped_output: last_left={} last_right={} playing={} decks_playing={:?}",idle[2046],idle[2047],rt.playing,rt.decks.each_ref().map(|d|d.playing)); measure("idle",&mut rt,1024);
 rt.apply(Command::LaunchScene{scene:0}); let n:usize=rt.tracks.iter().filter_map(|t|t.playing.map(|p|t.clips[p.scene as usize].notes.len())).sum();println!("demo active_clips={} note_count={n}",rt.tracks.iter().filter(|t|t.playing.is_some()).count()); measure("demo",&mut rt,1024);
 let mut rt=make();for t in &mut rt.tracks{t.fx.slots.push(FxSlot::new(FxId::Comp));}measure("one_fx_each_track",&mut rt,1024);
 let mut rt=make();rt.apply(Command::LaunchClip{track:2,scene:0});rt.tracks[2].fx.slots.push(FxSlot::new(FxId::Arp));measure("one_arp_chord",&mut rt,1024);
 let mut rt=make();rt.apply(Command::DeckAudio{deck:0,audio:sample(vec![0.25;96000],2)}); rt.xfader=0.;rt.master=1.; let mut out=vec![0.;2048];rt.process(&mut out);println!("paused_deck: playing={} pos={} final_left={} final_right={} max_abs={}",rt.decks[0].playing,rt.decks[0].pos,out[2046],out[2047],out.iter().map(|x|x.abs()).fold(0.,f32::max));
 let mut rt=make();rt.apply(Command::DeckAudio{deck:0,audio:sample((0..96000).map(|i|if i%2==0{0.25}else{0.}).collect(),2)});rt.xfader=0.;rt.master=1.;rt.decks[0].playing=true;rt.decks[0].eq.low_g=0.;rt.process(&mut out);println!("left_only_after_deck_eq: final_left={} final_right={}",out[2046],out[2047]);
 for vel in [1,127] {let mut rt=make();rt.decks[0].audio=None;rt.decks[1].audio=None;rt.apply(Command::LiveNoteOn{ch:0,note:36,vel});rt.process(&mut out);println!("drum_vel={vel} abs_sum={:.9}",out.iter().map(|x|x.abs()).sum::<f32>());}
 let mut chain=FxChain::new(48000.);println!("empty_fx_impulse={:?},{:?}",chain.tick_stereo(1.,48000.),chain.tick_stereo(0.,48000.));
 for param in [0.,1.] {let mut chain=FxChain::new(48000.);let mut slot=FxSlot::new(FxId::Delay);slot.p[0]=param;slot.mix=1.;slot.p[1]=0.;chain.slots.push(slot);let mut first=None;for i in 0..24001{let y=chain.tick(if i==0{1.}else{0.},48000.);if y.abs()>0.01&&first.is_none(){first=Some(i);}}println!("delay_param0={param} first_echo_sample={first:?}");}
 for count in [1,2] {let mut chain=FxChain::new(48000.);for _ in 0..count{let mut slot=FxSlot::new(FxId::Delay);slot.mix=1.;slot.p[1]=0.;chain.slots.push(slot);}let first=(0..48001).find(|i|chain.tick(if *i==0{1.}else{0.},48000.).abs()>0.01);println!("serial_delay_slots={count} first_echo={first:?}");}
 let mut rt=make();rt.decks[1].audio=None;let mut v=vec![0.;96000];v[2]=1.;rt.apply(Command::DeckAudio{deck:0,audio:sample(v,2)});rt.decks[0].playing=true;rt.xfader=0.;rt.bpm=120.;rt.fx_wet[0]=1.;rt.delay.fb=0.;let mut v=vec![0.;22000];rt.process(&mut v);println!("master_echo_120_bpm: configured_delay_samples={} first_echo_frame={:?}",rt.delay.time_samples,v.chunks_exact(2).position(|s|s[0].abs()>0.01||s[1].abs()>0.01));
 let mut chain=FxChain::new(48000.);let mut slot=FxSlot::new(FxId::Balance);slot.mix=0.;slot.p[0]=0.;chain.slots.push(slot);for _ in 0..2{chain.tick_stereo(0.8,48000.);}println!("balance_mix_zero={:?}",chain.tick_stereo(0.8,48000.));
 let mut rt=make();rt.decks[0].audio=None;rt.decks[1].audio=None;rt.process(&mut vec![0.;11998]);let snap=rt.snap.clone();let (tx,rx)=std::sync::mpsc::channel();let handle=std::thread::spawn(move||{let guard=snap.lock();tx.send(()).unwrap();std::thread::sleep(Duration::from_millis(50));drop(guard);});rx.recv().unwrap();let t=Instant::now();rt.process(&mut [0.;2]);let elapsed=t.elapsed();handle.join().unwrap();println!("snapshot_lock_contention: one_frame_wall_ms={:.3} reported_cpu_fraction={}",elapsed.as_secs_f64()*1000.,rt.snap.lock().cpu);
}
