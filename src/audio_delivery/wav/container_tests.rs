use super::*;
use std::process::Command;
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "delivery-containers-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn field<'a>(bytes: &'a [u8], id: &[u8; 4], big: bool) -> &'a [u8] {
    let mut p = 12;
    loop {
        assert!(p + 8 <= bytes.len());
        let n = if big {
            u32::from_be_bytes(bytes[p + 4..p + 8].try_into().unwrap())
        } else {
            u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap())
        } as usize;
        if &bytes[p..p + 4] == id {
            return &bytes[p + 8..p + 8 + n];
        }
        p += 8 + n + (n & 1);
    }
}
fn probe(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn native_interchange_headers_and_independent_decoder_preserve_order_tags_and_frames() {
    let files = Files::new();
    for (channels, mask) in [
        (1, None),
        (2, Some(0x3)),
        (3, Some(0xb)),
        (6, Some(0x60f)),
        (8, Some(0x63f)),
    ] {
        for container in [
            Container::Wave,
            Container::Bwf,
            Container::Rf64,
            Container::Aiff,
        ] {
            let path = files.0.join(format!(
                "{channels}-{container:?}.{}",
                if container == Container::Aiff {
                    "aiff"
                } else {
                    "wav"
                }
            ));
            let metadata = Metadata {
                title: "Aligned handoff".into(),
                artist: "Omatainer fixture".into(),
                description: "Independent channel impulse".into(),
                originator: "Omatainer".into(),
                originator_reference: "fixture-1".into(),
                date: "2026-10-15".into(),
                time: "12:34:56".into(),
                time_reference: (1_u64 << 32) + 12345,
                ..Default::default()
            };
            let mut w = Writer::container(
                &path,
                channels,
                48000,
                Encoding::Pcm24,
                false,
                Settings {
                    container,
                    wave_mask: mask,
                    metadata,
                    seed: 17,
                },
            )
            .unwrap();
            let frames = 127;
            let mut samples = vec![0.; frames * channels as usize];
            for j in 0..channels as usize {
                samples[(8 + 13 * j) * channels as usize + j] = (j + 1) as f32 / 32.;
            }
            for c in samples.chunks(13 * channels as usize) {
                w.append(c, 1.).unwrap();
            }
            assert_eq!(w.finish().unwrap(), frames as u64);
            let bytes = std::fs::read(&path).unwrap();
            let info = probe(&path);
            assert_eq!(info["streams"][0]["channels"], channels);
            assert_eq!(info["streams"][0]["sample_rate"], "48000");
            assert_eq!(info["streams"][0]["duration_ts"], frames);
            let decoded = Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
                .arg(&path)
                .args([
                    "-map",
                    "0:a:0",
                    "-c:a",
                    "pcm_f32le",
                    "-f",
                    "f32le",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            let floats: Vec<_> = decoded
                .stdout
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert_eq!(floats, samples);
            let native = crate::engine::decode::decode_audio(&path).unwrap();
            assert_eq!(native.sample.frames(), frames);
            assert_eq!(native.sample.data, samples);
            if container != Container::Aiff {
                let fmt = field(&bytes, b"fmt ", false);
                if let Some(mask) = mask {
                    assert_eq!(fmt.len(), 40);
                    assert_eq!(&fmt[..2], &[0xfe, 0xff]);
                    assert_eq!(&fmt[16..18], &[22, 0]);
                    assert_eq!(&fmt[18..20], &[24, 0]);
                    assert_eq!(u32::from_le_bytes(fmt[20..24].try_into().unwrap()), mask);
                    assert_eq!(
                        &fmt[24..],
                        &[1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]
                    );
                }
                if matches!(container, Container::Bwf | Container::Rf64) {
                    let bext = field(&bytes, b"bext", false);
                    assert_eq!(
                        u64::from_le_bytes(bext[338..346].try_into().unwrap()),
                        (1_u64 << 32) + 12345
                    );
                }
            }
            let report = serde_json::json!({"metadata":{"title":"Aligned handoff"},"qc":{"integrated_lufs":-23.0,"measured_final_file":true}});
            metadata::embed(&path, &report).unwrap();
            let patched = std::fs::read(&path).unwrap();
            let text = field(
                &patched,
                if container == Container::Aiff {
                    b"ANNO"
                } else {
                    b"iXML"
                },
                container == Container::Aiff,
            );
            assert!(String::from_utf8_lossy(text).contains("measured_final_file"));
            assert_eq!(probe(&path)["streams"][0]["duration_ts"], frames);
        }
    }
}
#[test]
fn rf64_counting_header_uses_64_bit_sizes_without_allocating_audio() {
    let settings = Settings {
        container: Container::Rf64,
        ..Default::default()
    };
    for frames in [
        0,
        1,
        (u32::MAX as u64 / 6) - 1,
        (u32::MAX as u64 / 6) + 1,
        1_u64 << 33,
    ] {
        let header = settings.header(2, 48000, Encoding::Pcm24, frames).unwrap();
        assert_eq!(&header[..12], b"RF64\xff\xff\xff\xffWAVE");
        assert_eq!(
            u64::from_le_bytes(header[20..28].try_into().unwrap()),
            header.len() as u64 + frames * 6 - 8
        );
        assert_eq!(
            u64::from_le_bytes(header[28..36].try_into().unwrap()),
            frames * 6
        );
        assert_eq!(
            u64::from_le_bytes(header[36..44].try_into().unwrap()),
            frames
        );
        assert_eq!(&header[header.len() - 4..], &u32::MAX.to_le_bytes());
    }
}
#[test]
fn valid_sparse_rf64_over_four_gib_has_independent_exact_bytecount_and_is_removed() {
    use std::os::unix::fs::MetadataExt;
    let files = Files::new();
    let path = files.0.join("sparse-over4g.wav");
    let mut w = Writer::container(
        &path,
        2,
        48000,
        Encoding::Pcm24,
        false,
        Settings {
            container: Container::Rf64,
            ..Default::default()
        },
    )
    .unwrap();
    let frames = (1_u64 << 32) / 6 + 19;
    let header = w.data_offset;
    w.frames = frames;
    w.finish().unwrap();
    let m = std::fs::metadata(&path).unwrap();
    assert_eq!(m.len(), header + frames * 6);
    assert!(m.len() > 1_u64 << 32);
    assert!(m.blocks() * 512 < 1024 * 1024);
    let p = probe(&path);
    assert_eq!(p["streams"][0]["duration_ts"], frames);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn native_rf64_readback_refuses_truncated_mismatched_and_overflowing_extents() {
    let files = Files::new();
    let source = files.0.join("valid.wav");
    let mut w = Writer::container(
        &source,
        2,
        48000,
        Encoding::Pcm24,
        false,
        Settings {
            container: Container::Rf64,
            ..Default::default()
        },
    )
    .unwrap();
    w.append(&[0.125, -0.25].repeat(127), 1.).unwrap();
    w.finish().unwrap();
    let valid = std::fs::read(&source).unwrap();
    assert_eq!(
        crate::engine::decode::decode_audio(&source)
            .unwrap()
            .sample
            .frames(),
        127
    );
    for (name, offset, value) in [
        ("mismatched-count", 36, 126_u64),
        ("partial-frame", 28, 127 * 6 - 1),
        ("overflow", 20, u64::MAX),
        ("truncated", 28, 128 * 6),
    ] {
        let mut b = valid.clone();
        b[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        let path = files.0.join(format!("{name}.wav"));
        std::fs::write(&path, b).unwrap();
        assert!(
            crate::engine::decode::decode_audio(&path).is_err(),
            "{name}"
        );
    }
    assert!(Writer::container(
        &files.0.join("wrong-mask.wav"),
        6,
        48000,
        Encoding::Pcm24,
        false,
        Settings {
            wave_mask: Some(0x3),
            ..Default::default()
        }
    )
    .is_err());
    assert!(!files.0.join("wrong-mask.wav").exists());
}
