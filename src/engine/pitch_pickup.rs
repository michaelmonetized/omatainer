use super::{midi::{Binding, MsgKind}, Command, RtEngine};
use serde::Serialize;

const OWNERS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Input {
    pub source: u64,
    pub context: [u64;3],
    pub channel: u8,
    pub binding: Binding,
    pub value: f32,
}
impl Input {
    /// Admit an absolute pitch assignment with its original physical identity.
    /// Takes this input; returns whether its owner, wire address, parameter range and value are supported.
    pub(crate) fn valid(self) -> bool {
        let spec=self.binding.controls.unwrap_or_default();
        self.source != 0 && self.channel < 16 && self.binding.deck < 2
            && (self.binding.ch==0xff || self.binding.ch==self.channel)
            && self.binding.data < 128 && (self.binding.kind!=MsgKind::Cc14 || self.binding.data<32) && self.binding.action == super::midi::Action::DeckPitch
            && matches!(self.binding.kind, MsgKind::Cc | MsgKind::Cc14 | MsgKind::Pitch)
            && self.binding.controls.unwrap_or_default().valid(self.binding.action)
            && self.value.is_finite() && (spec.min..=spec.max).contains(&self.value)
    }
    fn same_wire(self, other: Self) -> bool {
        self.source == other.source && self.channel == other.channel
            && self.binding.kind == other.binding.kind
            && (self.binding.kind == MsgKind::Pitch || self.binding.data == other.binding.data)
    }
    fn tolerance(self) -> f32 {
        let spec = self.binding.controls.unwrap_or_default();
        (spec.max - spec.min) * 0.5 / if self.binding.kind == MsgKind::Cc {127.0} else {16383.0} + 1e-6
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub physical: Option<f64>,
    pub target: f64,
    pub acquired: bool,
    pub sync: bool,
}
#[derive(Clone, Copy)]
struct Owner {
    input: Input,
    media: u64,
    range: u8,
    epoch: u64,
    sync: super::deck_sync::Mode,
    expected: f32,
    previous: Option<f32>,
    acquired: bool,
}
pub(super) struct State {
    owners: [Option<Owner>; OWNERS],
    next: usize,
    recent: [Option<usize>; 2],
}
impl Default for State {
    fn default() -> Self {Self {owners:[None;OWNERS],next:0,recent:[None;2]}}
}
impl State {
    /// Require a fresh crossing after a deliberate pitch or mode edit.
    /// Takes the exact deck; clears acquisition and prior samples without losing the last observed physical position.
    pub(super) fn rearm(&mut self, deck: usize) {
        for owner in self.owners.iter_mut().flatten().filter(|o|usize::from(o.input.binding.deck)==deck) {
            owner.acquired=false;
            owner.previous=None;
        }
    }
}
impl RtEngine {
    /// Publish one current pickup target without moving the software fader.
    /// Takes an exact deck; returns its last observed physical value and whether that input still owns the base pitch.
    pub(super) fn pitch_pickup_status(&self, deck: usize) -> Status {
        let d=&self.decks[deck];
        let owner=self.pitch_pickup.recent[deck].and_then(|index|self.pitch_pickup.owners[index])
            .filter(|o|usize::from(o.input.binding.deck)==deck && o.media==d.history_key && o.epoch==self.performance.input_epoch());
        Status {physical:owner.map(|o|f64::from(o.input.value)),target:f64::from(d.pitch),acquired:owner.is_some_and(|o|o.acquired && o.expected==d.pitch && o.range==d.pitch_range && o.sync==d.sync_mode()),sync:d.sync}
    }
    /// Preserve the current pitch until an absolute fader reaches its target.
    /// Takes a source-owned complete wire value; follows acquired inputs through the existing native pitch and Undo route, with no callback allocation.
    pub(super) fn absolute_pitch(&mut self, input: Input) {
        if !input.valid() {self.undo.reject(super::undo::Failure::Invalid);return;}
        let deck=usize::from(input.binding.deck);
        let d=&self.decks[deck];
        let state=&mut self.pitch_pickup;
        let index=state.owners.iter().position(|o|o.is_some_and(|o|input.same_wire(o.input))).unwrap_or_else(|| {
            let index=state.next;state.next=(state.next+1)%OWNERS;state.owners[index]=None;index
        });
        let same=state.owners[index].is_some_and(|o|o.input.binding==input.binding && o.input.context==input.context && o.media==d.history_key
            && o.range==d.pitch_range && o.epoch==self.performance.input_epoch() && o.sync==d.sync_mode() && o.expected==d.pitch);
        let mut owner=if same {state.owners[index].unwrap()} else {Owner {input,media:d.history_key,range:d.pitch_range,epoch:self.performance.input_epoch(),sync:d.sync_mode(),expected:d.pitch,previous:None,acquired:false}};
        owner.input=input;
        let target=d.pitch;
        let crossed=owner.previous.is_some_and(|p| (p-target)*(input.value-target)<=0.0);
        let reached=(input.value-target).abs()<=input.tolerance() || crossed;
        let next=if d.sync {owner.acquired=false;None} else if owner.acquired {Some(input.value)} else {owner.acquired=reached;None};
        owner.previous=Some(input.value);
        owner.expected=next.unwrap_or(target);
        if let Some(value)=next {self.apply(Command::DeckPitch {deck:input.binding.deck,value});}
        owner.expected=self.decks[deck].pitch;
        self.pitch_pickup.owners[index]=Some(owner);self.pitch_pickup.recent[deck]=Some(index);
    }
}
#[cfg(test)]
mod tests;
