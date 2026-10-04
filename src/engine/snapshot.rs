//! Two reusable frames cross from audio to a snapshot worker. Readers can hold
//! the public mutex indefinitely: audio runs out of frames and skips updates.
//! Only the worker grows buffers, builds UI values and retires old payloads.
//! Capture uses prepared capacity and allocates/frees nothing on audio. Worker
//! materialization clones variable metadata once per snapshot, proportional to
//! names/racks/banks, but only shares waveform Arcs: zero peak-data allocations
//! or copies regardless of media duration. See issue-59 validation for the
//! measured fixture budget; metadata is not subject to a universal byte cap.

use super::*;
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError, TrySendError};
use std::sync::atomic::{AtomicU64, Ordering};

const FRAMES: usize = 2;

pub(super) struct Publisher {
    free: Receiver<Box<Frame>>,
    ready: Sender<Box<Frame>>,
    // Failed sends retain their payload; dropping it here could retire the
    // last media reference on audio. Retry a full queue, retain on disconnect.
    pending: Option<Box<Frame>>,
    disconnected: bool,
    sequence: u64,
    empty_peaks: Arc<Vec<[f32; 3]>>,
    #[cfg(test)]
    published: Arc<AtomicU64>,
}

impl Publisher {
    pub fn new(snapshot: Arc<Mutex<Snapshot>>) -> Self {
        let (free_tx, free) = bounded(FRAMES);
        let (ready, ready_rx) = bounded::<Box<Frame>>(FRAMES);
        let empty_peaks = Arc::new(Vec::new());
        for _ in 0..FRAMES {
            free_tx
                .send(Box::new(Frame::new(empty_peaks.clone())))
                .unwrap();
        }
        let published = Arc::new(AtomicU64::new(0));
        let worker_published = published.clone();
        std::thread::Builder::new()
            .name("omatainer-snapshot".into())
            .spawn(move || {
                while let Ok(mut frame) = ready_rx.recv() {
                    if frame.complete {
                        let mut next = frame.materialize();
                        let mut current = snapshot.lock();
                        next.midi = std::mem::take(&mut current.midi);
                        *current = next;
                        worker_published.store(frame.sequence, Ordering::Release);
                    } else {
                        frame.prepare();
                    }
                    // All media refs have been taken/dropped by materialize
                    // on this worker. Audio only receives cleared ownership.
                    if free_tx.send(frame).is_err() {
                        break;
                    }
                }
            })
            .expect("start snapshot worker");
        Self {
            free,
            ready,
            pending: None,
            disconnected: false,
            sequence: 0,
            empty_peaks,
            #[cfg(test)]
            published,
        }
    }

    fn acquire(&mut self) -> Option<Box<Frame>> {
        if self.disconnected {
            return None;
        }
        if let Some(frame) = self.pending.take() {
            self.submit(frame);
            return None;
        }
        match self.free.try_recv() {
            Ok(mut frame) => {
                self.sequence += 1;
                frame.sequence = self.sequence;
                Some(frame)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                None
            }
        }
    }

    fn submit(&mut self, frame: Box<Frame>) {
        match self.ready.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(frame)) => self.pending = Some(frame),
            Err(TrySendError::Disconnected(frame)) => {
                self.disconnected = true;
                self.pending = Some(frame);
            }
        }
    }
}

struct Frame {
    values: Snapshot,
    samples: [Option<Arc<Sample>>; DECKS],
    timing: Option<Arc<midi_data::Conductor>>,
    track_names: Vec<usize>,
    scene_names: Vec<usize>,
    track_count: usize,
    scene_count: usize,
    clip_names: Vec<Vec<usize>>,
    deck_titles: [usize; DECKS],
    bank_names: Vec<usize>,
    bank_count: usize,
    fx_count: usize,
    fx_name_length: usize,
    complete: bool,
    sequence: u64,
}

fn reserve(value: &mut String, needed: usize) {
    value.reserve(needed.saturating_sub(value.len()));
}

fn copy(value: &mut String, source: &str) {
    debug_assert!(value.capacity() >= source.len());
    value.clear();
    value.push_str(source);
}

impl Frame {
    fn new(empty_peaks: Arc<Vec<[f32; 3]>>) -> Self {
        let values = Snapshot {
            session: Some(session::Layout::legacy((0..TRACKS).map(|_| String::new()), SCENES)),
            tracks: (0..TRACKS)
                .map(|_| TrackSnap {
                    clips: vec![ClipSnap::default(); SCENES],
                    ..TrackSnap::default()
                })
                .collect(),
            decks: (0..DECKS)
                .map(|_| DeckSnap {
                    peaks: empty_peaks.clone(),
                    ..DeckSnap::default()
                })
                .collect(),
            ..Snapshot::default()
        };
        Self {
            values,
            samples: std::array::from_fn(|_| None),
            timing: None,
            track_names: vec![0; session::MAX_TRACKS],
            scene_names: vec![0; session::MAX_SCENES],
            track_count: TRACKS, scene_count: SCENES,
            clip_names: vec![vec![0; session::MAX_SCENES]; session::MAX_TRACKS],
            deck_titles: [0; DECKS],
            bank_names: Vec::new(),
            bank_count: 0,
            fx_count: 0,
            fx_name_length: 16,
            complete: false,
            sequence: 0,
        }
    }

    // A capacity miss sends only growth requirements. Existing string storage
    // stays owned by this frame until the worker reserves more capacity.
    fn capture(&mut self, rt: &RtEngine) {
        debug_assert!(self.samples.iter().all(Option::is_none));
        debug_assert!(self.timing.is_none() && self.values.timing.is_none());
        self.complete = false;
        self.track_count = rt.session.tracks.len(); self.scene_count = rt.session.scenes.len();
        for (out, item) in self.scene_names.iter_mut().zip(&rt.session.scenes) { *out = item.name.len(); }
        let chain = if rt.fx_view >= 0 && rt.fx_view < rt.tracks.len() as i16 {
            &rt.tracks[rt.fx_view as usize].fx
        } else {
            &rt.scene_fx[if rt.fx_view >= crate::engine::session::SCENE_FX_BASE {
                (rt.fx_view as usize - crate::engine::session::SCENE_FX_BASE as usize).min(rt.scene_fx.len() - 1)
            } else {
                usize::from(rt.session.scene_order[0])
            }]
        };
        self.fx_count = chain.slots.len();
        self.bank_count = rt.sampler_banks.len();
        let mut fits = self.values.session.as_ref().is_some_and(|layout| layout.fits(&rt.session)) && self.values.tracks.len() == self.track_count && self.values.fx_slots.len() >= self.fx_count
            && self.values.sampler_banks.len() >= self.bank_count
            && self.values.sampler_instances.capacity() >= self.bank_count;
        self.fx_name_length = chain.slots.iter().map(|slot| slot.name().len()).max().unwrap_or(16).max(16);
        for (index, slot) in chain.slots.iter().enumerate() {
            fits &= self.values.fx_slots.get(index).is_some_and(|out| out.0.capacity() >= slot.name().len());
        }
        for (index, track) in rt.tracks.iter().take(self.track_count).enumerate() {
            self.track_names[index] = track.name.len().max(rt.session.tracks[index].name.len());
            fits &= self.values.tracks.get(index).is_some_and(|out| out.name.capacity() >= track.name.len() && out.clips.len() == self.scene_count);
            for (scene, clip) in track.clips.iter().take(self.scene_count).enumerate() {
                self.clip_names[index][scene] = clip.name.len();
                fits &= self.values.tracks.get(index).and_then(|t| t.clips.get(scene)).is_some_and(|out| out.name.capacity() >= clip.name.len());
            }
        }
        for (index, deck) in rt.decks.iter().enumerate() {
            self.deck_titles[index] = deck.title.len();
            fits &= self.values.decks[index].title.capacity() >= deck.title.len();
        }
        for ((needed, value), source) in self
            .bank_names
            .iter_mut()
            .zip(&self.values.sampler_banks)
            .zip(&rt.sampler_banks)
        {
            *needed = source.name().len();
            fits &= value.capacity() >= source.name().len();
        }
        if !fits {
            return;
        }

        let target = &mut self.values;
        target.session.as_mut().unwrap().copy_from_prepared(&rt.session);
        target.performance = rt.performance.status();
        let held = rt.note_recording.held_targets();
        for (track_index, (out, track)) in target.tracks.iter_mut().zip(&rt.tracks).enumerate() {
            copy(&mut out.name, &track.name);
            copy(&mut target.session.as_mut().unwrap().tracks[track_index].name, &track.name);
            out.gain = track.gain;
            out.pan = track.pan;
            out.mute = track.mute;
            out.solo = track.solo;
            out.armed = track.armed;
            out.meter = track.meter;
            out.playing_scene = track.playing.map(|p| p.scene as i16).unwrap_or(-1);
            out.clip_pending = track.playing.is_some_and(|p| p.last_beat < 0.0);
            out.clip_progress = track
                .playing
                .map(|p| {
                    if p.last_beat < 0.0 {
                        return 0.0;
                    }
                    let len = (track.clips[p.scene as usize].bars.max(0.25) * 4.0) as f64;
                    (p.last_beat.rem_euclid(len) / len) as f32
                })
                .unwrap_or(0.0);
            out.clip_looping = track.playing.is_some_and(|p| p.looping);
            for (scene_index, (out, clip)) in out.clips.iter_mut().zip(&track.clips).enumerate() {
                out.note_count = clip.notes.len();
                out.recording_held = held[(track_index * session::MAX_SCENES + scene_index) / 64] & (1u64 << ((track_index * session::MAX_SCENES + scene_index) % 64)) != 0;
                out.kind = match clip.kind {
                    ClipKind::Empty => 0,
                    ClipKind::Midi => 1,
                    ClipKind::Audio => 2,
                };
                copy(&mut out.name, &clip.name);
                out.bars = clip.bars;
                out.gain = clip_gain(clip.gain);
            }
        }
        for (index, (out, deck)) in target.decks.iter_mut().zip(&rt.decks).enumerate() {
            out.load_gate_word = rt.performance.deck_load_word(index);
            copy(&mut out.title, &deck.title);
            out.playing = deck.playing;
            out.previewing = deck.preview_position.is_some();
            out.media_active = deck.media_active();
            out.load_locked = rt.performance.deck_load_locked(index);
            out.media_key = deck.history_key;
            out.pos = deck.pos;
            out.frames = deck
                .audio
                .as_ref()
                .map(|a| a.frames() as f64)
                .unwrap_or(0.0);
            out.source_sample_rate = deck.audio.as_ref().map_or(0, |audio| audio.sr);
            out.playback_rate = deck.rate;
            out.touching = deck.touching;
            out.loop_start = deck.loop_start;
            out.loop_len = deck.loop_len;
            out.source_bpm = deck.audio.as_ref().map_or(0.0, |audio| audio.bpm);
            out.bpm = deck.musical_bpm();
            out.pitch = deck.pitch;
            out.gain = deck.gain;
            out.eq = [deck.eq[0].low_g, deck.eq[0].mid_g, deck.eq[0].high_g];
            out.filter = deck.filter_amt;
            out.vinyl = deck.vinyl;
            out.sync = deck.sync;
            out.keylock = deck.keylock;
            out.keylock_mode = deck.keylock_mode();
            out.pfl = deck.pfl;
            out.loop_on = deck.loop_on;
            out.hotcues = std::array::from_fn(|i| deck.hotcues[i].set);
            out.hotcue_positions = std::array::from_fn(|i| deck.hotcues[i].set.then_some(deck.hotcues[i].pos));
            out.cue_styles = deck.cue_styles;
            out.grid = deck.grid;
            out.receipt_key = deck.load_receipt.as_ref().map_or(0, load_receipt::Receipt::snapshot_key);
            out.meter = deck.meter;
            out.duration = deck
                .audio
                .as_ref()
                .map(|a| {
                    if a.sr == 0 {
                        0.0
                    } else {
                        a.frames() as f32 / a.sr as f32
                    }
                })
                .unwrap_or(0.0);
            out.eq_cut = deck.eq_cut;
            out.eq_solo = deck.eq_solo;
            out.pitch_range = deck.pitch_range;
            // Only clone references after every capacity check passes. The
            // worker always clears them before recycling this frame.
            self.samples[index] = deck.audio.clone();
        }
        target.playing = rt.playing;
        target.midi_clock = rt.midi_clock;
        target.recording = rt.recording;
        target.bpm = rt.bpm;
        target.beat = rt.beat;
        target.timeline_seconds = rt.timeline_seconds();
        target.file_conductor = rt.conductor.is_some();
        target.count_in_remaining = rt.count_in.as_ref().map_or(0.0, |count| count.remaining());
        if let Some(conductor) = &rt.conductor {
            let (bar, beat, meter) = conductor.position(rt.precise_midi_beat());
            target.bar = bar; target.beat_in_bar = beat;
            target.meter_numerator = meter.numerator;
            target.meter_denominator = 1u16 << meter.denominator_power;
        } else {
            target.bar = (rt.beat / 4.0).floor() as u32 + 1;
            target.beat_in_bar = (rt.beat % 4.0) as f32;
            target.meter_numerator = 4; target.meter_denominator = 4;
        }
        target.master = rt.master;
        target.xfader = rt.xfader;
        target.cue_mix = rt.cue_mix;
        target.view = match rt.view {
            View::Session => 0,
            View::Arrange => 1,
            View::Compose => 2,
        };
        target.selected_track = rt.selected_track;
        target.selected_scene = rt.selected_scene;
        target.compose_target = rt.compose_target;
        target.project_revision = rt.project.revision();
        target.sample_rate = rt.sr as u32;
        target.transport_epoch = rt.transport_epoch;
        target.selected_deck = rt.selected_deck;
        target.selected_deck_request = rt.selected_deck_request;
        target.audio = rt.telemetry.read();
        target.cpu = target.audio.last_callback.and_then(|sample| sample.render_cpu_fraction()).map(|value| value as f32);
        target.commands = rt.command_stats;
        target.submissions = rt.cmd_rx.submissions();
        target.fx_kind = rt.fx_kind;
        target.fx_wet = rt.fx_wet;
        target.metronome = rt.metronome;
        target.quant = rt.quant;
        target.quantize = rt.quantize;
        target.sampler_bank = rt.sampler_bank;
        target.sampler_inst = rt.sampler_inst;
        target.sampler_unavailable = rt.sampler_poly.offline.is_some() && rt.sampler_inst.synth().is_some();
        target.sampler_oct = rt.sampler_oct;
        target.sampler_revision = rt.sampler_revision;
        target.sampler_epoch = rt.undo.checkpoint().epoch;
        target.sampler_audition = rt.sampler_audition.as_ref().map(|active| active.id);
        target.sampler_instances.clear();
        target.sampler_instances.extend(rt.sampler_banks.iter().cloned());
        for (out, source) in target.sampler_banks.iter_mut().zip(&rt.sampler_banks) {
            copy(out, source.name());
        }
        target.fx_view = rt.fx_view;
        for (out, slot) in target.fx_slots.iter_mut().zip(&chain.slots) {
            copy(&mut out.0, slot.name());
            out.1 = slot.on;
            out.2 = slot.mix;
            out.3 = slot.p;
        }
        self.timing = rt.conductor.clone();
        self.complete = true;
    }

    fn prepare(&mut self) {
        self.values.tracks.resize_with(self.track_count, TrackSnap::default);
        for track in &mut self.values.tracks { track.clips.resize_with(self.scene_count, ClipSnap::default); }
        self.values.session.as_mut().unwrap().prepare_storage(&self.track_names[..self.track_count], &self.scene_names[..self.scene_count]);
        for (index, track) in self.values.tracks.iter_mut().enumerate() {
            reserve(&mut track.name, self.track_names[index]);
            for (scene, clip) in track.clips.iter_mut().enumerate() {
                reserve(&mut clip.name, self.clip_names[index][scene]);
            }
        }
        for (index, deck) in self.values.decks.iter_mut().enumerate() {
            reserve(&mut deck.title, self.deck_titles[index]);
        }
        if self.values.sampler_banks.len() < self.bank_count {
            self.values
                .sampler_banks
                .resize_with(self.bank_count, String::new);
            self.bank_names.resize(self.bank_count, 128);
        }
        self.values.sampler_instances.reserve(self.bank_count.saturating_sub(self.values.sampler_instances.len()));
        for (out, needed) in self.values.sampler_banks.iter_mut().zip(&self.bank_names) {
            reserve(out, *needed);
        }
        if self.values.fx_slots.len() < self.fx_count {
            self.values
                .fx_slots
                .resize_with(self.fx_count, Default::default);
        }
        for slot in &mut self.values.fx_slots {
            reserve(&mut slot.0, self.fx_name_length);
        }
    }

    // Only called before callback ownership or on the snapshot worker.
    fn materialize(&mut self) -> Snapshot {
        let mut next = self.values.clone();
        // Ownership travels once to the snapshot worker. Audio receives a
        // cleared frame and cannot retire the last old timeline reference.
        next.timing = self.timing.take();
        next.sampler_banks.truncate(self.bank_count);
        next.fx_slots.truncate(self.fx_count);
        for (deck, sample) in next.decks.iter_mut().zip(&mut self.samples) {
            if let Some(sample) = sample.take() {
                // One shared reference for the published snapshot, never a peak
                // vector copy. The temporary media owner also retires here.
                deck.peaks = sample.peaks.clone();
            }
            // Otherwise retain the shared empty waveform captured by new().
        }
        next
    }
}

impl RtEngine {
    pub fn publish(&mut self) {
        self.project.publish_timeline(self.timeline_seconds());
        #[cfg(test)]
        std::thread::sleep(self.telemetry_delays[2]);
        if let Some(mut frame) = self.publisher.acquire() {
            frame.capture(self);
            self.publisher.submit(frame);
        }
    }

    // Synchronous bootstrap is outside the callback. UI and IPC start with a
    // complete initial state, then accept asynchronously refreshed snapshots.
    pub(super) fn publish_initial(&self) {
        let mut frame = Frame::new(self.publisher.empty_peaks.clone());
        loop {
            frame.capture(self);
            if frame.complete {
                break;
            }
            frame.prepare();
        }
        let mut next = frame.materialize();
        let mut current = self.snap.lock();
        next.midi = std::mem::take(&mut current.midi);
        *current = next;
    }

    #[cfg(test)]
    pub(crate) fn publish_for_test(&mut self) {
        let target = self.publisher.sequence + 1;
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.publisher.published.load(Ordering::Acquire) < target {
            self.publish();
            assert!(Instant::now() < deadline, "snapshot worker did not publish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod waveform_tests;
