use super::*;
use crate::audio_delivery::{wav, Format, Options};
use std::io::{Seek, SeekFrom, Write};
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "delivery-qc-{}",
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
fn tone(path: &Path, channels: u16, seconds: u32, mut value: impl FnMut(u64, usize) -> f32) {
    let mut w = wav::Writer::new(path, channels, 48000, wav::Encoding::Float32, false).unwrap();
    for start in (0..u64::from(seconds) * 48000).step_by(1024) {
        let n = (u64::from(seconds) * 48000 - start).min(1024) as usize;
        let samples: Vec<_> = (0..n)
            .flat_map(|i| (0..channels as usize).map(|c| (start + i as u64, c)))
            .map(|(i, c)| value(i, c))
            .collect();
        w.append(&samples, 1.).unwrap();
    }
    w.finish().unwrap();
}
fn reference(path: &Path) -> String {
    let exe = process::executable("ffmpeg").unwrap();
    let mut command = Command::new(exe);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "info",
            "-threads",
            "1",
            "-i",
        ])
        .arg(path)
        .args([
            "-af",
            "ebur128=peak=true:framelog=verbose",
            "-f",
            "null",
            "-",
        ]);
    let mut output = Vec::new();
    let text = process::stream(&mut command, &AtomicBool::new(false), true, |b| {
        output.extend_from_slice(b);
        Ok(())
    })
    .unwrap();
    String::from_utf8(text).unwrap()
}
fn summary(text: &str, key: &str) -> f64 {
    text.lines()
        .rev()
        .find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or_else(|| panic!("Missing{key}: {text}"))
}
fn retain(case: &str, input: &Path, report: &Report, reference: &str) {
    if let Some(root) = std::env::var_os("OMATAINER_DELIVERY_RECEIPTS") {
        let root = PathBuf::from(root);
        assert!(root.is_absolute() && root.starts_with("/home"));
        std::fs::create_dir_all(&root).unwrap();
        let receipt = serde_json::json!({"schema":1,"case":case,"source_revision":std::env::var("OMATAINER_DELIVERY_REVISION").ok(),"status":"passed","input_sha256":process::sha256(input).unwrap(),"input_bytes":std::fs::metadata(input).unwrap().len(),"report":report,"independent_reference":{"implementation":"FFmpeg ebur128 filter; independent from bundled libebur128","tool":report.decoder,"raw":reference},"boundaries":["headless native software only","no device/listening/desktop","not full long-session endurance"]});
        std::fs::write(
            root.join(format!("{case}.json")),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn bs1770_calibrated_tone_and_dynamic_lra_match_independent_ffmpeg() {
    let files = Files::new();
    for dynamic in [false, true] {
        let path = files.0.join(format!("tone-{dynamic}.wav"));
        tone(&path, 1, if dynamic { 20 } else { 5 }, |i, _| {
            let gain = if dynamic {
                match i / 240000 {
                    0 => 0.1,
                    1 => 0.2,
                    2 => 0.05,
                    _ => 0.1,
                }
            } else {
                0.1
            };
            (std::f64::consts::TAU * 1000. * i as f64 / 48000.).sin() as f32 * gain
        });
        let report = measure(&path, 48000, 1, Some(4), &AtomicBool::new(false)).unwrap();
        let oracle = reference(&path);
        let integrated = summary(&oracle, "I:");
        let range = summary(&oracle, "LRA:");
        let peak = summary(&oracle, "Peak:");
        assert!(
            (report.integrated_lufs.unwrap() - integrated).abs() < 0.15,
            "{report:?}\n{oracle}"
        );
        assert!(
            (report.lra_lu.unwrap() - range).abs() < 1.0,
            "{report:?}\n{oracle}"
        );
        assert!(
            (report.true_peak_dbtp.unwrap() - peak).abs() < 0.4,
            "{report:?}\n{oracle}"
        );
        if !dynamic {
            assert!((report.integrated_lufs.unwrap() + 23.0).abs() < 0.15);
            assert!(report.short_term_max_lufs.is_some());
            assert!(report.momentary_max_lufs.is_some());
        }
        retain(
            if dynamic {
                "ebu-dynamic-lra"
            } else {
                "bs1770-mono-1khz-minus20dbfs"
            },
            &path,
            &report,
            &oracle,
        );
    }
}
#[test]
fn true_peak_intersample_silence_nonfinite_and_layout_targets_are_explicit() {
    let files = Files::new();
    let path = files.0.join("intersample.wav");
    tone(&path, 1, 3, |i, _| if i % 4 < 2 { -0.8 } else { 0.8 });
    let report = measure(&path, 48000, 1, Some(4), &AtomicBool::new(false)).unwrap();
    assert!(report.true_peak_dbtp.unwrap() > report.sample_peak_dbfs.unwrap() + 2.);
    let oracle = reference(&path);
    assert!((report.true_peak_dbtp.unwrap() - summary(&oracle, "Peak:")).abs() < 0.5);
    let targets = Targets {
        policy: Policy::Block,
        integrated_lufs: Some(-23.),
        ..Default::default()
    };
    assert!(!targets.violations(&report).unwrap().is_empty());
    retain(
        "bs1770-intersample-12khz-quadrature",
        &path,
        &report,
        &oracle,
    );
    let silent = files.0.join("silent.wav");
    tone(&silent, 1, 4, |_, _| 0.);
    let r = measure(&silent, 48000, 1, Some(4), &AtomicBool::new(false)).unwrap();
    assert!(
        r.integrated_lufs.is_none()
            && r.true_peak_dbtp.is_none()
            && r.sample_peak_dbfs.is_none()
            && r.lra_lu.is_none()
    );
    assert!(!targets.violations(&r).unwrap().is_empty());
    assert!(serde_json::to_string(&r).is_ok());
    let lfe = files.0.join("lfe.wav");
    tone(&lfe, 3, 4, |i, c| {
        if c == 2 {
            (std::f64::consts::TAU * 100. * i as f64 / 48000.).sin() as f32 * 0.2
        } else {
            0.
        }
    });
    let r = measure(&lfe, 48000, 3, Some(0xb), &AtomicBool::new(false)).unwrap();
    assert!(r.integrated_lufs.is_none());
    assert!(r.sample_peak_dbfs.is_some());
    let unknown = measure(&lfe, 48000, 3, None, &AtomicBool::new(false)).unwrap();
    assert!(!unknown.loudness_supported);
    assert!(unknown.integrated_lufs.is_none());
    assert!(!targets.violations(&unknown).unwrap().is_empty());
    let bad = files.0.join("nonfinite.wav");
    tone(&bad, 1, 1, |_, _| 0.1);
    let mut f = std::fs::OpenOptions::new().write(true).open(&bad).unwrap();
    f.seek(SeekFrom::Start(44)).unwrap();
    f.write_all(&f32::NAN.to_le_bytes()).unwrap();
    drop(f);
    assert!(measure(&bad, 48000, 1, Some(4), &AtomicBool::new(false))
        .unwrap_err()
        .contains("nonfinite"));
}
#[test]
fn final_lossy_measurement_binds_decoded_file_and_can_differ_from_source_peak() {
    let files = Files::new();
    let raw = files.0.join("source.f32");
    let samples: Vec<f32> = (0..48000 * 3)
        .map(|i| ((std::f64::consts::TAU * 12000. * i as f64 / 48000. + 0.7).sin() * 0.8) as f32)
        .collect();
    std::fs::write(
        &raw,
        samples
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let path = files.0.join("final.mp3");
    let options = Options {
        format: Format::Mp3,
        channels: 1,
        ..Default::default()
    };
    crate::audio_delivery::encode(
        &raw,
        &path,
        48000 * 3,
        0,
        1,
        &options,
        &AtomicBool::new(false),
        &Default::default(),
    )
    .unwrap();
    let r = measure(&path, 48000, 1, Some(4), &AtomicBool::new(false)).unwrap();
    let oracle = reference(&path);
    assert!(r.measured_final_encoded_audio);
    assert!((r.integrated_lufs.unwrap() - summary(&oracle, "I:")).abs() < 0.15);
    assert!((r.true_peak_dbtp.unwrap() - summary(&oracle, "Peak:")).abs() < 0.5);
    assert_ne!(r.decoded_pcm_sha256, process::sha256(&raw).unwrap());
    retain("post-mp3-encoded-measurement", &path, &r, &oracle);
}
