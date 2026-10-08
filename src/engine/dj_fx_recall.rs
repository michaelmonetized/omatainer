use super::{dj_fx_preset::Settings, session::{Axis, Layout}};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase { #[default] Idle, FadingOut, FadingIn, Applied, Stale, Rejected }

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct Receipt { pub token: u64, pub phase: Phase, pub last_rejected: u64 }

#[derive(Clone, Copy, Debug)]
pub(crate) struct Request { pub token: u64, pub namespace: [u64; 2], pub expected: [Settings; 2], pub desired: [Settings; 2] }
impl Request {
    /// Validate one complete reviewed recall before command admission.
    /// Takes its settings and token; returns whether all values are valid and the token is nonzero.
    pub fn valid(self) -> bool { self.token != 0 && self.namespace != [0, 0] && self.expected.iter().chain(&self.desired).all(Settings::valid) }
}

pub(crate) struct Transition {
    pending: Option<Request>,
    pub receipt: Receipt,
    pub gain: f32,
    start: f32,
    remaining: u32,
    length: u32,
}
impl Default for Transition {
    fn default() -> Self { Self { pending: None, receipt: Receipt::default(), gain: 1.0, start: 1.0, remaining: 0, length: 1 } }
}
fn valid_routes(settings: &[Settings; 2], layout: &Layout) -> bool {
    settings.iter().all(|unit| unit.sampler.is_none_or(|target| layout.tracks.iter().enumerate().any(|(index, track)| track.active && layout.resolves(Axis::Track, index, target))))
}
impl Transition {
    /// Begin a reviewed reset-after-fade recall.
    /// Takes the request, current controls, layout and rate; preserves controls unless both expected settings and every current-project target still match.
    pub fn begin(&mut self, request: Request, current: [Settings; 2], layout: &Layout, rate: f32) -> bool {
        if self.pending.is_some() || matches!(self.receipt.phase, Phase::FadingOut | Phase::FadingIn) { self.receipt.last_rejected = request.token; return false; }
        if !request.valid() { self.receipt = Receipt { token: request.token, phase: Phase::Rejected, last_rejected: request.token }; return false; }
        if request.namespace != layout.namespace || request.expected != current || !valid_routes(&request.desired, layout) {
            self.receipt = Receipt { token: request.token, phase: Phase::Stale, last_rejected: 0 }; return false;
        }
        self.pending = Some(request); self.receipt = Receipt { token: request.token, phase: Phase::FadingOut, last_rejected: 0 };
        self.start = self.gain; self.length = (rate * 0.005).ceil().max(1.0) as u32; self.remaining = self.length; true
    }
    /// Advance both units once per actual output frame.
    /// Takes current controls and layout; returns both desired units together only at zero contribution. The caller resets fixed histories and resolves sampler routes before rendering that frame.
    pub fn frame(&mut self, current: [Settings; 2], layout: &Layout) -> Option<[Settings; 2]> {
        if let Some(request) = self.pending {
            if request.namespace != layout.namespace || request.expected != current || !valid_routes(&request.desired, layout) {
                self.pending = None; self.receipt.phase = Phase::Stale; self.start = self.gain; self.remaining = self.length;
            } else {
                self.remaining -= 1; self.gain = self.start * self.remaining as f32 / self.length as f32;
                if self.remaining == 0 {
                    self.pending = None; self.receipt.phase = Phase::FadingIn; self.start = 0.0; self.remaining = self.length;
                    return Some(request.desired);
                }
                return None;
            }
        }
        if self.remaining > 0 {
            self.remaining -= 1; let fraction = self.remaining as f32 / self.length as f32;
            self.gain = 1.0 - (1.0 - self.start) * fraction;
            if self.remaining == 0 && self.receipt.phase == Phase::FadingIn { self.receipt.phase = Phase::Applied; }
        }
        None
    }
    /// Retire a pending recall when the audio owner stops or its rate changes.
    /// Takes no inputs; preserves currently applied controls, returns gain to unity and reports an unfinished transition as stale.
    pub fn retire(&mut self) {
        if self.pending.is_some() || self.receipt.phase == Phase::FadingIn { self.receipt.phase = Phase::Stale; }
        self.pending = None; self.gain = 1.0; self.remaining = 0;
    }
}
