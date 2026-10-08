use super::*;
use std::sync::atomic::AtomicUsize;

pub(crate) struct ContractProvider {
    pub auth: Authentication,
    pub calls: AtomicUsize,
    pub block: AtomicBool,
}
impl Default for ContractProvider {
    fn default() -> Self {
        Self {
            auth: Authentication::Public,
            calls: AtomicUsize::new(0),
            block: AtomicBool::new(false),
        }
    }
}
pub(crate) fn track() -> Track {
    Track {
        id: TrackId {
            provider: ProviderId::FreeToUse,
            item: "5a4ac3dd-b6e0-42fb-9f2b-bca2f58225a2".into(),
        },
        title: "Contract tone".into(),
        artists: "Omatainer test".into(),
        seconds: 0.1,
        premium: false,
    }
}
pub(crate) fn audio() -> Arc<crate::engine::dsp::Sample> {
    Arc::new(crate::engine::dsp::Sample { spectrum: None,
        name: "Own test tone".into(),
        path: String::new(),
        sr: 48000,
        ch: 1,
        data: (0..4800)
            .map(|i| ((i as f32 / 48000.0) * 440.0 * std::f32::consts::TAU).sin() * 0.1)
            .collect(),
        peaks: Arc::new(vec![]),
        bpm: 0.0,
    })
}
impl MusicProvider for ContractProvider {
    fn identity(&self) -> Identity {
        freetouse::FreeToUse::default().identity()
    }
    fn authentication(&self) -> Authentication {
        self.auth.clone()
    }
    fn capabilities(&self, license: License, track: Option<&Track>) -> Capabilities {
        let allowed = authorize(self.identity(), &self.auth, license, SystemTime::now()).is_ok();
        Capabilities {
            search: allowed,
            preview: allowed && track.is_none_or(|t| !t.premium),
            preview_voices: u8::from(allowed),
            ..Capabilities::default()
        }
    }
    fn search(&self, _: &str, offset: u32, request: &Request) -> Result<Page, Failure> {
        authorize(
            self.identity(),
            &self.auth,
            request.license,
            SystemTime::now(),
        )?;
        self.calls.fetch_add(1, Ordering::AcqRel);
        while self.block.load(Ordering::Acquire) {
            request.check()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        request.check()?;
        Ok(Page {
            tracks: vec![track()],
            offset,
            total: 1,
        })
    }
    fn preview(&self, id: &TrackId, request: &Request) -> Result<Preview, Failure> {
        self.search("", 0, request)?;
        if *id != track().id {
            return Err(Failure::InvalidRequest);
        }
        Ok(Preview {
            track: track(),
            audio: audio(),
            cancel: request.cancel.clone(),
        })
    }
}

#[test]
fn access_requires_current_authorization_license_and_platform() {
    let identity = freetouse::FreeToUse::default().identity();
    let now = SystemTime::now();
    for (auth, license, result) in [
        (
            Authentication::Public,
            License::Missing,
            Err(Failure::LicenseRequired),
        ),
        (
            Authentication::Public,
            License::NonCommercialAttribution,
            Ok(()),
        ),
        (
            Authentication::Account {
                expires: now + Duration::from_secs(60),
            },
            License::NonCommercialAttribution,
            Ok(()),
        ),
        (
            Authentication::Account { expires: now },
            License::NonCommercialAttribution,
            Err(Failure::TokenExpired),
        ),
        (
            Authentication::UnavailableRegion,
            License::NonCommercialAttribution,
            Err(Failure::RegionUnavailable),
        ),
    ] {
        assert_eq!(authorize(identity, &auth, license, now), result);
    }
    assert_eq!(
        authorize(
            Identity {
                authorization: "",
                ..identity
            },
            &Authentication::Public,
            License::NonCommercialAttribution,
            now
        ),
        Err(Failure::AuthorizationRequired)
    );
    assert_eq!(
        authorize(
            Identity {
                supported_platform: false,
                ..identity
            },
            &Authentication::Public,
            License::NonCommercialAttribution,
            now
        ),
        Err(Failure::UnsupportedPlatform)
    );
}

#[test]
fn contract_access_expiry_and_region_are_rechecked_before_every_operation() {
    let request = Request::new(
        License::NonCommercialAttribution,
        Arc::new(AtomicBool::new(false)),
    );
    for (auth, failure) in [
        (
            Authentication::Account {
                expires: SystemTime::UNIX_EPOCH,
            },
            Failure::TokenExpired,
        ),
        (
            Authentication::UnavailableRegion,
            Failure::RegionUnavailable,
        ),
    ] {
        let provider = ContractProvider {
            auth,
            ..ContractProvider::default()
        };
        assert_eq!(provider.search("", 0, &request).unwrap_err(), failure);
        assert_eq!(
            provider.preview(&track().id, &request).unwrap_err(),
            failure
        );
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
        assert_eq!(
            provider.capabilities(request.license, None),
            Capabilities::default()
        );
    }
}

#[test]
fn worker_cancel_discards_results_and_completed_preview_keeps_its_lease() {
    let provider = Arc::new(ContractProvider {
        block: AtomicBool::new(true),
        ..ContractProvider::default()
    });
    let mut job = worker::Job::start(
        provider.clone(),
        worker::Operation::Search {
            query: String::new(),
            offset: 0,
        },
        License::NonCommercialAttribution,
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while provider.calls.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    job.cancel();
    loop {
        if let Some(reply) = job.poll() {
            assert!(matches!(reply, Err(Failure::Cancelled)));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    provider.block.store(false, Ordering::Release);
    let mut job = worker::Job::start(
        provider,
        worker::Operation::Preview(track().id),
        License::NonCommercialAttribution,
        None,
    )
    .unwrap();
    let preview = loop {
        if let Some(reply) = job.poll() {
            match reply.unwrap() {
                worker::Reply::Preview(p) => break p,
                _ => panic!("unexpected page"),
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    drop(job);
    assert!(!preview.cancel.load(Ordering::Acquire));
    assert!(preview.audio.path.is_empty());
    assert!(preview
        .track
        .attribution()
        .contains("Contract tone by Omatainer test"));
}
