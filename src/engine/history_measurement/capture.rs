//! Only final OutputCallback conversion promotes samples into observations.
use super::{tracker::{Contribution, Tracker}, Episode, Frame, Observation, Windows, LANES};
use crate::engine::{performance, FxKind};
use crossbeam_channel::{Receiver, Sender};

const MAX_FRAMES: usize = 16_384;
const EVENTS: usize = 4_096;

#[derive(Clone, Copy)]
struct Rendered {
    episodes: [Option<Episode>; LANES],
    actual: [f32; 2],
    without: [[f32; 2]; LANES],
    error: [[f64; 2]; LANES],
}
impl Default for Rendered {
    fn default() -> Self { Self { episodes: [None; LANES], actual: [0.0; 2],
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
    clock: u64,
    windows: Option<Windows>,
    window_episodes: [Option<Episode>; LANES],
    sender: Sender<Observation>,
    receiver: Option<Receiver<Observation>>,
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
        Ok(Self { tracker, available: true, rate, sidecar, capture: false, frame_count: 0, active: false,
            clock: 0, windows: None, window_episodes: [None; LANES], sender,
            receiver: Some(receiver), incomplete: false, dropped: 0 })
    }
    pub fn take_receiver(&mut self) -> Option<Receiver<Observation>> { self.receiver.take() }
    /// These are renderer-owned boundary operations; the session service must
    /// acknowledge them through its request mailbox before reporting a start/end.
    pub fn start(&mut self) -> bool {
        if self.active { return false; }
        self.active = true; self.windows = None; self.window_episodes = [None; LANES];
        self.incomplete = self.tracker.incomplete; self.dropped = 0;
        true
    }
    pub fn end(&mut self) -> bool {
        if !self.active { return false; }
        self.flush(); self.windows = None; self.active = false;
        true
    }
    pub fn configure(&mut self, kinds: [FxKind; 3], wet: [f32; 3], spb: f64) {
        self.tracker.configure(kinds, wet, spb);
    }
    pub fn reset_dsp(&mut self) { self.tracker.reset(); }
    /// Off-callback rate preparation preserves the observation receiver and
    /// closes the previous rational-rate segment before replacing DSP storage.
    pub fn set_rate(&mut self, rate: u32) {
        self.flush(); self.windows = None; self.rate = rate;
        match Tracker::new(rate) {
            Ok(tracker) => { self.tracker = tracker; self.available = true; }
            Err(_) => { self.available = false; self.incomplete = true; }
        }
    }
    pub fn begin_output(&mut self, frames: usize) {
        self.capture = true;
        self.frame_count = frames;
        if self.active && frames > MAX_FRAMES {
            self.flush(); self.windows = None;
            self.incomplete = true;
            self.dropped = self.dropped.saturating_add(frames as u64);
        }
    }
    pub fn record_rendered(&mut self, index: usize, contribution: Contribution,
        before_limiter: [f32; 2], master: f32, safety: &performance::Output,
    ) {
        if !self.capture || !self.active || !self.available || self.frame_count > MAX_FRAMES { return; }
        if index >= self.frame_count { self.incomplete = true; return; }
        self.incomplete |= contribution.incomplete;
        let actual = safety.preview(before_limiter.map(|v| (v * master).tanh()));
        let mut record = Rendered { episodes: contribution.episodes, actual, ..Default::default() };
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
        if self.active && self.available && frames <= MAX_FRAMES {
            for i in 0..frames {
                let record = self.sidecar[i];
                if self.windows.is_none() || record.episodes != self.window_episodes {
                    self.flush();
                    self.window_episodes = record.episodes;
                    self.windows = self.clock.checked_add(i as u64)
                        .and_then(|first| Windows::new(self.rate, first, record.episodes));
                    if self.windows.is_none() { self.incomplete = true; }
                }
                let map = |pair: [f32; 2]| if channels == 1 { [0.5 * (pair[0] + pair[1]), 0.0] } else { pair };
                let analog = map(record.actual).map(f64::from);
                let digital = [cpal::Sample::to_sample::<f64>(output[i * channels]),
                    if channels == 1 { 0.0 } else { cpal::Sample::to_sample::<f64>(output[i * channels + 1]) }];
                let mapped_without = record.without.map(map);
                let frame = Frame { analog, digital,
                    without_analog: mapped_without.map(|p| p.map(f64::from)),
                    without_digital: mapped_without.map(|p| p.map(|v| cpal::Sample::to_sample::<f64>(T::from_sample(v)))),
                    error: record.error.map(|pair| if channels == 1 {
                        [0.5 * (pair[0] + pair[1]) + (analog[0].abs() + 1.0) * f64::from(f32::EPSILON) * 2.0, 0.0]
                    } else { pair }),
                };
                let observations = self.windows.as_mut().and_then(|w| w.push(&frame));
                self.publish(observations);
            }
        }
        if let Some(clock) = self.clock.checked_add(frames as u64) { self.clock = clock; }
        else { self.incomplete = true; self.active = false; self.flush(); }
    }
    fn flush(&mut self) {
        let observations = self.windows.as_mut().and_then(Windows::finish);
        self.publish(observations);
    }
    fn publish(&mut self, observations: Option<[Option<Observation>; LANES]>) {
        for observation in observations.into_iter().flatten().flatten() {
            if self.sender.try_send(observation).is_err() {
                self.incomplete = true; self.dropped = self.dropped.saturating_add(u64::from(observation.frames));
            }
        }
    }
}
