use super::*;
use crate::engine::audio::routing::model::{Direction, Group, MAX_PHYSICAL_CHANNELS};

/// Prepare the reviewed native render source.
/// Takes an owned coherent capture and export request; returns a stopped independent graph, exact ordered source channels and render width without opening devices.
pub(super) fn prepare(
    mut captured: Captured,
    request: &Export,
) -> Result<(Box<crate::engine::RtEngine>, Vec<usize>, usize), String> {
    let state = &mut captured.state;
    if state.mic_aux.is_some_and(|c| c.needs_input()) {return Err("Live mic/aux sources need performance recording".into());}
    if request.source == Source::Scene
        && state.session.as_ref().is_none_or(|s| {
            s.scenes
                .get(usize::from(request.scene))
                .is_none_or(|s| !s.active)
        })
    {
        return Err("Selected export scene no longer exists".into());
    }
    if state.tracks.iter().any(|t| {
        t.fx.iter()
            .any(|f| f.on && f.mix > 0.0 && f.offline.is_some())
    }) || state
        .scene_fx
        .iter()
        .flatten()
        .any(|f| f.on && f.mix > 0.0 && f.offline.is_some())
    {
        return Err(
            "An enabled device is unavailable; restore it or bypass it explicitly before exporting"
                .into(),
        );
    }
    for track in &state.tracks {
        let scene = if request.source == Source::Scene {
            Some(request.scene)
        } else {
            track.launch.map(|l| l.scene)
        };
        if scene
            .and_then(|s| track.clips.get(usize::from(s)))
            .is_some_and(|c| c.kind == crate::engine::ClipKind::Midi && !c.notes.is_empty())
            && track.kind != 0
            && track.synth.offline.is_some()
        {
            return Err("A sounding instrument is unavailable; restore it before exporting".into());
        }
    }
    let channels = if let Some(model) = &state.routing {
        if model.connections.iter().any(|c| {
            matches!(c.source.group, Group::Input(_)) && c.map.iter().any(|m| m.gain != 0.0)
        }) {
            return Err("This source uses physical input. Use real-time recording to include the hardware return".into());
        }
        let alias = request
            .output_alias
            .ok_or("Choose the exact saved output alias before exporting routed audio")?;
        let port = model
            .port(alias, Direction::Output)
            .ok_or("Reviewed output alias was removed")?;
        if !(1..=2).contains(&port.channels.len()) || model.monitor_output == Some(alias) {
            return Err(
                "Choose a mono/stereo program output alias, separate from headphones".into(),
            );
        }
        port.channels
            .iter()
            .map(|&c| usize::from(c))
            .collect::<Vec<_>>()
    } else {
        if request.output_alias.is_some() {
            return Err("Reviewed output alias no longer exists".into());
        }
        vec![0, 1]
    };
    let width = state
        .routing
        .as_ref()
        .map_or(2, |model| {
            model
                .ports
                .iter()
                .filter(|p| p.direction == Direction::Output)
                .flat_map(|p| p.channels.iter())
                .map(|&c| usize::from(c) + 1)
                .max()
                .unwrap_or(2)
        })
        .min(MAX_PHYSICAL_CHANNELS);
    state.quantize = false;
    state.metronome = false;
    if let Some(map) = &mut state.conductor {
        if let Some(options) = &mut std::sync::Arc::make_mut(map).native {
            options.count_in = 0;
        }
    }
    if request.source == Source::Scene {
        state.beat = 0.0;
        state.timeline_seconds = 0.0;
        for t in &mut state.tracks {
            t.launch = None;
        }
    }
    let mut rt = Prepared::from_state(captured.state, captured.media, request.options.rate)
        .map_err(|e| e.to_string())?
        .into_offline();
    if request.source == Source::Scene {
        rt.apply(Command::LaunchScene {
            scene: request.scene,
        });
    } else {
        rt.apply(Command::Play);
    }
    if request.decks {
        for deck in 0..crate::engine::DECKS {
            rt.apply(Command::DeckPlay { deck: deck as u8 });
        }
    }
    Ok((rt, channels, width))
}

/// Render the range and its explicit source-release tail.
/// Takes a capture, reviewed bounds, raw destination and cancellation/progress; writes exact finite ordered frames using the normal graph and limiter.
pub(super) fn run(
    captured: Captured,
    request: &Export,
    raw: &Path,
    bounds: (u64, u64, u64, u64),
    cancel: &AtomicBool,
    progress: &crate::background::Reporter,
) -> Result<(), String> {
    let (mut rt, channels, width) = prepare(captured, request)?;
    let mut output = create(raw)?;
    let mut rendered = [0_f32; 1024 * MAX_PHYSICAL_CHANNELS];
    let mut bytes = [0_u8; 1024 * 2 * 4];
    let mut position = 0_u64;
    let stop = bounds.0 + bounds.1;
    let end = stop + bounds.2;
    progress.progress(0, Some(end));
    while position < end {
        active(cancel)?;
        if position == stop {
            rt.stop_export_sources();
        }
        let boundary = if position < bounds.0 {
            bounds.0
        } else if position < stop {
            stop
        } else {
            end
        };
        let n = (boundary - position).min(1024) as usize;
        rt.process_interleaved(&mut rendered[..n * width], width);
        if position >= bounds.0 {
            for (frame, encoded) in rendered[..n * width]
                .chunks_exact(width)
                .zip(bytes.chunks_exact_mut(usize::from(request.options.channels) * 4))
            {
                let pair = if channels.len() == 1 {
                    [frame[channels[0]]; 2]
                } else {
                    [frame[channels[0]], frame[channels[1]]]
                };
                let source = if request.options.channels == 1 {
                    [0.5 * (pair[0] + pair[1]), 0.0]
                } else {
                    pair
                };
                for (sample, target) in source.iter().zip(encoded.chunks_exact_mut(4)) {
                    if !sample.is_finite() {
                        return Err("Independent renderer produced an invalid output frame".into());
                    }
                    target.copy_from_slice(&sample.to_le_bytes());
                }
            }
            output
                .write_all(&bytes[..n * usize::from(request.options.channels) * 4])
                .map_err(|e| format!("Audio render write failed: {e}"))?;
        }
        position += n as u64;
        progress.progress(position, Some(end));
    }
    output.sync_all().map_err(|e| e.to_string())?;
    active(cancel)
}
