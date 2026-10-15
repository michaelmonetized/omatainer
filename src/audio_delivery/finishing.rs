use super::{
    measurement::{self, Policy, Report, Targets},
    process, wav, Options,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, sync::atomic::AtomicBool};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub metadata: wav::Metadata,
    pub targets: Targets,
}
impl Config {
    /// Validate an authored delivery policy before rendering.
    /// Args: tags and targets. Returns an error without touching source or output files.
    pub fn validate(&self) -> Result<(), String> {
        self.metadata.validate()?;
        self.targets.validate()
    }
}
#[derive(Debug)]
pub(crate) struct Finished {
    pub report: Report,
    pub violations: Vec<String>,
    pub sha256: String,
    pub warning: Option<String>,
}

/// Measure, tag, and verify the actual encoded deliverable.
/// Args: staged file, exact output contract, authored metadata/targets and cancellation. Returns final-byte identity and final-file metrics; target blocks never publish a file.
pub(crate) fn finish(
    path: &Path,
    options: &Options,
    mask: Option<u32>,
    config: &Config,
    context: &serde_json::Value,
    cancel: &AtomicBool,
) -> Result<Finished, String> {
    config.validate()?;
    let measured = measurement::measure(path, options.rate, options.channels, mask, cancel)?;
    let violations = config.targets.violations(&measured)?;
    if config.targets.policy == Policy::Block && !violations.is_empty() {
        return Err(format!(
            "Delivery QC blocked publication: {}",
            violations.join("; ")
        ));
    }
    let embedded = serde_json::json!({"schema":2,"metadata":config.metadata,"targets":config.targets,"violations":violations,"context":context,"qc":{
        "integrated_lufs":measured.integrated_lufs,"short_term_max_lufs":measured.short_term_max_lufs,"momentary_max_lufs":measured.momentary_max_lufs,"lra_lu":measured.lra_lu,"sample_peak_dbfs":measured.sample_peak_dbfs,"true_peak_dbtp":measured.true_peak_dbtp,"full_scale_samples":measured.full_scale_samples,"frames":measured.frames,"rate":measured.rate,"channels":measured.channels,"wave_mask":mask,"loudness_supported":measured.loudness_supported,"decoded_pcm_sha256":measured.decoded_pcm_sha256,"standard":measured.standard,"meter_sha256":measured.meter.sha256,"decoder_sha256":measured.decoder.sha256,"probe_sha256":measured.probe.sha256,"measured_final_encoded_audio":true
    }});
    if options.format.external() {
        let tagged = path.with_file_name(format!("tagged.{}", options.format.extension()));
        if tagged.exists() {
            return Err("Metadata staging file already exists".into());
        }
        let mut command = Command::new(process::executable("ffmpeg")?);
        command
            .args([
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file,pipe",
                "-format_whitelist",
                "flac,mp3",
                "-i",
            ])
            .arg(path)
            .args(["-map", "0:a:0", "-c:a", "copy", "-map_metadata", "-1"]);
        for (key, value) in [
            ("title", config.metadata.title.as_str()),
            ("artist", config.metadata.artist.as_str()),
            ("ISRC", config.metadata.isrc.as_str()),
        ] {
            if !value.is_empty() {
                command.arg("-metadata").arg(format!("{key}={value}"));
            }
        }
        command
            .arg("-metadata")
            .arg(format!(
                "comment={}",
                serde_json::to_string(&embedded).map_err(|e| e.to_string())?
            ))
            .arg("-n")
            .arg(&tagged);
        process::capture(&mut command, cancel)?;
        let verified = measurement::measure(&tagged, options.rate, options.channels, mask, cancel)?;
        same_audio(&measured, &verified)?;
        super::active(cancel)?;
        std::fs::File::open(&tagged)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&tagged, path).map_err(|e| e.to_string())?;
    } else {
        wav::embed_report(path, &embedded)?;
        let verified = measurement::measure(path, options.rate, options.channels, mask, cancel)?;
        same_audio(&measured, &verified)?;
    }
    super::active(cancel)?;
    let warning = (config.targets.policy == Policy::Warn && !violations.is_empty())
        .then(|| format!("Delivery QC: {}", violations.join("; ")));
    Ok(Finished {
        sha256: process::sha256_cancelled(path, cancel)?,
        report: measured,
        violations,
        warning,
    })
}
fn same_audio(a: &Report, b: &Report) -> Result<(), String> {
    if a.frames != b.frames || a.decoded_pcm_sha256 != b.decoded_pcm_sha256 {
        return Err("Metadata attachment changed decoded audio; no publication".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_delivery::{encode_with_settings, Format};
    use std::io::Write;
    struct Files(std::path::PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn every_final_codec_embeds_metadata_without_changing_encoded_audio_and_enforces_targets() {
        let files = Files(std::env::temp_dir().join(format!(
            "delivery-finishing-{}",
            crate::sampler_bank::BankId::new().unwrap()
        )));
        std::fs::create_dir(&files.0).unwrap();
        let raw = files.0.join("source.f32");
        let mut source = std::fs::File::create(&raw).unwrap();
        for i in 0..48000 {
            let value = (std::f32::consts::TAU * 1000. * i as f32 / 48000.).sin() * 0.1;
            source.write_all(&value.to_le_bytes()).unwrap();
        }
        drop(source);
        let config = Config {
            metadata: wav::Metadata {
                title: "Final file fixture".into(),
                artist: "Omatainer".into(),
                isrc: "USAAA2600001".into(),
                ..Default::default()
            },
            targets: Targets {
                policy: Policy::Warn,
                integrated_lufs: Some(-10.),
                ..Default::default()
            },
        };
        for format in Format::ALL {
            let path = files.0.join(format!("{format:?}.{}", format.extension()));
            let options = Options {
                format,
                channels: 1,
                ..Default::default()
            };
            let settings = wav::Settings {
                metadata: config.metadata.clone(),
                wave_mask: Some(4),
                ..Default::default()
            };
            encode_with_settings(
                &raw,
                &path,
                48000,
                0,
                1,
                &options,
                Some(&settings),
                &AtomicBool::new(false),
                &Default::default(),
            )
            .unwrap();
            let finished = finish(
                &path,
                &options,
                Some(4),
                &config,
                &serde_json::json!({"fixture":"one-second-1khz"}),
                &AtomicBool::new(false),
            )
            .unwrap_or_else(|e| panic!("{format:?}:{e}"));
            assert_eq!(finished.report.frames, 48000, "{format:?}");
            assert!(finished.warning.is_some());
            assert!(!finished.violations.is_empty());
            assert_eq!(finished.sha256, process::sha256(&path).unwrap());
            let bytes = process::capture(
                Command::new(process::executable("ffprobe").unwrap())
                    .args(["-v", "error", "-show_format", "-of", "json"])
                    .arg(&path),
                &AtomicBool::new(false),
            )
            .unwrap();
            let tags: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                String::from_utf8_lossy(&bytes).contains("Final file fixture"),
                "{format:?}:{tags}"
            );
            if format.external() {
                let text = tags["format"]["tags"]["comment"]
                    .as_str()
                    .or_else(|| tags["format"]["tags"]["COMMENT"].as_str())
                    .unwrap();
                let json: serde_json::Value = serde_json::from_str(text).unwrap();
                assert_eq!(
                    json["qc"]["decoded_pcm_sha256"],
                    finished.report.decoded_pcm_sha256
                );
            }
            let mut block = config.clone();
            block.targets.policy = Policy::Block;
            let before = process::sha256(&path).unwrap();
            assert!(finish(
                &path,
                &options,
                Some(4),
                &block,
                &serde_json::json!({}),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("blocked publication"));
            assert_eq!(before, process::sha256(&path).unwrap());
        }
    }
}
