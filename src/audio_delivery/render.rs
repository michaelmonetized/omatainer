use super::*;
use crate::engine::audio::routing::model::{Direction, Group, MAX_PHYSICAL_CHANNELS};

/// Inspect the exact selected output without allocating a renderer or opening devices.
/// Takes one immutable captured state and reviewed export; returns physical source indices and actual encoded speaker identity.
pub(crate) fn output_contract(state:&crate::engine::project::State,request:&Export)->Result<crate::engine::audio::routing::speakers::OutputContract,String>{
    if let Some(model)=&state.routing{
        let alias=request.output_alias.ok_or("Choose the exact saved output alias before exporting routed audio")?;
        let port=model.port(alias,Direction::Output).ok_or("Reviewed output alias was removed")?;
        if model.monitor_output==Some(alias){return Err("Headphones are separate from program delivery".into());}
        let _=port;
        model.channel_contract(Group::Output(alias),request.options.channels)
    }else{if request.output_alias.is_some(){return Err("Reviewed output alias no longer exists".into());}crate::engine::audio::routing::speakers::OutputContract::stereo(vec![0,1],request.options.channels)}
}

/// Prepare the reviewed native render source.
/// Takes an owned coherent capture and export request; returns a stopped independent graph, exact ordered source channels and render width without opening devices.
pub(super) fn prepare(
    captured: Captured,
    request: &Export,
) -> Result<(Box<crate::engine::RtEngine>, Vec<usize>, usize), String> {
    prepare_cancelled(captured,request,&AtomicBool::new(false))
}
fn prepare_cancelled(mut captured:Captured,request:&Export,cancel:&AtomicBool) -> Result<(Box<crate::engine::RtEngine>,Vec<usize>,usize),String> {
    let state = &mut captured.state;
    if request.source==Source::Arrangement&&state.arrangement.is_none(){return Err("This project has no Arrangement song".into());}
    if let Some(model)=&mut state.arrangement{std::sync::Arc::make_mut(model).enabled=request.source==Source::Arrangement;}
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
    if state.tracks.iter().enumerate().any(|(slot, t)| {
        state.track_processing_required(slot) && t.fx.iter()
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
    if request.source==Source::Arrangement{if let Some(model)=&state.arrangement{for instance in &model.instances{let Some(slot)=state.session.as_ref().and_then(|layout|layout.tracks.iter().position(|t|t.active&&t.id==instance.track.id))else{continue};if model.sources.iter().find(|s|s.id==instance.source).is_some_and(|s|s.clip.kind==crate::engine::ClipKind::Midi&&!s.clip.notes.is_empty())&&state.track_processing_required(slot)&&state.tracks[slot].kind!=0&&state.tracks[slot].synth.offline.is_some(){return Err("A song instrument is unavailable; restore it before exporting".into());}}}}
    for (slot, track) in state.tracks.iter().enumerate() {
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
            && state.track_processing_required(slot)
        {
            return Err("A sounding instrument is unavailable; restore it before exporting".into());
        }
    }
    if state.routing.as_ref().is_some_and(|model|model.links().any(|c|matches!(c.source.group,Group::Input(_))&&c.map.iter().any(|m|m.gain!=0.))){return Err("This source uses physical input. Use real-time recording to include the hardware return".into());}
    let contract=output_contract(state,request)?;
    let channels=contract.channels.iter().map(|&c|usize::from(c)).collect::<Vec<_>>();
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
    if matches!(request.source,Source::Scene|Source::Arrangement) {
        state.beat = 0.0;
        state.timeline_seconds = 0.0;
        for t in &mut state.tracks {
            t.launch = None;
        }
    }
    let mut rt = Prepared::from_state_cancelled(captured.state, captured.media, request.options.rate,cancel)
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
    cancel: &std::sync::Arc<AtomicBool>,
    progress: &crate::background::Reporter,
) -> Result<(), String> {
    run_selected(captured,request,None,raw,bounds,cancel,progress)
}

/// Render an explicit contribution while keeping every source and sidechain feeder alive.
/// Takes the immutable capture, request, optional canonical selection and raw render destination; writes aligned ordered frames or refuses without publication.
pub(crate) fn run_selected(mut captured:Captured,request:&Export,selection:Option<&crate::engine::audio::routing::selection::Selection>,raw:&Path,bounds:(u64,u64,u64,u64),cancel:&std::sync::Arc<AtomicBool>,progress:&crate::background::Reporter)->Result<(),String>{
    let _contract=output_contract(&captured.state,request)?;
    let source_layout=captured.state.routing.as_ref().and_then(|m|request.output_alias.and_then(|id|m.speaker_layout(Group::Output(id)))).unwrap_or_default();
    let original_delay=if let Some(selection)=selection{
        let original=captured.state.routing.as_deref().cloned().unwrap_or_default();
        let alias=request.output_alias.unwrap_or(1);
        let layout=captured.state.session.as_ref().ok_or("Contribution rendering requires stable session identity")?;
        let delay=crate::engine::audio::routing::latency::offline_output_delay(&original,layout,request.options.rate,alias)?;
        let model=selection.plan(&captured,request.output_alias)?;
        captured.state.routing=Some(std::sync::Arc::new(model));
        Some(delay)
    }else{None};
    let mut source_request=request.clone();if selection.is_some()&&source_request.output_alias.is_none(){source_request.output_alias=Some(1);}
    let (mut rt, channels, width) = prepare_cancelled(captured, &source_request,cancel)?;
    if let Some(selection)=selection{let graph=rt.routing.as_mut().ok_or("Selected contribution has no prepared graph")?;let delay=graph.output_delay(source_request.output_alias.unwrap());if original_delay.is_some_and(|original|delay!=original){return Err("Selected contribution changes the reviewed program delay; connect it to the reviewed program and capture again".into());}graph.selection=Some(selection.policy);}
    rt.offline_plugin_cancel = cancel.clone();
    if rt.routing.as_ref().is_some_and(|graph| graph.model.plugins.iter().zip(&graph.plugins).any(|(saved,plugin)| !saved.bypass && plugin.endpoint.is_none())) { return Err("A native plugin is unavailable; relink its exact identity or explicitly bypass it before exporting".into()); }
    let mut output = create(raw)?;
    let mut rendered = vec![0_f32; 1024 * width];
    let mut bytes = vec![0_u8; 1024 * usize::from(_contract.channel_count) * 4];
    let mut position = 0_u64;
    let stop = bounds.0 + bounds.1;
    let delay = u64::from(source_request.output_alias.and_then(|alias| rt.routing.as_ref().map(|graph| graph.output_delay(alias))).unwrap_or(0));
    let write_start = bounds.0 + delay;
    let end = stop + bounds.2 + delay;
    progress.progress(0, Some(end));
    while position < end {
        active(cancel)?;
        if position == stop {
            rt.stop_export_sources();
        }
        let boundary = [write_start, stop, end].into_iter().filter(|boundary| *boundary > position).min().unwrap_or(end);
        let n = (boundary - position).min(1024) as usize;
        rt.process_interleaved(&mut rendered[..n * width], width);
        if rt.routing.as_ref().is_some_and(|graph| graph.model.plugins.iter().zip(&graph.plugins).any(|(saved,plugin)| !saved.bypass && (plugin.error.is_some() || plugin.endpoint.as_ref().is_some_and(|endpoint| endpoint.faulted())))) { return Err("An isolated plugin failed during export; no audio was published".into()); }
        if position >= write_start {
            for (frame, encoded) in rendered[..n * width]
                .chunks_exact(width)
                .zip(bytes.chunks_exact_mut(usize::from(request.options.channels) * 4))
            {
                let mut source=[0_f32;MAX_PHYSICAL_CHANNELS];
                for (sample,physical) in source.iter_mut().zip(&channels){*sample=frame[*physical];}
                if request.options.channels<=2 {
                    let pair=source_layout.downmix(&source);
                    source[0]=if request.options.channels==1 {0.5*(pair[0]+pair[1])} else {pair[0]};
                    source[1]=pair[1];
                }
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
