//! A bounded linear crossfade between a slot's processed and dry signal.
//! Initial state is immediate; only changes after the first sample fade.
#[derive(Clone, Debug, Default)]
pub(super) struct Bypass {
    initialized: bool,
    target: bool,
    from: f32,
    level: f32,
    remaining: u32,
    frames: u32,
}

impl Bypass {
    pub fn next(&mut self, enabled: bool, sample_rate: f32) -> f32 {
        let target = f32::from(enabled);
        if !self.initialized {
            self.initialized = true;
            self.target = enabled;
            self.level = target;
            return target;
        }
        if enabled != self.target {
            self.target = enabled;
            self.from = self.level;
            self.frames = (sample_rate * 0.005).ceil().max(1.0) as u32;
            self.remaining = self.frames;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            self.level = if self.remaining == 0 {
                target
            } else {
                let progress = 1.0 - self.remaining as f32 / self.frames as f32;
                self.from + (target - self.from) * progress
            };
        }
        self.level
    }
}
