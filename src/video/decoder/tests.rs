use super::*;
use std::sync::atomic::AtomicUsize;
pub(crate) struct Files(pub PathBuf);
impl Files {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-picture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn video(&self, name: &str, fps: &str, codec: &str) -> PathBuf {
        let path = self.0.join(name);
        let output = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size=32x32:rate={fps}"),
                "-frames:v",
                "24",
                "-c:v",
                codec,
                "-pix_fmt",
                "yuv420p",
                "-filter_threads",
                "1",
                "-n",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn actual_constant_rates_probe_and_nonkeyframe_seeks_match_sequential_pixels() {
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    for fps in [
        "24",
        "25",
        "30",
        "50",
        "60",
        "24000/1001",
        "30000/1001",
        "60000/1001",
    ] {
        let path = files.video(&format!("{}.mkv", fps.replace('/', "-")), fps, "ffv1");
        let clip = probe(path, &cancel).unwrap();
        assert_eq!(clip.info.frames, 24);
        let mut original = vec![];
        stream(&clip, 0, &cancel, |frame| {
            original.push(frame.rgba);
            Ok(())
        })
        .unwrap();
        for position in [1, 4, 8, 17, 23] {
            let mut first = None;
            let stop = AtomicBool::new(false);
            let result = stream(&clip, position, &stop, |frame| {
                assert_eq!(frame.index, position);
                first = Some(frame.rgba);
                Err("one exact frame inspected".into())
            });
            assert!(result.is_err());
            assert_eq!(
                first.unwrap(),
                original[position as usize],
                "{fps} frame {position}"
            );
        }
    }
}
#[test]
fn actual_h264_decode_and_cancellation_source_replacement_and_corruption_fail_safely() {
    let files = Files::new();
    let stop = AtomicBool::new(false);
    let path = files.video("sample.mp4", "25", "libx264");
    let clip = probe(path.clone(), &stop).unwrap();
    let mut frames = 0;
    stream(&clip, 0, &stop, |_| {
        frames += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(frames, 24);
    let stop = AtomicBool::new(true);
    assert!(probe(path.clone(), &stop).unwrap_err().contains("cancel"));
    assert!(stream(&clip, 0, &stop, |_| Ok(())).is_err());
    std::fs::write(&path, b"changed").unwrap();
    assert!(stream(&clip, 0, &AtomicBool::new(false), |_| Ok(()))
        .unwrap_err()
        .contains("changed"));
    assert!(probe(path, &AtomicBool::new(false)).is_err());
    assert!(probe(files.0.join("missing.mkv"), &AtomicBool::new(false)).is_err());
}

#[test]
fn nonzero_timestamps_seek_exactly_and_variable_rate_hdr_and_owned_child_cancellation_are_explicit()
{
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    let source = files.0.join("offset.mkv");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=32x32:rate=25",
            "-frames:v",
            "24",
            "-vf",
            "setpts=PTS+7/TB",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
            "-filter_threads",
            "1",
            "-n",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let clip = probe(source, &cancel).unwrap();
    assert_eq!(clip.info.first_pts, 7.0);
    let mut original = vec![];
    stream(&clip, 0, &cancel, |frame| {
        original.push(frame.rgba);
        Ok(())
    })
    .unwrap();
    let mut sought = None;
    let _ = stream(&clip, 8, &cancel, |frame| {
        sought = Some(frame.rgba);
        Err("inspected".into())
    });
    assert_eq!(sought.unwrap(), original[8]);
    for (name, extra) in [
        (
            "vfr.mkv",
            vec!["-vf", r"select=not(eq(n\,4))", "-fps_mode", "vfr"],
        ),
        (
            "hdr.mp4",
            vec![
                "-vf",
                "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
            ],
        ),
    ] {
        let source = files.0.join(name);
        let mut command = Command::new("ffmpeg");
        command.args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=32x32:rate=25",
            "-frames:v",
            "24",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
            "-filter_threads",
            "1",
        ]);
        if name == "hdr.mp4" {
            command.args(["-c:v", "libx264"]);
        }
        command.args(extra).arg("-n").arg(&source);
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(probe(source, &cancel).is_err(), "{name}");
    }
    let started = Instant::now();
    {
        let mut command = Command::new("sleep");
        command.arg("30");
        let mut process = Process::start(&mut command).unwrap();
        assert!(process
            .capture(16, Duration::from_secs(1), &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancel"));
    }
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn actual_two_hour_picture_and_ten_bit_prores_reach_their_exact_final_frames() {
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    let source = files.0.join("two-hours.mkv");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-threads",
            "1",
            "-f",
            "lavfi",
            "-i",
            "color=red:size=16x16:rate=25",
            "-frames:v",
            "180000",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
            "-filter_threads",
            "1",
            "-n",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let clip = probe(source, &cancel).unwrap();
    assert_eq!(clip.info.frames, 180000);
    assert_eq!(clip.info.rate.seconds(clip.info.frames), 7200.0);
    let mut first = None;
    let _ = stream(&clip, 0, &cancel, |frame| {
        first = Some(frame.rgba);
        Err("one frame".into())
    });
    let mut last = None;
    let _ = stream(&clip, 179999, &cancel, |frame| {
        last = Some(frame.rgba);
        Err("one frame".into())
    });
    assert_eq!(last.unwrap(), first.unwrap());
    let source = files.0.join("prores.mov");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-threads",
            "1",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=32x32:rate=25",
            "-frames:v",
            "24",
            "-c:v",
            "prores_ks",
            "-pix_fmt",
            "yuv422p10le",
            "-filter_threads",
            "1",
            "-n",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let clip = probe(source, &cancel).unwrap();
    assert_eq!(clip.info.codec, "prores");
    let mut count = 0;
    stream(&clip, 0, &cancel, |_| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 24);
}
