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
    pub(super) fn try_new(sr: f32) -> Result<Self, std::collections::TryReserveError> {
        Ok(Self {
            echo: [Delay::try_new((sr * 2.0) as usize)?, Delay::try_new((sr * 2.0) as usize)?],
            reverb: [Reverb::try_at_sample_rate(sr)?, Reverb::try_at_sample_rate(sr)?],
            filter: [[OnePole::lpf(sr, 1000.0); 2]; 2],
        })
    }
    pub(super) fn storage_bytes(&self) -> usize {
        self.echo.iter().map(Delay::storage_bytes).sum::<usize>()
            + self.reverb.iter().map(Reverb::storage_bytes).sum::<usize>()
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
    /// Apply a normalized effect parameter while retaining its histories.
    /// Takes output rate and value; sets echo feedback, reverb decay and lowpass cutoff.
    pub fn parameter(&mut self, sr: f32, value: f32) {
        for channel in 0..2 {
            self.echo[channel].fb = value * 0.85;
            self.reverb[channel].decay(value * 0.85);
            let coefficient = OnePole::lpf(sr, 40.0 * 400_f32.powf(value)).a;
            for pole in &mut self.filter[channel] { pole.a = coefficient; }
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

    pub(super) fn process_observed(
        &mut self,
        input: [f32; 2],
        kind: FxKind,
        wet: f32,
        bounds: &mut super::history_measurement::SlotBounds,
    ) -> [f32; 2] {
        self.process_bounded(input, kind, wet, [0.0; 2], bounds)
    }

    pub(super) fn process_bounded(
        &mut self, input: [f32; 2], kind: FxKind, wet: f32,
        input_error: [f64; 2], bounds: &mut super::history_measurement::SlotBounds,
    ) -> [f32; 2] {
        use super::history_measurement::error;
        std::array::from_fn(|channel| match kind {
            FxKind::Echo => {
                let delay = &mut self.echo[channel];
                let feedback = delay.fb;
                let mix = delay.mix;
                delay.tick_traced(input[channel], |delayed, value, wrapped| {
                    bounds.output_error[channel] = bounds.echo_error[channel].tick(
                        bounds.echo[channel].peak(), input[channel], delayed,
                        input_error[channel], feedback, mix, wrapped);
                    bounds.echo[channel].write(value, wrapped);
                })
            }
            FxKind::Reverb => {
                let mut errors = [0.0; 4];
                let feedback = self.reverb[channel].feedback();
                let mut sum = 0.0_f32;
                let mut magnitude = 0.0_f64;
                let output = self.reverb[channel].tick_traced(input[channel], |index, delayed, value, wrapped| {
                    let x = if index % 2 == 0 { input[channel] } else { -input[channel] };
                    errors[index] = bounds.reverb_error[channel][index].tick(
                        bounds.reverb[channel][index].peak(), x, delayed,
                        input_error[channel], feedback[index], 1.0, wrapped);
                    bounds.reverb[channel][index].write(value, wrapped);
                    sum += delayed;
                    magnitude += f64::from(delayed).abs() * 0.25;
                });
                bounds.output_error[channel] = error::mix(input_error[channel],
                    error::sum_four(errors, magnitude), input[channel], sum * 0.25, self.reverb[channel].mix);
                output
            }
            FxKind::Filter => {
                let [first, second] = &mut self.filter[channel];
                let first_error = error::pole(input_error[channel], bounds.filter_error[channel][0],
                    input[channel], first.z, first.a);
                let first_output = first.tick(input[channel]);
                let second_error = error::pole(first_error, bounds.filter_error[channel][1],
                    first_output, second.z, second.a);
                let filtered = second.tick(first_output);
                bounds.filter[channel] = [first.z, second.z];
                bounds.filter_error[channel] = [first_error, second_error];
                bounds.output_error[channel] = error::mix(input_error[channel], second_error,
                    input[channel], filtered, wet);
                input[channel] * (1.0 - wet) + filtered * wet
            }
        })
    }
}
