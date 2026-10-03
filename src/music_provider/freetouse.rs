use super::*;
use serde::Deserialize;
use std::io::Read;
use url::Url;

const API: &str = "https://api.freetouse.com/v3/";
pub(crate) const PAGE_SIZE: u32 = 12;
const JSON_LIMIT: usize = 1024 * 1024;
const AUDIO_LIMIT: usize = 32 * 1024 * 1024;

pub(crate) struct FreeToUse {
    agent: ureq::Agent,
    api: Url,
    #[cfg(test)]
    media_override: Option<Url>,
}
impl Default for FreeToUse {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(12)))
                    .https_only(true)
                    .max_redirects(0)
                    .proxy(None)
                    .http_status_as_error(false)
                    .build(),
            ),
            api: Url::parse(API).unwrap(),
            #[cfg(test)]
            media_override: None,
        }
    }
}

#[derive(Deserialize)]
struct Response<T> {
    ok: bool,
    data: Option<T>,
    pagination: Option<Pagination>,
}
#[derive(Deserialize)]
struct Pagination {
    offset: u32,
    count: u32,
    limit: u32,
}
#[derive(Deserialize)]
struct Artist {
    name: String,
}
#[derive(Deserialize)]
struct Files {
    mp3: String,
}
#[derive(Deserialize)]
struct RemoteTrack {
    id: String,
    title: String,
    artists: Vec<(u32, Artist)>,
    duration: f64,
    is_premium: bool,
    status: u32,
    files: Files,
}
impl RemoteTrack {
    /// Validate the documented catalog record before offering playback.
    /// Takes one API item; returns its stable remote identity and approved media URL.
    fn validated(self) -> Result<(Track, Url), Failure> {
        if self.status != 1
            || !valid_id(&self.id)
            || !valid_text(&self.title)
            || self.artists.is_empty()
            || self.artists.len() > 16
            || self.artists.iter().any(|(_, a)| !valid_text(&a.name))
            || !self.duration.is_finite()
            || self.duration <= 0.0
            || self.duration > 3600.0
        {
            return Err(Failure::InvalidResponse);
        }
        let media = Url::parse(&self.files.mp3).map_err(|_| Failure::InvalidResponse)?;
        if !approved_media(&media) {
            return Err(Failure::InvalidResponse);
        }
        Ok((
            Track {
                id: TrackId {
                    provider: ProviderId::FreeToUse,
                    item: self.id,
                },
                title: self.title,
                artists: self
                    .artists
                    .into_iter()
                    .map(|(_, a)| a.name)
                    .collect::<Vec<_>>()
                    .join(", "),
                seconds: self.duration,
                premium: self.is_premium,
            },
            media,
        ))
    }
}

/// Accept only the provider's observed HTTPS media origin.
/// Takes a parsed URL; returns whether it has no credentials, alternate port or fragment.
fn approved_media(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("eu.data.freetouse.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.as_str().len() <= 4096
}
/// Validate bounded catalog text.
/// Takes one field; rejects empty, oversized or control-bearing values.
fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
/// Validate a provider UUID.
/// Takes the remote identifier; returns whether its canonical field shape is supported.
fn valid_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

impl FreeToUse {
    /// Read a bounded HTTPS response on the provider worker.
    /// Takes an approved URL, byte limit and request; returns bytes after current cancellation checks.
    fn read(&self, url: &Url, limit: usize, request: &Request) -> Result<Vec<u8>, Failure> {
        request.check()?;
        #[cfg(test)]
        let url = if approved_media(url) {
            self.media_override.as_ref().unwrap_or(url)
        } else {
            url
        };
        let mut response = self
            .agent
            .get(url.as_str())
            .call()
            .map_err(|_| Failure::Unavailable)?;
        let status = response.status().as_u16();
        match status {
            200 => {}
            401 => return Err(Failure::TokenExpired),
            403 | 451 => return Err(Failure::RegionUnavailable),
            _ => return Err(Failure::Unavailable),
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<u64>().ok())
            .is_some_and(|bytes| bytes > limit as u64)
        {
            return Err(Failure::Capacity);
        }
        let mut reader = response.body_mut().as_reader();
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            request.check()?;
            let count = reader.read(&mut chunk).map_err(|_| Failure::Unavailable)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > limit {
                return Err(Failure::Capacity);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        request.check()?;
        Ok(bytes)
    }

    /// Recheck catalog status immediately before fetching any music.
    /// Takes a remote track ID and current request; returns validated current metadata and media location.
    fn current_track(&self, id: &TrackId, request: &Request) -> Result<(Track, Url), Failure> {
        if id.provider != ProviderId::FreeToUse || !valid_id(&id.item) {
            return Err(Failure::InvalidRequest);
        }
        let url = self.api.join(&format!("music/tracks/{}", id.item)).unwrap();
        let bytes = self.read(&url, JSON_LIMIT, request)?;
        let response: Response<RemoteTrack> =
            serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidResponse)?;
        if !response.ok {
            return Err(Failure::Unavailable);
        }
        let record = response.data.ok_or(Failure::Unavailable)?.validated()?;
        if record.0.id != *id {
            return Err(Failure::InvalidResponse);
        }
        Ok(record)
    }
}

impl MusicProvider for FreeToUse {
    fn identity(&self) -> Identity {
        Identity {
            id: ProviderId::FreeToUse,
            name: "Free To Use",
            authorization:
                "https://freetouse.com/blog/royalty-free-music-api-for-the-apps-you-build",
            terms: "https://freetouse.com/license",
            supported_platform: cfg!(target_os = "linux"),
        }
    }
    fn authentication(&self) -> Authentication {
        Authentication::Public
    }
    fn capabilities(&self, license: License, track: Option<&Track>) -> Capabilities {
        let search = authorize(
            self.identity(),
            &self.authentication(),
            license,
            SystemTime::now(),
        )
        .is_ok();
        let preview = search && track.is_none_or(|t| !t.premium && t.seconds <= 300.0);
        Capabilities {
            search,
            preview,
            preview_voices: u8::from(preview),
            ..Capabilities::default()
        }
    }
    fn search(&self, query: &str, offset: u32, request: &Request) -> Result<Page, Failure> {
        authorize(
            self.identity(),
            &self.authentication(),
            request.license,
            SystemTime::now(),
        )?;
        if query.len() > 1024 || query.chars().any(char::is_control) || offset > 100_000 {
            return Err(Failure::InvalidRequest);
        }
        let mut url = self
            .api
            .join(if query.trim().is_empty() {
                "music/tracks/all"
            } else {
                "music/tracks/search"
            })
            .unwrap();
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("limit", &PAGE_SIZE.to_string())
            .append_pair("offset", &offset.to_string());
        let bytes = self.read(&url, JSON_LIMIT, request)?;
        let response: Response<Vec<RemoteTrack>> =
            serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidResponse)?;
        if !response.ok {
            return Err(Failure::Unavailable);
        }
        let raw = response.data.ok_or(Failure::InvalidResponse)?;
        let pagination = response.pagination.ok_or(Failure::InvalidResponse)?;
        if raw.len() > PAGE_SIZE as usize
            || pagination.offset != offset
            || pagination.limit != PAGE_SIZE
            || pagination.count < offset.saturating_add(raw.len() as u32)
        {
            return Err(Failure::InvalidResponse);
        }
        let tracks = raw
            .into_iter()
            .map(|raw| raw.validated().map(|(track, _)| track))
            .collect::<Result<Vec<_>, _>>()?;
        if tracks
            .iter()
            .enumerate()
            .any(|(index, t)| tracks[..index].iter().any(|old| old.id == t.id))
        {
            return Err(Failure::InvalidResponse);
        }
        Ok(Page {
            tracks,
            offset,
            total: pagination.count,
        })
    }
    fn preview(&self, id: &TrackId, request: &Request) -> Result<Preview, Failure> {
        authorize(
            self.identity(),
            &self.authentication(),
            request.license,
            SystemTime::now(),
        )?;
        let (track, media) = self.current_track(id, request)?;
        if track.premium {
            return Err(Failure::PremiumLicenseRequired);
        }
        if track.seconds > 300.0 {
            return Err(Failure::Capacity);
        }
        let bytes = self.read(&media, AUDIO_LIMIT, request)?;
        let mut audio =
            crate::engine::decode::decode_provider_bytes(bytes, || request.check().is_err())
                .map_err(|error| match error.kind {
                    crate::engine::decode::DecodeFailureKind::Cancelled => Failure::Cancelled,
                    crate::engine::decode::DecodeFailureKind::Capacity => Failure::Capacity,
                    _ => Failure::InvalidResponse,
                })?
                .sample;
        request.check()?;
        if !matches!(audio.ch, 1 | 2) || audio.frames() as f64 / f64::from(audio.sr) > 300.0 {
            return Err(Failure::Capacity);
        }
        audio.name = track.title.clone();
        Ok(Preview {
            track,
            audio: Arc::new(audio),
            cancel: request.cancel.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
