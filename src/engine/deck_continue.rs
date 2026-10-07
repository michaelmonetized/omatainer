use super::{Command, DeckSnap, RtEngine, DECKS};
use std::sync::{Arc, atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering}};

#[derive(Debug)]
pub(crate) struct Lease(AtomicBool);
impl Lease {
    /// Own one explicitly enabled continuous-playback run.
    /// Takes no arguments; returns a new active lease outside the audio callback.
    pub(crate) fn new()->Arc<Self> {Arc::new(Self(AtomicBool::new(true)))}
    /// Revoke future automatic starts without changing manual playback.
    /// Takes this lease; disables every pending request that shares it.
    pub(crate) fn disable(&self) {self.0.store(false,Ordering::Release);}
    /// Read whether the original run remains enabled.
    /// Takes this lease; returns its current admission state without waiting.
    pub(crate) fn enabled(&self)->bool {self.0.load(Ordering::Acquire)}
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct Request(Arc<Inner>);
#[derive(Debug)]
struct Inner {
    deck:u8, media:u64, transport:u64, safety:u64, lease:Arc<Lease>,
    state:AtomicU8, started_end:AtomicU64, started_transport:AtomicU64,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum Outcome {Pending, Started {end:u64, transport:u64}, Refused}
impl Request {
    /// Capture one exact paused source for an automatic continuation.
    /// Takes its deck snapshot, safety epoch and enabled run; returns a source/transport-qualified request or a visible refusal.
    pub(crate) fn new(deck:u8,snapshot:&DeckSnap,safety:u64,lease:Arc<Lease>)->Result<Self,String> {
        if usize::from(deck)>=DECKS||snapshot.media_key==0||snapshot.frames<=0.0||!lease.enabled() {return Err("Load a track and enable continuous playback before starting it".into());}
        Ok(Self(Arc::new(Inner {deck,media:snapshot.media_key,transport:snapshot.transport_generation,safety,lease,state:AtomicU8::new(0),started_end:AtomicU64::new(0),started_transport:AtomicU64::new(0)})))
    }
    /// Read the renderer's exact start acknowledgement.
    /// Takes this request; returns pending, refusal or the counters captured before audio advances.
    pub(crate) fn outcome(&self)->Outcome {
        match self.0.state.load(Ordering::Acquire) {1=>Outcome::Started {end:self.0.started_end.load(Ordering::Relaxed),transport:self.0.started_transport.load(Ordering::Relaxed)},2=>Outcome::Refused,_=>Outcome::Pending}
    }
    /// Account for the fixed request and run ownership retired by the worker.
    /// Takes this request; returns retained bytes including both Arc counters.
    pub(super) fn bytes(&self)->usize {std::mem::size_of::<Self>()+std::mem::size_of::<Inner>()+std::mem::size_of::<Lease>()+4*std::mem::size_of::<usize>()}
    /// Finish a pending request after outer admission refuses it.
    /// Takes this request; records refusal once without replacing a prior start acknowledgement.
    pub(super) fn reject(&self) {let _=self.0.state.compare_exchange(0,2,Ordering::AcqRel,Ordering::Acquire);}
    /// Start only the still-owned quiet deck through ordinary transport admission.
    /// Takes the renderer; leaves changed sources, manual controls, revoked runs and safety transitions untouched.
    pub(super) fn apply(&self,rt:&mut RtEngine) {
        let request=&self.0;let index=usize::from(request.deck);
        if request.state.compare_exchange(0,3,Ordering::AcqRel,Ordering::Acquire).is_err() {return;}
        let admitted=request.lease.enabled()&&rt.performance.safety_epoch()==request.safety
            &&rt.decks.get(index).is_some_and(|deck|deck.history_key==request.media&&deck.transport_generation==request.transport&&!deck.playing&&!deck.touching&&deck.preview_position.is_none()&&deck.audio.is_some());
        if !admitted {request.state.store(2,Ordering::Release);return;}
        let end=rt.decks[index].natural_end;
        rt.apply(Command::DeckPlay {deck:request.deck});
        if rt.decks[index].playing {
            request.started_end.store(end,Ordering::Relaxed);request.started_transport.store(rt.decks[index].transport_generation,Ordering::Relaxed);request.state.store(1,Ordering::Release);
        } else {request.state.store(2,Ordering::Release);}
    }
}
