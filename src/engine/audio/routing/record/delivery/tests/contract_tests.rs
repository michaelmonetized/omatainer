use super::*;
use crate::engine::audio::routing::{
    model::{Group, Model},
    speakers::Layout,
};
use sha2::{Digest, Sha256};

fn chunk(bytes: &[u8], id: &[u8; 4]) -> (usize, usize) {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let count = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if &bytes[offset..offset + 4] == id {
            return (offset + 8, count);
        }
        offset += 8 + count + (count & 1);
    }
    panic!("Missing literal RIFF chunk {id:?}");
}

#[test]
fn declared_and_unknown_native_recording_headers_keep_pcm_order_and_recovery() {
    let files = Files::new();
    let tools = crate::audio_delivery::measurement::preflight(&AtomicBool::new(false)).unwrap();
    let mut model = Model::default();
    model.ports[0].channels = vec![3, 0, 2];
    model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
    let contract = model.channel_contract(Group::Output(1), 3).unwrap();
    let mut cases = Vec::new();
    for declared in [true, false] {
        for format in [Format::Float32, Format::Pcm24] {
            let recorder = Recorder::default();
            let q = request(&recorder, Some(vec![3, 0, 2]), format, 48000, 1);
            let folder = files.0.join(format!("declared-{declared}-{format:?}"));
            let worker = recorder.clone();
            let copy = q.clone();
            let path = folder.clone();
            let saved_contract = declared.then(|| contract.clone());
            let job = std::thread::spawn(move || {
                worker.write_delivery_with_contract(
                    &copy,
                    &path,
                    &AtomicBool::new(false),
                    saved_contract.as_ref(),
                )
            });
            let job = activate(&recorder, 48000, job);
            let input = (0..19)
                .flat_map(|_| [0.25_f32, 0.001, 0.125, -0.5])
                .collect::<Vec<_>>();
            recorder.converted(&input, 4);
            recorder.stop();
            let outcome = job.join().unwrap().unwrap();
            assert_eq!((outcome.frames, outcome.channels), (19, 3));
            assert!(outcome.warning.is_none(), "{:?}", outcome.warning);
            let wav = folder.join("take-0001.wav");
            let bytes = std::fs::read(&wav).unwrap();
            let (fmt, count) = chunk(&bytes, b"fmt ");
            assert_eq!(count, 40);
            assert_eq!(
                u16::from_le_bytes(bytes[fmt..fmt + 2].try_into().unwrap()),
                0xfffe
            );
            assert_eq!(
                u16::from_le_bytes(bytes[fmt + 2..fmt + 4].try_into().unwrap()),
                3
            );
            assert_eq!(&bytes[fmt + 16..fmt + 18], &22_u16.to_le_bytes());
            let mask = u32::from_le_bytes(bytes[fmt + 20..fmt + 24].try_into().unwrap());
            assert_eq!(mask, if declared { 0xB } else { 0 });
            assert_eq!(
                &bytes[fmt + 28..fmt + 40],
                &[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]
            );
            let (audio, size) = chunk(&bytes, b"data");
            assert_eq!(size, 19 * 3 * format.encoding().bytes());
            assert_eq!(bytes.len(), audio + size + (size & 1));
            let reference = std::process::Command::new(tools["ffmpeg"]["path"].as_str().unwrap())
                .args(["-v", "error", "-i"])
                .arg(&wav)
                .args(["-f", "f32le", "-acodec", "pcm_f32le", "pipe:1"])
                .output()
                .unwrap();
            assert!(
                reference.status.success(),
                "{}",
                String::from_utf8_lossy(&reference.stderr)
            );
            let pcm = reference
                .stdout
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(pcm.len(), 19 * 3);
            for frame in pcm.chunks_exact(3) {
                for (actual, expected) in frame.iter().zip([-0.5_f32, 0.25, 0.125]) {
                    assert!((actual - expected).abs() < 1e-6);
                }
            }
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(folder.join("recording.json")).unwrap())
                    .unwrap();
            if declared {
                assert_eq!(metadata["wave_mask"], 11);
                assert_eq!(
                    metadata["speaker_order"],
                    serde_json::json!(["front_left", "front_right", "low_frequency"])
                );
                assert_eq!(
                    metadata["physical_source_channels"],
                    serde_json::json!([3, 0, 2])
                );
            } else {
                assert!(metadata["wave_mask"].is_null());
                assert!(metadata["speaker_order"].is_null());
            }
            {
                let block = 3 * format.encoding().bytes();
                let complete = if format == Format::Float32 { 6 } else { 5 };
                let mut truncated = bytes[..audio + complete * block + 2].to_vec();
                if format == Format::Float32 {
                    truncated[audio + 5 * block..audio + 5 * block + 4]
                        .copy_from_slice(&f32::NAN.to_le_bytes());
                }
                std::fs::write(&wav, &truncated).unwrap();
                let plan =
                    crate::audio_delivery::recovery::review(&folder, &AtomicBool::new(false))
                        .unwrap();
                assert_eq!((plan.frames, plan.repairs), (5, 1));
                let fixed =
                    crate::audio_delivery::recovery::recover(&plan, &AtomicBool::new(false))
                        .unwrap();
                assert_eq!((fixed.frames, fixed.repairs), (5, 0));
                let repaired = std::fs::read(&wav).unwrap();
                let data = 5 * block;
                assert_eq!(repaired.len(), audio + data + (data & 1));
                assert_eq!(&repaired[fmt + 20..fmt + 24], &mask.to_le_bytes());
                if format == Format::Float32 {
                    let (fact, size) = chunk(&repaired, b"fact");
                    assert_eq!(size, 4);
                    assert_eq!(&repaired[fact..fact + 4], &5_u32.to_le_bytes());
                }
                let kept: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(folder.join("recording.json")).unwrap())
                        .unwrap();
                assert_eq!(kept["speaker_order"], metadata["speaker_order"]);
                let decoded = crate::engine::decode::decode_audio(&wav).unwrap();
                assert_eq!(decoded.sample.frames(), 5);
                assert_eq!(decoded.sample.ch, 3);
                if declared {
                    let mut changed = repaired;
                    changed[fmt + 20..fmt + 24].copy_from_slice(&7_u32.to_le_bytes());
                    std::fs::write(&wav, &changed).unwrap();
                    assert!(crate::audio_delivery::recovery::review(
                        &folder,
                        &AtomicBool::new(false)
                    )
                    .unwrap_err()
                    .contains("speaker"));
                    assert_eq!(changed, std::fs::read(&wav).unwrap());
                }
            }
            cases.push(serde_json::json!({"declared":declared,"format":format.title(),"frames":19,"wave_mask":mask,"file_sha256_before_recovery":format!("{:x}",Sha256::digest(&bytes)),"independent_pcm_sha256":format!("{:x}",Sha256::digest(&reference.stdout)),"partial_recovery_frames":5,"nonfinite_float_trimmed":format==Format::Float32,"metadata":metadata}));
        }
    }
    let recorder = Recorder::default();
    let q = request(&recorder, Some(vec![3, 0, 2]), Format::Float32, 48000, 1);
    let folded = model.channel_contract(Group::Output(1), 2).unwrap();
    let destination = files.0.join("fold-refused");
    assert!(recorder
        .write_delivery_with_contract(&q, &destination, &AtomicBool::new(false), Some(&folded))
        .unwrap_err()
        .contains("native"));
    assert!(!destination.exists());
    let mut stale = contract;
    stale.channels.swap(0, 1);
    assert!(recorder
        .write_delivery_with_contract(&q, &destination, &AtomicBool::new(false), Some(&stale))
        .is_err());
    assert!(!destination.exists());
    crate::audio_delivery::fixture::retain(
        "native-recording-speaker-contract",
        serde_json::json!({"status":"passed","cases":cases,"tools":tools,"folded_and_stale_refused_before_publication":true}),
    );
}

#[test]
fn recovery_rejects_fifo_manifest_without_waiting_for_a_writer() {
    let files = Files::new();
    std::fs::File::create(files.0.join(".recording.lock")).unwrap();
    let fifo = files.0.join("recording.json");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(
        crate::audio_delivery::recovery::review(&files.0, &AtomicBool::new(false))
            .unwrap_err()
            .contains("regular")
    );
}
