//! Worker-built setlists retain measurement and never replace an existing file.
use super::{Session, Source};
use crate::{engine::media_source::LibSource, library::{Catalog, Track}};
use std::{fmt::Write, os::unix::{ffi::OsStrExt, fs::OpenOptionsExt}, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Format { #[default] Json, Text, Csv, M3u8 }
impl Format {
    pub const ALL: [Self; 4] = [Self::Json, Self::Text, Self::Csv, Self::M3u8];
    pub fn name(self) -> &'static str { match self { Self::Json => "JSON", Self::Text => "Text", Self::Csv => "CSV", Self::M3u8 => "M3U8 playlist" } }
}

/// Encode an absolute local filename for a playlist.
/// Takes a POSIX path; returns a percent-encoded file URI without query or fragment.
fn file_uri(path: &Path) -> Result<String, String> {
    if !path.is_absolute() { return Err("playlist media needs an absolute path".into()); }
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) { uri.push(char::from(byte)); }
        else { write!(uri, "%{byte:02X}").unwrap(); }
    }
    Ok(uri)
}
fn csv_cell(value: &str) -> String {
    let prefix = if value.trim_start().starts_with(['=', '+', '-', '@']) { "'" } else { "" };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
fn state(session: &Session) -> &'static str { match session.state { super::State::Active => "active", super::State::Ended => "ended", super::State::Unclean => "interrupted" } }
fn status(entry: &super::Entry) -> &'static str { match entry.played_override { Some(true) => "manual_played", Some(false) => "manual_unplayed", None if entry.played() => "measured_played", None => "measured_unplayed" } }
fn uncertain(entry: &super::Entry) -> f64 { entry.rates.iter().fold(0.0, |seconds, r| seconds + (r.ambiguous as f64 + r.invalid as f64) / f64::from(r.rate)) }

/// Export the selected history snapshot.
/// Takes format, explicit location consent, catalog snapshot and work permit; returns bounded UTF-8 bytes or refuses unresolved playlist media.
pub(crate) fn encode(session: &Session, format: Format, include_locations: bool, catalog: Option<&Catalog>, permit: &crate::engine::performance::WorkPermit) -> Result<Vec<u8>, String> {
    session.validate()?; check(permit)?;
    if format == Format::Json { return session.export(); }
    let mut output = match format {
        Format::Text => format!("Omatainer setlist v1\nSession: {}\nState: {}\nStarted (Unix ns): {}\nEnded (Unix ns): {}\nIncomplete: {}\nDropped observation frames: {}\nOrder is history insertion order. Manual marks do not change measured duration.\n\n", session.id, state(session), session.started_ns, session.ended_ns.map(|v|v.to_string()).unwrap_or_else(||"pending".into()), session.incomplete, session.dropped_observation_frames),
        Format::Csv => "session_id,state,started_ns,ended_ns,incomplete,dropped_observation_frames,order,entry_id,source,track_id,version,deck,title,artist,status,measured_seconds,uncertain_seconds,first_active_ns,last_active_ns\r\n".into(),
        Format::M3u8 if !include_locations => return Err("M3U8 includes media locations; explicitly allow file locations before export".into()),
        Format::M3u8 => format!("#EXTM3U\n# Omatainer setlist v1; session {}; {}; incomplete {}\n# Played entries only, in history insertion order; manual marks are assertions.\n", session.id, state(session), session.incomplete),
        Format::Json => unreachable!(),
    };
    let mut tracks = std::collections::HashMap::new();
    if format == Format::M3u8 {
        let wanted: std::collections::HashSet<_> = session.entries.iter().filter(|entry|entry.played()).filter_map(|entry| match &entry.source { Source::Catalog {track_id,..}=>Some(track_id), _=>None }).collect();
        for (index, track) in catalog.ok_or("playlist catalog snapshot unavailable")?.tracks.iter().enumerate() {
            if index % 256 == 0 {check(permit)?;}
            if wanted.contains(&track.id.0) {tracks.insert(track.id.0.as_str(),track);}
        }
    }
    let mut references = 0;
    for (order, entry) in session.entries.iter().enumerate() {
        check(permit)?;
        let (origin, id, version) = match &entry.source {
            Source::Catalog { track_id, version, .. } => ("catalog", track_id.as_str(), version.to_string()),
            Source::External { .. } => ("external", "", String::new()), Source::Unresolved => ("unresolved", "", String::new()),
        };
        let deck = entry.deck.map(|deck| char::from(b'A'+deck).to_string()).unwrap_or_default();
        match format {
            Format::Text => {
                writeln!(output, "{}. {} — {}\n   {} · deck {} · {:.3} s measured · {:.3} s uncertain · catalog {} version {}", order+1, entry.source.title(), entry.source.artist(), status(entry), if deck.is_empty(){"none"}else{&deck}, entry.measured_seconds(), uncertain(entry), if id.is_empty(){origin}else{id}, if version.is_empty(){"none"}else{&version}).unwrap();
            },
            Format::Csv => {
                let row = [session.id.clone(), state(session).into(), session.started_ns.to_string(), session.ended_ns.map(|v|v.to_string()).unwrap_or_default(), session.incomplete.to_string(), session.dropped_observation_frames.to_string(), (order+1).to_string(), entry.id.to_string(), origin.into(), id.into(), version, deck, entry.source.title().into(), entry.source.artist().into(), status(entry).into(), format!("{:.6}", entry.measured_seconds()), format!("{:.6}", uncertain(entry)), entry.first_active_ns.map(|v|v.to_string()).unwrap_or_default(), entry.last_active_ns.map(|v|v.to_string()).unwrap_or_default()];
                output.push_str(&row.iter().map(|v|csv_cell(v)).collect::<Vec<_>>().join(",")); output.push_str("\r\n");
            },
            Format::M3u8 if entry.played() => {
                let reference = playlist_reference(&entry.source, tracks.get(id).copied(), permit)
                    .map_err(|error| format!("Playlist entry {} unavailable: {error}. No file was published.", order+1))?;
                writeln!(output, "#EXTINF:-1,{} — {}\n{}", entry.source.artist(), entry.source.title(), reference).unwrap(); references += 1;
            },
            _ => {},
        }
        if output.len() as u64 > super::storage::MAX_SESSION_BYTES { return Err("setlist export exceeds 16 MiB".into()); }
    }
    if format == Format::M3u8 && references == 0 { return Err("The session has no played entries to export as a playlist".into()); }
    Ok(output.into_bytes())
}
fn playlist_reference(source: &Source, track: Option<&Track>, permit: &crate::engine::performance::WorkPermit) -> Result<String, String> {
    let Source::Catalog { version, .. } = source else { return Err("external or unresolved entries have no verified local file; use text, CSV or JSON".into()); };
    let track = track.ok_or("catalog track is unavailable")?;
    let version = track.versions.get(*version as usize).ok_or("catalog version is unavailable")?;
    let fingerprint = version.fingerprint.ok_or("this source has no verified local file")?;
    let candidates = std::iter::once(&track.source).chain(track.previous_locations.iter().filter(|old|old.fingerprint==fingerprint).map(|old|&old.source));
    for source in candidates {
        check(permit)?;
        if !matches!(source, LibSource::File(_) | LibSource::Removable { .. }) { continue; }
        let Ok(location) = crate::media_location::Location::resolve(source) else { continue; };
        let Ok(file) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&location.path) else { continue; };
        if location.verify_file(&file, fingerprint).is_ok() { return file_uri(&location.path); }
    }
    Err("the exact recorded version is offline or changed; replacement audio was not substituted".into())
}

#[cfg(test)]
mod tests;

fn check(permit: &crate::engine::performance::WorkPermit) -> Result<(), String> { if permit.cancelled() { Err("Setlist export cancelled by performance protection".into()) } else { Ok(()) } }
