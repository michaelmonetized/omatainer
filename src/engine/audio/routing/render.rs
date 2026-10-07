//! Each ordered node renders once; all physical outputs share the safety envelope.
use super::{model::*, prepared::Prepared};
use crate::engine::{mixer_gain, RtEngine};

impl Prepared {
    /// Render one routed sample frame.
    /// Takes the renderer, solo state, click and output width; returns all mapped physical channels.
    pub(crate) fn render(
        &mut self,
        rt: &mut RtEngine,
        any_solo: bool,
        click: f32,
        channels: usize,
    ) -> [f32; MAX_PHYSICAL_CHANNELS] {
        self.begin_plugins(rt);
        self.begin();
        rt.mic_aux.begin();
        let gains = {
            #[cfg(test)]
            if rt.legacy_gain_math {
                mixer_gain::crossfader_gains(rt.crossfader_position(), rt.xfader_curve)
            } else {
                rt.xfader_gain.tick()
            }
            #[cfg(not(test))]
            rt.xfader_gain.tick()
        };
        let mut cue = [0.0; 2];
        let mut deck_pair = None;
        let record_alias = rt.routing_pipe.recorder.alias();
        for index in 0..self.nodes.len() {
            let mut send = None;
            let mut cue_source = None;
            self.gather(index);
            let input = [self.nodes[index].input[0], self.nodes[index].input[1]];
            let group = self.nodes[index].group;
            if matches!(group, Group::Plugin(_)) { self.render_plugin(index, rt); continue; }
            let taps = match group {
                Group::Track(id) => {
                    if let Some(slot) = self.nodes[index].slot.filter(|slot| {
                        rt.session
                            .tracks
                            .get(*slot)
                            .is_some_and(|item| item.active && item.id == id)
                    }) {
                        let timer = rt.load_profile.start();
                        let generated = self.generated_input(index);
                        rt.routing_track_input = Some([input[0] - generated[0], input[1] - generated[1]]);
                        rt.routing_track_generated = generated;
                        let latency=&mut self.latency;
                        let (left, right, pfl) = rt.render_track_aligned(slot, any_solo,|frame|latency.as_mut().map_or(frame,|latency|latency.generated(index,frame)));
                        if pfl {cue_source=Some(rt.routing_track_taps[1]);}
                        if !self.nodes[index].generated && !rt.tracks[slot].input_enabled(rt.recording || rt.routing_pipe.recorder.monitoring_inputs()) { self.nodes[index].valid = true; }
                        rt.routing_track_input = None;
                        rt.routing_track_generated = [0.;2];
                        rt.load_profile.track(slot, timer);
                        if !self.model.tracks_without_default_send.contains(&id) {
                            if let Some(scene) = self.scene_nodes[rt.tracks[slot].scene_bus] {
                                send=Some((scene,[left,right],self.nodes[index].valid));
                            }
                        }
                        rt.routing_track_taps
                    } else {
                        [[0.0; 2]; 3]
                    }
                }
                Group::Scene(id) => {
                    if let Some(slot) = self.nodes[index].slot.filter(|slot| {
                        rt.session
                            .scenes
                            .get(*slot)
                            .is_some_and(|item| item.active && item.id == id)
                    }) {
                        let timer = rt.load_profile.start();
                        let output = rt.load_profile.chain(
                            &mut rt.scene_fx[slot],
                            [input[0], input[1]],
                            rt.sr,
                            true,
                            slot,
                        );
                        rt.load_profile.scene(slot, timer);
                        send=Some((self.main,output,self.nodes[index].valid));
                        [input, output, output]
                    } else {
                        [[0.0; 2]; 3]
                    }
                }
                Group::Deck(deck) => {
                    let slot = usize::from(deck);
                    let pair = deck_pair.get_or_insert_with(|| rt.render_deck_pair());
                    let [left, right] = pair.audio[slot];
                    let mixed = rt.surface.deck_fx_at(slot,crate::engine::surface_controls::fx::Placement::PostFader,[left * gains[slot],right * gains[slot]],f64::from(rt.sr)*60.0/f64::from(rt.bpm.max(1.0)),rt.sr);
                    if let Some(latency)=&mut self.latency {
                        let aligned=latency.monitor_deck(index,rt.monitor.tap(slot));
                        rt.monitor.aligned_deck(slot,aligned);
                    }
                    if !self.model.decks_without_default_send[slot] {
                        send=Some((self.main,mixed,self.nodes[index].valid));
                    }
                    if rt.decks[slot].pfl {
                        cue_source=Some(rt.monitor.tap(slot));
                    }
                    [pair.taps[slot][0], [left, right], mixed]
                }
                Group::Main => {
                    rt.mic_aux.music_gain();
                    if let Some(latency) = &mut self.latency {
                        let (inputs, frames, valid, duck) = rt.mic_aux.contributions();
                        latency.publish_auxiliary(inputs, frames, valid, duck, &self.nodes);
                    }
                    let sends = rt.surface.render_sends(f64::from(rt.sr) * 60.0 / f64::from(rt.bpm.max(1.0)));
                    let generated=[click+sends[0],click+sends[1]];
                    let(generated,preview)=if let Some(latency)=&mut self.latency {
                        let(generated,preview)=latency.generated_main(index,generated,rt.tick_provider_preview());
                        (generated,Some(preview))
                    }else{(generated,None)};
                    let before = [input[0]+generated[0],input[1]+generated[1]];
                    let mut output = rt.surface.master_fx_at(crate::engine::surface_controls::fx::Placement::PreFader,before,f64::from(rt.sr)*60.0/f64::from(rt.bpm.max(1.0)),rt.sr);
                    for slot in 0..rt.master_fx.len() {
                        let timer = rt.load_profile.start();
                        output =
                            rt.master_fx[slot].process(output, rt.fx_kind[slot], rt.fx_wet[slot]);
                        rt.load_profile.master(slot, timer, rt.fx_kind[slot]);
                    }
                    let post_fx = output;
                    let preview=preview.unwrap_or_else(||rt.tick_provider_preview());
                    for channel in 0..2 {
                        output[channel] = (output[channel] + preview[channel]) * rt.master;
                    }
                    output = rt.surface.master_fx_at(crate::engine::surface_controls::fx::Placement::PostFader,output,f64::from(rt.sr)*60.0/f64::from(rt.bpm.max(1.0)),rt.sr);
                    [before, post_fx, output]
                }
                Group::Output(_) => {
                    let node=&mut self.nodes[index];
                    if let Some(latency) = &self.latency {
                        let (frames, valid, duck) = latency.auxiliary(index);
                        rt.mic_aux.add_aligned(group, node.width, &mut node.input, &mut node.valid, rt.master, frames, valid, duck);
                    } else { rt.mic_aux.add(group,node.width,&mut node.input,&mut node.valid,rt.master); }
                    continue;
                },
                Group::Record(id) => {
                    let node=&mut self.nodes[index];
                    if let Some(latency) = &self.latency {
                        let (frames, valid, duck) = latency.auxiliary(index);
                        rt.mic_aux.add_aligned(group, node.width, &mut node.input, &mut node.valid, rt.master, frames, valid, duck);
                    } else { rt.mic_aux.add(group,node.width,&mut node.input,&mut node.valid,rt.master); }
                    if record_alias == id {
                        let seconds = rt.timeline_seconds() - if rt.playing { 1.0 / f64::from(rt.sr) } else { 0.0 };
                        rt.routing_pipe.recorder.mark_origin(id, self.output_delay(id), seconds);
                        rt.routing_pipe.recorder.capture(id, self.nodes[index].input, self.nodes[index].valid);
                    }
                    continue;
                }
                Group::Input(_) | Group::Bus(_) => {
                    let input = self.nodes[index].input;
                    let taps = match group {
                        Group::Input(_) => {
                            self.nodes[index].valid = rt.routing_pipe.valid();
                            let mut frame = [0.0; MAX_PORT_CHANNELS];
                            let port = &self.model.ports[self.nodes[index].slot.unwrap()];
                            for (value, physical) in frame.iter_mut().zip(&port.channels) {
                                *value = rt.routing_input_frame[usize::from(*physical)];
                            }
                            rt.mic_aux.feed(port.id,frame,port.channels.len(),self.nodes[index].valid && port.channels.iter().all(|c|usize::from(*c)<rt.routing_pipe.channels()));
                            [frame; 3]
                        }
                        Group::Bus(_) => {
                            let bus = &self.model.buses[self.nodes[index].slot.unwrap()];
                            let gain = if bus.mute { 0.0 } else { bus.gain };
                            [input, input, input.map(|value| value * gain)]
                        }
                        _ => unreachable!(),
                    };
                    self.publish(index, taps);
                    continue;
                }
                Group::Plugin(_) => unreachable!(),
            };
            self.publish_stereo(index, taps);
            if let Some((destination,frame,valid))=send {self.source_send(index,destination,frame,valid);}
            if let Some(frame)=cue_source {for(channel,value)in cue.iter_mut().enumerate(){*value+=if matches!(group,Group::Deck(_)){frame[channel]}else{self.latency.as_ref().map_or(frame[channel],|latency|latency.cue(index,Tap::PostFx.index(),channel))};}}
        }
        let mut output = self.outputs(channels);
        let program_source = rt.mic_aux.program_alias().and_then(|id| self.nodes.iter().position(|node| node.group == Group::Output(id)));
        let main = [self.nodes[self.main].taps[2][0], self.nodes[self.main].taps[2][1]];
        let main = self.latency.as_mut().map_or(main, |latency| latency.monitor_program(self.main, main));
        let program = program_source.map_or(main, |index| { let node = &self.nodes[index]; [node.input[0], node.input[if node.width == 1 { 0 } else { 1 }]] });
        let program = if let Some((index, (mut frame, mut valid))) = program_source.and_then(|index| self.monitor_mix(index).map(|mix| (index, mix))) {
            let node = &self.nodes[index];
            let (frames, continuity, duck) = self.latency.as_ref().unwrap().auxiliary_monitor();
            rt.mic_aux.add_aligned(node.group, node.width, &mut frame, &mut valid, rt.master, frames, continuity, duck);
            [frame[0], frame[if node.width == 1 { 0 } else { 1 }]]
        } else { program };
        let headphone = rt.render_monitor(program, cue);
        let monitor_pair = rt.monitor.status.channels.filter(|_| rt.monitor.status.available);
        if let Some(pair) = monitor_pair { for (channel, value) in pair.into_iter().zip(headphone) { output[channel] = value; } }
        let mut peak = [0.0_f32; 2];
        for (channel, value) in output
            .iter()
            .take(channels.min(MAX_PHYSICAL_CHANNELS))
            .enumerate()
        {
            if monitor_pair.is_some_and(|pair| pair.contains(&channel)) { continue; }
            let slot = channel % 2;
            if !value.is_finite() {
                peak[slot] = f32::NAN;
            } else if peak[slot].is_finite() {
                peak[slot] = peak[slot].max(value.abs());
            }
        }
        rt.safety_output.observe(peak, rt.sr);
        let gain = rt.safety_output.output([1.0; 2])[0];
        for value in &mut output {
            *value = if value.is_finite() {
                crate::engine::limiter(*value) * gain
            } else {
                0.0
            };
        }
        rt.observe_master_meter([output[0], output[1]]);
        rt.plugin_midi.next();
        output
    }
}
