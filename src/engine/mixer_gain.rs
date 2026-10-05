//! Exact steady mixer curves with bounded, allocation-free gain-space ramps.
const TRANSITION_SECONDS: f32 = 0.005;

#[derive(Clone, Debug, Default)]
pub(super) struct GainPair {
    key: Option<[u32; 2]>,
    current: [f32; 2],
    target: [f32; 2],
    step: [f32; 2],
    remaining: u32,
    #[cfg(test)]
    pub calculations: usize,
}

impl GainPair {
    pub fn prepare(&mut self, controls: [f32; 2], sr: f32, curve: fn(f32, f32) -> [f32; 2]) {
        let key = controls.map(f32::to_bits);
        if self.key == Some(key) {
            return;
        }
        let target = curve(controls[0], controls[1]);
        #[cfg(test)]
        {
            self.calculations += 1;
        }
        if self.key.is_none() {
            self.current = target;
            self.target = target;
            self.remaining = 0;
        } else if self.target != target {
            self.target = target;
            self.remaining = (sr * TRANSITION_SECONDS).round().max(1.0) as u32;
            self.step =
                std::array::from_fn(|i| (target[i] - self.current[i]) / self.remaining as f32);
        }
        self.key = Some(key);
    }

    #[inline]
    pub fn tick(&mut self) -> [f32; 2] {
        if self.remaining != 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                // The exact target prevents accumulated ramp error and makes
                // zero/end stops exact after the finite transition.
                self.current = self.target;
            } else {
                for (gain, step) in self.current.iter_mut().zip(self.step) {
                    *gain += step;
                }
            }
        }
        self.current
    }
}

pub(super) fn pan_gains(gain: f32, pan: f32) -> [f32; 2] {
    [
        (1.0 - pan.max(0.0)).sqrt() * gain,
        (1.0 + pan.min(0.0)).sqrt() * gain,
    ]
}
pub(super) fn crossfader_gains(position: f32, curve: f32) -> [f32; 2] {
    let (a, b) = super::xfader_gains(position, curve);
    [a, b]
}

impl super::RtEngine {
    pub(super) fn prepare_mixer_gains(&mut self) {
        self.monitor.prepare(self.sr);
        self.surface.prepare(self.sr, self.crossfader_position(), self.xfader_curve);
        // The reference path models the previous loop without preparing a
        // cache. Production has only the cached path.
        #[cfg(test)]
        if self.legacy_gain_math {
            return;
        }
        self.xfader_gain
            .prepare([self.crossfader_position(), self.xfader_curve], self.sr, crossfader_gains);
        for track in &mut self.tracks {
            track
                .mixer_gain
                .prepare([track.gain, track.pan], self.sr, pan_gains);
        }
    }
}
