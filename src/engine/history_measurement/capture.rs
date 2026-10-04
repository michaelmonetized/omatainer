//! Only final OutputCallback conversion promotes samples into observations.
use super::{tracker::{Contribution, Tracker}, Episode, Frame, Observation, Windows, LANES};
use crate::engine::{performance, FxKind};
use super::control::{Action, Ack, Endpoint, Handle, Outcome, Progress, DigitalPlay};
use crossbeam_channel::{Receiver, Sender};

const MAX_FRAMES: usize = 16_384;
const EVENTS: usize = 4_096;

#[derive(Clone, Copy)]
struct Rendered {
    episodes: [Option<Episode>; LANES],
    playing: [u64; 2],
    actual: [f32; 2],
    without: [[f32; 2]; LANES],
    error: [[f64; 2]; LANES],
}
impl Default for Rendered {
    fn default() -> Self { Self { episodes: [None; LANES], playing: [0; 2], actual: [0.0; 2],
        without: [[0.0; 2]; LANES], error: [[0.0; 2]; LANES] } }
}

pub(in crate::engine) struct Measurement {
    pub tracker: Tracker,
    pub available: bool,
    rate: u32,
    sidecar: Vec<Rendered>,
    capture: bool,
    frame_count: usize,
    active: bool,
    prepare_state: u64,
    window_playing: Option<[u64; 2]>,
    session: u64,
    endpoint: Endpoint,
    callback_wall_ns: u64,
    window_wall_ns: u64,
    clock: u64,
    windows: Option<Windows>,
    window_episodes: [Option<Episode>; LANES],
    sender: Sender<Observation>,
    pub incomplete: bool,
    pub dropped: u64,
}

impl Measurement {
    pub fn new(rate: u32) -> Result<Self, &'static str> {
        let tracker = Tracker::new(rate)?;
        let mut sidecar = Vec::new();
        sidecar.try_reserve_exact(MAX_FRAMES).map_err(|_| "history conversion storage unavailable")?;
        sidecar.resize(MAX_FRAMES, Rendered::default());
        let (sender, receiver) = crossbeam_channel::bounded(EVENTS);
        Ok(Self { tracker, available: true, rate, sidecar, capture: false, frame_count: 0, active: false, prepare_state: 0, window_playing: None, session: 0, endpoint: Endpoint::new(receiver), callback_wall_ns: 0, window_wall_ns: 0,
            clock: 0, windows: None, window_episodes: [None; LANES], sender,
            incomplete: false, dropped: 0 })
    }
    pub fn take_receiver(&mut self) -> Option<Receiver<Observation>> { self.endpoint.handle.take_observations() }
    /// These are renderer-owned boundary operations; the session service must
    /// acknowledge them through its request mailbox before reporting a start/end.
    pub fn handle(&self) -> Handle { self.endpoint.handle.clone() }
    pub fn service_requests(&mut self, current: [u64; 2]) {
        let Some(request) = self.endpoint.request() else { return; };
        let session = match request.action { Action::Start(session) | Action::End(session) => session };
        let wall = self.endpoint.clock();
        let outcome = match request.action {
            Action::Start(_) if self.active => Outcome::AlreadyActive,
            Action::Start(_) if !self.available || wall.is_none() => Outcome::Unavailable,
            Action::Start(_) if !self.capture || self.frame_count == 0 || self.frame_count > MAX_FRAMES => Outcome::NoOutput,
            Action::Start(_) => { self.start(); self.session = session; Outcome::Started },
            Action::End(_) if !self.active || self.session != session => Outcome::WrongSession,
            Action::End(_) => { self.end(); Outcome::Ended },
        };
        let wall_ns = wall.unwrap_or_else(|| { self.incomplete = true; 0 });
        if outcome == Outcome::Started { self.callback_wall_ns = wall_ns; }
        self.endpoint.publish_status(if self.active { self.session } else { 0 }, self.incomplete, self.dropped);
        self.endpoint.acknowledge(Ack { request: request.id, session, outcome, wall_ns,
            frame: self.clock, rate: self.rate, incomplete: self.incomplete, dropped: self.dropped, current });
    }
    pub fn start(&mut self) -> bool {
        if self.active { return false; }
        self.active = true; self.session = 1; self.windows = None; self.window_playing = None; self.window_episodes = [None; LANES];
        self.incomplete = self.tracker.incomplete; self.dropped = 0;
        true
    }
    pub fn end(&mut self) -> bool {
        if !self.active { return false; }
        self.flush(); self.windows = None; self.window_playing = None; self.active = false;
        true
    }
    pub fn configure(&mut self, kinds: [FxKind; 3], wet: [f32; 3], spb: f64) {
        self.tracker.configure(kinds, wet, spb);
    }
    pub fn reset_dsp(&mut self) { self.tracker.reset(); }
    /// Off-callback rate preparation preserves the observation receiver and
    /// closes the previous rational-rate segment before replacing DSP storage.
    pub fn set_rate(&mut self, rate: u32) {
        self.flush(); self.windows = None; self.window_playing = None; self.rate = rate;
        match Tracker::new(rate) {
            Ok(tracker) => { self.tracker = tracker; self.available = true; }
            Err(_) => { self.available = false; self.incomplete = true; }
        }
    }
    pub fn begin_output(&mut self, frames: usize) {
        let state = self.endpoint.prepare_state();
        if state != self.prepare_state && !self.active { self.windows = None; self.window_playing = None; }
        self.prepare_state = state;
        self.capture = true;
        self.callback_wall_ns = self.endpoint.clock().unwrap_or(0);
        self.frame_count = frames;
        if (self.active || self.prepare_state & 1 != 0) && frames > MAX_FRAMES {
            self.flush(); self.windows = None;
            self.incomplete = true;
            self.dropped = self.dropped.saturating_add(frames as u64);
        }
    }
    pub fn record_rendered(&mut self, index: usize, contribution: Contribution,
        before_limiter: [f32; 2], master: f32, safety: &performance::Output, playing: [u64; 2],
    ) {
        if !self.capture || (!self.active && self.prepare_state & 1 == 0) || !self.available || self.frame_count > MAX_FRAMES { return; }
        if index >= self.frame_count { self.incomplete = true; return; }
        self.incomplete |= contribution.incomplete;
        let actual = safety.preview(before_limiter.map(|v| (v * master).tanh()));
        let mut record = Rendered { episodes: contribution.episodes, playing, actual, ..Default::default() };
        let safety_gain = f64::from(safety.preview([1.0; 2])[0]).abs();
        for lane in 0..LANES {
            record.without[lane] = safety.preview(std::array::from_fn(|c|
                ((before_limiter[c] - contribution.values[lane][c]) * master).tanh()));
            record.error[lane] = std::array::from_fn(|c| {
                let magnitude = f64::from(before_limiter[c]).abs() + f64::from(contribution.values[lane][c]).abs();
                // The isolated linear uncertainty passes through a 1-Lipschitz
                // limiter and non-amplifying emergency envelope. Keep a guard
                // for the f32 subtraction, scaling and nonlinear evaluation.
                (contribution.error[lane][c] + magnitude * f64::from(f32::EPSILON) * 8.0)
                    * f64::from(master).abs() * safety_gain
            });
        }
        self.sidecar[index] = record;
    }
    pub fn converted<T>(&mut self, output: &[T], channels: usize)
    where T: cpal::SizedSample + cpal::FromSample<f32>, f64: cpal::FromSample<T>,
    {
        if !self.capture { return; }
        self.capture = false;
        let channels = channels.max(1);
        let frames = output.len() / channels;
        if frames != self.frame_count { self.incomplete = true; return; }
        if (self.active || self.prepare_state & 1 != 0) && self.available && frames <= MAX_FRAMES {
            for i in 0..frames {
                let record = self.sidecar[i];
                if self.windows.is_none() || record.episodes != self.window_episodes {
                    self.flush();
                    self.window_playing = None;
                    self.window_episodes = record.episodes;
                    self.window_wall_ns = self.frame_wall_ns(i);
                    self.windows = self.clock.checked_add(i as u64)
                        .and_then(|first| Windows::new(self.rate, first, record.episodes));
                    if self.windows.is_none() { self.incomplete = true; }
                }
                let map = |pair: [f32; 2]| if channels == 1 { [0.5 * (pair[0] + pair[1]), 0.0] } else { pair };
                let analog = map(record.actual).map(f64::from);
                let digital = [cpal::Sample::to_sample::<f64>(output[i * channels]),
                    if channels == 1 { 0.0 } else { cpal::Sample::to_sample::<f64>(output[i * channels + 1]) }];
                let mapped_without = record.without.map(map);
                let error = record.error.map(|pair| if channels == 1 {
                    [0.5 * (pair[0] + pair[1]) + (analog[0].abs() + 1.0) * f64::from(f32::EPSILON) * 2.0, 0.0]
                } else { pair });
                let without_digital = mapped_without.map(|p| p.map(|v| cpal::Sample::to_sample::<f64>(T::from_sample(v))));
                let digital_error = std::array::from_fn(|lane| std::array::from_fn(|c| {
                    if !error[lane][c].is_finite() { return f64::INFINITY; }
                    let midpoint = f64::from(mapped_without[lane][c]);
                    let low = cpal::Sample::to_sample::<f64>(T::from_sample((midpoint - error[lane][c]).clamp(-1.0, 1.0) as f32));
                    let high = cpal::Sample::to_sample::<f64>(T::from_sample((midpoint + error[lane][c]).clamp(-1.0, 1.0) as f32));
                    (low - without_digital[lane][c]).abs().max((high - without_digital[lane][c]).abs())
                }));
                let frame = Frame { analog, digital,
                    without_analog: mapped_without.map(|p| p.map(f64::from)),
                    without_digital, error, digital_error,
                };
                let playing = self.window_playing.get_or_insert(record.playing);
                for (previous, current) in playing.iter_mut().zip(record.playing) { if *previous != current { *previous = 0; } }
                let observations = self.windows.as_mut().and_then(|w| w.push(&frame));
                let completed = observations.is_some();
                self.publish(observations);
                if completed { self.window_wall_ns = self.frame_wall_ns(i + 1); self.window_playing = None; }
            }
        }
        if let Some(clock) = self.clock.checked_add(frames as u64) { self.clock = clock; }
        else { self.incomplete = true; self.active = false; self.flush(); }
        self.endpoint.publish_status(if self.active { self.session } else { 0 }, self.incomplete, self.dropped);
        if self.active {
            let confirmed = self.windows.as_ref().map_or(self.clock, Windows::confirmed_frame);
            let wall_ns = self.endpoint.clock().unwrap_or(0);
            self.endpoint.progress(Progress { session: self.session, wall_ns, frame: confirmed, rate: self.rate,
                incomplete: self.incomplete || wall_ns == 0, dropped: self.dropped });
        }
    }
    fn frame_wall_ns(&self, _index: usize) -> u64 {
        // Civil timestamps have callback-entry resolution. Frame/rate counts
        // separately describe submitted digital duration; accelerated fixtures
        // and unknown driver scheduling cannot fabricate a per-sample wall time.
        self.callback_wall_ns
    }
    fn flush(&mut self) {
        let observations = self.windows.as_mut().and_then(Windows::finish);
        self.publish(observations);
    }
    fn publish(&mut self, observations: Option<[Option<Observation>; LANES]>) {
        for mut observation in observations.into_iter().flatten().flatten() {
            if self.prepare_state & 1 != 0 && observation.classification == super::Classification::Active
                && observation.frames >= self.rate.div_ceil(100) && observation.episode.deck < 2
                && self.window_playing.is_some_and(|playing| playing[usize::from(observation.episode.deck)] == observation.episode.load)
                && self.window_wall_ns != 0 {
                self.endpoint.record_play(DigitalPlay { deck: observation.episode.deck, load: observation.episode.load,
                    wall_ns: self.window_wall_ns, first_frame: observation.first_frame, frames: observation.frames, rate: observation.sample_rate });
            }
            if !self.active { continue; }
            observation.session = self.session; observation.wall_ns = self.window_wall_ns;
            if observation.wall_ns == 0 { self.incomplete = true; }
            if self.sender.try_send(observation).is_err() {
                self.incomplete = true; self.dropped = self.dropped.saturating_add(u64::from(observation.frames));
            }
        }
    }
}
