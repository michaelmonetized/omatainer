//! Explicit temporary physical loopback streams. No input is monitored to output.
use super::*;
use crate::engine::audio::config;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::{bounded, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

struct Returned<T> {
    value: Option<T>,
    sender: Sender<T>,
}
impl<T> std::ops::Deref for Returned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.as_ref().unwrap()
    }
}
impl<T> std::ops::DerefMut for Returned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().unwrap()
    }
}
impl<T> Drop for Returned<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            if let Err(error) = self.sender.try_send(value) {
                std::mem::forget(error.into_inner());
            }
        }
    }
}
struct Output {
    probe: Arc<Probe>,
    stamps: Vec<Stamp>,
    frames: usize,
    invalid: bool,
}
impl Output {
    fn new(probe: Arc<Probe>) -> Self {
        Self {
            probe,
            stamps: Vec::with_capacity(MAX_STAMPS),
            frames: 0,
            invalid: false,
        }
    }
    fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        data: &mut [T],
        channels: usize,
        channel: usize,
        ns: u64,
    ) {
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        if channels == 0 || channel >= channels || data.len() % channels != 0 {
            self.invalid = true;
            return;
        }
        if self.frames >= self.probe.samples.len() {
            return;
        }
        if self.stamps.len() == self.stamps.capacity() {
            self.invalid = true;
            return;
        }
        let count = (data.len() / channels).min(self.probe.samples.len() - self.frames);
        self.stamps.push(Stamp {
            first: self.frames,
            frames: count,
            callback_ns: ns,
        });
        for (frame, value) in data
            .chunks_exact_mut(channels)
            .zip(&self.probe.samples[self.frames..self.frames + count])
        {
            frame[channel] = T::from_sample(*value);
        }
        self.frames += count;
    }
}
fn input<T>(
    device: &cpal::Device,
    plan: &config::Plan,
    mut capture: Returned<Capture>,
    channel: usize,
    origin: Instant,
    full: Arc<AtomicBool>,
    fault: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = plan.channels as usize;
    device.build_input_stream(
        plan.config(),
        move |data: &[T], _| {
            capture.push(
                data,
                channels,
                channel,
                origin.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                |sample| sample.to_sample::<f32>(),
            );
            if capture.full() {
                full.store(true, Ordering::Release);
            }
        },
        move |error| {
            if error.kind() != cpal::ErrorKind::RealtimeDenied {
                fault.store(true, Ordering::Release);
            }
        },
        None,
    )
}
fn output<T>(
    device: &cpal::Device,
    plan: &config::Plan,
    mut output: Returned<Output>,
    channel: usize,
    origin: Instant,
    enabled: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    full: Arc<AtomicBool>,
    fault: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = plan.channels as usize;
    device.build_output_stream(
        plan.config(),
        move |data: &mut [T], _| {
            if !enabled.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
                for sample in data {
                    *sample = T::from_sample(0.0);
                }
                return;
            }
            output.render(
                data,
                channels,
                channel,
                origin.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            );
            if output.frames == output.probe.samples.len() {
                full.store(true, Ordering::Release);
            }
        },
        move |error| {
            if error.kind() != cpal::ErrorKind::RealtimeDenied {
                fault.store(true, Ordering::Release);
            }
        },
        None,
    )
}
macro_rules! formats {($format:expr,$call:ident,$($argument:expr),*)=>{match $format {
    cpal::SampleFormat::F32=>$call::<f32>($($argument),*),cpal::SampleFormat::F64=>$call::<f64>($($argument),*),
    cpal::SampleFormat::I8=>$call::<i8>($($argument),*),cpal::SampleFormat::I16=>$call::<i16>($($argument),*),
    cpal::SampleFormat::I32=>$call::<i32>($($argument),*),cpal::SampleFormat::I64=>$call::<i64>($($argument),*),
    cpal::SampleFormat::U8=>$call::<u8>($($argument),*),cpal::SampleFormat::U16=>$call::<u16>($($argument),*),
    cpal::SampleFormat::U32=>$call::<u32>($($argument),*),cpal::SampleFormat::U64=>$call::<u64>($($argument),*),
    _=>Err(cpal::ErrorKind::UnsupportedConfig.into()),
}}}
/// Explicit confirmed action only. The caller always restores session output.
pub(crate) fn run(
    request: &Request,
    cancel: &AtomicBool,
    stopped: Arc<AtomicBool>,
) -> Result<Measurement, String> {
    request.validate()?;
    let input_device = config::select_input_exact(&request.input).map_err(|e| e.to_string())?;
    let output_device = config::select_exact(&request.output).map_err(|e| e.to_string())?;
    let probe = Arc::new(Probe::new(request.output.rate, request.level_db)?);
    let (input_return, input_data) = bounded(1);
    let (output_return, output_data) = bounded(1);
    let capture = Returned {
        value: Some(Capture::new(request.input.rate)),
        sender: input_return,
    };
    let output_state = Returned {
        value: Some(Output::new(probe.clone())),
        sender: output_return,
    };
    let fault = Arc::new(AtomicBool::new(false));
    let input_full = Arc::new(AtomicBool::new(false));
    let output_full = Arc::new(AtomicBool::new(false));
    let enabled = Arc::new(AtomicBool::new(false));
    let origin = Instant::now();
    let input_stream = formats!(
        request.input.format,
        input,
        &input_device,
        &request.input,
        capture,
        request.input_channel as usize,
        origin,
        input_full.clone(),
        fault.clone()
    )
    .map_err(|e| format!("Calibration input: {e}"))?;
    let output_stream = formats!(
        request.output.format,
        output,
        &output_device,
        &request.output,
        output_state,
        request.output_channel as usize,
        origin,
        enabled.clone(),
        stopped.clone(),
        output_full.clone(),
        fault.clone()
    )
    .map_err(|e| format!("Calibration output: {e}"))?;
    let played = input_stream.play().and_then(|_| output_stream.play());
    let result = if let Err(error) = played {
        Err(format!("Calibration stream failed: {error}"))
    } else {
        if !cancel.load(Ordering::Acquire) && !stopped.load(Ordering::Acquire) {
            enabled.store(true, Ordering::Release);
        }
        wait_capture(
            cancel,
            &stopped,
            &fault,
            &input_full,
            &output_full,
            origin,
            Duration::from_secs(5),
        )
    };
    enabled.store(false, Ordering::Release);
    drop(output_stream);
    drop(input_stream); // verified ALSA callback joins
    let capture = input_data
        .recv()
        .map_err(|_| "Calibration input ownership did not return")?;
    let output = output_data
        .recv()
        .map_err(|_| "Calibration output ownership did not return")?;
    result?;
    if output.invalid || cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
        return Err("Calibration cancelled or output stamps overflowed; no measurement".into());
    }
    let result = analyze(&probe, &capture, &output.stamps)?;
    if cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
        return Err("Calibration cancelled; no measurement retained".into());
    }
    Ok(result)
}

fn wait_capture(
    cancel: &AtomicBool,
    stopped: &AtomicBool,
    fault: &AtomicBool,
    input_full: &AtomicBool,
    output_full: &AtomicBool,
    origin: Instant,
    limit: Duration,
) -> Result<(), String> {
    loop {
        if cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
            return Err("Calibration cancelled; no measurement retained".into());
        }
        if fault.load(Ordering::Acquire) {
            return Err("Calibration backend error; no measurement".into());
        }
        if input_full.load(Ordering::Acquire) && output_full.load(Ordering::Acquire) {
            return Ok(());
        }
        if origin.elapsed() >= limit {
            return Err("Calibration timed out; insufficient callback evidence".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_capture_wait_has_finite_timeout_and_cancellation_error_precedence() {
        let cancel = AtomicBool::new(false);
        let stopped = AtomicBool::new(false);
        let fault = AtomicBool::new(false);
        let input = AtomicBool::new(false);
        let output = AtomicBool::new(true);
        let start = Instant::now();
        assert!(wait_capture(
            &cancel,
            &stopped,
            &fault,
            &input,
            &output,
            start,
            Duration::from_millis(5)
        )
        .unwrap_err()
        .contains("timed out"));
        assert!(start.elapsed() < Duration::from_millis(200));
        input.store(true, Ordering::Release);
        assert!(wait_capture(
            &cancel,
            &stopped,
            &fault,
            &input,
            &output,
            Instant::now(),
            Duration::ZERO
        )
        .is_ok());
        cancel.store(true, Ordering::Release);
        assert!(wait_capture(
            &cancel,
            &stopped,
            &fault,
            &input,
            &output,
            Instant::now(),
            Duration::ZERO
        )
        .unwrap_err()
        .contains("cancelled"));
        cancel.store(false, Ordering::Release);
        stopped.store(true, Ordering::Release);
        assert!(wait_capture(
            &cancel,
            &stopped,
            &fault,
            &input,
            &output,
            Instant::now(),
            Duration::ZERO
        )
        .unwrap_err()
        .contains("cancelled"));
        stopped.store(false, Ordering::Release);
        fault.store(true, Ordering::Release);
        assert!(wait_capture(
            &cancel,
            &stopped,
            &fault,
            &input,
            &output,
            Instant::now(),
            Duration::ZERO
        )
        .unwrap_err()
        .contains("backend error"));
    }
    #[test]
    fn real_probe_output_routes_only_selected_channel_and_has_no_callback_heap_work() {
        let probe = Arc::new(Probe::new(48000, -40.0).unwrap());
        let mut output = Output::new(probe.clone());
        let mut frame = [9.0_f32; 512];
        for i in 0..(probe.samples.len() / 128) {
            assert_eq!(
                crate::engine::test_alloc::measure(|| output.render(&mut frame, 4, 2, i as u64)),
                crate::engine::test_alloc::Counts::default()
            );
            for (n, values) in frame.chunks_exact(4).enumerate() {
                assert_eq!(values[0], 0.0);
                assert_eq!(values[1], 0.0);
                assert_eq!(values[3], 0.0);
                assert_eq!(values[2], probe.samples[i * 128 + n]);
            }
        }
        assert!(!output.invalid);
        assert_eq!(output.frames, probe.samples.len());
        output.render(&mut frame, 4, 2, 99);
        assert!(frame.iter().all(|s| *s == 0.0));
    }
}
