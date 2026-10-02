use super::*;
use lofty::{
    config::WriteOptions,
    file::TaggedFileExt,
    tag::{ItemKey, Tag, TagExt, TagType},
};
use std::{fs, path::PathBuf};

// Locally generated one-pixel RGB PNG. No copyrighted image fixture is used.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 0, 3, 1,
    1, 0, 201, 254, 146, 239, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
struct Fixture {
    original: PathBuf,
    staged: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "omat-preservation-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        fs::create_dir(&directory).unwrap();
        let original = directory.join(name);
        let staged = directory.join(format!("staged-{name}"));
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .join(name),
            &original,
        )
        .unwrap();
        Self { original, staged }
    }
    fn stage_title(&self, format: FileType) {
        fs::copy(&self.original, &self.staged).unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(&self.staged)
            .unwrap();
        let cancel = AtomicBool::new(false);
        let version = id3_version(&mut file, format, &cancel).unwrap();
        let mut tags = super::super::read_tagged(&mut file, &cancel, true).unwrap();
        let kind = if format == FileType::Flac {
            TagType::VorbisComments
        } else {
            TagType::Id3v2
        };
        if tags.tag(kind).is_none() {
            tags.insert_tag(Tag::new(kind));
        }
        let tag = tags.tag_mut(kind).unwrap();
        assert!(tag.insert_text(ItemKey::TrackTitle, "Edited · 夜明け".into()));
        tag.save_to_path(
            &self.staged,
            WriteOptions::new()
                .lossy_text_encoding(false)
                .use_id3v23(version == Some(3)),
        )
        .unwrap();
    }
    fn verify(&self, format: FileType) -> Result<(), String> {
        verify(
            &mut File::open(&self.original).unwrap(),
            &mut File::open(&self.staged).unwrap(),
            format,
            ChangedFields {
                title: true,
                ..Default::default()
            },
            &AtomicBool::new(false),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.original.parent().unwrap());
    }
}
fn size(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 127) as u8,
        ((n >> 14) & 127) as u8,
        ((n >> 7) & 127) as u8,
        (n & 127) as u8,
    ]
}
fn frame(version: u8, id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut result = id.to_vec();
    result.extend_from_slice(&if version == 4 {
        size(payload.len())
    } else {
        (payload.len() as u32).to_be_bytes()
    });
    result.extend_from_slice(&[0, 0]);
    result.extend_from_slice(payload);
    result
}
fn id3_bytes(version: u8, frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.iter().flatten().copied().collect();
    let mut result = vec![b'I', b'D', b'3', version, 0, 0];
    result.extend_from_slice(&size(body.len()));
    result.extend(body);
    result
}
fn replace_mp3_tag(fixture: &Fixture, version: u8, frames: &[Vec<u8>]) {
    let original = fs::read(&fixture.original).unwrap();
    let offset = if original.starts_with(b"ID3") {
        10 + synchsafe(&original[6..10]).unwrap()
    } else {
        0
    };
    let mut tagged = id3_bytes(version, frames);
    tagged.extend_from_slice(&original[offset..]);
    fs::write(&fixture.original, tagged).unwrap();
}
fn artwork(description: &str) -> Vec<u8> {
    let mut result = b"\x03image/png\0\x03".to_vec();
    result.extend_from_slice(description.as_bytes());
    result.push(0);
    result.extend_from_slice(PNG);
    result
}

#[test]
fn real_four_format_title_writes_preserve_all_unrelated_container_bytes() {
    for (name, format) in [
        ("tone.mp3", FileType::Mpeg),
        ("tone.flac", FileType::Flac),
        ("tone-tags.wav", FileType::Wav),
        ("tone-tags.aiff", FileType::Aiff),
    ] {
        let fixture = Fixture::new(name);
        fixture.stage_title(format);
        fixture
            .verify(format)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn id3_versions_unicode_unknown_binary_private_and_artwork_survive_or_refuse_changes() {
    for version in [3, 4] {
        let fixture = Fixture::new("tone.mp3");
        let mut artist = vec![1, 0xff, 0xfe];
        for unit in "Björk · 作曲家".encode_utf16() {
            artist.extend_from_slice(&unit.to_le_bytes());
        }
        let frames = vec![
            frame(version, b"TIT2", b"\0Original title"),
            frame(version, b"TPE1", &artist),
            frame(version, b"TXXX", b"\0CUSTOM\0preserved value"),
            frame(version, b"XABC", b"\x01\xff\0unknown"),
            frame(version, b"PRIV", b"producer.example\0\xff\x01\0private"),
            frame(version, b"APIC", &artwork("Front cover")),
        ];
        replace_mp3_tag(&fixture, version, &frames);
        fixture.stage_title(FileType::Mpeg);
        fixture
            .verify(FileType::Mpeg)
            .unwrap_or_else(|error| panic!("ID3v2.{version}: {error}"));
        let mut staged = fs::read(&fixture.staged).unwrap();
        let index = staged.windows(7).position(|v| v == b"private").unwrap();
        staged[index] ^= 1;
        fs::write(&fixture.staged, staged).unwrap();
        assert!(fixture
            .verify(FileType::Mpeg)
            .unwrap_err()
            .contains("unrelated"));
    }
}

#[test]
fn a_parser_dropping_duplicate_frames_or_pictures_cannot_authorize_a_write() {
    for duplicate in [
        frame(4, b"TPE1", b"\x03Artist"),
        frame(4, b"APIC", &artwork("Front")),
    ] {
        let fixture = Fixture::new("tone.mp3");
        replace_mp3_tag(
            &fixture,
            4,
            &[
                frame(4, b"TIT2", b"\x03Before"),
                duplicate.clone(),
                duplicate,
            ],
        );
        fixture.stage_title(FileType::Mpeg);
        assert!(fixture
            .verify(FileType::Mpeg)
            .unwrap_err()
            .contains("unrelated"));
    }
}

#[test]
fn ancillary_riff_aiff_chunks_and_mpeg_secondary_tags_are_protected() {
    for (name, format, little) in [
        ("tone-tags.wav", FileType::Wav, true),
        ("tone-tags.aiff", FileType::Aiff, false),
    ] {
        let fixture = Fixture::new(name);
        let mut bytes = fs::read(&fixture.original).unwrap();
        bytes.extend_from_slice(b"XTRA");
        bytes.extend_from_slice(&if little {
            6u32.to_le_bytes()
        } else {
            6u32.to_be_bytes()
        });
        bytes.extend_from_slice(b"retain");
        let length = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&if little {
            length.to_le_bytes()
        } else {
            length.to_be_bytes()
        });
        fs::write(&fixture.original, bytes).unwrap();
        fixture.stage_title(format);
        fixture.verify(format).unwrap();
        let mut bytes = fs::read(&fixture.staged).unwrap();
        let index = bytes.windows(6).position(|v| v == b"retain").unwrap();
        bytes[index] = b'X';
        fs::write(&fixture.staged, bytes).unwrap();
        assert!(fixture.verify(format).is_err());
    }
    let fixture = Fixture::new("tone.mp3");
    let mut tag = Tag::new(TagType::Id3v1);
    assert!(tag.insert_text(ItemKey::Year, "2026".into()));
    assert!(tag.insert_text(ItemKey::TrackTitle, "Legacy".into()));
    tag.save_to_path(&fixture.original, WriteOptions::new())
        .unwrap();
    fixture.stage_title(FileType::Mpeg);
    fixture.verify(FileType::Mpeg).unwrap();
    let mut bytes = fs::read(&fixture.staged).unwrap();
    let n = bytes.len();
    bytes[n - 125] ^= 1;
    fs::write(&fixture.staged, bytes).unwrap();
    assert!(fixture.verify(FileType::Mpeg).is_err());
}

#[test]
fn flac_custom_comments_pictures_and_application_blocks_are_protected() {
    for with_custom in [false, true] {
        let fixture = Fixture::new("tone.flac");
        let mut source = File::open(&fixture.original).unwrap();
        let mut parsed =
            super::super::read_tagged(&mut source, &AtomicBool::new(false), true).unwrap();
        let tag = parsed.tag_mut(TagType::VorbisComments).unwrap();
        tag.push_picture(
            lofty::picture::Picture::unchecked(PNG.to_vec())
                .pic_type(lofty::picture::PictureType::CoverFront)
                .mime_type(lofty::picture::MimeType::Png)
                .description("Original cover")
                .build(),
        );
        if with_custom {
            let mut concrete = lofty::ogg::VorbisComments::from(tag.clone());
            concrete.insert("CUSTOM_VENDOR_FIELD".into(), "retain exactly".into());
            concrete
                .save_to_path(&fixture.original, WriteOptions::new())
                .unwrap();
        } else {
            tag.save_to_path(&fixture.original, WriteOptions::new())
                .unwrap();
        }
        let mut bytes = fs::read(&fixture.original).unwrap();
        let mut offset = 4;
        loop {
            let last = bytes[offset] & 128 != 0;
            let length =
                u32::from_be_bytes([0, bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
                    as usize;
            if last {
                bytes[offset] &= 127;
                offset += 4 + length;
                break;
            }
            offset += 4 + length;
        }
        bytes.splice(
            offset..offset,
            [0x82, 0, 0, 8, b'A', b'P', b'P', b'1', 1, 2, 3, 4],
        );
        fs::write(&fixture.original, bytes).unwrap();
        fixture.stage_title(FileType::Flac);
        if with_custom {
            // Lofty 0.24's conversion to generic Tag drops the Vorbis remainder.
            // The raw proof must detect it even though both parsed Tags agree.
            assert!(fixture
                .verify(FileType::Flac)
                .unwrap_err()
                .contains("unrelated"));
            continue;
        }
        fixture.verify(FileType::Flac).unwrap();
        let mut bytes = fs::read(&fixture.staged).unwrap();
        let at = bytes
            .windows(8)
            .position(|v| v == b"APP1\x01\x02\x03\x04")
            .unwrap();
        bytes[at + 7] ^= 1;
        fs::write(&fixture.staged, bytes).unwrap();
        assert!(fixture.verify(FileType::Flac).is_err());
    }
}

#[test]
fn opaque_header_flags_malformed_extents_cancel_and_unselected_title_changes_refuse() {
    let fixture = Fixture::new("tone.mp3");
    fixture.stage_title(FileType::Mpeg);
    assert!(verify(
        &mut File::open(&fixture.original).unwrap(),
        &mut File::open(&fixture.staged).unwrap(),
        FileType::Mpeg,
        ChangedFields::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    assert!(verify(
        &mut File::open(&fixture.original).unwrap(),
        &mut File::open(&fixture.staged).unwrap(),
        FileType::Mpeg,
        ChangedFields {
            title: true,
            ..Default::default()
        },
        &AtomicBool::new(true)
    )
    .unwrap_err()
    .contains("cancelled"));
    let mut original = fs::read(&fixture.original).unwrap();
    if !original.starts_with(b"ID3") {
        replace_mp3_tag(&fixture, 4, &[frame(4, b"TIT2", b"\x03Before")]);
        original = fs::read(&fixture.original).unwrap();
    }
    original[5] = 0x80;
    fs::write(&fixture.original, &original).unwrap();
    assert!(fixture
        .verify(FileType::Mpeg)
        .unwrap_err()
        .contains("sidecar"));
    original[5] = 0;
    original[6..10].copy_from_slice(&size(100_000_000));
    fs::write(&fixture.original, original).unwrap();
    assert!(fixture.verify(FileType::Mpeg).is_err());
}
