use super::{model::*, prepared::Prepared, plugins::Parameter};
use crate::{engine::RtEngine, plugin_host::realtime::Context};

impl Prepared {
    /// Schedule shared track events and adopt block latency before any graph input is gathered.
    /// Takes the renderer; queues bounded MIDI and updates existing compensation histories without plugin calls.
    pub(super) fn begin_plugins(&mut self, rt: &mut RtEngine) {
        rt.plugin_midi.mask = 0; rt.routing_plugin_instruments = 0;
        if self.plugins.is_empty() { return; }
        for (saved,plugin) in self.model.plugins.iter().zip(&self.plugins) {
            if let Some(slot) = plugin.midi_slot.filter(|slot| saved.instrument && rt.session.tracks.get(*slot).is_some_and(|t|t.active&&Some(t.id)==saved.midi_track)) { rt.routing_plugin_instruments |= 1 << slot; }
            if let Some(slot) = plugin.midi_slot.filter(|slot|plugin.endpoint.as_ref().is_some_and(|e|e.control.class.info.has_midi_input) && rt.session.tracks.get(*slot).is_some_and(|t|t.active&&Some(t.id)==saved.midi_track)) {
                rt.plugin_midi.mask |= 1 << slot;
                if saved.instrument { rt.routing_plugin_instruments |= 1 << slot; }
            }
        }
        for slot in 0..rt.session.tracks.len() {
            if rt.plugin_midi.mask & (1 << slot) == 0 { continue; }
            if rt.arrangement.enabled() { rt.render_arrangement_output(slot); }
            else { rt.clip_launch_tick(slot); rt.render_midi_output(slot); }
        }
        let mut reports = [(0,0); super::plugins::MAX_PLUGINS];
        for (index, plugin) in self.plugins.iter_mut().enumerate() {
            let delay = self.latency.as_ref().map_or(0, |l| l.plan.inputs[plugin.node]);
            let context = context(rt, delay);
            let saved = &self.model.plugins[index];
            if let Some(endpoint) = &mut plugin.endpoint { endpoint.begin_frame(context); }
            reports[index] = (plugin.node, plugin.endpoint.as_ref().and_then(|e|e.output_delay()).unwrap_or_else(|| self.latency.as_ref().map_or(saved.latency, |l| l.plan.taps[plugin.node][2])));
        }
        if self.latency.as_mut().is_some_and(|l| !l.plugins(&reports[..self.plugins.len()])) {
            for plugin in &mut self.plugins { if let Some(endpoint) = &plugin.endpoint { endpoint.refuse(); } }
        }
        for plugin in &mut self.plugins {
            if let Some(endpoint) = &mut plugin.endpoint {
                endpoint.recontextualize(context(rt, self.latency.as_ref().map_or(0, |l| l.plan.inputs[plugin.node])));
            }
        }
    }

    /// Render one processor with all declared buses and sample-offset events.
    /// Takes the prepared node and renderer; publishes explicit input, processed and mixer taps with deterministic dry bypass.
    pub(super) fn render_plugin(&mut self, node: usize, rt: &mut RtEngine) {
        let slot = self.nodes[node].slot.unwrap();
        let plugin = &mut self.plugins[slot];
        let saved = &self.model.plugins[slot];
        let input = self.nodes[node].input;
        let delay = self.latency.as_ref().map_or(0, |l| l.plan.inputs[node]);
        let context = context(rt, delay);
        let intrinsic = plugin.endpoint.as_mut().map_or(saved.latency, |e| e.begin_frame(context));
        plugin.dry.push(&input, self.nodes[node].valid);
        let mut output = [0.; MAX_PORT_CHANNELS];
        if let Some(endpoint) = &mut plugin.endpoint {
            let opened = endpoint.control.editor_open();
            if opened && !plugin.observed_editor { plugin.parameters.fill(None); plugin.initial = false; }
            plugin.observed_editor = opened;
            if plugin.initial {
                for value in plugin.parameters.iter().flatten() { if !endpoint.parameter(value.id,value.value) { endpoint.refuse(); } }
                plugin.initial = false;
            }
            if rt.playing {
                for lane in &saved.automation {
                    let corner = plugin.automation_beat.is_none_or(|previous| context.beat < previous || lane.points.partition_point(|p| p[0] <= previous) != lane.points.partition_point(|p| p[0] <= context.beat));
                    if (endpoint.block_offset() == 0 || endpoint.block_offset() == crate::plugin_host::BLOCK - 1 || corner) && !endpoint.parameter(lane.id,lane.at(context.beat.max(0.))) { endpoint.refuse(); }
                }
                plugin.automation_beat = Some(context.beat);
            } else { plugin.automation_beat = None; }
            if let Some(track) = plugin.midi_slot.filter(|slot|endpoint.control.class.info.has_midi_input && rt.session.tracks.get(*slot).is_some_and(|t|t.active&&Some(t.id)==saved.midi_track)) {
                if rt.plugin_midi.refused(track) { endpoint.refuse(); }
                for bytes in rt.plugin_midi.events(track) {
                    if plugin.midi.len() == plugin.midi.capacity() { endpoint.refuse(); break; }
                    plugin.midi.push_back((plugin.frame.saturating_add(u64::from(delay)), *bytes));
                }
            }
            while plugin.midi.front().is_some_and(|(frame,_)| *frame <= plugin.frame) {
                let (_,bytes) = plugin.midi.pop_front().unwrap();
                if !endpoint.midi(bytes) { endpoint.refuse(); }
            }
            output = endpoint.tick(input,context);
            if self.offline && endpoint.block_offset() == 0 {
                if let Err(error) = endpoint.control.barrier(&rt.offline_plugin_cancel) { plugin.error = Some(error); endpoint.refuse(); }
            }
            self.nodes[node].valid &= endpoint.available();
        } else { self.nodes[node].valid = false; }
        plugin.frame = plugin.frame.saturating_add(1);
        if saved.bypass {
            output = if saved.instrument { [0.; MAX_PORT_CHANNELS] } else { std::array::from_fn(|channel| if channel < usize::from(saved.inputs.first().copied().unwrap_or(0).min(saved.outputs[0])) { plugin.dry.sample(channel,intrinsic) } else { 0. }) };
            self.nodes[node].valid = plugin.dry.valid(intrinsic);
        }
        if saved.output_width() == 1 { output[1] = output[0]; }
        self.publish(node, [input,output,output]);
        if let Some(track) = self.plugins[slot].scene_slot.filter(|track|rt.session.tracks.get(*track).is_some_and(|t|t.active&&Some(t.id)==self.model.plugins[slot].scene_track)) {
            if let Some(scene) = self.scene_nodes[rt.tracks[track].scene_bus] { self.source_send(node,scene,[output[0],output[if self.model.plugins[slot].output_width() == 1 {0} else {1}]],self.nodes[node].valid); }
        }
    }

    /// Queue an explicit native parameter edit on its stable processor.
    /// Takes the plugin ID, parameter ID and normalized value; returns false for unavailable, stale or full processors.
    pub(crate) fn plugin_parameter(&mut self, id: u64, parameter: u32, value: f64) -> bool {
        if self.plugin_parameter_value(id,parameter).is_none() { return false; }
        let Some(slot) = self.model.plugins.iter().position(|p| p.id == id) else { return false; };
        let plugin = &mut self.plugins[slot];
        let Some(endpoint) = &mut plugin.endpoint else { return false; };
        if endpoint.control.editing() || !endpoint.control.class.parameters.iter().any(|p| p.id == parameter && !p.is_read_only) || !endpoint.parameter(parameter,value) { return false; }
        let parameters = &mut plugin.parameters;
        let Some(index) = parameters.iter().position(|p| p.is_some_and(|p| p.id == parameter)).or_else(|| parameters.iter().position(Option::is_none)) else { endpoint.refuse(); return false; };
        parameters[index] = Some(Parameter { id: parameter, value }); true
    }
    /// Read the latest accepted manual edit or isolated controller observation.
    /// Takes stable processor and parameter IDs; returns a value only when a subsequent edit can be admitted.
    pub(crate) fn plugin_parameter_value(&self, id: u64, parameter: u32) -> Option<f64> {
        let slot = self.model.plugins.iter().position(|p|p.id==id)?;
        let plugin = &self.plugins[slot]; let endpoint = plugin.endpoint.as_ref()?;
        if endpoint.control.editing() || !endpoint.parameter_room() { return None; }
        let observed = endpoint.control.value(parameter)?;
        plugin.parameters.iter().flatten().find(|p|p.id==parameter).map(|p|p.value).or(Some(observed))
    }
}

/// Resolve the musical context belonging to the delayed processor input.
/// Takes the native renderer and complete upstream delay; returns absolute sample/quarter-note positions, tempo, meter and playing state.
fn context(rt: &RtEngine, delay: u32) -> Context {
    let seconds = (rt.timeline_seconds() - if rt.playing { 1. / f64::from(rt.sr) } else { 0. } - f64::from(delay) / f64::from(rt.sr)).max(0.);
    let beat = rt.conductor.as_ref().map_or_else(|| (rt.precise_midi_beat() - if rt.playing { rt.last_midi_step } else { 0. } - f64::from(delay) / f64::from(rt.sr) * f64::from(rt.bpm) / 60.).max(0.), |c| c.beat_at_seconds(seconds));
    let signature = rt.conductor.as_ref().map(|c| { let meter=c.position(beat).2; [i32::from(meter.numerator), 1i32 << meter.denominator_power] }).unwrap_or_else(|| rt.scenes.timing.map_or([4,4], |timing| [i32::from(timing.signature.numerator),1i32 << timing.signature.denominator_power]));
    Context { bpm: rt.conductor.as_ref().map_or(f64::from(rt.bpm), |c| 60_000_000. / c.micros_exact_at(beat)), beat, sample_position: (seconds * f64::from(rt.sr)).round() as i64, playing: rt.playing, signature, upstream_delay:delay }
}
