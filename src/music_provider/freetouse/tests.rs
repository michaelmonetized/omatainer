use super::*;
use serde_json::json;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;

struct Server {
    api: Url,
    replies: Arc<Mutex<VecDeque<(u16, Vec<u8>, Option<usize>)>>>,
    paths: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(replies: Vec<(u16, Vec<u8>, Option<usize>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api = Url::parse(&format!("http://{}/v3/", listener.local_addr().unwrap())).unwrap();
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (queue, seen, done) = (replies.clone(), paths.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        seen.lock()
                            .unwrap()
                            .push(line.split_whitespace().nth(1).unwrap().into());
                        let mut consumed = line.len();
                        loop {
                            line.clear();
                            reader.read_line(&mut line).unwrap();
                            consumed += line.len();
                            assert!(consumed < 8192);
                            if line == "\r\n" || line.is_empty() {
                                break;
                            }
                        }
                        let (status, bytes, length) = queue
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or((500, vec![], None));
                        write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", length.unwrap_or(bytes.len())).unwrap();
                        let _ = stream.write_all(&bytes);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("fixture server: {error}"),
                }
            }
        });
        Self {
            api,
            replies,
            paths,
            stop,
            thread: Some(thread),
        }
    }
    fn provider(&self) -> FreeToUse {
        FreeToUse {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(2)))
                    .proxy(None)
                    .http_status_as_error(false)
                    .max_redirects(0)
                    .build(),
            ),
            api: self.api.clone(),
            media_override: Some(self.api.join("audio").unwrap()),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn record() -> serde_json::Value {
    json!({"id":"5a4ac3dd-b6e0-42fb-9f2b-bca2f58225a2","title":"Contract tone","artists":[[1,{"name":"Own test artist"}]],"duration":0.02,"is_premium":false,"status":1,"files":{"mp3":"https://eu.data.freetouse.com/music/fixture.mp3"}})
}
fn page(record: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"ok":true,"data":[record],"pagination":{"limit":12,"offset":0,"count":13}}),
    )
    .unwrap()
}
fn single(record: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"ok":true,"data":record})).unwrap()
}
fn request() -> Request {
    Request::new(
        License::NonCommercialAttribution,
        Arc::new(AtomicBool::new(false)),
    )
}
fn wav() -> Vec<u8> {
    let samples: Vec<i16> = (0..160)
        .map(|i| ((i as f32 * 440.0 / 8000.0 * std::f32::consts::TAU).sin() * 3200.0) as i16)
        .collect();
    let length = (samples.len() * 2) as u32;
    let mut bytes = Vec::from(b"RIFF".as_slice());
    bytes.extend_from_slice(&(length + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8000u32.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&length.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[test]
fn actual_http_catalog_lookup_and_decode_preserve_remote_identity_and_current_capabilities() {
    let server = Server::new(vec![
        (200, page(record()), None),
        (200, single(record()), None),
        (200, wav(), None),
    ]);
    let provider = server.provider();
    let request = request();
    let page = provider.search("chill & piano", 0, &request).unwrap();
    assert_eq!(page.total, 13);
    assert_eq!(page.tracks.len(), 1);
    let preview = provider.preview(&page.tracks[0].id, &request).unwrap();
    assert_eq!(preview.track.id, page.tracks[0].id);
    assert!(preview.audio.path.is_empty());
    assert_eq!(preview.audio.frames(), 160);
    assert!(preview.audio.data.iter().any(|x| *x != 0.0));
    let capabilities = provider.capabilities(request.license, Some(&preview.track));
    assert!(capabilities.preview && capabilities.search);
    assert_eq!(capabilities.decks, 0);
    assert_eq!(capabilities.preview_voices, 1);
    assert!(!capabilities.offline && !capabilities.recording && !capabilities.stems);
    let paths = server.paths.lock().unwrap();
    assert!(paths[0].contains("query=chill+%26+piano&limit=12&offset=0"));
    assert!(paths[1].ends_with(&preview.track.id.item));
    assert_eq!(paths[2], "/v3/audio");
    assert!(server.replies.lock().unwrap().is_empty());
}

#[test]
fn premium_changes_and_revoked_license_never_fetch_audio() {
    let mut premium = record();
    premium["is_premium"] = true.into();
    let server = Server::new(vec![
        (200, page(record()), None),
        (200, single(premium), None),
    ]);
    let provider = server.provider();
    let request = request();
    let page = provider.search("tone", 0, &request).unwrap();
    assert_eq!(
        provider.preview(&page.tracks[0].id, &request).unwrap_err(),
        Failure::PremiumLicenseRequired
    );
    let missing = Request {
        license: License::Missing,
        ..request.clone()
    };
    assert_eq!(
        provider.search("", 0, &missing).unwrap_err(),
        Failure::LicenseRequired
    );
    assert_eq!(
        provider.preview(&page.tracks[0].id, &missing).unwrap_err(),
        Failure::LicenseRequired
    );
    assert_eq!(server.paths.lock().unwrap().len(), 2);
}

#[test]
fn token_expiry_region_errors_redirects_and_bounds_fail_closed() {
    for (status, failure) in [
        (401, Failure::TokenExpired),
        (403, Failure::RegionUnavailable),
        (451, Failure::RegionUnavailable),
        (302, Failure::Unavailable),
        (500, Failure::Unavailable),
    ] {
        let server = Server::new(vec![(status, vec![], None)]);
        assert_eq!(
            server.provider().search("tone", 0, &request()).unwrap_err(),
            failure
        );
        assert_eq!(server.paths.lock().unwrap().len(), 1);
    }
    let server = Server::new(vec![(200, vec![], Some(JSON_LIMIT + 1))]);
    assert_eq!(
        server.provider().search("", 0, &request()).unwrap_err(),
        Failure::Capacity
    );
    let cancel = request();
    cancel.cancel.store(true, Ordering::Release);
    assert_eq!(
        server.provider().search("", 0, &cancel).unwrap_err(),
        Failure::Cancelled
    );
    assert_eq!(server.paths.lock().unwrap().len(), 1);
}

#[test]
fn malformed_metadata_and_unapproved_media_origins_cannot_enter_preview() {
    for (field, value) in [
        ("id", json!("../../secret")),
        ("title", json!("\n")),
        ("duration", json!(-1)),
        ("status", json!(0)),
        ("files", json!({"mp3":"http://eu.data.freetouse.com/audio"})),
        (
            "files",
            json!({"mp3":"https://eu.data.freetouse.com.attacker.example/audio"}),
        ),
        (
            "files",
            json!({"mp3":"https://name:secret@eu.data.freetouse.com/audio"}),
        ),
        (
            "files",
            json!({"mp3":"https://eu.data.freetouse.com:8443/audio"}),
        ),
        ("artists", json!([])),
    ] {
        let mut raw = record();
        raw[field] = value;
        let server = Server::new(vec![(200, page(raw), None)]);
        assert_eq!(
            server.provider().search("", 0, &request()).unwrap_err(),
            Failure::InvalidResponse,
            "{field}"
        );
    }
    let server = Server::new(vec![
        (200, single(record()), None),
        (200, b"not audio".to_vec(), None),
    ]);
    assert_eq!(
        server
            .provider()
            .preview(&crate::music_provider::tests::track().id, &request())
            .unwrap_err(),
        Failure::InvalidResponse
    );
}

#[test]
#[ignore = "explicit live public metadata check; no music is fetched"]
fn live_public_freetouse_catalog_search_and_current_lookup() {
    let provider = FreeToUse::default();
    let request = request();
    let page = provider.search("lofi", 0, &request).unwrap();
    assert!(!page.tracks.is_empty());
    let (current, media) = provider
        .current_track(&page.tracks[0].id, &request)
        .unwrap();
    assert_eq!(current.id, page.tracks[0].id);
    assert!(approved_media(&media));
    println!(
        "{}",
        json!({"provider":"FreeToUse","authentication":"public","query":"lofi","count":page.total,
        "track_id":current.id.item,"title":current.title,"artists":current.artists,"premium":current.premium,
        "media_origin":media.origin().ascii_serialization(),"audio_fetched":false,"paid_license_configured":false})
    );
}
