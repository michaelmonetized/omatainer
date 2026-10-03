//! Remote catalog identities and licensed playback stay separate from local media.
pub(crate) mod freetouse;
pub(crate) mod worker;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderId {
    FreeToUse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrackId {
    pub provider: ProviderId,
    pub item: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Authentication {
    Public,
    Account { expires: SystemTime },
    UnavailableRegion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum License {
    #[default]
    Missing,
    NonCommercialAttribution,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Identity {
    pub id: ProviderId,
    pub name: &'static str,
    pub authorization: &'static str,
    pub terms: &'static str,
    pub supported_platform: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub search: bool,
    pub preview: bool,
    pub offline: bool,
    pub stems: bool,
    pub recording: bool,
    pub decks: u8,
    pub preview_voices: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    LicenseRequired,
    PremiumLicenseRequired,
    AuthorizationRequired,
    TokenExpired,
    RegionUnavailable,
    UnsupportedPlatform,
    Cancelled,
    InvalidResponse,
    Unavailable,
    Capacity,
    InvalidRequest,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LicenseRequired => "Enable only for non-commercial use with attribution. Commercial use needs a license for Omatainer; none is configured.",
            Self::PremiumLicenseRequired => "Premium music needs a paid license; none is configured.",
            Self::AuthorizationRequired => "Provider authorization is unavailable.",
            Self::TokenExpired => "Provider access expired. Sign in again.",
            Self::RegionUnavailable => "This provider is unavailable in the current region.",
            Self::UnsupportedPlatform => "This provider is not supported on this platform.",
            Self::Cancelled => "Provider request cancelled.",
            Self::InvalidResponse => "The provider returned invalid or unsupported catalog data.",
            Self::Unavailable => "The provider could not be reached. Retry when online.",
            Self::Capacity => "The provider response exceeds the preview limit.",
            Self::InvalidRequest => "The provider request is invalid.",
        })
    }
}
impl std::error::Error for Failure {}

#[derive(Clone, Debug)]
pub(crate) struct Track {
    pub id: TrackId,
    pub title: String,
    pub artists: String,
    pub seconds: f64,
    pub premium: bool,
}
impl Track {
    /// Keep music credits with its remote identity.
    /// Takes this catalog track; returns a complete copyable attribution.
    pub fn attribution(&self) -> String {
        format!(
            "{} by {} — Free To Use: https://freetouse.com",
            self.title, self.artists
        )
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub tracks: Vec<Track>,
    pub offset: u32,
    pub total: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Preview {
    pub track: Track,
    pub audio: Arc<crate::engine::dsp::Sample>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct Request {
    pub license: License,
    pub cancel: Arc<AtomicBool>,
    pub expires: Instant,
}
impl Request {
    /// Bound one explicit catalog or preview operation.
    /// Takes its license and cancellation flag; returns a 20-second request lease.
    pub fn new(license: License, cancel: Arc<AtomicBool>) -> Self {
        Self {
            license,
            cancel,
            expires: Instant::now() + Duration::from_secs(20),
        }
    }
    /// Reject obsolete worker work before publishing or downloading.
    /// Takes this request; returns cancellation or success.
    pub fn check(&self) -> Result<(), Failure> {
        if self.cancel.load(Ordering::Acquire) || Instant::now() >= self.expires {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Check current access before enabling any provider operation.
/// Takes provider identity, current authentication, license and clock; returns an access failure or success.
pub(crate) fn authorize(
    identity: Identity,
    auth: &Authentication,
    license: License,
    now: SystemTime,
) -> Result<(), Failure> {
    if !identity.supported_platform {
        return Err(Failure::UnsupportedPlatform);
    }
    if identity.authorization.is_empty() || identity.terms.is_empty() {
        return Err(Failure::AuthorizationRequired);
    }
    match auth {
        Authentication::Public => {}
        Authentication::Account { expires } if *expires > now => {}
        Authentication::Account { .. } => return Err(Failure::TokenExpired),
        Authentication::UnavailableRegion => return Err(Failure::RegionUnavailable),
    }
    if license == License::Missing {
        return Err(Failure::LicenseRequired);
    }
    Ok(())
}

/// A provider supplies current access, bounded catalog pages and licensed audio.
/// Arguments are explicit request leases and stable remote IDs; results never masquerade as local files.
pub(crate) trait MusicProvider: Send + Sync {
    fn identity(&self) -> Identity;
    fn authentication(&self) -> Authentication;
    fn capabilities(&self, license: License, track: Option<&Track>) -> Capabilities;
    fn search(&self, query: &str, offset: u32, request: &Request) -> Result<Page, Failure>;
    fn preview(&self, id: &TrackId, request: &Request) -> Result<Preview, Failure>;
}

#[cfg(test)]
pub(crate) mod tests;
