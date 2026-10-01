use crate::engine::RtEngine;
pub mod config;
use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Clone, Debug)]
pub struct OutputInfo {
    pub backend: String,
    pub format: String,
    pub plan: config::Plan,
}

pub struct AudioOut {
    pub info: OutputInfo,
    pub sr: u32,
    pub _stream: cpal::Stream,
}

pub fn start(rt: RtEngine) -> anyhow::Result<AudioOut> {
    start_with_settings(rt, &crate::preferences::Audio::default())
}

pub fn start_with_settings(mut rt: RtEngine, settings: &crate::preferences::Audio) -> anyhow::Result<AudioOut> {
    let (device, plan) = config::select(settings)?;
    let cfg = plan.config();
    let sr = plan.rate;
    let info = OutputInfo {
        backend: plan.backend.clone(),
        format: plan.format.to_string(),
        plan: plan.clone(),
    };
    rt.set_sample_rate(sr);
    let errors = rt.telemetry.clone();
    let err_fn = move |e| errors.error(&e);
    let stream = match plan.format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &cfg, rt, err_fn)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg, rt, err_fn)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg, rt, err_fn)?,
        other => anyhow::bail!("unsupported sample format {other}"),
    };
    stream.play()?;
    Ok(AudioOut {
        info,
        sr,
        _stream: stream,
    })
}

fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    rt: RtEngine,
    err_fn: impl Fn(cpal::StreamError) + Send + 'static,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut callback = OutputCallback::new(rt, cfg.channels as usize);
    let stream = device.build_output_stream(
        cfg,
        move |data: &mut [T], info| {
            let timestamp = info.timestamp();
            callback.render_timed(data, timestamp.playback.duration_since(&timestamp.callback));
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

/// The CPAL closure owns this state. Producers can submit commands and read
/// snapshots, but cannot lock, inspect or mutate the renderer between blocks.
pub(crate) struct OutputCallback {
    rt: RtEngine,
    channels: usize,
    buffer: Vec<f32>,
    #[cfg(test)]
    conversion_delay: std::time::Duration,
}

impl OutputCallback {
    pub(crate) fn new(rt: RtEngine, channels: usize) -> Self {
        Self {
            rt,
            channels,
            buffer: Vec::new(),
            #[cfg(test)]
            conversion_delay: std::time::Duration::ZERO,
        }
    }

    pub(crate) fn render<T>(&mut self, data: &mut [T])
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        self.render_timed(data, None);
    }

    pub(crate) fn render_timed<T>(&mut self, data: &mut [T], latency: Option<std::time::Duration>)
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let started = std::time::Instant::now();
        if self.buffer.len() < data.len() {
            self.buffer.resize(data.len(), 0.0);
        }
        let slice = &mut self.buffer[..data.len()];
        slice.fill(0.0);
        self.rt.process_interleaved(slice, self.channels);
        #[cfg(test)]
        std::thread::sleep(self.conversion_delay);
        for (destination, source) in data.iter_mut().zip(slice) {
            *destination = T::from_sample(*source);
        }
        self.rt.telemetry.record_output(
            started.elapsed(),
            data.len() / self.channels.max(1),
            self.rt.sr as u32,
            self.rt.render_cpu_ns,
            self.channels as u16,
            latency,
        );
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "audio_metrics_tests.rs"]
mod metrics_tests;
