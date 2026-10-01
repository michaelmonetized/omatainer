//! Device discovery and configuration planning stay outside the render callback.
use super::*;
use crate::preferences::Audio;

#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub format: cpal::SampleFormat,
    pub buffer: Option<(u32, u32)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub default: bool,
    pub defaults: Option<(u16, u32, cpal::SampleFormat)>,
    pub ranges: Vec<Range>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub backend: String,
    pub devices: Vec<Device>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub backend: String,
    pub device: String,
    pub channels: u16,
    pub rate: u32,
    pub format: cpal::SampleFormat,
    pub buffer: Option<u32>,
    pub warning: Option<String>,
}
impl Plan {
    pub fn route(&self) -> String {
        if self.channels == 1 {
            "Main L+R summed to output 1".into()
        } else if self.channels == 2 {
            "Main left/right to outputs 1/2".into()
        } else {
            format!(
                "Main left/right to outputs 1/2; outputs 3–{} silent",
                self.channels
            )
        }
    }
    pub fn config(&self) -> cpal::StreamConfig {
        cpal::StreamConfig {
            channels: self.channels,
            sample_rate: cpal::SampleRate(self.rate),
            buffer_size: self
                .buffer
                .map(cpal::BufferSize::Fixed)
                .unwrap_or(cpal::BufferSize::Default),
        }
    }
}
fn supported(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
    )
}
fn inspect(device: &cpal::Device, is_default: bool) -> Device {
    let name = device.name().unwrap_or_else(|_| "Unnamed output".into());
    let defaults = device.default_output_config().ok().map(|config| {
        (
            config.channels(),
            config.sample_rate().0,
            config.sample_format(),
        )
    });
    let (ranges, error) = match device.supported_output_configs() {
        Ok(configs) => (
            configs
                .take(4096)
                .filter(|config| supported(config.sample_format()))
                .map(|config| Range {
                    channels: config.channels(),
                    min_rate: config.min_sample_rate().0,
                    max_rate: config.max_sample_rate().0,
                    format: config.sample_format(),
                    buffer: match config.buffer_size() {
                        cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
                        cpal::SupportedBufferSize::Unknown => None,
                    },
                })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Device {
        name,
        default: is_default,
        defaults,
        ranges,
        error,
    }
}

pub fn discover() -> Result<Inventory, String> {
    let host = cpal::default_host();
    let default = host.default_output_device();
    let mut devices: Vec<_> = host
        .output_devices()
        .map_err(|error| error.to_string())?
        .take(256)
        .map(|device| inspect(&device, false))
        .collect();
    // Use the actual default device object, even when names are duplicated.
    if let Some(default) = default {
        let current = inspect(&default, true);
        if let Some(index) = devices
            .iter()
            .position(|device| device.name == current.name)
        {
            devices[index] = current;
        } else {
            devices.push(current);
        }
    }
    Ok(Inventory {
        backend: host.id().name().into(),
        devices,
    })
}

pub fn plan(settings: &Audio, inventory: &Inventory) -> Result<Plan, String> {
    let matches: Vec<_> = inventory
        .devices
        .iter()
        .filter(|device| {
            settings
                .device
                .as_ref()
                .map(|name| device.name == *name)
                .unwrap_or(device.default)
        })
        .collect();
    if matches.is_empty() {
        return Err(format!(
            "Audio output {} is unavailable",
            settings.device.as_deref().unwrap_or("System default")
        ));
    }
    if matches.len() != 1 {
        return Err(
            "Audio device name is ambiguous; choose System default or rename duplicate devices"
                .into(),
        );
    }
    let device = matches[0];
    if let Some(error) = &device.error {
        return Err(format!("{}: {error}", device.name));
    }
    let (default_channels, default_rate, default_format) = device
        .defaults
        .ok_or("Device has no default output configuration")?;
    let channels = settings.channels.unwrap_or(default_channels);
    let rate = settings.sample_rate.unwrap_or(default_rate);
    let mut matches: Vec<_> = device
        .ranges
        .iter()
        .filter(|range| {
            range.channels == channels
                && (range.min_rate..=range.max_rate).contains(&rate)
                && settings.buffer_frames.is_none_or(|frames| {
                    range
                        .buffer
                        .is_none_or(|(min, max)| (min..=max).contains(&frames))
                })
        })
        .collect();
    matches.sort_by_key(|range| {
        if range.format == default_format {
            0
        } else {
            match range.format {
                cpal::SampleFormat::F32 => 1,
                cpal::SampleFormat::I16 => 2,
                _ => 3,
            }
        }
    });
    let range = matches.first().ok_or_else(|| {
        format!(
            "{} does not advertise {rate} Hz, {channels} channels and the requested buffer",
            device.name
        )
    })?;
    Ok(Plan {
        backend: inventory.backend.clone(), device: device.name.clone(), channels, rate, format: range.format,
        buffer: settings.buffer_frames,
        warning: (settings.buffer_frames.is_some() && range.buffer.is_none()).then(|| "The device does not advertise buffer limits; opening the stream may still reject this buffer".into()),
    })
}

/// Revalidate immediately before opening; the preview may have become stale.
pub(super) fn select(settings: &Audio) -> anyhow::Result<(cpal::Device, Plan)> {
    let host = cpal::default_host();
    let device = if let Some(name) = &settings.device {
        let mut matches: Vec<_> = host
            .output_devices()?
            .filter(|device| device.name().ok().as_ref() == Some(name))
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "configured audio output {name:?} is missing or ambiguous"
        );
        matches.remove(0)
    } else {
        host.default_output_device()
            .context("no default audio output (PipeWire/ALSA)")?
    };
    let inventory = Inventory {
        backend: host.id().name().into(),
        devices: vec![inspect(&device, true)],
    };
    let plan = plan(settings, &inventory).map_err(anyhow::Error::msg)?;
    Ok((device, plan))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn inventory() -> Inventory {
        Inventory {
            backend: "fixture".into(),
            devices: vec![Device {
                name: "Studio interface".into(),
                default: true,
                defaults: Some((2, 48000, cpal::SampleFormat::F32)),
                error: None,
                ranges: vec![
                    Range {
                        channels: 2,
                        min_rate: 44100,
                        max_rate: 96000,
                        format: cpal::SampleFormat::F32,
                        buffer: Some((64, 1024)),
                    },
                    Range {
                        channels: 4,
                        min_rate: 48000,
                        max_rate: 48000,
                        format: cpal::SampleFormat::I16,
                        buffer: None,
                    },
                ],
            }],
        }
    }
    #[test]
    fn exact_configuration_forwarding_and_routes_never_silently_fall_back() {
        let inventory = inventory();
        let mut settings = Audio::default();
        let default = plan(&settings, &inventory).unwrap();
        assert_eq!(default.rate, 48000);
        assert_eq!(default.channels, 2);
        settings.device = Some("Studio interface".into());
        settings.sample_rate = Some(96000);
        settings.buffer_frames = Some(512);
        let chosen = plan(&settings, &inventory).unwrap();
        let native = chosen.config();
        assert_eq!(native.sample_rate.0, 96000);
        assert_eq!(native.buffer_size, cpal::BufferSize::Fixed(512));
        assert_eq!(native.channels, 2);
        for bad in [32, 2048] {
            settings.buffer_frames = Some(bad);
            assert!(plan(&settings, &inventory).is_err());
        }
        settings.buffer_frames = Some(256);
        settings.sample_rate = Some(192000);
        assert!(plan(&settings, &inventory).is_err());
        settings.sample_rate = Some(48000);
        settings.channels = Some(4);
        let chosen = plan(&settings, &inventory).unwrap();
        assert!(chosen.warning.is_some());
        assert!(chosen.route().contains("3–4 silent"));
        assert_eq!(chosen.format, cpal::SampleFormat::I16);
        settings.device = Some("Missing controller".into());
        assert!(plan(&settings, &inventory)
            .unwrap_err()
            .contains("Missing controller"));
        settings.device = Some("Studio interface".into());
        let mut ambiguous = inventory.clone();
        ambiguous.devices.push(ambiguous.devices[0].clone());
        assert!(plan(&settings, &ambiguous).is_err());
        let mut mono = default;
        mono.channels = 1;
        assert!(mono.route().contains("summed"));
    }
}
