//! Fixed-clock last-play presentation. Only visible cached cells need formatting.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) struct Formatted {
    pub label: String,
    pub tooltip: String,
    pub next_change: Option<SystemTime>,
}

pub(super) fn format(played: Option<SystemTime>, now: SystemTime) -> Formatted {
    let Some(played) = played else {
        return Formatted {
            label: "—".into(),
            tooltip: "No recorded playback time".into(),
            next_change: None,
        };
    };
    let stamp = utc_timestamp(played);
    let mut tooltip = format!("Played: {}", stamp.precise);
    let Ok(age) = now.duration_since(played) else {
        tooltip.push_str("\nFuture timestamp relative to this computer's clock");
        return Formatted {
            label: "Future time".into(),
            tooltip,
            next_change: Some(played),
        };
    };
    let seconds = age.as_secs();
    let (label, next_seconds) = if seconds < 60 {
        ("Just now".into(), Some(60))
    } else if seconds < 3600 {
        (
            format!("{} min ago", seconds / 60),
            Some((seconds / 60 + 1) * 60),
        )
    } else if seconds < 86400 {
        (
            format!("{} h ago", seconds / 3600),
            Some((seconds / 3600 + 1) * 3600),
        )
    } else if seconds < 7 * 86400 {
        (
            format!("{} d ago", seconds / 86400),
            Some((seconds / 86400 + 1) * 86400),
        )
    } else {
        (
            stamp
                .date
                .map(|date| format!("{date} UTC"))
                .unwrap_or_else(|| "Long ago".into()),
            None,
        )
    };
    Formatted {
        label,
        tooltip,
        next_change: next_seconds
            .and_then(|seconds| played.checked_add(Duration::from_secs(seconds))),
    }
}

struct Timestamp {
    date: Option<String>,
    precise: String,
}
fn utc_timestamp(time: SystemTime) -> Timestamp {
    // Floor negative fractional seconds so one nanosecond before the epoch is
    // 1969-12-31 23:59:59.999999999, never 1970 with a negative fraction.
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(value) => value.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    let seconds = nanos.div_euclid(1_000_000_000);
    let fraction = nanos.rem_euclid(1_000_000_000);
    if let Ok(seconds) = libc::time_t::try_from(seconds) {
        let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
        // gmtime_r writes caller-owned storage, uses UTC, and does not mutate
        // process timezone/locale. A null return denotes an unsupported range.
        let result = unsafe { libc::gmtime_r(&seconds, calendar.as_mut_ptr()) };
        if !result.is_null() {
            let calendar = unsafe { calendar.assume_init() };
            let year = calendar.tm_year as i64 + 1900;
            let year = if (0..=9999).contains(&year) {
                format!("{year:04}")
            } else {
                format!("{year:+07}")
            };
            let date = format!("{year}-{:02}-{:02}", calendar.tm_mon + 1, calendar.tm_mday);
            let precise = format!(
                "{date} {:02}:{:02}:{:02}.{fraction:09} UTC",
                calendar.tm_hour, calendar.tm_min, calendar.tm_sec
            );
            return Timestamp {
                date: Some(date),
                precise,
            };
        }
    }
    Timestamp {
        date: None,
        precise: format!("Unix epoch offset {nanos} ns (UTC; outside calendar range)"),
    }
}

#[cfg(test)]
mod tests;
