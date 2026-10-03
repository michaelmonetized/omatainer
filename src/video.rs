//! One local picture clip uses video frames for placement and audio samples for transport.
use crate::engine::media_source::FileFingerprint;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub(crate) mod decoder;
pub(crate) mod render;

pub const MAX_FRAMES: u64 = 1_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rate {
    pub numerator: u32,
    pub denominator: u32,
}
impl Rate {
    /// Validate the supported constant frame rates.
    /// Takes the exact fraction; returns its nominal timecode rate or an error.
    pub fn nominal(self) -> Result<u64, String> {
        match (self.numerator,self.denominator) {
            (24,1)|(25,1)|(30,1)|(50,1)|(60,1) => Ok(u64::from(self.numerator)),
            (24000,1001) => Ok(24), (30000,1001) => Ok(30), (60000,1001) => Ok(60),
            _ => Err("Supported constant rates: 24, 25, 30, 50, 60, 24000/1001, 30000/1001 and 60000/1001".into()),
        }
    }
    /// Convert an integer video frame to project seconds.
    /// Takes a frame index; returns an absolute time without an accumulated playback clock.
    pub fn seconds(self, frame: u64) -> f64 {
        frame as f64 * f64::from(self.denominator) / f64::from(self.numerator)
    }
    /// Select the frame containing an audio transport position.
    /// Takes finite nonnegative seconds; returns the containing frame, tolerating only floating-point boundary roundoff.
    pub fn frame(self, seconds: f64) -> u64 {
        (seconds.max(0.0) * f64::from(self.numerator) / f64::from(self.denominator) + 1e-8).floor()
            as u64
    }
    /// Convert a video boundary to the first aligned audio sample.
    /// Takes frame and audio rate; returns a rounded rational sample position without drift.
    pub fn sample(self, frame: u64, sample_rate: u32) -> u64 {
        let n = u128::from(frame) * u128::from(self.denominator) * u128::from(sample_rate);
        ((n + u128::from(self.numerator) / 2) / u128::from(self.numerator)) as u64
    }
    /// Format an absolute frame count as non-drop or supported drop-frame timecode.
    /// Takes signed frame count and drop-frame mode; returns a signed HH:MM:SS:FF or HH:MM:SS;FF string.
    pub fn timecode(self, frame: i64, drop: bool) -> Result<String, String> {
        let nominal = self.nominal()?;
        let mut n = frame.unsigned_abs();
        if drop {
            let skip = match (self.numerator, self.denominator) {
                (30000, 1001) => 2,
                (60000, 1001) => 4,
                _ => return Err("Drop-frame timecode requires 30000/1001 or 60000/1001".into()),
            };
            let ten = nominal * 600 - skip * 9;
            let minute = nominal * 60 - skip;
            let rest = n % ten;
            n += skip * 9 * (n / ten) + skip * (rest.saturating_sub(skip) / minute);
        }
        Ok(format!(
            "{}{:02}:{:02}:{:02}{}{:02}",
            if frame < 0 { "-" } else { "" },
            n / (nominal * 3600),
            (n / (nominal * 60)) % 60,
            (n / nominal) % 60,
            if drop { ';' } else { ':' },
            n % nominal
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub rate: Rate,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub first_pts: f64,
    pub codec: String,
}
impl Info {
    /// Check bounded decoder metadata before allocating or restoring a clip.
    /// Takes this metadata; returns an error for unsupported shape, codec, rate or duration.
    pub fn validate(&self) -> Result<(), String> {
        self.rate.nominal()?;
        if self.frames == 0
            || self.frames > MAX_FRAMES
            || !(1..=4096).contains(&self.width)
            || !(1..=2160).contains(&self.height)
            || !self.first_pts.is_finite()
            || self.first_pts.abs() > 86400.0
            || !matches!(
                self.codec.as_str(),
                "h264" | "hevc" | "prores" | "vp8" | "vp9" | "ffv1" | "mpeg4"
            )
        {
            return Err("Unsupported video: up to one million constant-rate frames, 4096×2160, H.264/HEVC/ProRes/VP8/VP9/FFV1/MPEG-4".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub frame: u64,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub info: Info,
    pub trim_in: u64,
    pub trim_out: u64,
    pub placement: u64,
    pub timecode_offset: i64,
    pub drop_frame: bool,
    pub preview_offset_ms: i16,
    pub locators: Vec<Locator>,
    pub detached: bool,
}
impl Clip {
    /// Validate saved picture placement and locator metadata.
    /// Takes this clip; returns success only for a local source and bounded frame-based edits.
    pub fn validate(&self) -> Result<(), String> {
        self.info.validate()?;
        if !self.path.is_absolute()
            || self.path.as_os_str().len() > 4096
            || self.trim_in >= self.trim_out
            || self.trim_out > self.info.frames
            || self.placement > MAX_FRAMES
            || self.timecode_offset.unsigned_abs() > 10_000_000
            || !(-2000..=2000).contains(&self.preview_offset_ms)
            || self.locators.len() > 128
            || self.locators.iter().any(|l| {
                l.frame > MAX_FRAMES * 2
                    || l.name.is_empty()
                    || l.name.len() > 128
                    || l.name.chars().any(char::is_control)
            })
        {
            return Err("Invalid video path, trim, placement, timecode offset or locator".into());
        }
        self.info
            .rate
            .timecode(self.timecode_offset, self.drop_frame)?;
        Ok(())
    }
    /// Map the current audio position into the trimmed picture.
    /// Takes transport seconds; returns the source frame or black outside the clip.
    pub fn source_frame(&self, seconds: f64) -> Option<u64> {
        let seconds = seconds + f64::from(self.preview_offset_ms) / 1000.0;
        if seconds < 0.0 {
            return None;
        }
        let frame = self.info.rate.frame(seconds);
        frame
            .checked_sub(self.placement)
            .map(|n| n + self.trim_in)
            .filter(|&n| n < self.trim_out)
    }
    /// Find the exclusive end of the picture on the project clock.
    /// Takes this clip; returns a project frame boundary.
    pub fn end(&self) -> u64 {
        self.placement + self.trim_out - self.trim_in
    }
}
#[cfg(test)]
mod tests;
