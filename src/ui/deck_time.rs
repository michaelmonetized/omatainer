//! Deck-local presentation preferences. Project persistence can serialize these
//! independently of audio state; the readout uses one captured deck snapshot.
use crate::engine::DeckSnap;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_WARNING_LEAD_SECONDS: u16 = 300;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TimeMode {
    Elapsed,
    #[default]
    Remaining,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DeckTimeSettings {
    pub mode: TimeMode,
    /// Estimated wall seconds before file end. Zero explicitly disables alerts.
    pub warning_lead_seconds: u16,
}

impl Default for DeckTimeSettings {
    fn default() -> Self {
        Self {
            mode: TimeMode::Remaining,
            warning_lead_seconds: 30,
        }
    }
}

impl DeckTimeSettings {
    pub fn warning_lead(&self) -> u16 {
        self.warning_lead_seconds.min(MAX_WARNING_LEAD_SECONDS)
    }

    pub fn button_label(&self) -> &'static str {
        match self.mode {
            TimeMode::Elapsed => "Elapsed ▾",
            TimeMode::Remaining => "Remain est. ▾",
        }
    }
}

pub(super) struct Readout {
    pub text: String,
    pub status: &'static str,
    pub warning: bool,
    pub tooltip: String,
    #[cfg(test)]
    pub elapsed: Option<f64>,
    #[cfg(test)]
    pub remaining: Option<f64>,
}

impl Readout {
    pub fn from_snapshot(snap: &DeckSnap, settings: DeckTimeSettings) -> Self {
        let valid = snap.source_sample_rate > 0
            && snap.frames.is_finite()
            && snap.frames > 0.0
            && snap.pos.is_finite()
            && snap.pos >= 0.0
            && snap.pos <= snap.frames;
        let elapsed =
            valid.then(|| snap.pos.clamp(0.0, snap.frames) / snap.source_sample_rate as f64);
        // playback_rate is source seconds / output seconds, before the renderer
        // converts it to source frames / output frame. Do not apply that sample
        // rate ratio again here. It includes sync, smoothing and scratch rate.
        let remaining = elapsed.and_then(|elapsed| {
            let rate = f64::from(snap.playback_rate);
            (rate.is_finite() && rate > 0.0)
                .then(|| (snap.frames / snap.source_sample_rate as f64 - elapsed).max(0.0) / rate)
                .filter(|seconds| seconds.is_finite())
        });
        let looping = valid
            && snap.loop_on
            && snap.loop_start.is_finite()
            && snap.loop_len.is_finite()
            && snap.loop_start >= 0.0
            && snap.loop_len > 1.0
            && snap.loop_start + snap.loop_len <= snap.frames
            && snap.pos >= snap.loop_start
            && snap.pos < snap.loop_start + snap.loop_len;
        let warning = snap.playing
            && !snap.touching
            && !looping
            && settings.warning_lead() > 0
            && remaining.is_some_and(|seconds| seconds <= f64::from(settings.warning_lead()));
        let status = if !valid {
            "NO AUDIO"
        } else if snap.touching {
            "SCRATCH"
        } else if !snap.playing {
            "PAUSED"
        } else if looping {
            "LOOP"
        } else if warning {
            "RUNOUT"
        } else if remaining.is_none() {
            "NO ESTIMATE"
        } else {
            ""
        };
        let text = match settings.mode {
            TimeMode::Elapsed => format_time(elapsed, false),
            TimeMode::Remaining => format_time(remaining, true),
        };
        let mut tooltip = match settings.mode {
            TimeMode::Elapsed => "Elapsed: source playhead time, not time spent listening.".to_owned(),
            TimeMode::Remaining => "Remaining: estimated wall time to file end at the captured playback rate; rate changes alter the estimate.".to_owned(),
        };
        if looping {
            tooltip.push_str(" Valid repeating loop: no runout alert; the file-end estimate ignores future loop repeats.");
        } else if snap.touching || remaining.is_none() {
            tooltip.push_str(" Scratching, reverse or stationary playback does not trigger a runout alert; a remaining estimate requires forward motion.");
        } else if !snap.playing {
            tooltip.push_str(" Paused: no runout alert.");
        }
        if settings.warning_lead() == 0 {
            tooltip.push_str(" Runout warnings are off.");
        } else {
            tooltip.push_str(&format!(
                " Runout warning lead: {} s.",
                settings.warning_lead()
            ));
        }
        Self {
            text,
            status,
            warning,
            tooltip,
            #[cfg(test)]
            elapsed,
            #[cfg(test)]
            remaining,
        }
    }
}

fn format_time(seconds: Option<f64>, remaining: bool) -> String {
    let Some(seconds) = seconds.filter(|seconds| seconds.is_finite() && *seconds >= 0.0) else {
        return "—:——".into();
    };
    // Integer tenths prevent rounding 59.99 to an invalid 0:60.0. Values beyond
    // the representable clock stay explicitly unknown rather than saturating.
    if seconds >= u64::MAX as f64 / 10.0 {
        return "—:——".into();
    }
    let tenths = (seconds * 10.0).floor() as u64;
    let prefix = if remaining { "−" } else { "" };
    if tenths >= 36_000 {
        format!(
            "{prefix}{}:{:02}:{:02}.{}",
            tenths / 36_000,
            tenths / 600 % 60,
            tenths / 10 % 60,
            tenths % 10
        )
    } else {
        format!(
            "{prefix}{}:{:02}.{}",
            tenths / 600,
            tenths / 10 % 60,
            tenths % 10
        )
    }
}
