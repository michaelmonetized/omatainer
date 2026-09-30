use crate::engine::RtEngine;
use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct AudioOut {
    pub sr: u32,
    pub _stream: cpal::Stream,
}

pub fn start(mut rt: RtEngine) -> anyhow::Result<AudioOut> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no default audio output (PipeWire/ALSA)")?;
    let cfg = device.default_output_config().context("output config")?;
    let sr = cfg.sample_rate().0;
    rt.set_sample_rate(sr);
    let err_fn = |e| eprintln!("omatainer audio: {e}");
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &cfg.into(), rt, err_fn)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg.into(), rt, err_fn)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg.into(), rt, err_fn)?,
        other => anyhow::bail!("unsupported sample format {other}"),
    };
    stream.play()?;
    Ok(AudioOut {
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
        move |data: &mut [T], _| callback.render(data),
        err_fn,
        None,
    )?;
    Ok(stream)
}

/// The CPAL closure owns this state. Producers can submit commands and read
/// snapshots, but cannot lock, inspect or mutate the renderer between blocks.
pub(super) struct OutputCallback {
    rt: RtEngine,
    channels: usize,
    buffer: Vec<f32>,
}

impl OutputCallback {
    pub(super) fn new(rt: RtEngine, channels: usize) -> Self {
        Self {
            rt,
            channels,
            buffer: Vec::new(),
        }
    }

    pub(super) fn render<T>(&mut self, data: &mut [T])
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        if self.buffer.len() < data.len() {
            self.buffer.resize(data.len(), 0.0);
        }
        let slice = &mut self.buffer[..data.len()];
        slice.fill(0.0);
        self.rt.process_interleaved(slice, self.channels);
        for (destination, source) in data.iter_mut().zip(slice) {
            *destination = T::from_sample(*source);
        }
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
