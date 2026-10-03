//! Offline scoring owns a separate renderer and publishes a new output folder.
use super::*;
use crate::engine::{
    performance::WorkPermit,
    project::{Captured, Prepared},
    Command,
};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::Command as ChildCommand,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Video render cancelled before publication".into())
    } else {
        Ok(())
    }
}
fn create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())
}
/// Render the selected native scene from project zero against the edited picture.
/// Takes a coherent project capture, picture, new output folder and optional-work permit; returns the durable directory publication outcome.
pub(crate) fn run(
    mut captured: Captured,
    clip: &Clip,
    destination: &Path,
    permit: &WorkPermit,
) -> Result<crate::project_file::SaveOutcome, String> {
    clip.validate()?;
    let cancel = permit.cancel();
    active(&cancel)?;
    if !destination.is_absolute() || destination.as_os_str().len() > 4096 {
        return Err("Render needs a new absolute output folder".into());
    }
    if FileFingerprint::read(&clip.path) != Some(clip.fingerprint) {
        return Err("Video source changed; import it again before rendering".into());
    }
    let sample_rate = 48000;
    let frames = clip.info.rate.sample(clip.end(), sample_rate);
    let bytes = frames
        .checked_mul(8)
        .filter(|&n| n <= 2 * 1024 * 1024 * 1024)
        .ok_or("Score WAV exceeds the 2 GiB render limit; trim or move picture earlier")?;
    let stage = crate::portable_project::Stage::new(
        destination
            .parent()
            .ok_or("Render folder needs an existing parent")?,
    )?;
    let scene = captured.state.selected_scene;
    captured.state.beat = 0.0;
    captured.state.timeline_seconds = 0.0;
    captured.state.quantize = false;
    captured.state.metronome = false;
    for track in &mut captured.state.tracks {
        track.launch = None;
    }
    if let Some(map) = &mut captured.state.conductor {
        if let Some(settings) = &mut std::sync::Arc::make_mut(map).native {
            settings.count_in = 0;
        }
    }
    let mut rt = Prepared::from_state(captured.state, captured.media, sample_rate)
        .map_err(|e| e.to_string())?
        .into_offline();
    rt.apply(Command::LaunchScene {
        scene: scene as u16,
    });
    rt.apply(Command::Play);
    let mut wav = create(&stage.path.join("score.wav"))?;
    wav.write_all(b"RIFF")
        .and_then(|_| wav.write_all(&(36 + bytes as u32).to_le_bytes()))
        .and_then(|_| wav.write_all(b"WAVEfmt "))
        .and_then(|_| wav.write_all(&16u32.to_le_bytes()))
        .and_then(|_| wav.write_all(&3u16.to_le_bytes()))
        .and_then(|_| wav.write_all(&2u16.to_le_bytes()))
        .and_then(|_| wav.write_all(&sample_rate.to_le_bytes()))
        .and_then(|_| wav.write_all(&(sample_rate * 8).to_le_bytes()))
        .and_then(|_| wav.write_all(&8u16.to_le_bytes()))
        .and_then(|_| wav.write_all(&32u16.to_le_bytes()))
        .and_then(|_| wav.write_all(b"data"))
        .and_then(|_| wav.write_all(&(bytes as u32).to_le_bytes()))
        .map_err(|e| e.to_string())?;
    let mut audio = [0.0f32; 2048];
    let mut encoded = [0u8; 8192];
    let mut position = 0;
    while position < frames {
        active(&cancel)?;
        let block = (frames - position).min(1024) as usize;
        rt.process(&mut audio[..block * 2]);
        if audio[..block * 2].iter().any(|s| !s.is_finite()) {
            return Err("Offline renderer produced an invalid sample".into());
        }
        for (value, bytes) in audio[..block * 2].iter().zip(encoded.chunks_exact_mut(4)) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        wav.write_all(&encoded[..block * 8])
            .map_err(|e| e.to_string())?;
        position += block as u64;
    }
    wav.sync_all().map_err(|e| e.to_string())?;
    drop(wav);
    drop(rt);
    active(&cancel)?;
    let output = stage.path.join("picture.mkv");
    let mut command = ChildCommand::new("ffmpeg");
    command.args(["-v","error","-nostdin","-threads","1","-protocol_whitelist","file,pipe","-format_whitelist","mov,matroska,webm,avi,mpegts","-i"]).arg(&clip.path)
        .arg("-i").arg(stage.path.join("score.wav"))
        .args(["-map","0:v:0","-map","1:a:0","-filter_threads","1","-vf",&format!("trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS,tpad=start={}:start_mode=add:color=black",clip.trim_in,clip.trim_out,clip.placement),
            "-c:v","ffv1","-level","3","-c:a","pcm_f32le","-fs","12884901888","-n"]).arg(&output);
    decoder::Process::start(&mut command)?.capture(1024, Duration::from_secs(600), &cancel)?;
    let rendered = decoder::probe(output.clone(), &cancel)?;
    if rendered.info.frames != clip.end() || rendered.info.rate != clip.info.rate {
        return Err("Rendered picture frame count or rate differs from its score alignment; output was not published".into());
    }
    if FileFingerprint::read(&clip.path) != Some(clip.fingerprint) {
        return Err("Video changed during rendering; output was not published".into());
    }
    OpenOptions::new()
        .write(true)
        .open(&output)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    let mut alignment = create(&stage.path.join("alignment.json"))?;
    let metadata = serde_json::json!({"schema":1,"picture":clip,"selected_scene":scene,"sample_rate":sample_rate,"audio_frames":frames,"start_seconds":0,"picture_first_sample":clip.info.rate.sample(clip.placement,sample_rate),"end_frame_exclusive":clip.end(),"video_source_audio_included":false,"render":"selected scene loops from project zero; native mixer and effects; fresh DSP state"});
    alignment
        .write_all(&serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?)
        .and_then(|_| alignment.sync_all())
        .map_err(|e| e.to_string())?;
    let _guard = permit.commit().map_err(|e| e.to_string())?;
    crate::portable_project::publish(stage, destination, &cancel)
}
