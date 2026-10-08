use super::model::{Group, Model, Source, Tap, MAX_PORT_CHANNELS};
use crate::engine::session::Layout;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

pub(super) const MAX_STORAGE: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Status {
    pub enabled: bool,
    pub rate: u32,
    pub program_frames: u32,
    pub monitor_frames: u32,
    pub reserve_frames: u32,
    pub history_bytes: usize,
    pub transition_frames: u32,
    pub priming_frames: u32,
    pub low_latency_monitor: bool,
}

/// Preview exact causal delays without allocating audio histories or opening devices.
/// Takes a saved graph, retained session and rate; returns scalar timing and each stable node's three tap delays, or an admission refusal.
pub(crate) fn preview(
    model: &Model,
    layout: &Layout,
    rate: u32,
) -> Result<(Status, Vec<(Group, [u32; 3])>), String> {
    let order = model.order(layout)?;
    let Some(plan) = Plan::new(model, layout, &order, rate)? else {
        return Ok((Status::default(), Vec::new()));
    };
    let status = Status {
        enabled: true,
        rate,
        program_frames: plan.program,
        monitor_frames: if plan.monitor_bypass {
            0
        } else {
            plan.monitor_target
        },
        reserve_frames: plan.reserve,
        history_bytes: plan.storage_bytes,
        low_latency_monitor: plan.monitor_bypass,
        ..Default::default()
    };
    Ok((status, order.into_iter().zip(plan.taps).collect()))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configuration {
    pub reserve_micros: u32,
    pub low_latency_monitor: bool,
    pub reports: Vec<Report>,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            reserve_micros: 100_000,
            low_latency_monitor: false,
            reports: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub group: Group,
    pub external_micros: u32,
    pub processing_micros: u32,
}

#[derive(Clone, Debug)]
pub(super) struct Plan {
    pub inputs: Vec<u32>,
    pub taps: Vec<[u32; 3]>,
    pub program: u32,
    pub reserve: u32,
    pub required: Vec<[bool; 3]>,
    pub native: Vec<bool>,
    pub monitor_bypass: bool,
    pub main: usize,
    pub monitor_target: u32,
    pub monitor: Vec<bool>,
    pub storage_bytes: usize,
    incoming: Vec<Vec<(usize, usize)>>,
    external: Vec<u32>,
    processing: Vec<u32>,
    terminals: Vec<bool>,
    live_inputs: Vec<bool>,
    monitor_external: u32,
    terminal_floor: u32,
    absolute: Vec<Option<u32>>,
}
impl Configuration {
    /// Validate saved latency reports independently of connected devices.
    /// Takes the retained routing model and session; returns a refusal for stale groups, repeated reports or unbounded offsets.
    pub(super) fn validate(&self, model: &Model, layout: &Layout) -> Result<(), String> {
        if !(1_000..=2_000_000).contains(&self.reserve_micros) || self.reports.len() > 1024 {
            return Err("Latency reserve must be 1–2000 ms with at most 1024 reports".into());
        }
        let mut groups = std::collections::BTreeSet::new();
        for report in &self.reports {
            if model.width(report.group, layout).is_none()
                || match report.group {
                    Group::Track(id) => !layout.tracks.iter().any(|item| item.id == id),
                    Group::Scene(id) => !layout.scenes.iter().any(|item| item.id == id),
                    _ => false,
                }
                || !groups.insert(report.group)
                || report.external_micros > self.reserve_micros
                || report.processing_micros > self.reserve_micros
                || matches!(report.group, Group::Plugin(_)) && (report.processing_micros != 0 || report.external_micros != 0)
                || matches!(
                    report.group,
                    Group::Input(_) | Group::Output(_) | Group::Record(_)
                ) && report.processing_micros != 0
            {
                return Err("Latency reports need unique retained sources, bounded offsets and processing lookahead only on processing nodes".into());
            }
        }
        Ok(())
    }
}
impl Plan {
    /// Resolve every explicit and implicit path before allocating its history.
    /// Takes the saved model, retained session, topological order and output rate; returns exact causal source delays or a bounded refusal.
    pub(super) fn new(
        model: &Model,
        layout: &Layout,
        order: &[Group],
        rate: u32,
    ) -> Result<Option<Self>, String> {
        let default = Configuration::default();
        let config = match &model.latency { Some(config) => config, None if !model.plugins.is_empty() => &default, None => return Ok(None) };
        config.validate(model, layout)?;
        if !(8_000..=192_000).contains(&rate) {
            return Err("Latency compensation requires 8–192 kHz".into());
        }
        let frames =
            |micros: u32| ((u64::from(micros) * u64::from(rate) + 500_000) / 1_000_000) as u32;
        let reserve = frames(config.reserve_micros);
        let indices: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(index, group)| (*group, index))
            .collect();
        let mut incoming = vec![Vec::new(); order.len()];
        let mut required = vec![[false; 3]; order.len()];
        let mut connect = |source: Source, destination: Group| {
            let from = indices[&source.group];
            required[from][source.tap.index()] = true;
            incoming[indices[&destination]].push((from, source.tap.index()));
        };
        for connection in &model.connections {
            connect(connection.source, connection.destination);
        }
        for track in &layout.tracks {
            if !model.tracks_without_default_send.contains(&track.id) {
                for scene in &layout.scenes {
                    connect(
                        Source {
                            group: Group::Track(track.id),
                            tap: Tap::PostMixer,
                        },
                        Group::Scene(scene.id),
                    );
                }
            }
        }
        for scene in &layout.scenes {
            connect(
                Source {
                    group: Group::Scene(scene.id),
                    tap: Tap::PostMixer,
                },
                Group::Main,
            );
        }
        for plugin in &model.plugins { if plugin.scene_track.is_some() { for scene in &layout.scenes { connect(Source {group:Group::Plugin(plugin.id),tap:Tap::PostMixer},Group::Scene(scene.id)); } } }
        for deck in 0..2 {
            if !model.decks_without_default_send[usize::from(deck)] {
                connect(
                    Source {
                        group: Group::Deck(deck),
                        tap: Tap::PostMixer,
                    },
                    Group::Main,
                );
            }
        }
        let reports: BTreeMap<_, _> = config
            .reports
            .iter()
            .map(|report| (report.group, *report))
            .collect();
        let mut inputs = vec![0; order.len()];
        let mut taps = vec![[0; 3]; order.len()];
        for (index, group) in order.iter().enumerate() {
            if incoming[index].iter().any(|(source, _)| *source >= index) {
                return Err("Latency path order is invalid".into());
            }
            let input = incoming[index]
                .iter()
                .map(|(source, tap)| taps[*source][*tap])
                .max()
                .unwrap_or(0);
            let report = reports.get(group);
            let external = report.map_or(0, |report| frames(report.external_micros));
            let processing = report.map_or(0, |report| frames(report.processing_micros)).saturating_add(if let Group::Plugin(id) = group { model.plugins.iter().find(|p| p.id == *id).unwrap().latency } else { 0 });
            inputs[index] = input;
            taps[index] = [
                input + external,
                input + external + processing,
                input + external + processing,
            ];
            if taps[index][2] > reserve {
                return Err("A complete latency path exceeds the reserved interval; increase the reserve before applying".into());
            }
        }
        let live_input = order
            .iter()
            .enumerate()
            .filter(|(_, group)| matches!(group, Group::Input(_)))
            .map(|(index, _)| taps[index][2])
            .max()
            .unwrap_or(0);
        for (index, group) in order.iter().enumerate() {
            if matches!(group, Group::Output(_) | Group::Record(_)) {
                let own = taps[index][2] - inputs[index];
                inputs[index] = inputs[index].max(live_input);
                taps[index] = [inputs[index] + own; 3];
            }
        }
        let program = order
            .iter()
            .enumerate()
            .filter(|(_, group)| matches!(group, Group::Output(_) | Group::Record(_)))
            .map(|(index, _)| taps[index][2])
            .max()
            .unwrap_or(taps[indices[&Group::Main]][2]);
        if program > reserve {
            return Err("A complete latency path exceeds the reserved interval; include live input and terminal offsets in the reserve".into());
        }
        for (index, group) in order.iter().enumerate() {
            if matches!(group, Group::Output(_) | Group::Record(_)) {
                let own = taps[index][2] - inputs[index];
                inputs[index] = program - own;
                taps[index] = [program; 3];
            }
            if matches!(group, Group::Track(_) | Group::Deck(_)) {
                required[index][Tap::PostFx.index()] = true;
            }
        }
        let main = indices[&Group::Main];
        let monitor_target = program
            - model
                .monitor_output
                .and_then(|id| reports.get(&Group::Output(id)))
                .map_or(0, |report| frames(report.external_micros));
        let native: Vec<_> = order
            .iter()
            .map(|group| matches!(group, Group::Track(_) | Group::Main))
            .collect();
        let monitor: Vec<_> = order
            .iter()
            .map(|group| matches!(group, Group::Deck(_) | Group::Main))
            .collect();
        let samples: usize = 6 + order
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let width = model.width(*group, layout).unwrap().max(model.input_width(*group, layout).unwrap());
                width * required[index].iter().filter(|needed| **needed).count()
                    + if native[index] {
                        if index == main {
                            4
                        } else {
                            2
                        }
                    } else {
                        0
                    }
                    + if monitor[index] { 2 } else { 0 }
            })
            .sum::<usize>();
        let histories: usize = 2
            + required
                .iter()
                .map(|taps| taps.iter().filter(|needed| **needed).count())
                .sum::<usize>()
            + native.iter().filter(|needed| **needed).count()
            + monitor.iter().filter(|needed| **needed).count();
        let bytes = samples
            .checked_mul(4)
            .and_then(|samples| samples.checked_add(histories))
            .and_then(|samples| samples.checked_mul(reserve as usize + 1))
            .and_then(|samples| samples.checked_add(super::positions::Positions::storage_bytes(reserve, layout)))
            .ok_or("Latency history size overflow")?;
        if bytes > MAX_STORAGE {
            return Err(
                "Latency histories exceed 64 MiB; reduce the reserve or routing width".into(),
            );
        }
        Ok(Some(Self {
            external: order.iter().map(|g| reports.get(g).map_or(0, |r| frames(r.external_micros))).collect(),
            processing: order.iter().map(|g| reports.get(g).map_or(0, |r| frames(r.processing_micros)).saturating_add(if let Group::Plugin(id) = g { model.plugins.iter().find(|p| p.id == *id).unwrap().latency } else { 0 })).collect(),
            terminals: order.iter().map(|g| matches!(g, Group::Output(_) | Group::Record(_))).collect(),
            live_inputs: order.iter().map(|g| matches!(g, Group::Input(_))).collect(),
            monitor_external: model.monitor_output.and_then(|id| reports.get(&Group::Output(id))).map_or(0, |r| frames(r.external_micros)),
            terminal_floor: 0,
            absolute: vec![None;order.len()],
            incoming,
            inputs,
            taps,
            program,
            reserve,
            required,
            native,
            monitor_bypass: config.low_latency_monitor,
            main,
            monitor_target,
            monitor,
            storage_bytes: bytes,
        }))
    }

    /// Recalculate a prepared delay graph without allocating or changing storage.
    /// Takes the current intrinsic processor delays; returns false when any complete path exceeds its admitted reserve.
    fn refresh(&mut self) -> bool {
        for index in 0..self.inputs.len() {
            let input = self.incoming[index].iter().map(|(source,tap)| self.taps[*source][*tap]).max().unwrap_or(0);
            self.inputs[index] = input;
            self.taps[index] = [input.saturating_add(self.external[index]), input.saturating_add(self.external[index]).saturating_add(self.processing[index]), input.saturating_add(self.external[index]).saturating_add(self.processing[index])];
            if let Some(absolute) = self.absolute[index] { self.taps[index][1] = absolute; self.taps[index][2] = absolute; }
            if self.taps[index][2] > self.reserve { return false; }
        }
        let live = self.live_inputs.iter().enumerate().filter(|(_,live)| **live).map(|(i,_)| self.taps[i][2]).max().unwrap_or(0);
        self.program = self.terminals.iter().enumerate().filter(|(_,terminal)| **terminal).map(|(i,_)| self.inputs[i].max(live).saturating_add(self.external[i]).saturating_add(self.processing[i])).max().unwrap_or(self.taps[self.main][2]).max(self.terminal_floor);
        if self.program > self.reserve { return false; }
        for (index, terminal) in self.terminals.iter().enumerate() { if *terminal { self.inputs[index] = self.program - self.external[index] - self.processing[index]; self.taps[index] = [self.program;3]; } }
        self.monitor_target = self.program.saturating_sub(self.monitor_external);
        true
    }
}

#[derive(Clone, Debug)]
pub(super) struct History {
    data: Box<[f32]>,
    valid: Box<[bool]>,
    width: usize,
    head: usize,
    filled: u32,
    frames: usize,
}
impl History {
    /// Allocate one bounded channel history before renderer admission.
    /// Takes exact channel width and maximum delay; returns zeroed storage with no callback allocation.
    pub(super) fn new(width: usize, reserve: u32) -> Self {
        assert!((1..=MAX_PORT_CHANNELS).contains(&width));
        let frames = reserve as usize + 1;
        Self {
            data: vec![0.0; frames * width].into_boxed_slice(),
            valid: vec![false; frames].into_boxed_slice(),
            width,
            head: frames - 1,
            filled: 0,
            frames,
        }
    }
    /// Retain the current source sample once for every downstream consumer.
    /// Takes the native channel frame and original continuity; writes exactly this history's prepared width.
    #[inline]
    pub(super) fn push(&mut self, frame: &[f32], valid: bool) {
        self.head += 1;
        if self.head == self.frames { self.head = 0; }
        self.data[self.head * self.width..(self.head + 1) * self.width]
            .copy_from_slice(&frame[..self.width]);
        self.valid[self.head] = valid;
        self.filled = self.filled.saturating_add(1).min(self.frames as u32);
    }
    /// Read a causal source sample without advancing its shared history.
    /// Takes a channel and bounded delay; returns silence while that original sample has not reached this history.
    #[inline]
    pub(super) fn sample(&self, channel: usize, delay: u32) -> f32 {
        if channel >= self.width || delay >= self.filled || delay as usize >= self.frames {
            return 0.0;
        }
        let frame = self.frame(delay);
        self.data[frame * self.width + channel]
    }
    /// Read the original continuity of a retained sample.
    /// Takes its causal delay; returns false for unavailable or incomplete source history.
    #[inline]
    pub(super) fn valid(&self, delay: u32) -> bool {
        delay < self.filled
            && (delay as usize) < self.frames
            && self.valid[self.frame(delay)]
    }
    #[inline]
    fn frame(&self, delay: u32) -> usize {
        let delay = delay as usize;
        if delay <= self.head { self.head - delay } else { self.frames - (delay - self.head) }
    }
    /// Read one original sample and its continuity together.
    /// Takes a channel and causal delay; returns silence and unavailable continuity outside the filled history.
    #[inline]
    fn read(&self, channel: usize, delay: u32) -> (f32, bool) {
        if channel >= self.width || delay >= self.filled || delay as usize >= self.frames { return (0.0, false); }
        let frame = self.frame(delay);
        (self.data[frame * self.width + channel], self.valid[frame])
    }
    /// Resolve a settled delay or a changing pair of original samples.
    /// Takes a channel, old/new causal delays and transition gain; returns their audio and original continuity without reading unused history.
    #[inline]
    fn blended(&self, channel: usize, old: u32, delay: u32, mix: f32) -> (f32, bool) {
        if mix >= 1.0 || old == delay { return self.read(channel, delay); }
        if mix <= 0.0 { return self.read(channel, old); }
        let (before, old_valid) = self.read(channel, old);
        let (after, valid) = self.read(channel, delay);
        (before * (1.0 - mix) + after * mix, old_valid && valid)
    }
    /// Count prepared sample storage for admission and retained Undo budgets.
    /// Takes this history; returns its complete owned heap bytes.
    pub(super) fn bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<f32>() + self.valid.len()
    }
    /// Transfer compatible history during an applied latency change.
    /// Takes the prior history; swaps existing buffers only when rate, width and reserve geometry agree.
    pub(super) fn inherit(&mut self, prior: &mut Self) -> bool {
        if self.width != prior.width || self.frames != prior.frames {
            return false;
        }
        std::mem::swap(self, prior);
        true
    }
}

#[derive(Clone, Debug)]
pub(super) struct Runtime {
    pub plan: Plan,
    prior: Plan,
    candidate: Plan,
    histories: Vec<[Option<History>; 3]>,
    pub native: Vec<Option<History>>,
    monitor: Vec<Option<History>>,
    remaining: u32,
    transition_frames: u32,
    rate: u32,
    processed: u32,
    auxiliary: [History; 2],
    auxiliary_inputs: [Option<(u64, usize)>; 2],
    offline: bool,
}
impl Runtime {
    /// Allocate every used tap and native source before graph activation.
    /// Takes its validated plan, native channel widths and rate; returns fully prepared histories for the graph renderer.
    pub(super) fn new(plan: Plan, widths: &[usize], rate: u32) -> Self {
        let reserve = plan.reserve;
        let histories = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                std::array::from_fn(|tap| {
                    plan.required[index][tap].then(|| History::new(*width, plan.reserve))
                })
            })
            .collect();
        let native = plan
            .native
            .iter()
            .enumerate()
            .map(|(index, needed)| {
                needed.then(|| History::new(if index == plan.main { 4 } else { 2 }, plan.reserve))
            })
            .collect();
        let monitor = plan
            .monitor
            .iter()
            .map(|needed| needed.then(|| History::new(2, plan.reserve)))
            .collect();
        Self {
            candidate: plan.clone(),
            prior: plan.clone(),
            plan,
            histories,
            native,
            monitor,
            remaining: 0,
            transition_frames: (rate / 100).max(1),
            rate,
            processed: 0,
            auxiliary: std::array::from_fn(|_| History::new(3, reserve)),
            auxiliary_inputs: [None; 2],
            offline: false,
        }
    }
    /// Adopt reported plugin block delays using already-admitted histories.
    /// Takes stable prepared plugin indices and their current delays; returns a bounded reserve refusal without touching the active plan.
    pub(super) fn plugins(&mut self, reports: &[(usize,u32)]) -> bool {
        if reports.iter().all(|(index,delay)| self.plan.taps[*index][2] == *delay) { return true; }
        self.candidate.inputs.copy_from_slice(&self.plan.inputs);
        self.candidate.taps.copy_from_slice(&self.plan.taps);
        self.candidate.processing.copy_from_slice(&self.plan.processing);
        self.candidate.absolute.copy_from_slice(&self.plan.absolute);
        for (index,delay) in reports { self.candidate.absolute[*index] = Some(*delay); }
        if !self.candidate.refresh() { return false; }
        self.prior.inputs.copy_from_slice(&self.plan.inputs);
        self.prior.taps.copy_from_slice(&self.plan.taps);
        self.prior.program = self.plan.program; self.prior.monitor_target = self.plan.monitor_target;
        std::mem::swap(&mut self.plan, &mut self.candidate);
        self.remaining = if self.offline {0} else {self.transition_frames};
        true
    }
    /// Keep offline output time fixed across processor latency changes.
    /// Takes an admitted graph; reserves its maximum terminal delay, which the exporter trims exactly once.
    pub(super) fn offline(&mut self) {
        self.plan.terminal_floor = self.plan.reserve;
        self.offline = true;
        assert!(self.plan.refresh());
        self.prior.inputs.copy_from_slice(&self.plan.inputs); self.prior.taps.copy_from_slice(&self.plan.taps);
        self.prior.program = self.plan.program; self.prior.monitor_target = self.plan.monitor_target;
        self.candidate.terminal_floor = self.plan.terminal_floor;
        self.remaining = 0;
    }
    /// Advance one shared delay transition per graph frame.
    /// Takes this runtime; advances only fixed counters regardless of downstream fanout.
    pub(super) fn begin(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
        self.processed = self.processed.saturating_add(1);
    }
    #[inline]
    fn blend(&self) -> f32 {
        if self.remaining == 0 { 1.0 } else { 1.0 - self.remaining as f32 / self.transition_frames as f32 }
    }
    /// Retain every required tap after its original node renders.
    /// Takes exact source index, channel frames and continuity; pushes each used history once.
    pub(super) fn publish(
        &mut self,
        index: usize,
        taps: &[super::prepared::Frame; 3],
        valid: bool,
    ) {
        for (tap, history) in self.histories[index].iter_mut().enumerate() {
            if let Some(history) = history {
                history.push(&taps[tap], valid);
            }
        }
    }
    /// Read one sample with causal alignment into its downstream destination.
    /// Takes source/tap, destination and channel; returns the bounded old/new delay blend and retained continuity.
    #[inline]
    pub(super) fn routed(
        &self,
        source: usize,
        tap: usize,
        destination: usize,
        channel: usize,
    ) -> (f32, bool) {
        self.routed_at(
            source,
            tap,
            channel,
            self.plan.inputs[destination],
            self.prior.inputs[destination],
        )
    }
    #[inline]
    fn routed_at(
        &self,
        source: usize,
        tap: usize,
        channel: usize,
        target: u32,
        prior: u32,
    ) -> (f32, bool) {
        let history = self.histories[source][tap]
            .as_ref()
            .expect("Prepared latency source");
        let delay = target.saturating_sub(self.plan.taps[source][tap]);
        let old = prior.saturating_sub(self.prior.taps[source][tap]);
        let mix = self.blend();
        history.blended(channel, old, delay, mix)
    }
    /// Read an exact route at the headphone summing time before terminal compensation.
    /// Takes source, tap and channel; returns the same original sample aligned to the monitor output and its retained continuity.
    pub(super) fn routed_monitor(&self, source: usize, tap: usize, channel: usize) -> (f32, bool) {
        self.routed_at(
            source,
            tap,
            channel,
            self.plan.monitor_target,
            self.prior.monitor_target,
        )
    }
    /// Align generated source audio with that node's routed inputs.
    /// Takes its exact node and generated stereo frame; returns delayed native audio without borrowing another node's source clock.
    pub(super) fn generated(&mut self, index: usize, frame: [f32; 2]) -> [f32; 2] {
        let mix = self.blend();
        let old = self.prior.inputs[index];
        let delay = self.plan.inputs[index];
        let history = self.native[index].as_mut().expect("Prepared native source");
        history.push(&frame, true);
        std::array::from_fn(|channel| {
            history.blended(channel, old, delay, mix).0
        })
    }
    /// Align internal sends, metronome and post-effect previews with program audio.
    /// Takes the main node and its two independent stereo source stages; returns each source delayed to its actual summing point.
    pub(super) fn generated_main(
        &mut self,
        index: usize,
        frame: [f32; 2],
        preview: [f32; 2],
    ) -> ([f32; 2], [f32; 2]) {
        let mix = self.blend();
        let history = self.native[index].as_mut().expect("Prepared main sources");
        history.push(&[frame[0], frame[1], preview[0], preview[1]], true);
        let read = |channel, old, delay| {
            history.blended(channel, old, delay, mix).0
        };
        (
            std::array::from_fn(|channel| {
                read(channel, self.prior.inputs[index], self.plan.inputs[index])
            }),
            std::array::from_fn(|channel| {
                read(
                    channel + 2,
                    self.prior.taps[index][1],
                    self.plan.taps[index][1],
                )
            }),
        )
    }
    /// Read an aligned cue sample or the deliberate immediate monitoring path.
    /// Takes its source/tap and channel; returns the source aligned to program time unless low-latency monitoring was selected.
    pub(super) fn cue(&self, source: usize, tap: usize, channel: usize) -> f32 {
        let history = self.histories[source][tap]
            .as_ref()
            .expect("Prepared cue source");
        let delay = if self.plan.monitor_bypass {
            0
        } else {
            self.plan
                .monitor_target
                .saturating_sub(self.plan.taps[source][tap])
        };
        let old = if self.prior.monitor_bypass {
            0
        } else {
            self.prior
                .monitor_target
                .saturating_sub(self.prior.taps[source][tap])
        };
        let mix = self.blend();
        history.blended(channel, old, delay, mix).0
    }
    /// Retain original deck headphone audio independently of its channel fader.
    /// Takes its exact node and pre-fader stereo; returns aligned or explicitly immediate headphone audio.
    pub(super) fn monitor_deck(&mut self, index: usize, frame: [f32; 2]) -> [f32; 2] {
        let mix = self.blend();
        let tap = Tap::PostFx.index();
        let delay = if self.plan.monitor_bypass {
            0
        } else {
            self.plan
                .monitor_target
                .saturating_sub(self.plan.taps[index][tap])
        };
        let old = if self.prior.monitor_bypass {
            0
        } else {
            self.prior
                .monitor_target
                .saturating_sub(self.prior.taps[index][tap])
        };
        let history = self.monitor[index]
            .as_mut()
            .expect("Prepared headphone deck");
        history.push(&frame, true);
        std::array::from_fn(|channel| {
            history.blended(channel, old, delay, mix).0
        })
    }

    /// Align the selected program monitor to the headphone output's actual time.
    /// Takes the main source node and original stereo; retains master monitoring alignment while the immediate override affects cue sources only.
    pub(super) fn monitor_program(&mut self, source: usize, frame: [f32; 2]) -> [f32; 2] {
        let mix = self.blend();
        let delay = self
            .plan
            .monitor_target
            .saturating_sub(self.plan.taps[source][2]);
        let old = self
            .prior
            .monitor_target
            .saturating_sub(self.prior.taps[source][2]);
        let history = self.monitor[source]
            .as_mut()
            .expect("Prepared program monitor");
        history.push(&frame, true);
        std::array::from_fn(|channel| {
            history.blended(channel, old, delay, mix).0
        })
    }

    /// Retain processed mic/aux audio and its original detector envelope once.
    /// Takes stable input aliases, voice frames, continuity, duck gain and prepared nodes; changes source ownership using counter resets only.
    pub(super) fn publish_auxiliary(
        &mut self,
        inputs: [Option<u64>; 2],
        frames: [[f32; 2]; 2],
        valid: [bool; 2],
        duck: f32,
        nodes: &[super::prepared::Node],
    ) {
        for voice in 0..2 {
            let input = inputs[voice].and_then(|alias| {
                nodes
                    .iter()
                    .position(|node| node.group == Group::Input(alias))
                    .map(|index| (alias, index))
            });
            if input != self.auxiliary_inputs[voice] {
                self.auxiliary[voice].filled = 0;
                self.auxiliary_inputs[voice] = input;
            }
            self.auxiliary[voice].push(&[frames[voice][0], frames[voice][1], duck], valid[voice]);
        }
    }

    /// Align processed voice audio and automatic ducking to one exact terminal mix.
    /// Takes its destination index; returns independent voice frames, original continuity and the mic detector's matching envelope.
    pub(super) fn auxiliary(&self, destination: usize) -> ([[f32; 2]; 2], [bool; 2], f32) {
        self.auxiliary_at(
            self.plan.inputs[destination],
            self.prior.inputs[destination],
        )
    }
    /// Read the independently aligned program monitor's voice contributions.
    /// Takes this runtime; returns voice audio, continuity and detector gain at the headphone summing point.
    pub(super) fn auxiliary_monitor(&self) -> ([[f32; 2]; 2], [bool; 2], f32) {
        self.auxiliary_at(self.plan.monitor_target, self.prior.monitor_target)
    }
    fn auxiliary_at(&self, target: u32, prior: u32) -> ([[f32; 2]; 2], [bool; 2], f32) {
        let mix = self.blend();
        let mut frames = [[0.0; 2]; 2];
        let mut valid = [true; 2];
        let mut duck = 1.0;
        for voice in 0..2 {
            let Some((_, source)) = self.auxiliary_inputs[voice] else {
                continue;
            };
            let delay = target.saturating_sub(self.plan.taps[source][2]);
            let old = prior.saturating_sub(self.prior.taps[source][2]);
            let history = &self.auxiliary[voice];
            for channel in 0..2 {
                let (value, complete) = history.blended(channel, old, delay, mix);
                frames[voice][channel] = value;
                valid[voice] &= complete;
            }
            if voice == 0 {
                let gain = |delay| {
                    if delay < history.filled {
                        history.sample(2, delay)
                    } else {
                        1.0
                    }
                };
                duck = gain(old) * (1.0 - mix) + gain(delay) * mix;
            }
        }
        (frames, valid, duck)
    }
    /// Preserve original source histories across a compatible dynamic graph edit.
    /// Takes the active runtime and old/new stable group orders; reuses existing storage and starts one ten-millisecond alignment transition.
    pub(super) fn inherit(
        &mut self,
        old: &mut Self,
        groups: &[super::prepared::Node],
        prior_groups: &[super::prepared::Node],
    ) {
        if self.rate != old.rate
            || self.plan.reserve != old.plan.reserve
            || groups.len() != prior_groups.len()
            || groups
                .iter()
                .zip(prior_groups)
                .any(|(next, prior)| next.group != prior.group)
        {
            return;
        }
        self.plan.absolute.copy_from_slice(&old.plan.absolute);
        self.plan.terminal_floor = old.plan.terminal_floor;
        self.plan.refresh();
        self.candidate.absolute.copy_from_slice(&self.plan.absolute);
        self.candidate.terminal_floor = self.plan.terminal_floor;
        self.prior.inputs.copy_from_slice(&old.plan.inputs);
        self.prior.taps.copy_from_slice(&old.plan.taps);
        self.prior.program = old.plan.program;
        self.prior.monitor_bypass = old.plan.monitor_bypass;
        self.prior.monitor_target = old.plan.monitor_target;
        for (index, taps) in self.histories.iter_mut().enumerate() {
            for (tap, history) in taps.iter_mut().enumerate() {
                if let (Some(history), Some(prior)) =
                    (history.as_mut(), old.histories[index][tap].as_mut())
                {
                    history.inherit(prior);
                }
            }
            if let (Some(history), Some(prior)) =
                (self.native[index].as_mut(), old.native[index].as_mut())
            {
                history.inherit(prior);
            }
            if let (Some(history), Some(prior)) =
                (self.monitor[index].as_mut(), old.monitor[index].as_mut())
            {
                history.inherit(prior);
            }
        }
        for (next, prior) in self.auxiliary.iter_mut().zip(&mut old.auxiliary) {
            next.inherit(prior);
        }
        self.auxiliary_inputs = old.auxiliary_inputs;
        self.remaining = self.transition_frames;
        self.processed = old.processed;
    }
    /// Read bounded renderer timing without exposing source histories.
    /// Takes this runtime; returns actual program delay, preparation, monitor policy and transition progress.
    pub(super) fn status(&self) -> Status {
        Status {
            enabled: true,
            rate: self.rate,
            program_frames: self.plan.program,
            monitor_frames: if self.plan.monitor_bypass {
                0
            } else {
                self.plan.monitor_target
            },
            reserve_frames: self.plan.reserve,
            history_bytes: self.plan.storage_bytes,
            transition_frames: self.remaining,
            priming_frames: self
                .plan
                .program
                .max(self.prior.program)
                .saturating_sub(self.processed),
            low_latency_monitor: self.plan.monitor_bypass,
        }
    }
    /// Retire delayed audio at a source or safety boundary without clearing large buffers.
    /// Takes this runtime; invalidates all retained samples using bounded counters and returns no owned storage.
    pub(super) fn reset(&mut self) {
        for history in self
            .histories
            .iter_mut()
            .flatten()
            .chain(self.native.iter_mut())
            .chain(self.monitor.iter_mut())
            .flatten()
        {
            history.filled = 0;
        }
        for history in &mut self.auxiliary {
            history.filled = 0;
        }
        self.prior.inputs.copy_from_slice(&self.plan.inputs);
        self.prior.taps.copy_from_slice(&self.plan.taps);
        self.prior.program = self.plan.program;
        self.prior.monitor_bypass = self.plan.monitor_bypass;
        self.prior.monitor_target = self.plan.monitor_target;
        self.remaining = 0;
        self.processed = 0;
    }
    /// Account for every prepared sample, validity flag and plan vector.
    /// Takes this runtime; returns retained heap bytes charged to graph admission and Undo.
    pub(super) fn bytes(&self) -> usize {
        self.auxiliary.iter().map(History::bytes).sum::<usize>()
            + self
                .histories
                .iter()
                .flatten()
                .filter_map(Option::as_ref)
                .map(History::bytes)
                .sum::<usize>()
            + self
                .native
                .iter()
                .filter_map(Option::as_ref)
                .map(History::bytes)
                .sum::<usize>()
            + self
                .monitor
                .iter()
                .filter_map(Option::as_ref)
                .map(History::bytes)
                .sum::<usize>()
            + self.histories.capacity() * std::mem::size_of::<[Option<History>; 3]>()
            + self.native.capacity() * std::mem::size_of::<Option<History>>()
            + self.monitor.capacity() * std::mem::size_of::<Option<History>>()
            + [&self.plan, &self.prior, &self.candidate]
                .iter()
                .map(|plan| {
                    plan.inputs.capacity() * 4
                        + plan.taps.capacity() * 12
                        + plan.required.capacity() * 3
                        + plan.native.capacity()
                        + plan.monitor.capacity()
                        + plan.external.capacity()*4 + plan.processing.capacity()*4 + plan.terminals.capacity() + plan.live_inputs.capacity()
                        + plan.absolute.capacity()*std::mem::size_of::<Option<u32>>()
                        + plan.incoming.capacity()*std::mem::size_of::<Vec<(usize,usize)>>() + plan.incoming.iter().map(|links| links.capacity()*std::mem::size_of::<(usize,usize)>()).sum::<usize>()
                })
                .sum::<usize>()
    }
}
