mod engine;
mod theme;
fn main() {
 let (_tx,rx)=crossbeam_channel::bounded(16);
 let snap=std::sync::Arc::new(parking_lot::Mutex::new(engine::Snapshot::default()));
 let mut rt=engine::RtEngine::new(48000.0,rx,snap);
 rt.apply(engine::Command::LaunchScene {scene:8});
}
