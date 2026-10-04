//! A stopped, explicitly requested tone addresses one physical channel at a time.
use super::model::MAX_PHYSICAL_CHANNELS;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Default)]
pub(crate) struct Probe {
    channel: u32,
    frame: u32,
}
impl Probe {
    /// Generate one bounded channel test.
    /// Takes the atomic request, output rate, channel count and stopped permission; returns a one-second, −40 dBFS tone or silence.
    pub(crate) fn sample(
        &mut self,
        request: &AtomicU32,
        rate: u32,
        channels: usize,
        stopped: bool,
    ) -> Option<(usize, f32)> {
        let channel = request.load(Ordering::Acquire);
        if channel == 0
            || !stopped
            || channel as usize > channels.min(MAX_PHYSICAL_CHANNELS)
            || rate == 0
        {
            self.channel = 0;
            self.frame = 0;
            if channel != 0 {
                request.store(0, Ordering::Release);
            }
            return None;
        }
        if self.channel != channel {
            self.channel = channel;
            self.frame = 0;
        }
        if self.frame >= rate {
            self.channel = 0;
            self.frame = 0;
            request.store(0, Ordering::Release);
            return None;
        }
        let ramp = rate.saturating_mul(5).div_ceil(1000).max(1);
        let gain = (self.frame.min(rate - 1 - self.frame) as f32 / ramp as f32).min(1.0);
        let sample = (std::f64::consts::TAU * 997.0 * f64::from(self.frame) / f64::from(rate)).sin()
            as f32
            * 0.01
            * gain;
        self.frame += 1;
        Some((channel as usize - 1, sample))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_is_bounded_ramped_cancellable_and_refuses_playing_or_absent_channels() {
        let request = AtomicU32::new(64);
        let mut probe = Probe::default();
        let mut peak = 0.0_f32;
        for _ in 0..48000 {
            let (channel, value) = probe.sample(&request, 48000, 64, true).unwrap();
            assert_eq!(channel, 63);
            peak = peak.max(value.abs());
        }
        assert!((peak - 0.01).abs() < 0.000001);
        assert!(probe.sample(&request, 48000, 64, true).is_none());
        request.store(64, Ordering::Release);
        assert!(probe.sample(&request, 48000, 2, true).is_none());
        request.store(1, Ordering::Release);
        assert!(probe.sample(&request, 48000, 64, false).is_none());
        request.store(1, Ordering::Release);
        assert_eq!(probe.sample(&request, 48000, 64, true).unwrap(), (0, 0.0));
        request.store(0, Ordering::Release);
        assert!(probe.sample(&request, 48000, 64, true).is_none());
    }
}
