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
        self.begin();
        let gains = {
            #[cfg(test)]
            if rt.legacy_gain_math {
                mixer_gain::crossfader_gains(rt.xfader, rt.xfader_curve)
            } else {
                rt.xfader_gain.tick()
            }
            #[cfg(not(test))]
            rt.xfader_gain.tick()
        };
        let mut cue = [0.0; 2];
        let record_alias = rt.routing_pipe.recorder.alias();
        for index in 0..self.nodes.len() {
            self.gather(index);
            let input = [self.nodes[index].input[0], self.nodes[index].input[1]];
            let group = self.nodes[index].group;
            let taps = match group {
                Group::Track(id) => {
                    if let Some(slot) = self.nodes[index].slot.filter(|slot| {
                        rt.session
                            .tracks
                            .get(*slot)
                            .is_some_and(|item| item.active && item.id == id)
                    }) {
                        let timer = rt.load_profile.start();
                        rt.routing_track_input = Some([input[0], input[1]]);
                        let (left, right, _) = rt.render_track_cached(slot, any_solo);
                        rt.routing_track_input = None;
                        rt.load_profile.track(slot, timer);
                        if !self.model.tracks_without_default_send.contains(&id) {
                            if let Some(scene) = self.scene_nodes[rt.tracks[slot].scene_bus] {
                                self.legacy_send(scene, [left, right]);
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
                        self.legacy_send(self.main, output);
                        [input, output, output]
                    } else {
                        [[0.0; 2]; 3]
                    }
                }
                Group::Deck(deck) => {
                    let slot = usize::from(deck);
                    let timer = rt.load_profile.start();
                    let (left, right) = rt.render_deck(slot);
                    rt.load_profile.deck(slot, timer);
                    let mixed = [left * gains[slot], right * gains[slot]];
                    if !self.model.decks_without_default_send[slot] {
                        self.legacy_send(self.main, mixed);
                    }
                    if rt.decks[slot].pfl {
                        cue[0] += left;
                        cue[1] += right;
                    }
                    [rt.routing_deck_taps[0], [left, right], mixed]
                }
                Group::Main => {
                    let before = [input[0] + click, input[1] + click];
                    let mut output = before;
                    for slot in 0..rt.master_fx.len() {
                        let timer = rt.load_profile.start();
                        output =
                            rt.master_fx[slot].process(output, rt.fx_kind[slot], rt.fx_wet[slot]);
                        rt.load_profile.master(slot, timer, rt.fx_kind[slot]);
                    }
                    let post_fx = output;
                    let preview = rt.tick_provider_preview();
                    for channel in 0..2 {
                        output[channel] = (output[channel] * (1.0 - rt.cue_mix)
                            + cue[channel] * rt.cue_mix
                            + preview[channel])
                            * rt.master;
                    }
                    [before, post_fx, output]
                }
                Group::Output(_) => continue,
                Group::Record(id) => {
                    if record_alias == id {
                        rt.routing_pipe.recorder.capture(id, self.nodes[index].input);
                    }
                    continue;
                }
                Group::Input(_) | Group::Bus(_) => {
                    let input = self.nodes[index].input;
                    let taps = match group {
                        Group::Input(_) => {
                            let mut frame = [0.0; MAX_PORT_CHANNELS];
                            let port = &self.model.ports[self.nodes[index].slot.unwrap()];
                            for (value, physical) in frame.iter_mut().zip(&port.channels) {
                                *value = rt.routing_input_frame[usize::from(*physical)];
                            }
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
            };
            self.publish_stereo(index, taps);
        }
        let mut output = self.outputs(channels);
        let mut peak = [0.0_f32; 2];
        for (channel, value) in output
            .iter()
            .take(channels.min(MAX_PHYSICAL_CHANNELS))
            .enumerate()
        {
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
        output
    }
}
