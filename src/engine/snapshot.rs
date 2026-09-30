//! Two reusable frames cross from audio to a snapshot worker. Readers can hold
//! the public mutex indefinitely: audio runs out of frames and skips updates.
//! Only the worker grows buffers, builds UI values and retires old payloads.

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
    #[cfg(test)]
    published: Arc<AtomicU64>,
}

impl Publisher {
    pub fn new(snapshot: Arc<Mutex<Snapshot>>) -> Self {
        let (free_tx, free) = bounded(FRAMES);
        let (ready, ready_rx) = bounded::<Box<Frame>>(FRAMES);
        for _ in 0..FRAMES {
            free_tx.send(Box::new(Frame::new())).unwrap();
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
    track_names: [usize; TRACKS],
    clip_names: [[usize; SCENES]; TRACKS],
    deck_titles: [usize; DECKS],
    bank_names: Vec<usize>,
    bank_count: usize,
    fx_count: usize,
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
    fn new() -> Self {
        let values = Snapshot {
            tracks: (0..TRACKS)
                .map(|_| TrackSnap {
                    clips: vec![ClipSnap::default(); SCENES],
                    ..TrackSnap::default()
                })
                .collect(),
            decks: vec![DeckSnap::default(); DECKS],
            ..Snapshot::default()
        };
        Self {
            values,
            samples: std::array::from_fn(|_| None),
            track_names: [0; TRACKS],
            clip_names: [[0; SCENES]; TRACKS],
            deck_titles: [0; DECKS],
            bank_names: Vec::new(),
            bank_count: 0,
            fx_count: 0,
            complete: false,
            sequence: 0,
        }
    }

    // A capacity miss sends only growth requirements. Existing string storage
    // stays owned by this frame until the worker reserves more capacity.
    fn capture(&mut self, rt: &RtEngine) {
        debug_assert!(self.samples.iter().all(Option::is_none));
        self.complete = false;
        let chain = if rt.fx_view >= 0 && rt.fx_view < TRACKS as i16 {
            &rt.tracks[rt.fx_view as usize].fx
        } else {
            &rt.scene_fx[if rt.fx_view >= 100 {
                (rt.fx_view as usize - 100).min(SCENES - 1)
            } else {
                0
            }]
        };
        self.fx_count = chain.slots.len();
        self.bank_count = rt.sampler_banks.len();
        let mut fits = self.values.fx_slots.len() >= self.fx_count
            && self.values.sampler_banks.len() >= self.bank_count;
        for (index, track) in rt.tracks.iter().enumerate() {
            self.track_names[index] = track.name.len();
            fits &= self.values.tracks[index].name.capacity() >= track.name.len();
            for (scene, clip) in track.clips.iter().enumerate() {
                self.clip_names[index][scene] = clip.name.len();
                fits &= self.values.tracks[index].clips[scene].name.capacity() >= clip.name.len();
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
            *needed = source.len();
            fits &= value.capacity() >= source.len();
        }
        if !fits {
            return;
        }

        let target = &mut self.values;
        for (out, track) in target.tracks.iter_mut().zip(&rt.tracks) {
            copy(&mut out.name, &track.name);
            out.gain = track.gain;
            out.pan = track.pan;
            out.mute = track.mute;
            out.solo = track.solo;
            out.armed = track.armed;
            out.meter = track.meter;
            out.playing_scene = track.playing.map(|p| p.scene as i8).unwrap_or(-1);
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
            for (out, clip) in out.clips.iter_mut().zip(&track.clips) {
                out.kind = match clip.kind {
                    ClipKind::Empty => 0,
                    ClipKind::Midi => 1,
                    ClipKind::Audio => 2,
                };
                copy(&mut out.name, &clip.name);
                out.bars = clip.bars;
            }
        }
        for (index, (out, deck)) in target.decks.iter_mut().zip(&rt.decks).enumerate() {
            copy(&mut out.title, &deck.title);
            out.playing = deck.playing;
            out.pos = deck.pos;
            out.frames = deck
                .audio
                .as_ref()
                .map(|a| a.frames() as f64)
                .unwrap_or(0.0);
            out.bpm = deck.bpm;
            out.pitch = deck.pitch;
            out.gain = deck.gain;
            out.eq = [deck.eq[0].low_g, deck.eq[0].mid_g, deck.eq[0].high_g];
            out.filter = deck.filter_amt;
            out.vinyl = deck.vinyl;
            out.sync = deck.sync;
            out.keylock = deck.keylock;
            out.pfl = deck.pfl;
            out.loop_on = deck.loop_on;
            out.hotcues = std::array::from_fn(|i| deck.hotcues[i].set);
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
        target.recording = rt.recording;
        target.bpm = rt.bpm;
        target.beat = rt.beat;
        target.bar = (rt.beat / 4.0).floor() as u32 + 1;
        target.beat_in_bar = (rt.beat % 4.0) as f32;
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
        target.selected_deck = rt.selected_deck;
        target.cpu = rt.cpu_acc;
        target.commands = rt.command_stats;
        target.submissions = rt.cmd_rx.submissions();
        target.fx_wet = rt.fx_wet;
        target.metronome = rt.metronome;
        target.quant = rt.quant;
        target.quantize = rt.quantize;
        target.sampler_bank = rt.sampler_bank;
        target.sampler_inst = rt.sampler_inst;
        target.sampler_oct = rt.sampler_oct;
        for (out, source) in target.sampler_banks.iter_mut().zip(&rt.sampler_banks) {
            copy(out, source);
        }
        target.fx_view = rt.fx_view;
        for (out, slot) in target.fx_slots.iter_mut().zip(&chain.slots) {
            copy(&mut out.0, slot.id().name());
            out.1 = slot.on;
            out.2 = slot.mix;
            out.3 = slot.p;
        }
        self.complete = true;
    }

    fn prepare(&mut self) {
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
        for (out, needed) in self.values.sampler_banks.iter_mut().zip(&self.bank_names) {
            reserve(out, *needed);
        }
        if self.values.fx_slots.len() < self.fx_count {
            self.values
                .fx_slots
                .resize_with(self.fx_count, Default::default);
        }
        for slot in &mut self.values.fx_slots {
            reserve(&mut slot.0, 16);
        }
    }

    // Only called before callback ownership or on the snapshot worker.
    fn materialize(&mut self) -> Snapshot {
        let mut next = self.values.clone();
        next.sampler_banks.truncate(self.bank_count);
        next.fx_slots.truncate(self.fx_count);
        for (deck, sample) in next.decks.iter_mut().zip(&mut self.samples) {
            deck.peaks = Arc::new(
                sample
                    .take()
                    .map(|sample| sample.peaks.clone())
                    .unwrap_or_default(),
            );
        }
        next
    }
}

impl RtEngine {
    pub fn publish(&mut self) {
        if let Some(mut frame) = self.publisher.acquire() {
            frame.capture(self);
            self.publisher.submit(frame);
        }
    }

    // Synchronous bootstrap is outside the callback. UI and IPC start with a
    // complete initial state, then accept asynchronously refreshed snapshots.
    pub(super) fn publish_initial(&self) {
        let mut frame = Frame::new();
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
    pub(super) fn publish_for_test(&mut self) {
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
