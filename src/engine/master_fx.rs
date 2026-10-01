//! Three legacy controller slots with independent stereo histories. All storage
//! is prepared before streaming; changing type resets the selected history in
//! constant time rather than reallocating or clearing delay buffers.
use super::{
    dsp::{Delay, OnePole, Reverb},
    FxKind,
};

pub(super) struct MasterSlot {
    pub echo: [Delay; 2],
    pub reverb: [Reverb; 2],
    filter: [[OnePole; 2]; 2],
}

impl MasterSlot {
    pub fn new(sr: f32) -> Self {
        Self {
            echo: std::array::from_fn(|_| Delay::new((sr * 2.0) as usize)),
            reverb: std::array::from_fn(|_| Reverb::at_sample_rate(sr)),
            filter: [[OnePole::lpf(sr, 1000.0); 2]; 2],
        }
    }
    pub fn configure(&mut self, wet: f32, samples_per_beat: f64) {
        for channel in 0..2 {
            self.echo[channel].time_samples = (samples_per_beat * 0.75) as f32;
            self.echo[channel].mix = wet;
            self.reverb[channel].mix = wet;
        }
    }
    pub fn reset(&mut self, kind: FxKind) {
        for channel in 0..2 {
            match kind {
                FxKind::Echo => self.echo[channel].reset_history(),
                FxKind::Reverb => self.reverb[channel].reset_history(),
                FxKind::Filter => {
                    for pole in &mut self.filter[channel] {
                        pole.z = 0.0;
                    }
                }
            }
        }
    }
    pub fn process(&mut self, input: [f32; 2], kind: FxKind, wet: f32) -> [f32; 2] {
        std::array::from_fn(|channel| match kind {
            FxKind::Echo => self.echo[channel].tick(input[channel]),
            FxKind::Reverb => self.reverb[channel].tick(input[channel]),
            FxKind::Filter => {
                let [first, second] = &mut self.filter[channel];
                let filtered = second.tick(first.tick(input[channel]));
                input[channel] * (1.0 - wet) + filtered * wet
            }
        })
    }
}
