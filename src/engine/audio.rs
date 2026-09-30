use crate::engine::RtEngine;
use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::Arc;

pub struct AudioOut {
    pub sr: u32,
    pub _stream: cpal::Stream,
}

pub fn start(rt: Arc<Mutex<RtEngine>>) -> anyhow::Result<AudioOut> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no default audio output (PipeWire/ALSA)")?;
    let cfg = device.default_output_config().context("output config")?;
    let sr = cfg.sample_rate().0;
    {
        let mut e = rt.lock();
        e.set_sample_rate(sr);
    }
    let err_fn = |e| eprintln!("omatainer audio: {e}");
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &cfg.into(), rt, err_fn)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg.into(), rt, err_fn)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg.into(), rt, err_fn)?,
        other => anyhow::bail!("unsupported sample format {other}"),
    };
    stream.play()?;
    Ok(AudioOut { sr, _stream: stream })
}

fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    rt: Arc<Mutex<RtEngine>>,
    err_fn: impl Fn(cpal::StreamError) + Send + 'static,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let ch = cfg.channels as usize;
    thread_local! {
        static BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }
    let stream = device.build_output_stream(
        cfg,
        move |data: &mut [T], _| {
            BUF.with(|buf| {
                let mut buf = buf.borrow_mut();
                if buf.len() < data.len() {
                    buf.resize(data.len(), 0.0);
                }
                let slice = &mut buf[..data.len()];
                slice.fill(0.0);
                if let Some(mut e) = rt.try_lock() {
                    e.process_interleaved(slice, ch);
                }
                for (i, s) in slice.iter().enumerate() {
                    data[i] = T::from_sample(*s);
                }
            });
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}
