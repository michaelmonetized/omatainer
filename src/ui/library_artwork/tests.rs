use super::*;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture},
    tag::{Tag, TagExt, TagType},
};
use std::time::{Duration, Instant};

fn picture(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([220, 30, 10])));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, format).unwrap();
    bytes.into_inner()
}
#[test]
fn actual_png_and_jpeg_pixels_preserve_aspect_with_bounded_thumbnails() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg] {
        let image = thumbnail(&picture(format, 240, 120)).unwrap();
        assert_eq!(image.size, [96, 48]);
        assert!(image
            .pixels
            .iter()
            .all(|pixel| pixel.r() > 210 && pixel.g() < 40 && pixel.b() < 20));
    }
}
#[test]
fn corrupt_unsupported_oversized_and_large_dimension_artwork_is_refused() {
    assert!(thumbnail(b"invalid artwork").is_err());
    assert!(thumbnail(b"GIF89a\x01\0\x01\0")
        .unwrap_err()
        .contains("PNG and JPEG"));
    assert!(thumbnail(&vec![0; 4 * 1024 * 1024 + 1])
        .unwrap_err()
        .contains("4 MiB"));
    assert!(thumbnail(&picture(ImageFormat::Png, 4097, 1))
        .unwrap_err()
        .contains("4096"));
}
#[test]
fn real_embedded_cover_is_version_checked_and_read_only() {
    let files = crate::ui::library_annotations::tests::Files::new();
    let path = files.0.join("Cover.mp3");
    std::fs::write(
        &path,
        include_bytes!("../../../tests/fixtures/audio/tone.mp3"),
    )
    .unwrap();
    let mut tag = Tag::new(TagType::Id3v2);
    tag.push_picture(
        Picture::unchecked(picture(ImageFormat::Png, 40, 20))
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let fingerprint = FileFingerprint::read(&path).unwrap();
    let performance = Handle::default();
    let job = Job {
        key: Key {
            source: LibSource::File(path.clone()),
            fingerprint,
        },
        id: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        work: performance.optional_work().unwrap(),
    };
    assert_eq!(read(&job).unwrap().unwrap().size, [40, 20]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&path, b"replacement").unwrap();
    assert!(read(&job).unwrap_err().contains("changed"));
    job.cancel.store(true, Ordering::Release);
    assert!(read(&job).unwrap_err().contains("cancelled"));
}
#[test]
fn visible_worker_caches_absence_and_protection_does_not_strand_pending_rows() {
    let files = crate::ui::library_annotations::tests::Files::new();
    let path = files.0.join("One.flac");
    let item = LibItem {
        source: LibSource::File(path.clone()),
        title: "Coverless".into(),
        artist: String::new(),
        bpm: Bpm::UNKNOWN,
        key: String::new(),
        length: None,
        last_play: None,
        fingerprint: FileFingerprint::read(&path),
    };
    let ctx = egui::Context::default();
    let performance = Handle::default();
    let mut cache = Artwork::default();
    performance.set_enabled(true).unwrap();
    cache.begin_frame(&ctx);
    assert!(cache
        .visible(&item, &performance, true)
        .1
        .contains("deferred"));
    assert!(cache.worker.is_none());
    performance.set_enabled(false).unwrap();
    cache.visible(&item, &performance, true);
    performance.set_enabled(true).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        cache.begin_frame(&ctx);
        cache.visible(&item, &performance, true);
        cache.end_frame(&ctx);
        if cache.rows.is_empty() {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    performance.set_enabled(false).unwrap();
    loop {
        cache.begin_frame(&ctx);
        let (_, label) = cache.visible(&item, &performance, true);
        cache.end_frame(&ctx);
        if label == "No embedded artwork" {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(cache.rows.len(), 1);
    let entry = cache.rows.values().next().unwrap().id;
    cache.begin_frame(&ctx);
    assert_eq!(
        cache.visible(&item, &performance, true).1,
        "No embedded artwork"
    );
    assert_eq!(cache.rows.values().next().unwrap().id, entry);
}

#[test]
fn native_artwork_rows_request_the_actual_embedded_cover_and_retry_explicitly() {
    let files = crate::ui::library_annotations::tests::Files::new();
    let path = files.0.join("Cover.mp3");
    std::fs::write(
        &path,
        include_bytes!("../../../tests/fixtures/audio/tone.mp3"),
    )
    .unwrap();
    let mut tag = Tag::new(TagType::Id3v2);
    tag.push_picture(
        Picture::unchecked(picture(ImageFormat::Png, 40, 20))
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    let mut gui = crate::ui::library_annotations::tests::Gui::new(&files);
    gui.app.library_annotations.open = false;
    gui.app.lib_filter = "title:Cover".into();
    gui.frame(vec![]);
    assert_eq!(gui.app.library_view.indices.len(), 1);
    gui.click("layout…");
    gui.click("Artwork");
    gui.click("Preview layout");
    gui.wait(|gui| {
        gui.app.library_artwork.rows.iter().any(|(key, entry)| {
            key.source == LibSource::File(path.clone()) && matches!(entry.state, State::Ready(_))
        })
    });
    let ready = gui
        .app
        .library_artwork
        .rows
        .values()
        .find_map(|entry| {
            if let State::Ready(texture) = &entry.state {
                Some(texture.size())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(ready, [40, 20]);
    let previous = gui.app.library_artwork.next;
    gui.click("Retry artwork");
    gui.wait(|gui| {
        gui.app.library_artwork.next > previous
            && gui
                .app
                .library_artwork
                .rows
                .values()
                .any(|entry| matches!(entry.state, State::Ready(_)))
    });
    gui.app.library_layout.live.current_mut().density =
        crate::preferences::library_layout::Density::Compact;
    gui.frame(vec![]);
    assert!(gui
        .app
        .library_artwork
        .rows
        .values()
        .all(|entry| !matches!(entry.state, State::Loading)));
}
#[test]
fn invisible_requests_are_cancelled_and_thumbnail_cache_evicts_the_oldest_entry() {
    let files = crate::ui::library_annotations::tests::Files::new();
    let path = files.0.join("One.flac");
    let item = LibItem {
        source: LibSource::File(path.clone()),
        title: "One".into(),
        artist: String::new(),
        bpm: Bpm::UNKNOWN,
        key: String::new(),
        length: None,
        last_play: None,
        fingerprint: FileFingerprint::read(&path),
    };
    let ctx = egui::Context::default();
    let performance = Handle::default();
    let mut cache = Artwork::default();
    cache.begin_frame(&ctx);
    cache.visible(&item, &performance, true);
    let cancel = cache.rows.values().next().unwrap().cancel.clone();
    cache.begin_frame(&ctx);
    cache.end_frame(&ctx);
    assert!(cache.rows.is_empty());
    assert!(cancel.load(Ordering::Acquire));
    let fingerprint = item.fingerprint.unwrap();
    for index in 0..CACHE_LIMIT {
        cache.rows.insert(
            Key {
                source: LibSource::File(format!("/synthetic/{index}").into()),
                fingerprint,
            },
            Entry {
                id: 0,
                seen: index as u64,
                cancel: Arc::new(AtomicBool::new(false)),
                state: State::Empty,
            },
        );
    }
    cache.frame = 100;
    cache.visible(&item, &performance, true);
    assert_eq!(cache.rows.len(), CACHE_LIMIT);
    assert!(!cache.rows.contains_key(&Key {
        source: LibSource::File("/synthetic/0".into()),
        fingerprint
    }));
    cache.retry();
    assert!(cache.rows.is_empty());
}
