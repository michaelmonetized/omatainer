pub(crate) mod keylock;
#[cfg(test)]
mod keylock_tests;
pub(crate) mod project;
pub(crate) mod live_set;
pub(crate) mod midi_edit;
pub(crate) mod midi_data;
pub(crate) mod midi_interchange;
pub(crate) mod undo;
mod mixer_gain;
pub(crate) mod monitor;
#[cfg(test)]
mod track_gain_tests;
mod video_transport;
pub(crate) mod provider_preview;
mod arp;
mod deck_filter;
#[cfg(test)]
mod deck_filter_tests;

pub mod instrument;
pub(crate) mod sampler_pad;
pub(crate) mod sampler;
#[cfg(test)]
pub(crate) mod sampler_identity_tests;
pub use instrument::{SamplerInstrument, SynthInstrument};
mod recording;
#[cfg(test)]
mod arp_tests;
#[cfg(test)]
mod fx_allocation_tests;
#[cfg(test)]
mod fx_control_tests;
#[cfg(test)]
mod mute_lifecycle_tests;
#[cfg(test)]
pub(crate) mod test_alloc;
pub mod audio;
pub mod audio_metrics;
pub(crate) mod history_measurement;
pub mod performance;
pub(crate) mod remote;
pub(crate) mod diagnostics;
mod master_fx;
#[cfg(test)]
mod master_fx_tests;
mod control;
pub use control::{CommandPort, SubmissionError, SubmissionOutcome};
pub(crate) use control::{CommandStats, SubmissionStats, QueuePressure};
pub(crate) mod ui_requests;
pub(crate) mod media_source;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod quantized_launch_tests;
pub mod decode;
pub mod dsp;
#[cfg(test)]
mod svf_tests;
pub mod media_load;
pub(crate) mod media_analysis;
pub(crate) mod media_health;
pub(crate) mod load_receipt;
pub(crate) mod preparation;
pub(crate) mod cue_metadata;
pub(crate) mod beatgrid;
#[cfg(test)]
mod keylock_quality_tests;
#[cfg(test)]
mod keylock_showload_tests;
pub mod fx;
pub mod midi;
#[cfg(test)]
mod master_stereo_tests;
mod midi_schedule;
mod metronome;
#[cfg(test)]
mod midi_schedule_tests;
#[cfg(test)]
mod sample_rate_tests;
mod snapshot;
pub(crate) mod audible;
pub mod session;
#[cfg(test)]
mod scene_stereo_tests;
#[cfg(test)]
mod scene_ownership_tests;

#[cfg(test)]
mod clip_lifecycle_tests;
#[cfg(test)]
mod deck_loop_tests;
#[cfg(test)]
mod deck_stereo_tests;
#[cfg(test)]
mod deck_transition_tests;
#[cfg(test)]
mod input_ownership_tests;
#[cfg(test)]
mod pad_routing_tests;
#[cfg(test)]
mod stopped_deck_tests;
#[cfg(test)]
mod recording_position_tests;
#[cfg(test)]
mod recording_duration_tests;
#[cfg(test)]
mod compose_tests;

use crate::engine::dsp::{
    detect_bpm, limiter, peaks_3band, resample_mono, synth_drum, xfader_gains, Poly,
    InputKey, Sample, ThreeBand,
};
#[cfg(test)]
use crate::engine::dsp::{Delay, Reverb};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const TRACKS: usize = 8;
pub const SCENES: usize = 8;
pub const DECKS: usize = 2;
pub const HOTCUES: usize = 8;

#[derive(Clone, Copy)]
enum DeckTransition {
    Jump,
    Jog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Session,
    Arrange,
    Compose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipKind {
    Empty,
    Midi,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MidiNote {
    #[serde(default)]
    pub channel:u8,
    #[serde(default="midi_data::release_velocity")]
    pub release_vel:u8,
    #[serde(default)]
    pub source_timing:Option<midi_data::TickTiming>,
    #[serde(default)]
    pub id: midi_edit::NoteId,
    #[serde(default)]
    pub muted: bool,
    pub pitch: u8,
    pub start: f32,
    pub len: f32,
    pub vel: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    #[serde(default)]
    pub(crate) lanes: Option<Arc<midi_data::Lanes>>,
    #[serde(default)]
    pub region: Option<midi_edit::Region>,
    pub kind: ClipKind,
    pub name: String,
    pub bars: f32,
    pub notes: Vec<MidiNote>,
    pub gain: f32,
    #[serde(skip)]
    pub audio: Option<Arc<Sample>>,
}

impl Clip {
    pub(super) fn retained_bytes(&self) -> usize {
        self.name.capacity() + self.notes.capacity() * std::mem::size_of::<MidiNote>()
            + self.lanes.as_ref().map_or(0, |lanes| lanes.bytes())
    }

    pub fn empty() -> Self {
        Self {
            lanes: None,
            region: None,
            kind: ClipKind::Empty,
            name: String::new(),
            bars: 1.0,
            notes: Vec::new(),
            gain: 1.0,
            audio: None,
        }
    }
    pub fn occupied(&self) -> bool {
        self.kind != ClipKind::Empty
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlayingClip {
    pub scene: u16,
    pub start_beat: f64,
    // Explicit MIDI regions use the compensated clock; legacy clips retain
    // their original transport and rendered audio.
    midi_start_beat: f64,
    pub last_beat: f64,
    pub looping: bool,
}

/// A finite drum hit captures its clip level when triggered. Live hits use unity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DrumVoice {
    pub sample: usize,
    pub position: f64,
    pub clip_gain: f32,
    pub velocity: f32,
}

#[derive(Clone, Debug)]
pub struct TrackRt {
    pub name: String,
    pub clips: Vec<Clip>,
    pub playing: Option<PlayingClip>,
    project_resume: Option<PlayingClip>,
    // The bus stays selected through stops/tails until a new clip starts.
    pub scene_bus: usize,
    pub gain: f32,
    pub pan: f32,
    mixer_gain: mixer_gain::GainPair,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub kind: u8, // 0 drums 1 bass 2 keys 3 pad 4 audio
    pub poly: Poly,
    pub eq: ThreeBand,
    eq_right: ThreeBand,
    pub meter: f32,
    pub drum_samples: [Arc<Sample>; 6],
    pub drum_pos: [Option<DrumVoice>; 16],
    pub fx: fx::FxChain,
    pub arp_note: Option<u8>,
    arp_cache: arp::ChordCache,
    midi_schedule: midi_schedule::MidiSchedule,
    midi_output: midi::routing::playback::Playback,
    // Capture playback policy belongs only to the current launch.
    recorded_playback: Vec<Option<recording::RecordedPlayback>>,
}

impl TrackRt {
    /// Worker/setup only. Reserve structural growth before the graph reaches audio.
    fn empty(sr: f32, name: String, kind: u8, drums: [Arc<Sample>; 6], scenes: usize) -> Self {
        let mut clips: Vec<_> = (0..scenes).map(|_| Clip::empty()).collect();
        clips.reserve(session::MAX_SCENES - clips.len());
        Self {
            name, clips, playing: None, project_resume: None, scene_bus: 0,
            gain: 0.8, pan: 0.0, mixer_gain: mixer_gain::GainPair::default(),
            mute: false, solo: false, armed: false, kind,
            poly: Poly::new(sr, match kind { 0 => SynthInstrument::Analog, 1 => SynthInstrument::Keys, _ => SynthInstrument::Pad }, 8),
            eq: ThreeBand::new(sr), eq_right: ThreeBand::new(sr), meter: 0.0,
            drum_samples: drums, drum_pos: [None; 16], fx: fx::FxChain::new(sr),
            arp_note: None, arp_cache: arp::ChordCache::default(),
            midi_schedule: midi_schedule::MidiSchedule::default(),
            midi_output: midi::routing::playback::Playback::default(),
            recorded_playback: Vec::new(),
        }
    }
    pub(super) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.name.capacity()
            + self.clips.capacity() * std::mem::size_of::<Clip>()
            + self.clips.iter().map(Clip::retained_bytes).sum::<usize>()
            + self.poly.voices.capacity() * std::mem::size_of::<dsp::Voice>()
            + self.poly.filters.capacity() * std::mem::size_of::<dsp::Svf>()
            + self.poly.offline.as_ref().map_or(0, |device| device.bytes())
            + self.fx.retained_bytes() + self.midi_schedule.retained_bytes()
            + self.recorded_playback.capacity() * std::mem::size_of::<Option<recording::RecordedPlayback>>()
    }

    fn clip_notes_changed(&mut self, scene: usize, beat: f64, midi_beat: f64) {
        if self.playing.or(self.project_resume).is_some_and(|p| p.scene as usize == scene) {
            self.midi_output.invalidate();
            self.arp_cache.invalidate();
            let arp_active = self.midi_schedule.paused;
            self.rebuild_midi_schedule(beat, midi_beat);
            self.midi_schedule.paused = arp_active;
        }
    }

    fn rebuild_midi_schedule(&mut self, beat: f64, midi_beat: f64) {
        if let Some(playing) = self.playing.or(self.project_resume) {
            let clip = &self.clips[playing.scene as usize];
            let elapsed = (self.project_resume.is_some() || playing.last_beat >= 0.0)
                .then_some(if clip.region.is_some() { midi_beat - playing.midi_start_beat } else { beat - playing.start_beat });
            self.midi_schedule.rebuild_region(
                &clip.notes,
                clip.bars.max(0.25) as f64 * 4.0,
                elapsed,
                playing.looping,
                &self.recorded_playback,
                clip.region,
            );
        }
    }

    fn release_clip_notes(&mut self) {
        self.midi_output.invalidate();
        self.poly.release_clip();
        self.arp_note = None;
        self.arp_cache.invalidate();
        self.midi_schedule.reset();
        self.recorded_playback.clear();
    }

    fn stop_clip(&mut self) {
        self.playing = None;
        self.project_resume = None;
        self.release_clip_notes();
        // Drum samples are finite one-shots and keep their natural tails.
    }
}

#[derive(Clone, Debug)]
pub struct HotCue {
    pub set: bool,
    pub pos: f64,
}

#[derive(Clone, Debug)]
pub struct DeckRt {
    load_receipt: Option<load_receipt::Receipt>,
    playback_active: bool,
    pub audio: Option<Arc<Sample>>,
    pub pos: f64,
    pub rate: f32,
    pub target_rate: f32,
    pub pitch: f32, // -1..1 mapped around 1.0
    pub playing: bool,
    preview_position: Option<f64>,
    pub cue_pos: f64,
    pub touching: bool,
    // Admission cannot hold more than MAX_COMMANDS gates across all inputs.
    // Matching that bound per deck guarantees every accepted owner fits here.
    touch_sources: [Option<u64>; control::MAX_COMMANDS],
    pub vinyl: bool,
    pub keylock: bool,
    pub sync: bool,
    pub gain: f32,
    pub eq: [ThreeBand; 2],
    pub(crate) source_gain: crate::track_gain::Resolved,
    pub filter: [deck_filter::ChannelFilter; 2],
    filter_position: f32,
    pub filter_morph: f32, // 0.5 = bypass-ish, 0 LP 1 HP. 0.5 + offset
    pub filter_amt: f32,   // 0.5 = noon
    pub pfl: bool,
    pub hotcues: [HotCue; HOTCUES],
    pub cue_styles: [cue_metadata::Style; HOTCUES],
    pub grid: Option<beatgrid::Grid>,
    pub loop_on: bool,
    pub loop_start: f64,
    pub loop_len: f64,
    pub bpm: f32,
    pub meter: f32,
    pub title: String,
    pub scratch: f32,
    pub eq_cut: [bool; 4],
    pub eq_solo: i8,
    pub eq_store: [f32; 4],
    pub pitch_range: u8,
    pub sync_bpm: f32,
    keylock_dsp: keylock::Processor,
    keylock_render_mode: keylock::Mode,
    rate_smoothing: f32,
    last_output: [f32; 2],
    history_key: u64,
    history_last: history_measurement::parts::Parts,
    history_transition: history_measurement::parts::Parts,
    transition_from: [f32; 2],
    transition_remaining: u32,
    transition_frames: u32,
}

impl DeckRt {
    /// Check whether this deck is rendering source audio.
    /// Takes this deck; returns true for play, touch or an explicitly admitted paused preview.
    fn rendering(&self) -> bool { self.playing || self.touching || self.preview_position.is_some() }
    /// Keep media replacement guarded while this deck renders or fades.
    /// Takes its renderer state; returns whether load protection still owns audible activity.
    fn media_active(&self) -> bool {
        self.rendering() || self.last_output.iter().any(|sample| sample.abs() > 0.0001)
    }
    /// Keep source trim fixed until its audible state settles.
    /// Takes this deck; returns true for rendering, retained output or an outgoing nonzero fade.
    fn source_gain_active(&self) -> bool {
        self.rendering() || self.last_output.iter().any(|sample| *sample != 0.0)
            || self.transition_remaining > 0 && self.transition_from.iter().any(|sample| *sample != 0.0)
    }
    fn stop_preview(&mut self, rate: f32) {
        if let Some(position) = self.preview_position.take() { self.transition_to(position, rate, DeckTransition::Jump); }
    }
    fn clear_loop(&mut self) {
        self.loop_on = false;
        self.loop_start = 0.0;
        self.loop_len = 0.0;
    }

    fn new(sr: f32) -> Self {
        Self {
            load_receipt: None,
            playback_active: false,
            audio: None,
            pos: 0.0,
            rate: 1.0,
            target_rate: 1.0,
            pitch: 0.5,
            playing: false,
            preview_position: None,
            cue_pos: 0.0,
            touching: false,
            touch_sources: [None; control::MAX_COMMANDS],
            vinyl: true,
            keylock: false,
            sync: false,
            gain: 0.85,
            source_gain: crate::track_gain::Resolved::default(),
            eq: [ThreeBand::new(sr); 2],
            filter: [deck_filter::ChannelFilter::default(); 2],
            filter_position: 0.5,
            filter_morph: 0.5,
            filter_amt: 0.5,
            pfl: false,
            cue_styles: [cue_metadata::Style::default(); HOTCUES],
            grid: None,
            hotcues: std::array::from_fn(|_| HotCue {
                set: false,
                pos: 0.0,
            }),
            loop_on: false,
            loop_start: 0.0,
            loop_len: 0.0,
            bpm: 124.0,
            meter: 0.0,
            title: String::new(),
            scratch: 0.0,
            eq_cut: [false; 4],
            eq_solo: -1,
            eq_store: [1.0, 1.0, 1.0, 0.85],
            pitch_range: 0,
            sync_bpm: 124.0,
            keylock_dsp: keylock::Processor::new(sr),
            keylock_render_mode: keylock::Mode::Off,
            rate_smoothing: dsp::rate_blend(0.08, sr),
            last_output: [0.0; 2],
            history_key: 0,
            history_last: Default::default(),
            history_transition: Default::default(),
            transition_from: [0.0; 2],
            transition_remaining: 0,
            transition_frames: 0,
        }
    }

    /// All playhead jumps and source-mode changes reset the overlap history.
    /// Jumps blend from the last emitted frame for ceil(2 ms * output rate)
    /// frames while playback timing advances normally. Jogging is a continuous
    /// performance gesture: update its source immediately without restarting a
    /// fade or clearing its filter history for every controller message.
    fn transition_to(&mut self, pos: f64, sr: f32, transition: DeckTransition) {
        self.pos = pos;
        let source_rate = self.audio.as_ref().map_or(sr, |audio| audio.sr as f32);
        self.keylock_dsp.reset(pos, source_rate as f64 / sr as f64);
        self.keylock_render_mode = self.keylock_mode();
        match transition {
            DeckTransition::Jump => {
                self.fade_from_last_output(sr);
                for eq in &mut self.eq {
                    eq.low.z = 0.0;
                    eq.high.z = 0.0;
                }
                self.filter = [deck_filter::ChannelFilter::default(); 2];
            }
            DeckTransition::Jog => self.transition_remaining = 0,
        }
    }

    fn fade_from_last_output(&mut self, sr: f32) {
        self.history_transition = self.history_last;
        self.transition_from = self.last_output;
        self.transition_frames = (sr as f64 * 0.002).ceil().max(2.0) as u32;
        self.transition_remaining = self.transition_frames;
    }

    fn sample_at(&self, pos: f64) -> (f32, f32) {
        let Some(audio) = &self.audio else { return (0.0, 0.0) };
        keylock::Source {
            audio,
            loop_on: self.loop_on,
            loop_start: self.loop_start,
            loop_len: self.loop_len,
        }.at(pos)
    }

    fn transition_output(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut output = input;
        let mut history_mix = 1.0;
        if self.transition_remaining > 0 {
            let mix = (self.transition_frames - self.transition_remaining) as f32
                / (self.transition_frames - 1) as f32;
            for channel in 0..2 {
                output[channel] = self.transition_from[channel] * (1.0 - mix)
                    + input[channel] * mix;
            }
            history_mix = mix;
            self.transition_remaining -= 1;
        }
        self.history_last = history_measurement::parts::Parts::transition(
            self.history_transition, self.history_key, input, history_mix);
        self.last_output = output;
        output
    }

    /// Shared by rendering and publication: an armed but stopped/empty deck
    /// has no active rate to qualify and must not display a fallback warning.
    fn keylock_mode(&self) -> keylock::Mode {
        if !self.keylock {
            keylock::Mode::Off
        } else if self.audio.is_none() {
            keylock::Mode::NoMedia
        } else if !self.rendering() {
            keylock::Mode::Stopped
        } else {
            keylock::mode(true, self.touching, self.rate)
        }
    }

    fn pitch_rate(&self) -> f32 {
        let span = match self.pitch_range {
            1 => 0.16,
            2 => 0.50,
            _ => 0.08,
        };
        1.0 + (self.pitch - 0.5) * 2.0 * span
    }

    fn play_rate(&self) -> f32 {
        if self.rendering() {
            self.pitch_rate()
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxKind {
    Echo,
    Reverb,
    Filter,
}

impl FxKind {
    pub fn name(self) -> &'static str { match self { Self::Echo => "Echo", Self::Reverb => "Reverb", Self::Filter => "Filter" } }
    fn next(self) -> Self { match self { Self::Echo => Self::Reverb, Self::Reverb => Self::Filter, Self::Filter => Self::Echo } }
}

pub struct RtEngine {
    midi_routing: Arc<midi::routing::Shared>,
    midi_output_mask:u128,
    midi_output_budget:usize,
    undo: undo::Journal,
    pub project: project::Handle,
    pub performance: performance::Handle,
    safety_output: performance::Output,
    project_pending: Option<Box<project::Task>>,
    project_waiting: Option<Box<project::Task>>,
    project_sealed: bool,
    live_set: Option<Box<live_set::Stage>>,
    pub sr: f32,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub beat: f64,
    pub(crate) transport_epoch: u64,
    timeline_anchor: f64,
    timeline_frames: u64,
    remote_schedule: remote::Schedule,
    provider_preview: Option<provider_preview::Active>,
    beat_roundoff: f64,
    midi_beat: f64,
    midi_beat_reference: f64,
    #[cfg(test)]
    current_sample_frame: u64,
    pub(crate) conductor: Option<Arc<midi_data::Conductor>>,
    last_midi_step: f64,
    pub quant: f32,
    pub view: View,
    pub xfader: f32,
    pub xfader_curve: f32,
    xfader_gain: mixer_gain::GainPair,
    #[cfg(test)]
    legacy_gain_math: bool,
    pub master: f32,
    pub cue_mix: f32,
    monitor: monitor::Monitor,
    pub tracks: Vec<Box<TrackRt>>,
    pub session: session::Layout,
    pub decks: [DeckRt; DECKS],
    // Each control owns its selected processor and both channel histories.
    master_fx: [master_fx::MasterSlot; 3],
    history_measurement: Option<history_measurement::capture::Measurement>,
    pub fx_kind: [FxKind; 3],
    pub fx_wet: [f32; 3],
    pub tap: Vec<Instant>,
    pub selected_track: usize,
    pub selected_scene: usize,
    pub selected_deck: usize,
    pub selected_deck_request: u64,
    pub cmd_rx: control::CommandReceiver,
    pub command_stats: control::CommandStats,
    command_batch: Box<control::CommandBatch>,
    pub snap: Arc<Mutex<Snapshot>>,
    publisher: snapshot::Publisher,
    audible: audible::Writer,
    pub midi_clock: MidiClockInput,
    #[cfg(test)]
    pub(crate) load_test_hooks: [Option<Box<dyn FnOnce() + Send>>; 2],
    telemetry: Arc<audio_metrics::Telemetry>,
    render_cpu_ns: Option<u64>,
    load_profile: diagnostics::FrameProfile,
    #[cfg(test)]
    telemetry_delays: [Duration; 3],
    frames_done: u64,
    note_recording: recording::Recording,
    metronome: bool,
    metro: metronome::Click,
    count_in: Option<metronome::CountIn>,
    pub(crate) routing: Option<Box<audio::routing::prepared::Prepared>>,
    routing_track_input: Option<[f32; 2]>,
    routing_track_taps: [[f32; 2]; 3],
    routing_deck_taps: [[f32; 2]; 2],
    pub(crate) routing_input_frame: [f32; audio::routing::model::MAX_PHYSICAL_CHANNELS],
    pub(crate) routing_pipe: audio::routing::input::Pipe,
    routing_probe: audio::routing::probe::Probe,
    pub quantize: bool,
    pub sampler_bank: usize,
    pub sampler_inst: SamplerInstrument,
    pub sampler_oct: i8,
    pub sampler_poly: Poly,
    pub sampler_banks: Vec<sampler::Bank>,
    pub sampler_revision: u64,
    // Renderer lifetime ownership; never swapped or finally dropped by an
    // audio callback when installing a separately prepared project graph.
    sampler_assets: crate::sampler_bank::assets::Owner,
    pub pad_voices: [Option<crate::sampler_bank::resident::Voice>; 16],
    pub sampler_audition: Option<sampler::ActiveAudition>,
    pad_destinations: [usize; 16],
    pad_output: [[f32; 2]; session::MAX_TRACKS],
    pad_targets: [Option<PadTarget>; 16],
    pub builtin: [Option<Arc<Sample>>; 2],
    builtin_levels: [Option<crate::track_gain::Level>; 2],
    pub fx_view: i16,
    pub scene_fx: Vec<fx::FxChain>,
    pub compose_target: Option<ComposeTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeTarget {
    pub track: usize,
    pub scene: usize,
}

#[derive(Clone, Copy, Debug)]
struct PadTarget {
    track: usize,
    pitch: u8,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DeckSnap {
    pub media_active: bool,
    pub previewing: bool,
    pub load_locked: bool,
    pub media_key: u64,
    pub load_gate_word: u64,
    pub title: String,
    pub playing: bool,
    pub pos: f64,
    pub frames: f64,
    pub source_sample_rate: u32,
    /// Source seconds per output second, before the source/output frame ratio.
    pub playback_rate: f32,
    pub touching: bool,
    pub loop_start: f64,
    pub loop_len: f64,
    /// Immutable decoded source tempo hint; never overwritten by a manual grid.
    pub source_bpm: f32,
    pub bpm: f32,
    pub pitch: f32,
    pub gain: f32,
    pub eq: [f32; 3],
    pub source_gain: crate::track_gain::Policy,
    pub source_gain_db: f32,
    pub source_gain_active: bool,
    pub source_level: Option<crate::track_gain::Level>,
    pub filter: f32,
    pub vinyl: bool,
    pub sync: bool,
    pub keylock: bool,
    pub keylock_mode: keylock::Mode,
    pub pfl: bool,
    pub loop_on: bool,
    pub hotcues: [bool; HOTCUES],
    #[serde(skip)]
    pub receipt_key: usize,
    pub hotcue_positions: [Option<f64>; HOTCUES],
    pub cue_styles: [cue_metadata::Style; HOTCUES],
    pub grid: Option<beatgrid::Grid>,
    pub meter: f32,
    #[serde(skip)]
    pub peaks: std::sync::Arc<Vec<[f32; 3]>>,
    pub duration: f32,
    pub eq_cut: [bool; 4],
    pub eq_solo: i8,
    pub pitch_range: u8,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TrackSnap {
    pub name: String,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub meter: f32,
    pub playing_scene: i16,
    pub clip_progress: f32,
    pub clip_pending: bool,
    pub clip_looping: bool,
    pub clips: Vec<ClipSnap>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ClipSnap {
    /// Renderer-confirmed note data; lesson/UI observers never inspect live Vecs.
    pub note_count: usize,
    pub recording_held: bool,
    pub kind: u8,
    pub name: String,
    pub bars: f32,
    pub gain: f32,
}

/// Observable clock-consumer hook. Counting accepted ticks is deliberately
/// separate from the future tempo/phase synchronization policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MidiClockInput {
    pub ticks: u64,
    pub last_source: Option<u64>,
}

impl MidiClockInput {
    fn receive_tick(&mut self, source: u64) {
        self.ticks = self.ticks.saturating_add(1);
        self.last_source = Some(source);
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub(crate) builtin_levels: [Option<crate::track_gain::Level>; 2],
    pub session: Option<session::Layout>,
    pub performance: performance::Status,
    pub project_revision: u64,
    pub(crate) sample_rate: u32,
    pub(crate) transport_epoch: u64,
    pub compose_target: Option<ComposeTarget>,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f32,
    pub beat: f64,
    pub timeline_seconds: f64,
    pub bar: u32,
    pub beat_in_bar: f32,
    pub meter_numerator: u8,
    pub meter_denominator: u16,
    pub file_conductor: bool,
    #[serde(skip)]
    pub(crate) timing: Option<Arc<midi_data::Conductor>>,
    pub master: f32,
    pub xfader: f32,
    pub cue_mix: f32,
    pub monitor: monitor::Status,
    pub view: u8,
    pub selected_track: usize,
    pub selected_scene: usize,
    pub selected_deck: usize,
    pub selected_deck_request: u64,
    pub tracks: Vec<TrackSnap>,
    pub decks: Vec<DeckSnap>,
    pub midi: Vec<String>,
    pub midi_feedback: midi::FeedbackStats,
    pub midi_input: midi::InputStats,
    pub midi_clock: MidiClockInput,
    /// Legacy alias: actual last render-thread CPU divided by callback budget.
    pub cpu: Option<f32>,
    pub audio: audio_metrics::AudioMetrics,
    pub commands: control::CommandStats,
    pub submissions: control::SubmissionStats,
    pub fx_kind: [FxKind; 3],
    pub fx_wet: [f32; 3],
    pub metronome: bool,
    #[serde(skip)]
    pub count_in_remaining: f32,
    pub quant: f32,
    pub quantize: bool,
    pub sampler_bank: usize,
    pub sampler_inst: SamplerInstrument,
    pub sampler_unavailable: bool,
    pub sampler_oct: i8,
    pub sampler_banks: Vec<String>,
    #[serde(skip)]
    pub sampler_instances: Vec<sampler::Bank>,
    pub sampler_audition: Option<u64>,
    pub sampler_revision: u64,
    pub sampler_epoch: u64,
    pub fx_view: i16,
    pub fx_slots: Vec<(String, bool, f32, [f32; 4])>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            builtin_levels: [None, None],
            session: None,
            performance: performance::Status::default(),
            project_revision: 0,
            sample_rate: 0,
            transport_epoch: 0,
            compose_target: None,
            playing: false,
            recording: false,
            bpm: 124.0,
            beat: 0.0,
            timeline_seconds: 0.0,
            bar: 1,
            beat_in_bar: 0.0,
            meter_numerator: 4,
            meter_denominator: 4,
            file_conductor: false,
            timing: None,
            master: 0.85,
            xfader: 0.5,
            cue_mix: 0.0,
            monitor: monitor::Status::default(),
            view: 0,
            selected_track: 0,
            selected_scene: 0,
            selected_deck: 0,
            selected_deck_request: 0,
            tracks: Vec::new(),
            decks: Vec::new(),
            midi: Vec::new(),
            midi_feedback: midi::FeedbackStats::default(),
            midi_input: midi::InputStats::default(),
            midi_clock: MidiClockInput::default(),
            cpu: None,
            audio: audio_metrics::AudioMetrics::default(),
            commands: control::CommandStats::default(),
            submissions: control::SubmissionStats::default(),
            fx_kind: [FxKind::Echo, FxKind::Reverb, FxKind::Filter],
            fx_wet: [0.0; 3],
            metronome: false,
            count_in_remaining: 0.0,
            quant: 1.0,
            quantize: true,
            sampler_bank: 0,
            sampler_inst: SamplerInstrument::Samples,
            sampler_unavailable: false,
            sampler_oct: 3,
            sampler_banks: vec!["Kit".into()],
            sampler_instances: Vec::new(),
            sampler_audition: None,
            sampler_revision: 0,
            sampler_epoch: 0,
            fx_view: -1,
            fx_slots: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Remote(remote::Request),
    ProviderPreview(provider_preview::Request),
    SessionEdit(session::Request),
    SessionControl(session::Scoped),
    PerformanceMode(bool),
    SafetyStop(performance::Safety),
    RecoverPerformance,
    Undo,
    Redo,
    Gesture { id: u64, command: Box<Command> },
    ReservedStop { lane: u8, ticket: u64, target: Option<session::Reference> },
    Play,
    Stop,
    TogglePlay,
    TimelineSeek(f64),
    Record,
    Tap(Instant),
    MidiClock { source: u64 },
    SetBpm(f32),
    LaunchClip { track: u8, scene: u16 },
    LaunchScene { scene: u16 },
    StopTrack { track: u8 },
    DeckPlay { deck: u8 },
    DeckCue { deck: u8 },
    DeckSync { deck: u8 },
    DeckJog { deck: u8, delta: f32 },
    DeckTouch { deck: u8, on: bool },
    MidiDeckTouch { source: u64, deck: u8, on: bool },
    DeckPitch { deck: u8, value: f32 },
    DeckGain { deck: u8, value: f32 },
    DeckEq { deck: u8, band: u8, value: f32 },
    DeckFilter { deck: u8, value: f32 },
    DeckPfl { deck: u8 },
    DeckHotCue { deck: u8, pad: u8, del: bool },
    DeckGrid { deck: u8, grid: Option<beatgrid::Grid>, receipt: load_receipt::Receipt, ack: beatgrid::GridEditAck },
    DeckSourceGain { deck: u8, gain: crate::track_gain::Resolved, receipt: load_receipt::Receipt, ack: beatgrid::GridEditAck },
    DeckCuePoint { deck: u8, pad: u8, del: bool, receipt: load_receipt::Receipt },
    DeckCueStyle { deck: u8, pad: u8, style: cue_metadata::Style, receipt: load_receipt::Receipt },
    DeckLoop { deck: u8, beats: f32 },
    DeckLoopIn { deck: u8 },
    DeckLoopOut { deck: u8 },
    DeckLoadSelected { deck: u8 },
    DeckLoadFile { deck: u8, path: std::path::PathBuf },
    PrepareSelected { all: bool },
    DeckPreview { deck: u8, expected: u64, on: bool },
    DeckLoadLock { deck: u8, enabled: bool },
    DeckVinyl { deck: u8 },
    DeckKeylock { deck: u8 },
    DeckAudio { deck: u8, audio: Arc<Sample> },
    DeckDecoded { request: media_load::LoadToken, audio: Arc<Sample> },
    DeckLoadRequested { deck: u8, media: load_receipt::Media, receipt: load_receipt::Receipt },
    DeckRestorePreparation { deck: u8, receipt: load_receipt::Receipt, preparation: preparation::Preparation },
    LibraryFence { acknowledged: Arc<std::sync::atomic::AtomicBool> },
    DeckSeek { deck: u8, frac: f32 },
    DeckUnload { deck: u8 },
    LoadBuiltin { deck: u8, stem: u8 },
    Xfader(f32),
    Master(f32),
    CueMix(f32),
    Monitor(monitor::Control),
    TrackGain { track: u8, value: f32 },
    ClipGain { track: u8, scene: u16, value: f32 },
    TrackPan { track: u8, value: f32 },
    Mute { track: u8 },
    Solo { track: u8 },
    Arm { track: u8 },
    Browse(f32),
    BrowseCrates(f32),
    CrateReturn,
    Select { track: usize, scene: usize },
    ComposeArm { track: usize, scene: usize },
    ComposeDisarm,
    SelectDeck(usize),
    SelectDeckRequested { deck: usize, request: u64 },
    SetView(View),
    LiveNoteOn { source: u64, ch: u8, note: u8, vel: u8 },
    RoutedNoteOn { source: u64, ch: u8, note: u8, vel: u8, track: u8, target: Option<session::Reference> },
    LiveNoteOff { source: u64, ch: u8, note: u8 },
    MidiEdit(midi_edit::Request),
    MidiImport(midi_interchange::Request),
    MidiAudition { id: u64, track: u8, note: u8, vel: u8, on: bool },
    SetNotes { track: u8, scene: u16, notes: Vec<MidiNote> },
    FxWet { slot: u8, value: f32 },
    FxSelect { slot: u8 },
    Quant(f32),
    Metronome,
    NudgeBpm(f32),
    ToggleQuant,
    DeckLoopDouble { deck: u8 },
    DeckLoopHalf { deck: u8 },
    DeckReloop { deck: u8 },
    DeckMatch,
    DeckEqCut { deck: u8, band: u8 },
    DeckEqSolo { deck: u8, band: u8 },
    DeckPitchRange { deck: u8 },
    FireClip { track: u8, scene: u16, looping: bool },
    ToggleScene { scene: u16 },
    RestartScene { scene: u16 },
    AddScene { scene: u16 },
    SamplerPad { pad: u8, on: bool },
    SamplerPadPressure { pad: u8, pressure: f32 },
    SamplerSlotStop { pad: u8 },
    SamplerBank(usize),
    SamplerEdit(sampler::Edit),
    SamplerAudition(sampler::Audition),
    SamplerAuditionStop { id: u64 },
    SamplerInst(SamplerInstrument),
    SamplerOct(i8),
    OpenFxTrack(u8),
    OpenFxScene(u16),
    CloseFx,
    FxAdd(u8),
    FxToggle(usize),
    FxMix { slot: usize, value: f32 },
    FxParam { slot: usize, p: u8, value: f32 },
}

impl RtEngine {
    #[cfg(test)]
    pub fn new(
        sr: f32,
        cmd_rx: impl Into<control::CommandReceiver>,
        snap: Arc<Mutex<Snapshot>>,
    ) -> Self {
        Self::try_new(sr, cmd_rx, snap).expect("prepare test renderer")
    }
    pub fn try_new(
        sr: f32,
        cmd_rx: impl Into<control::CommandReceiver>,
        snap: Arc<Mutex<Snapshot>>,
    ) -> Result<Self, String> {
        Self::try_new_with_sampler_owner(sr, cmd_rx, snap, None)
    }
    /// Prepare one renderer and its bounded factory assets.
    /// Takes its rate, command receiver, snapshot owner and optional fixture asset account; returns the complete renderer.
    fn try_new_with_sampler_owner(
        sr: f32,
        cmd_rx: impl Into<control::CommandReceiver>,
        snap: Arc<Mutex<Snapshot>>,
        sampler_owner: Option<crate::sampler_bank::assets::Owner>,
    ) -> Result<Self, String> {
        midi_edit::initialize()?;
        let (sampler_assets, sampler_banks) = match sampler_owner {
            Some(owner) => sampler::initial_with_owner(sr as u32, owner)?,
            None => sampler::initial(sr as u32)?,
        };
        let cmd_rx = cmd_rx.into();
        let telemetry = cmd_rx.telemetry();
        let performance = cmd_rx.performance().clone();
        let drums = build_kit(sr as u32);
        let names = [
            "Drums", "Bass", "Keys", "Pad", "Perc", "Vocal", "FX", "Spare",
        ];
        let kinds = [0u8, 1, 2, 3, 0, 4, 3, 1];
        let mut tracks: Vec<Box<TrackRt>> = (0..TRACKS)
            .map(|i| Box::new(TrackRt::empty(sr, names[i].into(), kinds[i], drums.clone(), SCENES)))
            .collect();
        tracks.reserve(session::MAX_TRACKS - tracks.len());
        let session = session::Layout::fresh(names.iter().map(|n| (*n).into()), SCENES);
        let mut e = Self {
            session,
            midi_routing:cmd_rx.midi_routing(),
            midi_output_mask:0,
            midi_output_budget:256,
            undo: undo::Journal::default(),
            project: project::Handle::new(sr as u32, performance.clone()),
            performance,
            safety_output: performance::Output::default(),
            project_pending: None,
            project_waiting: None,
            project_sealed: false,
            live_set: None,
            sr,
            playing: false,
            recording: false,
            bpm: 124.0,
            beat: 0.0,
            beat_roundoff: 0.0,
            transport_epoch: 0,
            timeline_anchor: 0.0,
            timeline_frames: 0,
            remote_schedule: remote::Schedule::default(),
            provider_preview: None,
            midi_beat: 0.0,
            midi_beat_reference: 0.0,
            #[cfg(test)]
            current_sample_frame: 0,
            conductor: None,
            last_midi_step: 0.0,
            quant: 1.0,
            view: View::Session,
            xfader: 0.5,
            xfader_curve: 0.35,
            xfader_gain: mixer_gain::GainPair::default(),
            #[cfg(test)]
            legacy_gain_math: false,
            master: 0.85,
            cue_mix: 0.0,
            monitor: monitor::Monitor::default(),
            tracks,
            decks: [DeckRt::new(sr), DeckRt::new(sr)],
            master_fx: std::array::from_fn(|_| master_fx::MasterSlot::new(sr)),
            history_measurement: history_measurement::capture::Measurement::new(sr as u32).ok(),
            fx_kind: [FxKind::Echo, FxKind::Reverb, FxKind::Filter],
            fx_wet: [0.0, 0.0, 0.0],
            tap: Vec::new(),
            selected_track: 0,
            selected_scene: 0,
            selected_deck: 0,
            selected_deck_request: 0,
            cmd_rx,
            command_stats: control::CommandStats::default(),
            command_batch: Box::new(control::CommandBatch::empty()),
            publisher: snapshot::Publisher::new(snap.clone()),
            audible: audible::Writer::new(),
            snap,
            midi_clock: MidiClockInput::default(),
            #[cfg(test)]
            load_test_hooks: [None, None],
            telemetry,
            render_cpu_ns: None,
            load_profile: diagnostics::FrameProfile::default(),
            #[cfg(test)]
            telemetry_delays: [Duration::ZERO; 3],
            frames_done: 0,
            note_recording: recording::Recording::default(),
            metronome: false,
            metro: metronome::Click::new(sr),
            count_in: None,
            routing: None,
            routing_track_input: None,
            routing_track_taps: [[0.0; 2]; 3],
            routing_deck_taps: [[0.0; 2]; 2],
            routing_input_frame: [0.0; audio::routing::model::MAX_PHYSICAL_CHANNELS],
            routing_pipe: audio::routing::input::Pipe::default(),
            routing_probe: audio::routing::probe::Probe::default(),
            quantize: true,
            sampler_bank: 0,
            sampler_inst: SamplerInstrument::Samples,
            sampler_oct: 3,
            sampler_poly: Poly::new(sr, SynthInstrument::Keys, 8),
            sampler_banks,
            sampler_revision: 1,
            sampler_assets,
            pad_voices: std::array::from_fn(|_| None),
            sampler_audition: None,
            pad_destinations: [0; 16],
            pad_output: [[0.0; 2]; session::MAX_TRACKS],
            pad_targets: [None; 16],
            builtin: [None, None],
            builtin_levels: [None, None],
            fx_view: -1,
            scene_fx: { let mut racks: Vec<_> = (0..SCENES).map(|_| fx::FxChain::new(sr)).collect(); racks.reserve(session::MAX_SCENES - racks.len()); racks },
            compose_target: None,
        };
        e.midi_routing.identity.publish(&e.session);
        e.seed_demo();
        let (stem_a, stem_b) = demo_stems(sr as u32, e.bpm);
        e.builtin = [Some(stem_a), Some(stem_b)];
        e.builtin_levels = std::array::from_fn(|stem| e.builtin[stem].as_ref().and_then(|sample| crate::track_gain::measure_channels(&sample.data, sample.ch, || false).ok()));
        for deck in 0..DECKS as u8 {
            e.apply(Command::DeckLoadRequested {
                deck, media: load_receipt::Media::Builtin(deck), receipt: load_receipt::Receipt::new().with_source_level(e.builtin_levels[deck as usize]).map_err(str::to_string)?,
            });
        }
        e.publish_initial();
        Ok(e)
    }

    /// Prepare a stopped/unowned renderer before constructing its output
    /// callback. This allocates effect buffers and generated samples and must
    /// never be called from `process` or a running device callback.
    ///
    /// Changed rates discard voices/tails/filter history, preserving musical
    /// positions and controls. Ordinary clips chase their current notes on the
    /// next sample; arpeggiators resume at the next step. Equal rates are a no-op.
    pub fn set_sample_rate(&mut self, sr: u32) -> Result<(), String> {
        if sr == 0 || sr as f32 == self.sr {
            return Ok(());
        }
        let effect_bytes=self.tracks.iter().flat_map(|t|&t.fx.slots).chain(self.scene_fx.iter().flat_map(|r|&r.slots))
            .map(|slot|fx::FxSlot::required_storage(slot.id(),sr as f32)).sum::<usize>();
        if effect_bytes>session::MAX_PROCESSOR_BYTES {return Err("Output rate would exceed the 256 MiB session effect-buffer limit; remove effects or choose a lower rate".into());}
        let sampler_banks = self.sampler_rate_banks(sr)?;
        self.timeline_anchor = self.timeline_seconds();
        self.timeline_frames = 0;
        self.sr = sr as f32;
        self.project.set_sample_rate(sr);
        let active_history=self.active_recording_history();
        self.undo.prepare_sample_rate(self.sr,active_history);
        self.metro = metronome::Click::new(self.sr);
        self.xfader_gain = mixer_gain::GainPair::default();
        for track in &mut self.tracks { track.mixer_gain = mixer_gain::GainPair::default(); }
        // Rate changes reconstruct all preallocated master histories; type and
        // wet controls remain intact and are configured on the next block.
        self.master_fx = std::array::from_fn(|_| master_fx::MasterSlot::new(sr as f32));
        if let Some(history) = &mut self.history_measurement { history.set_rate(sr); }
        let drums = build_kit(sr);
        self.install_sampler_rate_banks(sampler_banks);
        for voice in &mut self.pad_voices { *voice = None; }
        if let Some(active) = &self.sampler_audition { active.ended(); }
        self.sampler_audition = None;
        self.pad_output.fill([0.0; 2]);
        self.sampler_poly.set_sample_rate(self.sr);
        for chain in &mut self.scene_fx {
            chain.set_sample_rate(self.sr);
        }
        let midi_beat = self.precise_midi_beat();
        for t in &mut self.tracks {
            t.poly.set_sample_rate(self.sr);
            t.eq.set_sample_rate(self.sr);
            t.eq_right.set_sample_rate(self.sr);
            t.fx.set_sample_rate(self.sr);
            t.drum_samples = drums.clone();
            t.drum_pos.fill(None);
            t.release_clip_notes();
            t.rebuild_midi_schedule(self.beat, midi_beat);
            t.meter = 0.0;
        }
        for d in &mut self.decks {
            for eq in &mut d.eq {
                eq.set_sample_rate(self.sr);
            }
            d.filter = [deck_filter::ChannelFilter::default(); 2];
            d.filter_position = d.filter_amt;
            d.keylock_dsp = keylock::Processor::new(self.sr);
            d.rate_smoothing = dsp::rate_blend(0.08, self.sr);
            d.last_output = [0.0; 2];
            d.transition_to(d.pos, self.sr, DeckTransition::Jump);
            d.meter = 0.0;
        }
        Ok(())
    }

    fn seed_demo(&mut self) {
        // House drums: kick every beat, snare 2/4, hats 8ths.
        let mut drums = Vec::new();
        for b in 0..4 {
            drums.push(MidiNote {
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 36,
                start: b as f32,
                len: 0.25,
                vel: if b % 2 == 0 { 110 } else { 96 },
            });
            drums.push(MidiNote {
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 42,
                start: b as f32,
                len: 0.12,
                vel: 70,
            });
            drums.push(MidiNote {
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 42,
                start: b as f32 + 0.5,
                len: 0.12,
                vel: 88,
            });
        }
        drums.push(MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 38,
            start: 1.0,
            len: 0.25,
            vel: 108,
        });
        drums.push(MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 38,
            start: 3.0,
            len: 0.25,
            vel: 108,
        });
        drums.push(MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 39,
            start: 3.5,
            len: 0.2,
            vel: 90,
        });
        self.tracks[0].clips[0] = Clip {
            lanes: None,
            region: None,
            kind: ClipKind::Midi,
            name: "House Kit".into(),
            bars: 1.0,
            notes: drums,
            gain: 1.0,
            audio: None,
        };
        self.tracks[1].clips[0] = Clip {
            lanes: None,
            region: None,
            kind: ClipKind::Midi,
            name: "Bassline".into(),
            bars: 1.0,
            notes: vec![
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 36, start: 0.0, len: 0.7, vel: 100 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 36, start: 0.75, len: 0.2, vel: 80 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 43, start: 1.5, len: 0.45, vel: 96 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 41, start: 2.5, len: 0.45, vel: 90 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 36, start: 3.0, len: 0.4, vel: 100 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 38, start: 3.5, len: 0.4, vel: 86 },
            ],
            gain: 0.95,
            audio: None,
        };
        self.tracks[2].clips[0] = Clip {
            lanes: None,
            region: None,
            kind: ClipKind::Midi,
            name: "Stab".into(),
            bars: 2.0,
            notes: vec![
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 60, start: 0.0, len: 0.45, vel: 78 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 64, start: 0.0, len: 0.45, vel: 70 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 67, start: 0.0, len: 0.45, vel: 70 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 62, start: 4.0, len: 0.45, vel: 74 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 65, start: 4.0, len: 0.45, vel: 68 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 69, start: 4.0, len: 0.45, vel: 68 },
            ],
            gain: 0.7,
            audio: None,
        };
        self.tracks[3].clips[0] = Clip {
            lanes: None,
            region: None,
            kind: ClipKind::Midi,
            name: "Pad".into(),
            bars: 2.0,
            notes: vec![
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 48, start: 0.0, len: 7.5, vel: 64 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 55, start: 0.0, len: 7.5, vel: 52 },
                MidiNote { channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false, pitch: 60, start: 0.0, len: 7.5, vel: 48 },
            ],
            gain: 0.55,
            audio: None,
        };
        // scene 2 variation
        let mut d2 = self.tracks[0].clips[0].notes.clone();
        d2.push(MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 46,
            start: 1.75,
            len: 0.3,
            vel: 80,
        });
        self.tracks[0].clips[1] = Clip {
            lanes: None,
            region: None,
            kind: ClipKind::Midi,
            name: "Fill".into(),
            bars: 1.0,
            notes: d2,
            gain: 1.0,
            audio: None,
        };
    }

    // A separate compensated clock gives new source-time MIDI regions exact
    // long-run boundaries without changing legacy clip/deck/metronome output.
    // Project installation and direct numerical fixtures can reposition beat.
    fn precise_midi_beat(&self) -> f64 {
        if self.beat == self.midi_beat_reference { self.midi_beat } else { self.beat }
    }

    fn sync_midi_clock(&mut self) {
        if self.beat != self.midi_beat_reference {
            self.timeline_anchor = self.conductor.as_ref().map_or(self.beat * 60.0 / f64::from(self.bpm), |map| map.seconds_at(self.beat));
            self.timeline_frames = 0;
            self.midi_beat = self.beat;
            self.midi_beat_reference = self.beat;
            self.beat_roundoff = 0.0;
        }
    }

    fn start_count_in(&mut self) {
        if !self.playing {
            self.count_in = self.conductor.as_ref().and_then(|map| metronome::CountIn::new(map, self.precise_midi_beat(), self.sr as u32));
            self.metro.reset();
        }
    }

    pub fn process(&mut self, out: &mut [f32]) {
        self.process_channels(out, 2);
    }

    fn process_channels(&mut self, out: &mut [f32], channels: usize) {
        for deck in &mut self.decks {
            if let Some((true,grid))=deck.load_receipt.as_ref().and_then(load_receipt::Receipt::grid_protection) {
                if deck.grid!=grid {deck.grid=grid;deck.publish_preparation();}
            }
        }
        self.performance_tick();
        self.command_batch.receive_into(&self.cmd_rx);
        self.command_stats.record(&self.command_batch);
        for index in 0..control::COMMANDS_PER_BLOCK {
            if let Some(command) = self.command_batch.discarded[index].take() { self.undo.retire_command(command); }
        }
        for index in 0..control::COMMANDS_PER_BLOCK {
            if let Some(command) = self.command_batch.commands[index].take() { self.apply(command); }
        }
        self.project_tick();
        self.live_set_tick(channels);
        self.routing_pipe.shared.explicit.store(self.routing.is_some(), std::sync::atomic::Ordering::Release);
        self.prepare_midi_output_block();
        self.sync_midi_clock();
        self.remote_maintain();
        self.maintain_provider_preview();
        self.routing_pipe.recorder.preview(self.provider_preview.is_some());
        if let Some(history) = &mut self.history_measurement {
            history.routing_compatibility(self.routing.is_none() && !self.live_set.as_ref().is_some_and(|stage| stage.transitioning()));
            history.service_requests([self.decks[0].history_key, self.decks[1].history_key]);
        }
        self.performance.publish_decks(self.deck_activity());
        self.performance.try_recover(|| self.cmd_rx.is_empty() && !self.cmd_rx.pending_project_ui_requests());
        self.undo.publish();
        self.cmd_rx.set_history_available(self.undo.available());
        #[cfg(test)]
        std::thread::sleep(self.telemetry_delays[0]);
        let cpu_start = audio_metrics::thread_cpu_ns();
        #[cfg(test)]
        std::thread::sleep(self.telemetry_delays[1]);
        let frames = out.len() / channels;
        self.routing_pipe.begin_block(self.sr as u32, frames);
        if frames > 0 { self.prepare_mixer_gains(); }
        let spb = (self.sr as f64) * 60.0 / self.bpm as f64;
        for (slot, wet) in self.master_fx.iter_mut().zip(self.fx_wet) { slot.configure(wet, spb); }
        if let Some(history) = &mut self.history_measurement { history.configure(self.fx_kind, self.fx_wet, spb); }

        let conductor_seconds = self.conductor.as_ref().map(|c| c.seconds_at(self.precise_midi_beat()));
        let mut transport_frames = 0usize;
        let mut conductor_spb = spb;
        let any_solo = self.tracks.iter().enumerate().any(|(slot,t)| self.session.tracks.get(slot).is_some_and(|item| item.active) && t.solo);
        let profiling = self.telemetry.profiler.enabled.load(std::sync::atomic::Ordering::Relaxed);

        for i in 0..frames {
            self.routing_input_frame = self.routing_pipe.frame(self.sr as u32);
            self.remote_tick();
            if self.count_in.as_ref().is_some_and(|count| count.finished()) { self.count_in = None; }
            let counting_in = self.count_in.is_some();
            let count_click = self.count_in.as_mut().and_then(|count| count.tick(self.sr as u32));
            #[cfg(test)]
            { self.current_sample_frame = self.frames_done + i as u64; }
            self.load_profile.begin(profiling, self.frames_done + i as u64, self.sr);
            // Compose holds still have a musical duration with the transport
            // stopped. This clock integrates actual tempo and never loops.
            let midi_position = self.precise_midi_beat();
            let spb = self.conductor.as_ref().map_or(spb, |c| {
                let micros = c.micros_exact_at(midi_position);
                self.bpm = (60000000.0 / f64::from(micros)) as f32;
                f64::from(self.sr) * f64::from(micros) / 1000000.0
            });
            if self.conductor.is_some() && spb != conductor_spb {
                for (slot, wet) in self.master_fx.iter_mut().zip(self.fx_wet) { slot.configure(wet, spb); }
                if let Some(history) = &mut self.history_measurement { history.configure(self.fx_kind, self.fx_wet, spb); }
                conductor_spb = spb;
            }
            self.last_midi_step = 1.0 / spb;
            // A mapped transport adds its analytically integrated sample span
            // below; recording and playback must share the same ramp interval.
            if !counting_in && (!self.playing || self.conductor.is_none()) { self.note_recording.clock += self.last_midi_step; }
            let beat_start = self.beat;
            if self.playing && !counting_in {
                transport_frames += 1;
                self.timeline_frames += 1;
                // Compensate accumulated rounding so a long clip cannot move
                // an exact note boundary to the preceding output sample.
                if let Some(seconds) = conductor_seconds {
                    let next = self.conductor.as_ref().unwrap().beat_at_seconds(seconds + transport_frames as f64 / f64::from(self.sr));
                    self.last_midi_step = next - self.midi_beat;
                    self.midi_beat = next; self.beat = next;
                    self.midi_beat_reference = next; self.beat_roundoff = 0.0;
                } else {
                let step = 1.0 / spb - self.beat_roundoff;
                let next = self.midi_beat + step;
                self.beat_roundoff = (next - self.midi_beat) - step;
                self.midi_beat = next;
                self.beat += 1.0 / spb;
                self.midi_beat_reference = self.beat;
                }
            }
            if self.playing && !counting_in && self.conductor.is_some() { self.note_recording.clock += self.last_midi_step; }
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            let mut cue_l = 0.0f32;
            let mut cue_r = 0.0f32;
            self.monitor.begin();

            let timer = self.load_profile.start();
            self.pad_output = self.tick_pad_sources();
            self.load_profile.pads(timer);
            if let Some(mut routing) = self.routing.take() {
                let click = self.render_click(counting_in, count_click, beat_start);
                let frame = routing.render(self, any_solo, click, channels);
                let output = &mut out[i * channels..(i + 1) * channels];
                output.fill(0.0);
                let count = channels.min(audio::routing::model::MAX_PHYSICAL_CHANNELS);
                output[..count].copy_from_slice(&frame[..count]);
                self.render_output_probe(output);
                self.routing = Some(routing);
                self.audible.push(&self.decks);
                if self.load_profile.active { self.telemetry.profiler.publish(&self.load_profile); }
                continue;
            }
            let mut scene_inputs = [[0.0_f32; 2]; session::MAX_SCENES];
            for ti in 0..self.tracks.len() {
                if !self.session.tracks.get(ti).is_some_and(|item| item.active) { continue; }
                let timer = self.load_profile.start();
                let (tl, tr, pfl) = self.render_track_cached(ti, any_solo);
                self.load_profile.track(ti, timer);
                if pfl {
                    cue_l += tl;
                    cue_r += tr;
                }
                let bus = self.tracks[ti].scene_bus;
                scene_inputs[bus][0] += tl;
                scene_inputs[bus][1] += tr;
            }
            // Keep every scene's own history advancing, including zero-input
            // tails after all of its tracks have stopped or moved elsewhere.
            for (scene, (chain, input)) in self.scene_fx.iter_mut().zip(scene_inputs).enumerate() {
                if !self.session.scenes.get(scene).is_some_and(|item| item.active) || chain.slots.is_empty() && input == [0.0,0.0] { continue; }
                let timer = self.load_profile.start();
                let [sl, sr] = self.load_profile.chain(chain, input, self.sr, true, scene);
                self.load_profile.scene(scene, timer);
                l += sl;
                r += sr;
            }

            let timer = self.load_profile.start();
            let (al, ar) = self.render_deck(0);
            self.load_profile.deck(0, timer);
            let timer = self.load_profile.start();
            let (bl, br) = self.render_deck(1);
            self.load_profile.deck(1, timer);
            let [ga, gb] = {
                #[cfg(test)]
                if self.legacy_gain_math {
                    mixer_gain::crossfader_gains(self.xfader, self.xfader_curve)
                } else { self.xfader_gain.tick() }
                #[cfg(not(test))]
                self.xfader_gain.tick()
            };
            let dl = al * ga + bl * gb;
            let dr = ar * ga + br * gb;
            l += dl;
            r += dr;
            if self.decks[0].pfl {
                cue_l += al;
                cue_r += ar;
            }
            if self.decks[1].pfl {
                cue_l += bl;
                cue_r += br;
            }

            let click = self.render_click(counting_in, count_click, beat_start);
            l += click;
            r += click;

            // Three legacy controls select real processors in a serial chain.
            for slot in 0..self.master_fx.len() {
                let timer = self.load_profile.start();
                [l, r] = self.master_fx[slot].process([l, r], self.fx_kind[slot], self.fx_wet[slot]);
                self.load_profile.master(slot, timer, self.fx_kind[slot]);
            }

            let cm = self.cue_mix;
            l = l * (1.0 - cm) + cue_l * cm;
            r = r * (1.0 - cm) + cue_r * cm;
            if let Some(history) = self.history_measurement.as_mut().filter(|history| history.available) {
                let contribution = history.tracker.process(
                    [self.decks[0].history_last, self.decks[1].history_last],
                    [self.decks[0].history_key, self.decks[1].history_key],
                    [[al, ar], [bl, br]], [ga, gb], [self.decks[0].pfl, self.decks[1].pfl], cm);
                history.record_rendered(i, contribution, [l, r], self.master, &self.safety_output,
                    std::array::from_fn(|deck| if self.decks[deck].playing { self.decks[deck].history_key } else { 0 }));
            }
            let preview = self.tick_provider_preview();
            l += preview[0];
            r += preview[1];
            let headphone = self.monitor.render([l * self.master, r * self.master], channels);
            let main = [l * self.master, r * self.master];
            self.safety_output.observe(std::array::from_fn(|channel| if !main[channel].is_finite() || !headphone[channel].is_finite() { f32::NAN } else { main[channel].abs().max(headphone[channel].abs()) }), self.sr);
            l = limiter(l * self.master);
            r = limiter(r * self.master);
            let headphone = self.safety_output.preview(headphone.map(limiter));
            let [l, r] = self.safety_output.output([l, r]);
            let output = &mut out[i * channels..(i + 1) * channels];
            output.fill(0.0);
            if channels == 1 { output[0] = 0.5 * (l + r); } else { output[0] = l; output[1] = r; }
            if channels >= 4 { output[2..4].copy_from_slice(&headphone); }
            self.render_output_probe(output);
            self.audible.push(&self.decks);
            if self.load_profile.active { self.telemetry.profiler.publish(&self.load_profile); }
        }
        self.live_set_mix(out, channels);
        self.project.publish_timeline(self.timeline_seconds());
        self.load_profile.active = false;
        self.render_cpu_ns = cpu_start.and_then(|start| audio_metrics::thread_cpu_ns()?.checked_sub(start));
        if frames > 0 && self.has_held_project_notes() { self.project.edited();self.history_held_changed(); }
        self.frames_done += frames as u64;
        if self.frames_done % (self.sr as u64 / 8).max(1) < frames as u64 {
            self.publish();
        }
        self.performance.publish_output(&self.safety_output);
        self.performance.publish_decks(self.deck_activity());
        self.project_finish_block();
    }

    // Direct numerical fixtures treat each call as a one-frame block. The
    // real callback prepares all control gains once before its sample loop.
    #[cfg(test)]
    fn render_track(&mut self, ti: usize, any_solo: bool) -> (f32, f32, bool) {
        let track = &mut self.tracks[ti];
        track.mixer_gain.prepare([track.gain, track.pan], self.sr, mixer_gain::pan_gains);
        self.render_track_cached(ti, any_solo)
    }

    /// Advance the shared metronome once.
    /// Takes count-in and beat events; returns the current click sample.
    fn render_click(&mut self, counting_in: bool, count_click: Option<bool>, beat_start: f64) -> f32 {
        if let Some(conductor) = self.conductor.as_ref().filter(|_| counting_in || self.metronome && self.playing) {
            let settings = conductor.native.unwrap_or_default();
            let event = if counting_in { count_click } else { conductor.click_between(beat_start, self.beat) };
            #[cfg(test)]
            if let (Some(trace), Some(accent)) = (&mut self.metro.trace, event) { trace.push((self.current_sample_frame, accent)); }
            self.metro.tick_with_gains(true, event, settings.accent_gain, settings.beat_gain)
            } else {
            self.metro.tick(self.metronome && self.playing, beat_start, self.beat)
        }
    }

    /// Apply an explicitly requested physical-channel test.
    /// Takes a completed output frame; writes a bounded tone only while all transports are stopped and safety remains in force.
    fn render_output_probe(&mut self, output: &mut [f32]) {
        let stopped = !self.playing && !self.recording && !self.decks.iter().any(|deck| deck.playing || deck.touching) && !self.performance.status().protected;
        if let Some((channel, value)) = self.routing_probe.sample(&self.routing_pipe.shared.probe, self.sr as u32, output.len(), stopped) {
            output[channel] = self.safety_output.preview([value, 0.0])[0];
        }
    }

    fn render_track_cached(&mut self, ti: usize, any_solo: bool) -> (f32, f32, bool) {
        self.render_midi_output(ti);
        let mut fallback = [0.0; 2];
        let silent = self.tracks[ti].mute || (any_solo && !self.tracks[ti].solo);
        let playing = self.tracks[ti].playing;
        // Pending launches do not emit or advance clip-local state. The
        // engine beat denotes the end of this output sample's beat interval.
        let explicit_region = playing.is_some_and(|p| self.tracks[ti].clips[p.scene as usize].region.is_some());
        let clock = if explicit_region { self.precise_midi_beat() } else { self.beat };
        if let Some(p) = playing.filter(|p| self.count_in.is_none() && clock > (if explicit_region { p.midi_start_beat } else { p.start_beat }) + midi_schedule::BEAT_EPSILON)
        {
            let scene = p.scene as usize;
            self.tracks[ti].scene_bus = scene;
            let clip_beats = self.tracks[ti].clips[scene].bars.max(0.25) as f64 * 4.0;
            let region = self.tracks[ti].clips[scene].region;
            let repeating = region.map_or(p.looping, |region| region.repeating(p.looping));
            let duration = region.map_or(clip_beats, |region| region.end - region.start);
            let period = region.map_or(clip_beats, |region| region.period());
            let elapsed = clock - if explicit_region { p.midi_start_beat } else { p.start_beat };
            let ending = !repeating && elapsed > duration + midi_schedule::BEAT_EPSILON;
            {
                // Match the event heap's half-open sample interval. An exact
                // endpoint belongs to the next sample, including arp/loop steps.
                let sample_elapsed = (elapsed - midi_schedule::BEAT_EPSILON).max(0.0);
                let local = region.map_or_else(|| sample_elapsed.rem_euclid(clip_beats),
                    |region| region.position(sample_elapsed, p.looping).unwrap_or(region.end));
                if !ending && self.tracks[ti].kind != 0 && self.tracks[ti].poly.offline.is_some() {
                    let clip = &self.tracks[ti].clips[scene];
                    if let Some(audio) = &clip.audio {
                        let phase = local / (f64::from(clip.bars) * 4.0) * audio.frames() as f64;
                        let (l, r) = audio.at(phase);
                        let gain = clip_gain(clip.gain); fallback = [l * gain, r * gain];
                    }
                }
                let prev = p.last_beat;
                if self.tracks[ti].clips[scene].kind == ClipKind::Midi {
                    let kind = self.tracks[ti].kind;
                    let gain = clip_gain(self.tracks[ti].clips[scene].gain);
                    let arp = self.tracks[ti]
                        .fx
                        .slots
                        .iter()
                        .any(|s| s.id() == fx::FxId::Arp && s.on);
                    if arp {
                        let track = &mut self.tracks[ti];
                        if !track.midi_schedule.paused {
                            track.poly.release_clip();
                            track.midi_schedule.reset();
                            track.midi_schedule.paused = true;
                        }
                        let notes = &track.clips[scene].notes;
                        let recorded = &track.recorded_playback;
                        let loop_origin = region.map_or_else(|| (sample_elapsed / clip_beats).floor() * clip_beats,
                            |region| {
                                let first = region.loop_end - region.start;
                                if !repeating || sample_elapsed < first { -region.start }
                                else { first + ((sample_elapsed - first) / period).floor() * period - region.loop_start }
                            });
                        let visibility = if ending { 2 } else if repeating
                            && region.is_some_and(|region| sample_elapsed >= region.loop_end - region.start) { 1 } else { 0 };
                        track.arp_cache.refresh_region_visible(notes, local, prev, clip_beats, visibility, |index| {
                            !ending && region.is_none_or(|region| !repeating
                                || sample_elapsed < region.loop_end - region.start
                                || notes[index].source_start() >= region.loop_start)
                            && recorded.get(index).and_then(Option::as_ref).is_none_or(|policy| {
                                let phase = region.map_or(notes[index].source_start(),
                                    |region| notes[index].source_start() - region.loop_start);
                                let recurs = repeating && region.is_none_or(|region|
                                    notes[index].source_start() >= region.loop_start && (notes[index].source_start()) < region.loop_end);
                                policy.first_onset(phase, period, recurs)
                                    .is_some_and(|first| loop_origin + notes[index].source_start()
                                        >= first - midi_schedule::BEAT_EPSILON)
                            })
                        });
                        let step = (local * 4.0).floor() as i64;
                        let prev_step = (prev * 4.0).floor() as i64;
                        let advance = step != prev_step || local < prev;
                        // A rest or an edit removing the current pitch ends its gate
                        // immediately, without retriggering between sixteenths.
                        if advance || track.arp_note.is_some_and(|n| !track.arp_cache.contains(n)) {
                            if let Some(old) = track.arp_note.take() {
                                track.poly.note_off_clip(old);
                            }
                        }
                        let pitch = advance.then(|| track.arp_cache.pitch(step)).flatten();
                        if let Some(pitch) = pitch {
                            if kind == 0 {
                                let velocity = track.arp_cache.velocity(pitch) as f32 / 127.0;
                                self.trig_drum_with_gain(ti, pitch, velocity, gain);
                            } else {
                                self.tracks[ti].poly.note_on_clip_with_gain(pitch, 0.9, gain);
                            }
                            self.tracks[ti].arp_note = Some(pitch);
                        }
                    } else {
                        let sample_step = if self.conductor.is_some() { self.last_midi_step } else { self.bpm as f64 / 60.0 / self.sr as f64 };
                        let previous_sample = self.beat - sample_step;
                        let previous_midi_sample = self.precise_midi_beat() - sample_step;
                        let track = &mut self.tracks[ti];
                        track.arp_cache.invalidate();
                        if let Some(old) = track.arp_note.take() {
                            track.poly.note_off_clip(old);
                        }
                        if track.midi_schedule.paused || !track.midi_schedule.has_length(period)
                        {
                            track.rebuild_midi_schedule(previous_sample, previous_midi_sample);
                        }
                        while let Some(gate) =
                            self.tracks[ti].midi_schedule.next_due(elapsed, p.looping)
                        {
                            #[cfg(test)]
                            if let Some(trace) = &mut self.tracks[ti].midi_schedule.sample_trace {
                                trace.push((self.current_sample_frame, gate));
                            }
                            match gate {
                                midi_schedule::Gate::On(pitch, velocity) if kind == 0 => {
                                    self.trig_drum_with_gain(ti, pitch, velocity as f32 / 127.0, gain);
                                }
                                midi_schedule::Gate::On(pitch, velocity) => {
                                    self.tracks[ti]
                                        .poly
                                        .note_on_clip_with_gain(pitch, velocity as f32 / 127.0, gain);
                                }
                                midi_schedule::Gate::Off(pitch) if kind != 0 => {
                                    self.tracks[ti].poly.note_off_clip(pitch);
                                }
                                midi_schedule::Gate::Off(_) => {}
                            }
                        }
                    }
                }
                if let Some(playing) = self.tracks[ti].playing.as_mut() {
                    playing.last_beat = local;
                }
            }
            if ending {
                self.finish_recording_track(ti);
                self.tracks[ti].stop_clip();
            }
        }
        // Mute/solo gates the output, never the musical clock or DSP history.
        // Drum one-shots, synth releases and effect tails advance naturally.
        let s = if self.tracks[ti].kind == 0 {
            self.tick_drums(ti)
        } else {
            self.tracks[ti].poly.tick(self.sr)
        };
        let [pad_l, pad_r] = std::mem::take(&mut self.pad_output[ti]);
        let track = &mut self.tracks[ti];
        track.eq_right.low_g = track.eq.low_g;
        track.eq_right.mid_g = track.eq.mid_g;
        track.eq_right.high_g = track.eq.high_g;
        let mut raw = [s + pad_l + fallback[0], s + pad_r + fallback[1]];
        if let Some(input) = self.routing_track_input {
            raw[0] += input[0]; raw[1] += input[1];
        }
        let l = track.eq.tick(raw[0]);
        let r = track.eq_right.tick(raw[1]);
        let [fl, fr] = self.load_profile.chain(&mut track.fx, [l, r], self.sr, false, ti);
        track.meter = track.meter * 0.93 + if silent { 0.0 } else { (l.abs() + r.abs()) * 0.035 };
        let [gl, gr] = {
            #[cfg(test)]
            if self.legacy_gain_math {
                mixer_gain::pan_gains(track.gain, track.pan)
            } else { track.mixer_gain.tick() }
            #[cfg(not(test))]
            track.mixer_gain.tick()
        };
        let output = if silent { [0.0; 2] } else { [fl * gl, fr * gr] };
        self.routing_track_taps = [raw, [fl, fr], output];
        (output[0], output[1], false)
    }

    fn trig_drum(&mut self, ti: usize, pitch: u8, vel: f32) {
        self.trig_drum_with_gain(ti, pitch, vel, 1.0);
    }

    fn trig_drum_with_gain(&mut self, ti: usize, pitch: u8, vel: f32, clip_gain: f32) {
        // A zero-velocity onset is a release, never a new one-shot or a steal.
        // Existing finite hits keep playing under the drum release policy.
        if !vel.is_finite() || vel <= 0.0 { return; }
        let velocity = vel.min(1.0);
        let idx = match pitch {
            36 | 35 => 0, // kick
            38 | 40 => 1, // snare
            42 | 44 => 2, // closed hat
            39 | 37 => 3, // clap
            46 => 4,      // open hat
            _ => 5,       // tom
        };
        let slots = &mut self.tracks[ti].drum_pos;
        if let Some(slot) = slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(DrumVoice { sample: idx, position: 0.0, clip_gain, velocity });
        } else if let Some(slot) = slots.first_mut() {
            *slot = Some(DrumVoice { sample: idx, position: 0.0, clip_gain, velocity });
        }
    }

    fn tick_drums(&mut self, ti: usize) -> f32 {
        let mut s = 0.0;
        let sr = self.sr as f64;
        // The renderer exclusively owns the track. Borrow its immutable bank
        // beside the mutable voice slots; inactive slots never access samples.
        let track = &mut self.tracks[ti];
        let samples = &track.drum_samples;
        for slot in &mut track.drum_pos {
            if let Some(voice) = slot {
                let samp = &samples[voice.sample];
                let (l, _) = samp.at(voice.position);
                s += l * voice.clip_gain * voice.velocity;
                voice.position += samp.sr as f64 / sr;
                if voice.position >= samp.frames() as f64 {
                    *slot = None;
                }
            }
        }
        s
    }

    fn render_deck(&mut self, di: usize) -> (f32, f32) {
        let sr = self.sr as f64;
        {
            let d = &mut self.decks[di];
            let mapped_sync = d.sync.then(|| d.mapped_sync_step(self.sr)).flatten();
            if let Some((_,rate)) = mapped_sync { d.target_rate = rate; }
            else if d.sync {
                if d.audio.is_some() {
                    let bpm = d.musical_bpm();
                    if bpm > 1.0 { d.target_rate = d.sync_bpm / bpm; }
                }
            } else {
                d.target_rate = d.play_rate();
            }
            if d.touching {
                d.rate = d.scratch;
            } else {
                if mapped_sync.is_some() { d.rate = d.target_rate; }
                else { d.rate += (d.target_rate - d.rate) * d.rate_smoothing; }
                if d.keylock
                    && (d.target_rate == keylock::MIN_RATIO || d.target_rate == 1.0
                        || d.target_rate == keylock::MAX_RATIO)
                    && (d.rate - d.target_rate).abs() <= keylock::boundary_tolerance(d.rate_smoothing)
                {
                    // f32 smoothing otherwise stalls beside the exact target,
                    // missing unity bypass or supported-rate reentry. Only the
                    // three declared boundaries converge; arbitrary targets
                    // and unlocked playback retain the original trajectory.
                    d.rate = d.target_rate;
                }
                d.scratch *= 0.85;
            }
            let before_position = d.pos;
            if d.rendering() {
                if let Some((position,_)) = mapped_sync.filter(|_| !d.touching) { d.pos = position; }
                else { d.pos += d.rate as f64 * (d.audio.as_ref().map(|a| a.sr as f64).unwrap_or(sr) / sr); }
            }
            let mut position = d.pos;
            let natural_wrap = d.loop_on && d.loop_len > 1.0 && d.loop_start >= 0.0
                && d.audio.as_ref().is_some_and(|audio| d.loop_start + d.loop_len <= audio.frames() as f64)
                && before_position >= d.loop_start
                && before_position < d.loop_start + d.loop_len
                && position >= d.loop_start + d.loop_len;
            if d.loop_on && d.loop_len > 1.0 {
                if position >= d.loop_start + d.loop_len {
                    position = if mapped_sync.is_some() && !d.touching {
                        d.mapped_loop_wrap(position)
                    } else { None }.unwrap_or_else(|| d.loop_start + (position - d.loop_start) % d.loop_len);
                }
                if position < d.loop_start {
                    position = d.loop_start;
                }
            }
            if let Some(a) = &d.audio {
                if position >= a.frames() as f64 {
                    position = 0.0;
                    if !d.loop_on {
                        d.playing = false;
                        if let Some(saved) = d.preview_position.take() { position = saved; }
                    }
                }
                if position < 0.0 {
                    position = 0.0;
                }
            }
            if position != d.pos {
                if natural_wrap && d.keylock_mode() == keylock::Mode::Locked {
                    d.pos = position;
                    d.keylock_dsp.natural_wrap();
                } else {
                    d.transition_to(position, self.sr, DeckTransition::Jump);
                }
            }
        }
        {
            let d = &mut self.decks[di];
            if !d.playing {
                d.playback_active = false;
            } else if !d.playback_active && d.audio.as_ref().is_some_and(|sample| {
                let frames = sample.frames();
                sample.sr > 0 && frames >= 2 && d.pos.is_finite()
                    && d.pos >= 0.0 && d.pos < (frames - 1) as f64
            }) {
                // Credit source rendering, before gain/crossfader processing.
                // Loops stay in the same episode; an actually rendered pause,
                // EOF, replacement or unload ends it. No per-sample timestamp.
                if let Some(receipt) = &d.load_receipt { receipt.record_playback(); }
                d.playback_active = true;
            }
        }
        if !self.decks[di].rendering() {
            // Retire the last output through the bounded transition envelope.
            // A paused source must never be read repeatedly as a DC signal.
            let d = &mut self.decks[di];
            let [l, r] = d.transition_output([0.0; 2]);
            d.meter = d.meter * 0.9 + (l.abs() + r.abs()) * 0.05;
            self.routing_deck_taps = [[0.0; 2], [l, r]];
            let [l, r] = self.monitor.deck(di, [l, r]);
            return (l, r);
        }
        // Vinyl contact follows the hand directly; OLA resumes from the
        // release position rather than replaying grains from before the jog.
        let mode = {
            let d = &mut self.decks[di];
            let mode = d.keylock_mode();
            if mode != d.keylock_render_mode {
                if mode == keylock::Mode::Locked || d.keylock_render_mode == keylock::Mode::Locked {
                    let source_step = d.audio.as_ref().map_or(sr, |audio| audio.sr as f64) / sr;
                    d.keylock_dsp.reset(d.pos, source_step);
                    // A rate-mode transition retires overlap through the existing
                    // envelope but preserves the continuous deck filter histories.
                    // A seek/play/release already owns its two-millisecond
                    // envelope. Crossing the supported-rate boundary during
                    // that ramp must not restart or extend its settling time.
                    if d.transition_remaining == 0 {
                        d.fade_from_last_output(self.sr);
                    }
                }
                d.keylock_render_mode = mode;
            }
            mode
        };
        let (mut l, mut r) = if mode == keylock::Mode::Locked {
            self.deck_grain(di, sr)
        } else {
            self.decks[di].sample_at(self.decks[di].pos)
        };
        l *= self.decks[di].source_gain.linear();
        r *= self.decks[di].source_gain.linear();
        self.routing_deck_taps[0] = [l, r];
        let g = self.decks[di].gain;
        l *= g;
        r *= g;
        l = self.decks[di].eq[0].tick(l);
        r = self.decks[di].eq[1].tick(r);
        let deck = &mut self.decks[di];
        deck.filter_position = deck_filter::slew(deck.filter_position, deck.filter_amt, self.sr);
        let curve = deck_filter::Curve::at(deck.filter_position, self.sr);
        l = deck.filter[0].process(l, curve);
        r = deck.filter[1].process(r, curve);
        [l, r] = self.decks[di].transition_output([l, r]);
        self.decks[di].meter = self.decks[di].meter * 0.9 + ((l.abs() + r.abs()) * 0.5) * 0.1;
        self.routing_deck_taps[1] = [l, r];
        let [l, r] = self.monitor.deck(di, [l, r]);
        (l, r)
    }

    fn deck_grain(&mut self, di: usize, sr: f64) -> (f32, f32) {
        let d = &mut self.decks[di];
        let Some(audio) = &d.audio else { return (0.0, 0.0) };
        if !d.rendering() { return (0.0, 0.0); }
        let source = keylock::Source {
            audio,
            loop_on: d.loop_on,
            loop_start: d.loop_start,
            loop_len: d.loop_len,
        };
        d.keylock_dsp.render(source, d.pos, audio.sr as f64 / sr)
    }

    fn tick_pad_sources(&mut self) -> [[f32; 2]; session::MAX_TRACKS] {
        // Every source advances once, even when its destination is muted.
        // Routes outlive gate release so release envelopes keep their mixer.
        let mut buses = [[0.0; 2]; session::MAX_TRACKS];
        for (voice, filter) in self.sampler_poly.voices.iter_mut()
            .zip(self.sampler_poly.filters.iter_mut())
        {
            let sample = if self.sampler_poly.offline.is_some() { 0.0 } else { voice.tick(self.sr, voice.cutoff, filter) };
            if let Some(InputKey::Pad(pad)) = voice.input {
                let bus = &mut buses[self.pad_destinations[pad as usize % 16]];
                bus[0] += sample;
                bus[1] += sample;
            }
        }
        for slot in &mut self.pad_voices {
            if let Some(voice) = slot {
                if let Some((l, r)) = voice.tick(self.sr as f64) {
                    buses[voice.track][0] += l;
                    buses[voice.track][1] += r;
                }
                if voice.position >= voice.end && !voice.playback.repeat {
                    *slot = None;
                }
            }
        }
        if let Some(active) = &mut self.sampler_audition {
            let voice = &mut active.voice;
            if let Some((l, r)) = voice.tick(self.sr as f64) {
                buses[voice.track][0] += l; buses[voice.track][1] += r;
            }
            if voice.position >= voice.end { self.finish_sampler_audition(); }
        }
        buses
    }

    fn deck_touch(&mut self, source: u64, deck: u8, on: bool) {
        let d = &mut self.decks[deck as usize % DECKS];
        if on { d.stop_preview(self.sr); }
        let existing = d.touch_sources.iter().position(|owner| *owner == Some(source));
        if on {
            if existing.is_none() {
                if let Some(empty) = d.touch_sources.iter_mut().find(|owner| owner.is_none()) {
                    *empty = Some(source);
                }
            }
        } else if let Some(index) = existing {
            d.touch_sources[index] = None;
        }
        let touching = d.touch_sources.iter().any(Option::is_some);
        if d.touching != touching {
            d.touching = touching;
            d.transition_to(d.pos, self.sr, DeckTransition::Jump);
        }
        if !touching { d.scratch = 0.0; }
    }

    fn live_note_on(&mut self, source:u64, ch:u8, note:u8, vel:u8, t:usize) {
                let input = InputKey::Midi { source, ch: ch & 15, note };
                self.release_input(input);
                if vel == 0 {
                    return;
                }
                if self.tracks[t].kind == 0 {
                    self.trig_drum(t, note, vel as f32 / 127.0);
                } else {
                    self.tracks[t].poly.note_on_input(note, vel as f32 / 127.0, input);
                }
                if self.recording && self.playing {
                    let scene = self.selected_scene;
                    if self.recording_position(t, scene).is_none() { return; }
                    if !self.history_record_target(t,scene) {return;}
                    if self.tracks[t].clips[scene].kind == ClipKind::Empty {
                        let clip=&mut self.tracks[t].clips[scene];
                        clip.kind=ClipKind::Midi;clip.name.clear();clip.name.push_str("Take");clip.bars=1.0;clip.gain=1.0;
                    }
                    self.begin_recording_note(input, t, scene, note, vel);
                }
    }

    fn release_input(&mut self, input: InputKey) {
        self.finish_recording_input(input);
        // Voice pools are bounded; their exact gate metadata is the routing
        // record even after selection changes or a voice is stolen.
        let midi_beat = self.precise_midi_beat();
        for track in &mut self.tracks {
            track.poly.note_off_input(input);
            track.recorded_input_released(input, self.beat, midi_beat);
        }
        self.sampler_poly.note_off_input(input);
    }

    fn launch_start(&self) -> f64 {
        let quant = self.quant.max(0.0) as f64;
        if quant <= 0.0 || !self.playing {
            return self.beat;
        }
        let nearest = (self.beat / quant).round() * quant;
        if (self.beat - nearest).abs() <= midi_schedule::BEAT_EPSILON {
            nearest
        } else {
            (self.beat / quant).ceil() * quant
        }
    }

    fn launch_clip(&mut self, track: usize, scene: u16, start: f64) {
        let scene_index = scene as usize;
        if track >= self.tracks.len() || scene_index >= self.scene_fx.len() {
            return;
        }
        self.finish_recording_track(track);
        self.tracks[track].stop_clip();
        if self.tracks[track].clips[scene_index].occupied() {
            self.tracks[track].playing = Some(PlayingClip {
                scene,
                start_beat: start,
                midi_start_beat: start + (self.precise_midi_beat() - self.beat),
                last_beat: -0.0001,
                looping: true,
            });
            let midi_beat = self.precise_midi_beat();
            self.tracks[track].rebuild_midi_schedule(self.beat, midi_beat);
            self.start_count_in();
            self.playing = true;
            self.selected_track = track;
            self.selected_scene = scene_index;
        }
    }

    pub fn apply(&mut self, c: Command) {
        match c {
            Command::PerformanceMode(enabled) => { let _ = self.performance.set_enabled(enabled); return; }
            Command::SafetyStop(safety) => { self.performance.request_safety(safety); self.performance_tick(); return; }
            Command::RecoverPerformance => { let _ = self.performance.acknowledge_inputs_released(); return; }
            _ => {}
        }
        self.publish_deck_guards();
        if let Err(reason) = self.performance.check(&c, Some(self.deck_activity())) {
            self.performance.reject(reason);
            performance::reject_receipt(&c);
            self.undo.retire_command(c);
            return;
        }
        self.refresh_history_protection();
        match c {
            Command::Remote(request) => { self.remote_request(request); return; }
            Command::ProviderPreview(request) => { self.apply_provider_preview(request); return; }
            Command::Undo => {self.history_replay(false);return;}
            Command::Redo => {self.history_replay(true);return;}
            command @ Command::DeckCuePoint { .. } => {
                if let Command::DeckCuePoint { deck, pad, del, receipt } = &command {
                    if self.decks.get(*deck as usize).is_some_and(|d| {
                        d.audio.is_some() && (*pad as usize) < HOTCUES
                            && receipt.state() == load_receipt::State::Current
                            && d.load_receipt.as_ref().is_some_and(|r| r.same_request(receipt))
                    }) {
                        self.apply(Command::DeckHotCue { deck: *deck, pad: *pad, del: *del });
                    } else { self.undo.reject(undo::Failure::Invalid); }
                }
                self.undo.retire_command(command);
                return;
            }
            Command::SessionControl(mut scoped) => {
                if scoped.current(self) {
                    let next = std::mem::replace(scoped.command.as_mut(), Command::ComposeDisarm);
                    self.apply(next);
                } else {
                    self.undo.reject(undo::Failure::Invalid);
                    performance::reject_receipt(&scoped.command);
                }
                self.undo.retire_box(scoped.command); return;
            }
            Command::Gesture {id,mut command} => {
                let next=std::mem::replace(command.as_mut(),Command::ComposeDisarm);
                let old=self.undo.gesture_id();self.undo.set_gesture(id);
                self.apply(next);self.undo.set_gesture(old);
                self.undo.retire_box(command);return;
            }
            _=>{}
        }
        if matches!(&c,Command::SetNotes {notes,..} if notes.iter().any(|note|!note.interchange_valid())) {
            self.undo.reject(undo::Failure::Invalid);
            self.undo.retire_command(c);
            return;
        }
        if let Command::DeckAudio { deck, .. } | Command::DeckUnload { deck } = &c {
            if !self.performance.claim_deck_media(*deck as usize % DECKS, self.deck_activity(), None, None) {
                self.performance.reject(performance::Error::PlayingDeck);
                self.undo.retire_command(c);
                return;
            }
        }
        let Some(c)=self.history_before(c) else{return;};
        self.apply_plain(c);
    }
    fn apply_plain(&mut self, c: Command) {
        let preparation_deck = match &c {
            Command::DeckCue { deck } | Command::DeckHotCue { deck, .. } | Command::DeckCueStyle { deck, .. }
            | Command::DeckLoop { deck, .. } | Command::DeckLoopIn { deck }
            | Command::DeckLoopOut { deck } | Command::DeckLoopDouble { deck }
            | Command::DeckLoopHalf { deck } | Command::DeckReloop { deck }
            | Command::DeckSeek { deck, .. } => Some(*deck as usize % DECKS),
            _ => None,
        };
        // Validate before any command can launch, select, or alter another scene.
        // In particular, FireClip must not change a previous clip's looping flag
        // after an invalid LaunchClip, and Select must not clamp into a real cell.
        let scene = match &c {
            Command::LaunchClip { scene, .. }
            | Command::LaunchScene { scene }
            | Command::SetNotes { scene, .. }
            | Command::ClipGain { scene, .. }
            | Command::FireClip { scene, .. }
            | Command::ToggleScene { scene }
            | Command::RestartScene { scene }
            | Command::AddScene { scene }
            | Command::OpenFxScene(scene) => Some(*scene as usize),
            Command::Select { track, scene } | Command::ComposeArm { track, scene } => {
                if *track >= self.tracks.len() {
                    return;
                }
                Some(*scene)
            }
            _ => None,
        };
        if scene.is_some_and(|scene| scene >= self.scene_fx.len() || !self.session.scenes[scene].active) {
            return;
        }
        if let Some(deck) = preparation_deck { self.decks[deck].stop_preview(self.sr); }
        if matches!(&c,Command::Select {..}|Command::SelectDeck(_)|Command::SelectDeckRequested {..}|Command::SetView(_)|Command::OpenFxTrack(_)|Command::OpenFxScene(_)|Command::CloseFx) {self.undo.untracked_change();}
        if self.project_command_edits(&c) { self.project.edited(); }
        match c {
            Command::ProviderPreview(_)|Command::Undo|Command::Redo|Command::Gesture {..}|Command::DeckCuePoint {..}|Command::PerformanceMode(_)|Command::SafetyStop(_)|Command::RecoverPerformance=>unreachable!(),
            Command::ReservedStop { lane, ticket, target } => {
                if lane == 0 { self.apply(Command::Stop); }
                else if usize::from(lane) >= control::SAMPLER_STOP_BASE { self.apply(Command::SamplerSlotStop { pad: (usize::from(lane) - control::SAMPLER_STOP_BASE) as u8 }); }
                else if target.is_none_or(|reference| self.session.resolves(session::Axis::Track, usize::from(lane - 1), reference)) { self.apply(Command::StopTrack { track: lane - 1 }); }
                self.cmd_rx.complete_stop(lane as usize, ticket);
            }
            Command::MidiClock { source } => self.midi_clock.receive_tick(source),
            Command::Play => {
                self.start_count_in();
                self.resume_project_clips();
                self.playing = true;
            }
            Command::Stop => {
                self.transport_epoch = self.transport_epoch.wrapping_add(1);
                self.history_finish_take();
                self.finish_recording_all();
                self.playing = false;
                for deck in &mut self.decks { deck.stop_preview(self.sr); }
                self.recording = false;
                self.compose_target = None;
                self.metro.reset();
                self.count_in = None;
                for t in &mut self.tracks {
                    t.stop_clip();
                }
                for t in 0..self.tracks.len() {self.midi_routing.clear_clip(t as u8);}
            }
            Command::TimelineSeek(seconds) => self.seek_timeline(seconds),
            Command::TogglePlay => {
                if self.playing {
                    self.apply(Command::Stop);
                } else {
                    self.start_count_in();
                    self.resume_project_clips();
                    self.playing = true;
                    // launch scene 0 if nothing running
                    if self.tracks.iter().all(|t| t.playing.is_none()) {
                        self.apply(Command::LaunchScene { scene: 0 });
                    }
                }
            }
            Command::Record => {
                self.history_finish_take();
                if self.recording {
                    self.finish_recording_all();
                }
                self.recording = !self.recording;
            }
            Command::Tap(t) => {
                self.retire_conductor();
                self.tap.push(t);
                self.tap.retain(|x| t.duration_since(*x) < Duration::from_secs(3));
                if self.tap.len() >= 2 {
                    let dts: Vec<f32> = self.tap.windows(2).map(|w| w[1].duration_since(w[0]).as_secs_f32()).collect();
                    let avg = dts.iter().sum::<f32>() / dts.len() as f32;
                    if avg > 0.2 && avg < 1.5 {
                        self.bpm = (60.0 / avg).clamp(60.0, 200.0);
                    }
                }
            }
            Command::SetBpm(b) => { self.retire_conductor(); self.bpm = b.clamp(40.0, 240.0); },
            Command::NudgeBpm(d) => { self.retire_conductor(); self.bpm = (self.bpm + d).clamp(40.0, 240.0); },
            Command::LaunchClip { track, scene } => {
                self.launch_clip(track as usize, scene, self.launch_start());
            }
            Command::LaunchScene { scene } => {
                // Capture before the first launch starts a stopped transport.
                let start = self.launch_start();
                for t in 0..self.tracks.len() {
                    self.launch_clip(t, scene, start);
                }
            }
            Command::StopTrack { track } => {
                if (track as usize) < self.tracks.len() {
                    self.finish_recording_track(track as usize);
                    self.tracks[track as usize].stop_clip();
                }
            }
            Command::DeckPlay { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.stop_preview(self.sr);
                if d.audio.is_none() {
                    return;
                }
                d.playing = !d.playing;
                let pos = if d.playing && d.pos < 1.0 { d.cue_pos } else { d.pos };
                d.transition_to(pos, self.sr, DeckTransition::Jump);
            }
            Command::DeckCue { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.stop_preview(self.sr);
                if d.playing {
                    d.playing = false;
                    d.transition_to(d.cue_pos, self.sr, DeckTransition::Jump);
                } else {
                    d.cue_pos = d.pos;
                }
            }
            Command::DeckPreview { deck, expected, on } => {
                let Some(d) = self.decks.get_mut(usize::from(deck)) else { return; };
                if expected == 0 || d.history_key != expected { return; }
                if on {
                    if d.playing || d.touching || d.audio.is_none() || d.preview_position.is_some() { return; }
                    d.preview_position = Some(d.pos);
                    d.transition_to(d.cue_pos, self.sr, DeckTransition::Jump);
                } else { d.stop_preview(self.sr); }
            }
            Command::DeckSync { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.sync = !d.sync;
            }
            Command::DeckJog { deck, delta } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if d.touching || !d.playing {
                    d.transition_to(d.pos + delta as f64 * 400.0, self.sr, DeckTransition::Jog);
                    d.scratch = delta * 18.0;
                } else {
                    d.rate = (d.target_rate + delta * 0.15).clamp(0.0, 4.0);
                }
            }
            Command::DeckTouch { deck, on } => {
                self.deck_touch(0, deck, on);
            }
            Command::MidiDeckTouch { source, deck, on } => {
                self.deck_touch(source, deck, on);
            }
            Command::DeckPitch { deck, value } => {
                self.decks[deck as usize % DECKS].pitch = value.clamp(0.0, 1.0);
            }
            Command::DeckGain { deck, value } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.eq_store[3] = value.clamp(0.0, 1.5);
                if !d.eq_cut[3] {
                    d.gain = d.eq_store[3];
                }
            }
            Command::DeckEq { deck, band, value } => {
                let g = eq_gain(value);
                let d = &mut self.decks[deck as usize % DECKS];
                d.eq_store[band as usize % 4] = g;
                if !d.eq_cut[band as usize % 4] && (d.eq_solo < 0 || d.eq_solo == band as i8) {
                    for eq in &mut d.eq {
                        match band {
                            0 => eq.low_g = g,
                            1 => eq.mid_g = g,
                            _ => eq.high_g = g,
                        }
                    }
                }
            }
            Command::DeckFilter { deck, value } => {
                if value.is_finite() {
                    self.decks[deck as usize % DECKS].filter_amt = value.clamp(0.0, 1.0);
                }
            }
            Command::DeckPfl { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.pfl = !d.pfl;
            }
            Command::DeckHotCue { deck, pad, del } => {
                let d = &mut self.decks[deck as usize % DECKS];
                let i = pad as usize % HOTCUES;
                if del {
                    d.hotcues[i].set = false;
                    d.cue_styles[i] = cue_metadata::Style::default();
                } else if d.hotcues[i].set {
                    d.transition_to(d.hotcues[i].pos, self.sr, DeckTransition::Jump);
                    d.playing = true;
                } else {
                    d.hotcues[i] = HotCue {
                        set: true,
                        pos: d.pos,
                    };
                }
            }
            command @ Command::DeckGrid { .. } => {
                if let Command::DeckGrid { deck, grid, ack, .. } = &command {
                    let deck = &mut self.decks[*deck as usize];
                    deck.grid = *grid;
                    deck.publish_preparation();
                    // Release acknowledgement only after coherent preparation is visible.
                    ack.applied();
                }
                self.undo.retire_command(command);
            }
            command @ Command::DeckSourceGain { .. } => {
                if let Command::DeckSourceGain { deck, gain, ack, .. } = &command {
                    let deck = &mut self.decks[*deck as usize];
                    deck.source_gain = *gain;
                    deck.publish_preparation();
                    ack.applied();
                }
                self.undo.retire_command(command);
            }
            command @ Command::DeckCueStyle { .. } => {
                if let Command::DeckCueStyle { deck, pad, style, .. } = &command {
                    // Identity, indices, set state and no-op were checked before
                    // history capture. The renderer is the sole deck writer.
                    self.decks[*deck as usize].cue_styles[*pad as usize] = *style;
                }
                self.undo.retire_command(command);
            }
            Command::DeckLoop { deck, beats } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if d.loop_on {
                    d.loop_on = false;
                } else {
                    d.loop_on = true;
                    d.loop_start = if self.quantize && d.grid.is_some() { d.grid_snap(d.pos, self.sr, self.bpm) } else { d.pos };
                    d.loop_len = d.grid_span(d.loop_start, beats as f64, self.sr, self.bpm);
                }
                d.transition_to(d.pos, self.sr, DeckTransition::Jump);
            }
            Command::DeckLoopIn { deck } => {
                let q = self.quantize;
                let d = &mut self.decks[deck as usize % DECKS];
                let mut pos = d.pos;
                if q {
                    pos = d.grid_snap(pos, self.sr, self.bpm);
                }
                d.loop_start = pos;
                if d.loop_on {
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                }
            }
            Command::DeckLoopOut { deck } => {
                let q = self.quantize;
                let d = &mut self.decks[deck as usize % DECKS];
                let mut pos = d.pos;
                if q {
                    pos = d.grid_snap(pos, self.sr, self.bpm);
                }
                d.loop_len = (pos - d.loop_start).abs().max(64.0);
                d.loop_on = true;
                d.transition_to(d.pos, self.sr, DeckTransition::Jump);
            }
            Command::DeckVinyl { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.vinyl = !d.vinyl;
            }
            Command::DeckKeylock { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.keylock = !d.keylock;
                d.transition_to(d.pos, self.sr, DeckTransition::Jump);
            }
            Command::LibraryFence { acknowledged } => {
                acknowledged.store(true, std::sync::atomic::Ordering::Release);
                self.undo.retire_command(Command::LibraryFence { acknowledged });
            }
            Command::DeckRestorePreparation { deck, receipt, preparation } => {
                if let Some(d) = self.decks.get_mut(deck as usize) {
                    if d.load_receipt.as_ref().is_some_and(|current| current.same_request(&receipt))
                        && receipt.preparation().is_some_and(|(revision, _)| revision == 2)
                        && preparation.source_gain == d.source_gain.policy() {
                        d.restore_preparation(preparation);
                        d.publish_preparation();
                        self.project.edited();
                        self.undo.untracked_change();
                    }
                }
                self.undo.retire_command(Command::DeckRestorePreparation { deck, receipt, preparation });
            }
            command @ Command::DeckLoadRequested { .. } => {
                if let Command::DeckLoadRequested { deck, media, receipt } = &command {
                    self.apply_media_request(*deck, media, receipt);
                }
                // The request retains any final token, receipt and rejected PCM
                // references until the recycler owns the complete command.
                self.undo.retire_command(command);
            }
            command @ Command::DeckDecoded { .. } => {
                if let Command::DeckDecoded { request, audio } = &command {
                    if (request.deck as usize) < DECKS && request.is_current() {
                        self.apply(Command::DeckAudio { deck: request.deck, audio: audio.clone() });
                    }
                }
                self.undo.retire_command(command);
            }
            Command::DeckAudio { deck, audio } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if let Some(receipt) = d.load_receipt.take() { receipt.supersede(); }
                d.playback_active = false;
                d.clear_loop();
                d.title.clear();
                d.title.push_str(&audio.name);
                d.bpm = audio.bpm;
                d.cue_pos = 0.0;
                d.playing = false;
                d.preview_position = None;
                d.cue_styles = [cue_metadata::Style::default(); HOTCUES];
                d.grid = None;
                d.source_gain = crate::track_gain::Resolved::default();
                d.hotcues = std::array::from_fn(|_| HotCue {
                    set: false,
                    pos: 0.0,
                });
                d.audio = Some(audio);
                d.history_key = history_measurement::parts::unresolved_key();
                d.transition_to(0.0, self.sr, DeckTransition::Jump);
            }
            Command::DeckUnload { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if let Some(receipt) = d.load_receipt.take() { receipt.supersede(); }
                d.playback_active = false;
                d.audio = None;
                d.history_key = 0;
                d.title.clear();
                d.playing = false;
                d.preview_position = None;
                d.cue_pos = 0.0;
                d.clear_loop();
                d.cue_styles = [cue_metadata::Style::default(); HOTCUES];
                d.grid = None;
                d.source_gain = crate::track_gain::Resolved::default();
                d.hotcues = std::array::from_fn(|_| HotCue {
                    set: false,
                    pos: 0.0,
                });
                d.transition_to(0.0, self.sr, DeckTransition::Jump);
            }
            Command::DeckSeek { deck, frac } => {
                let d = &mut self.decks[deck as usize % DECKS];
                let frames = d.audio.as_ref().map(|a| a.frames() as f64).unwrap_or(0.0);
                d.transition_to((frac.clamp(0.0, 1.0) as f64 * frames).max(0.0), self.sr, DeckTransition::Jump);
                d.cue_pos = d.pos;
            }
            Command::DeckLoadLock { deck, enabled } => {
                let _ = self.performance.set_deck_load_lock(deck as usize, enabled);
            }
            Command::DeckLoadSelected { .. } | Command::PrepareSelected { .. } => {
                // Producers capture selection before routing to the GUI.
                // A raw renderer call without that capture must fail visibly.
                self.cmd_rx.reject_uncaptured_ui_load();
            }
            command @ Command::DeckLoadFile { .. } => {
                self.cmd_rx.reject_uncaptured_ui_load();
                self.undo.retire_command(command);
            }
            Command::Xfader(v) => self.xfader = v.clamp(0.0, 1.0),
            Command::Master(v) => self.master = v.clamp(0.0, 1.5),
            Command::CueMix(v) => self.cue_mix = v.clamp(0.0, 1.0),
            Command::Monitor(control) => self.monitor.apply(control),
            Command::TrackGain { track, value } => {
                if (track as usize) < self.tracks.len() {
                    self.tracks[track as usize].gain = value.clamp(0.0, 1.5);
                }
            }
            Command::ClipGain { track, scene, value } => {
                if value.is_finite() {
                    if let Some(track) = self.tracks.get_mut(track as usize) {
                        track.clips[scene as usize].gain = clip_gain(value);
                    }
                }
            }
            Command::TrackPan { track, value } => {
                if (track as usize) < self.tracks.len() {
                    self.tracks[track as usize].pan = (value * 2.0 - 1.0).clamp(-1.0, 1.0);
                }
            }
            Command::Mute { track } => {
                if (track as usize) < self.tracks.len() {
                    self.tracks[track as usize].mute = !self.tracks[track as usize].mute;
                }
            }
            Command::Solo { track } => {
                if (track as usize) < self.tracks.len() {
                    self.tracks[track as usize].solo = !self.tracks[track as usize].solo;
                }
            }
            Command::Arm { track } => {
                if (track as usize) < self.tracks.len() {
                    self.tracks[track as usize].armed = !self.tracks[track as usize].armed;
                }
            }
            Command::Browse(_) | Command::BrowseCrates(_) | Command::CrateReturn => {
                // Browsing must resolve the GUI's published filtered view on a
                // control producer. The renderer has no independent selection.
                self.cmd_rx.reject_uncaptured_ui_load();
            }
            Command::Select { track, scene } => {
                self.selected_track = track;
                self.selected_scene = scene;
            }
            Command::ComposeArm { track, scene } => {
                // Only an explicit arm changes the write destination. Browsing
                // and playback are free to change their own selection.
                let target = ComposeTarget { track, scene };
                if self.compose_target != Some(target) {
                    self.finish_recording_pads();
                }
                self.compose_target = Some(target);
                self.selected_track = track;
                self.selected_scene = scene;
                let clip = &mut self.tracks[track].clips[scene];
                if clip.kind == ClipKind::Empty {
                    clip.kind = ClipKind::Midi;
                    clip.name.clear();clip.name.push_str("Clip");
                    clip.bars = 1.0;
                }
            }
            Command::ComposeDisarm => {
                self.history_finish_take();
                if self.compose_target.take().is_some() {
                    self.finish_recording_pads();
                }
            }
            Command::SelectDeck(d) => self.selected_deck = d.min(DECKS - 1),
            Command::SelectDeckRequested { deck, request } => {
                self.selected_deck = deck.min(DECKS - 1);
                self.selected_deck_request = request;
            }
            Command::SetView(v) => self.view = v,
            Command::RoutedNoteOn { source, ch, note, vel, track, target } => {
                if usize::from(track) < self.tracks.len() && self.session.tracks[usize::from(track)].active && target.is_none_or(|reference| self.session.resolves(session::Axis::Track, usize::from(track), reference)) && ch<=15 && note<=127 && vel<=127 {
                    self.live_note_on(source,ch,note,vel,usize::from(track));
                }
            }
            Command::LiveNoteOn { source, ch, note, vel } => self.live_note_on(source,ch,note,vel,self.selected_track),
            Command::LiveNoteOff { source, ch, note } => {
                self.release_input(InputKey::Midi { source, ch: ch & 15, note });
            }
            Command::MidiEdit(request) => self.apply_midi_edit(request),
            Command::Remote(_) | Command::SessionControl(_) | Command::SessionEdit(_) | Command::MidiImport(_) => unreachable!("import is applied atomically in history admission"),
            Command::MidiAudition { id, track, note, vel, on } => {
                let input = InputKey::Preview(id);
                self.release_input(input);
                if on && (track as usize) < self.tracks.len() && note <= 127 && vel <= 127 {
                    let t = track as usize;
                    if self.tracks[t].kind == 0 { self.trig_drum(t, note, vel as f32 / 127.0); }
                    else { self.tracks[t].poly.note_on_input(note, vel as f32 / 127.0, input); }
                }
            }
            Command::SetNotes { track, scene, notes } => {
                let t = track as usize;
                let s = scene as usize;
                if t < self.tracks.len() && s < self.scene_fx.len() {
                    // Replacing the note list cancels captures into that list;
                    // a later physical release must not alter the replacement.
                    self.cancel_recording_clip(t, s);
                    if self.tracks[t].playing.or(self.tracks[t].project_resume).map(|p| p.scene) == Some(scene) {
                        self.tracks[t].release_clip_notes();
                    }
                    if self.tracks[t].clips[s].kind == ClipKind::Empty {
                        self.tracks[t].clips[s].kind = ClipKind::Midi;
                        self.tracks[t].clips[s].name.clear();self.tracks[t].clips[s].name.push_str("Clip");
                        self.tracks[t].clips[s].bars = 1.0;
                    }
                    self.tracks[t].clips[s].notes = notes;
                    let midi_beat = self.precise_midi_beat();
                    self.tracks[t].clip_notes_changed(s, self.beat, midi_beat);
                }
            }
            Command::FxWet { slot, value } => {
                if let Some(wet) = self.fx_wet.get_mut(slot as usize) {
                    if value.is_finite() { *wet = value.clamp(0.0, 1.0); }
                }
            }
            Command::FxSelect { slot } => {
                if let Some(kind) = self.fx_kind.get_mut(slot as usize) {
                    *kind = kind.next();
                    self.master_fx[slot as usize].reset(*kind);
                }
            }
            Command::Quant(q) => self.quant = q,
            Command::Metronome => {
                self.metronome = !self.metronome;
                self.metro.reset();
            }
            Command::ToggleQuant => {
                self.quantize = !self.quantize;
                self.quant = if self.quantize { 1.0 } else { 0.0 };
            }
            Command::DeckLoopDouble { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if d.loop_on {
                    d.loop_len = if d.grid.is_some() {
                        let beats = d.grid_beats_between(d.loop_start, d.loop_start + d.loop_len, self.sr, self.bpm);
                        d.grid_span(d.loop_start, beats * 2.0, self.sr, self.bpm)
                    } else { d.loop_len * 2.0 };
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                }
            }
            Command::DeckLoopHalf { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                if d.loop_on {
                    d.loop_len = if d.grid.is_some() {
                        let beats = d.grid_beats_between(d.loop_start, d.loop_start + d.loop_len, self.sr, self.bpm);
                        d.grid_span(d.loop_start, beats * 0.5, self.sr, self.bpm)
                    } else { d.loop_len * 0.5 }.max(64.0);
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                }
            }
            Command::DeckReloop { deck } => {
                let q = self.quantize;
                let d = &mut self.decks[deck as usize % DECKS];
                let mut pos = d.pos;
                if q {
                    pos = d.grid_snap(pos, self.sr, self.bpm);
                }
                d.loop_start = pos;
                d.loop_len = d.grid_span(pos, 16.0, self.sr, self.bpm);
                d.loop_on = true;
                d.transition_to(d.pos, self.sr, DeckTransition::Jump);
            }
            Command::DeckMatch => {
                let fav = if self.xfader <= 0.5 { 0 } else { 1 };
                let oth = 1 - fav;
                if self.decks[fav].audio.is_none() { return; }
                let target_bpm = if self.decks[fav].sync && self.decks[fav].grid.is_some_and(|grid| !grid.anchors().is_empty()) { self.decks[fav].sync_bpm }
                    else { self.decks[fav].musical_bpm().max(1.0) * self.decks[fav].pitch_rate() };
                self.decks[oth].sync = true;
                self.decks[oth].sync_bpm = target_bpm;
                if self.decks[fav].playing && self.decks[oth].playing {
                    if self.decks[oth].audio.is_some() {
                        let phase = self.decks[fav].grid_phase(self.decks[fav].pos, self.sr, self.bpm);
                        let beat = self.decks[oth].grid_beat_at(self.decks[oth].pos, self.sr, self.bpm).floor();
                        let mut position = self.decks[oth].grid_position_at(beat + phase, self.sr, self.bpm);
                        if self.decks[oth].grid.is_some() {
                            position = position.clamp(0.0, self.decks[oth].audio.as_ref().unwrap().frames() as f64);
                        }
                        self.decks[oth].transition_to(position, self.sr, DeckTransition::Jump);
                    }
                }
            }
            Command::DeckEqCut { deck, band } => {
                let d = &mut self.decks[deck as usize % DECKS];
                let b = band as usize % 4;
                d.eq_cut[b] = !d.eq_cut[b];
                if b < 3 {
                    let g = if d.eq_cut[b] { 0.0 } else { d.eq_store[b] };
                    for eq in &mut d.eq {
                        match b {
                            0 => eq.low_g = g,
                            1 => eq.mid_g = g,
                            _ => eq.high_g = g,
                        }
                    }
                } else {
                    d.gain = if d.eq_cut[3] { 0.0 } else { d.eq_store[3] };
                }
            }
            Command::DeckEqSolo { deck, band } => {
                let d = &mut self.decks[deck as usize % DECKS];
                let b = band as i8;
                if d.eq_solo == b {
                    d.eq_solo = -1;
                    for eq in &mut d.eq {
                        eq.low_g = if d.eq_cut[0] { 0.0 } else { d.eq_store[0] };
                        eq.mid_g = if d.eq_cut[1] { 0.0 } else { d.eq_store[1] };
                        eq.high_g = if d.eq_cut[2] { 0.0 } else { d.eq_store[2] };
                    }
                } else {
                    d.eq_solo = b;
                    for eq in &mut d.eq {
                        eq.low_g = if b == 0 { d.eq_store[0] } else { 0.0 };
                        eq.mid_g = if b == 1 { d.eq_store[1] } else { 0.0 };
                        eq.high_g = if b == 2 { d.eq_store[2] } else { 0.0 };
                        if b == 3 {
                            eq.low_g = d.eq_store[0];
                            eq.mid_g = d.eq_store[1];
                            eq.high_g = d.eq_store[2];
                        }
                    }
                }
            }
            Command::DeckPitchRange { deck } => {
                let d = &mut self.decks[deck as usize % DECKS];
                d.pitch_range = (d.pitch_range + 1) % 3;
            }
            Command::FireClip { track, scene, looping } => {
                self.apply(Command::LaunchClip { track, scene });
                if let Some(p) = self.tracks.get_mut(track as usize).and_then(|t| t.playing.as_mut()) {
                    p.looping = looping;
                }
            }
            Command::ToggleScene { scene } => {
                let active = self.tracks.iter().any(|t| t.playing.map(|p| p.scene) == Some(scene));
                if active {
                    for t in 0..self.tracks.len() {
                        if self.tracks[t].playing.or(self.tracks[t].project_resume).map(|p| p.scene) == Some(scene) {
                            self.finish_recording_track(t);
                            self.tracks[t].stop_clip();
                        }
                    }
                } else {
                    self.apply(Command::LaunchScene { scene });
                }
            }
            Command::RestartScene { scene } => {
                self.apply(Command::LaunchScene { scene });
            }
            Command::AddScene { scene } => {
                let start = self.launch_start();
                for t in 0..self.tracks.len() {
                    if self.tracks[t].clips[scene as usize].occupied() {
                        self.launch_clip(t, scene, start);
                    }
                }
            }
            Command::LoadBuiltin { deck, stem } => {
                if let Some(a) = self.builtin.get(stem as usize).and_then(|s| s.clone()) {
                    self.apply(Command::DeckAudio { deck, audio: a });
                }
            }
            Command::SamplerSlotStop { pad } => self.stop_sampler_slot(pad),
            Command::SamplerPad { pad, on } => self.apply_sampler_pad(pad % 16, on, 1.0),
            Command::SamplerPadPressure { pad, pressure } => {
                if pad < 16 && pressure.is_finite() && (0.0..=1.0).contains(&pressure) { self.apply_sampler_pad(pad, true, pressure); }
            },
            Command::SamplerBank(i) => self.sampler_bank = i.min(self.sampler_banks.len().saturating_sub(1)),
            Command::SamplerEdit(edit) => self.apply_sampler_edit(edit),
            Command::SamplerAudition(request) => self.apply_sampler_audition(request),
            Command::SamplerAuditionStop { id } => {
                if self.sampler_audition.as_ref().is_some_and(|active| active.id == id) {
                    self.finish_sampler_audition();
                }
            },
            Command::SamplerInst(i) => {
                if i.synth().is_some() && self.sampler_poly.offline.is_some() {
                    if !self.undo.can_retire_device(self.sampler_poly.offline.as_ref().unwrap().bytes()) { return; }
                    let device = self.sampler_poly.offline.take().unwrap();
                    self.undo.retire_device(device);
                    self.sampler_poly.set_sample_rate(self.sr);
                }
                self.sampler_inst = i;
                if let Some(kind) = i.synth() {
                    // Selection affects new gates. Held voices keep their
                    // original instrument, envelope and mixer destination.
                    self.sampler_poly.select_instrument(kind);
                }
            }
            Command::SamplerOct(d) => {
                let old = self.sampler_oct;
                self.sampler_oct = (self.sampler_oct as i16 + d as i16).clamp(1, 7) as i8;
                let dn = (self.sampler_oct - old) * 12;
                if dn != 0 {
                    for (pad, target) in self.pad_targets.iter_mut().enumerate() {
                        if let Some(target) = target {
                            target.pitch = (target.pitch as i16 + dn as i16).clamp(0, 127) as u8;
                            let input = InputKey::Pad(pad as u8);
                            self.sampler_poly.transpose_input(input, dn);
                        }
                    }
                    let rate = 2f32.powi((self.sampler_oct - 3) as i32);
                    for slot in &mut self.pad_voices {
                        if let Some(voice) = slot {
                            voice.rate = rate as f64;
                        }
                    }
                }
            }
            Command::OpenFxTrack(t) => {
                self.fx_view = if self.fx_view == t as i16 { -1 } else { t as i16 };
            }
            Command::OpenFxScene(s) => {
                let id = session::SCENE_FX_BASE + s as i16;
                self.fx_view = if self.fx_view == id { -1 } else { id };
            }
            Command::CloseFx => self.fx_view = -1,
            Command::FxAdd(kind) => {
                if self.active_chain().slots.len()>=128 {return;}
                let Some(&id) = fx::FxId::all().get(kind as usize) else { return };
                if !id.supports_scene() && !(0..self.tracks.len() as i16).contains(&self.fx_view) {
                    return;
                }
                let slot = fx::FxSlot::new(id, self.sr);
                self.active_chain().slots.push(slot);
            }
            Command::FxToggle(slot) => {
                let c = self.active_chain();
                if let Some(s) = c.slots.get_mut(slot) {
                    s.on = !s.on;
                }
            }
            Command::FxMix { slot, value } => {
                let c = self.active_chain();
                if let Some(s) = c.slots.get_mut(slot) {
                    s.set_control(None, value);
                }
            }
            Command::FxParam { slot, p, value } => {
                let c = self.active_chain();
                if let Some(s) = c.slots.get_mut(slot) {
                    s.set_control(Some(p), value);
                }
            }
        }
        if let Some(deck) = preparation_deck { self.decks[deck].publish_preparation(); }
        self.publish_deck_guards();
    }

    fn apply_media_request(&mut self, deck: u8, media: &load_receipt::Media,
        receipt: &load_receipt::Receipt) {
        use load_receipt::{Media, State};
        if receipt.state() != State::Pending { return; }
        if deck as usize >= DECKS {
            if receipt.claim() { receipt.finish(State::Unavailable); }
            return;
        }
        if receipt.deck_generation().is_some_and(|generation|generation!=self.performance.deck_load_word(deck as usize)) {
            receipt.supersede();
            return;
        }
        if matches!(media, Media::Unload) {
            if !receipt.claim() { return; }
            if !self.performance.claim_deck_media(deck as usize, self.deck_activity(), receipt.deck_approval(), receipt.deck_generation()) {
                self.performance.reject(performance::Error::PlayingDeck);
                receipt.finish(State::Protected);
                return;
            }
            let Some(command) = self.history_before(Command::DeckUnload { deck }) else {
                receipt.finish(State::Unavailable);
                return;
            };
            self.apply_plain(command);
            receipt.finish(State::Current);
            return;
        }
        let audio = match media {
            Media::Unload => unreachable!(),
            Media::Builtin(stem) => {
                if receipt.source_level().is_some() && receipt.source_level() != self.builtin_levels.get(*stem as usize).copied().flatten() {
                    receipt.supersede(); return;
                }
                self.builtin.get(*stem as usize).and_then(Clone::clone)
            },
            Media::Decoded { token, audio } => {
                if token.deck != deck || !token.is_current() {
                    receipt.supersede();
                    return;
                }
                Some(audio.clone())
            }
        };
        #[cfg(test)]
        if let Some(hook) = self.load_test_hooks[0].take() { hook(); }
        // Cancellation and application race for one atomic transition. The
        // caller still owns the complete request if cancellation wins here.
        if receipt.source_gain().is_none() { if receipt.claim() { receipt.finish(State::Unavailable); } return; }
        if !receipt.claim() { return; }
        #[cfg(test)]
        if let Some(hook) = self.load_test_hooks[1].take() { hook(); }
        if !self.performance.claim_deck_media(deck as usize, self.deck_activity(), receipt.deck_approval(), receipt.deck_generation()) {
            self.performance.reject(performance::Error::PlayingDeck);
            receipt.finish(State::Protected);
            return;
        }
        if let Some(audio) = audio {
            // Application acknowledgement follows the actual mutation outcome,
            // not an unrelated retirement notice published during the capture.
            let Some(command) = self.history_before(Command::DeckAudio { deck, audio }) else {
                receipt.finish(State::Unavailable);
                return;
            };
            self.apply_plain(command);
            let d = &mut self.decks[deck as usize];
            d.source_gain = receipt.source_gain().unwrap_or_default();
            if let Some(preparation) = receipt.initial_preparation() { d.restore_preparation(preparation); }
            d.history_key = receipt.history_key();
            d.load_receipt = Some(receipt.clone());
            d.publish_preparation();
            receipt.finish(State::Current);
        } else { receipt.finish(State::Unavailable); }
    }

    // Playing targets use their own launch origin. An unlaunched target has
    // an explicit compose cursor at zero; pending targets are monitor-only.
    fn recording_position(&self, track: usize, scene: usize) -> Option<f32> {
        if self.count_in.is_some() { return None; }
        let t = self.tracks.get(track)?;
        let clip = t.clips.get(scene)?;
        match t.playing.filter(|p| p.scene as usize == scene) {
            Some(p) if if clip.region.is_some() { self.precise_midi_beat() < p.midi_start_beat } else { self.beat < p.start_beat } => None,
            Some(p) => clip.region.map_or_else(|| Some((self.beat - p.start_beat)
                .rem_euclid(clip.bars.max(0.25) as f64 * 4.0) as f32),
                |region| region.position(self.precise_midi_beat() - p.midi_start_beat, p.looping).map(|position| position as f32)),
            None => Some(clip.region.map_or(0.0, |region| region.start as f32)),
        }
    }

    fn active_chain(&mut self) -> &mut fx::FxChain {
        let scene_count = self.scene_fx.len(); let track_count = self.tracks.len();
        if self.fx_view >= session::SCENE_FX_BASE {
            &mut self.scene_fx[(self.fx_view as usize - session::SCENE_FX_BASE as usize).min(scene_count - 1)]
        } else if self.fx_view >= 0 {
            &mut self.tracks[self.fx_view as usize % track_count].fx
        } else {
            &mut self.scene_fx[usize::from(self.session.scene_order[0])]
        }
    }

    /// Render native channel frames.
    /// Takes an interleaved buffer and channel count; writes complete frames and silences incomplete tails.
    pub fn process_interleaved(&mut self, data: &mut [f32], ch: usize) {
        let channels = ch.max(1);
        let complete = data.len() / channels * channels;
        data[complete..].fill(0.0);
        self.process_channels(&mut data[..complete], channels);
    }

}

fn sampler_pitch(_inst: SamplerInstrument, oct: i8, pad: u8) -> u8 {
    sampler_pad::PadIdentity::new(pad.min(15)).midi_note(oct)
}

fn eq_gain(v: f32) -> f32 {
    // 0 = kill, 0.5 = unity, 1 = +12dB-ish
    if v <= 0.5 {
        (v * 2.0).powf(1.4)
    } else {
        1.0 + (v - 0.5) * 2.4
    }
}

fn splat(buf: &mut [f32], at: usize, samp: &Sample, vel: f32) {
    let n = samp.frames();
    for i in 0..n {
        let dst = at + i;
        if dst * 2 + 1 >= buf.len() {
            break;
        }
        let (l, r) = samp.at(i as f64);
        buf[dst * 2] += l * vel;
        buf[dst * 2 + 1] += r * vel;
    }
}

fn demo_stems(sr: u32, bpm: f32) -> (Arc<Sample>, Arc<Sample>) {
    let spb = sr as f64 * 60.0 / bpm.max(1.0) as f64;
    let frames = (spb * 16.0) as usize;
    let kit = build_kit(sr);
    let mut drums = vec![0.0f32; frames * 2];
    let mut harm = vec![0.0f32; frames * 2];
    for b in 0..16 {
        let t = b as f64;
        splat(&mut drums, (t * spb) as usize, &kit[0], if b % 2 == 0 { 1.0 } else { 0.86 });
        splat(&mut drums, (t * spb) as usize, &kit[2], 0.5);
        splat(&mut drums, ((t + 0.5) * spb) as usize, &kit[2], 0.68);
        if b % 4 == 1 || b % 4 == 3 {
            splat(&mut drums, (t * spb) as usize, &kit[1], 0.95);
        }
    }
    splat(&mut drums, (14.5 * spb) as usize, &kit[3], 0.8);
    splat(&mut drums, (15.0 * spb) as usize, &kit[4], 0.55);

    let bass = [
        (0.0f32, 36u8, 0.7f32, 100u8),
        (0.75, 36, 0.2, 80),
        (1.5, 43, 0.45, 96),
        (2.5, 41, 0.45, 90),
        (3.0, 36, 0.4, 100),
        (3.5, 38, 0.4, 86),
    ];
    let mut events: Vec<(usize, bool, u8, f32)> = Vec::new();
    for bar in 0..4 {
        for (st, pitch, len, vel) in bass {
            let on = ((bar as f64 * 4.0 + st as f64) * spb) as usize;
            let off = on + (len as f64 * spb) as usize;
            events.push((on, true, pitch, vel as f32 / 127.0));
            events.push((off.min(frames.saturating_sub(1)), false, pitch, 0.0));
        }
        let pad_on = (bar as f64 * 4.0 * spb) as usize;
        events.push((pad_on, true, 48, 0.35));
        events.push((pad_on, true, 55, 0.28));
        events.push((
            (((bar as f64 * 4.0 + 3.9) * spb) as usize).min(frames.saturating_sub(1)),
            false,
            48,
            0.0,
        ));
        events.push((
            (((bar as f64 * 4.0 + 3.9) * spb) as usize).min(frames.saturating_sub(1)),
            false,
            55,
            0.0,
        ));
    }
    events.sort_by_key(|e| e.0);
    let mut bass_poly = Poly::new(sr as f32, SynthInstrument::Analog, 8);
    let mut pad_poly = Poly::new(sr as f32, SynthInstrument::Pad, 4);
    let mut ei = 0;
    for i in 0..frames {
        while ei < events.len() && events[ei].0 == i {
            let (_f, on, pitch, vel) = events[ei];
            if on {
                if pitch < 45 {
                    bass_poly.note_on(pitch, vel);
                } else {
                    pad_poly.note_on(pitch, vel);
                }
            } else if pitch < 45 {
                bass_poly.note_off(pitch);
            } else {
                pad_poly.note_off(pitch);
            }
            ei += 1;
        }
        let s = bass_poly.tick(sr as f32) * 1.1 + pad_poly.tick(sr as f32) * 0.7;
        harm[i * 2] += s;
        harm[i * 2 + 1] += s;
    }

    let mk = |name: &str, data: Vec<f32>| {
        let peaks = peaks_3band(&data, 2, 2048);
        Arc::new(Sample {
            name: name.into(),
            sr,
            ch: 2,
            bpm,
            peaks: peaks.into(),
            path: format!("builtin:{name}"),
            data,
        })
    };
    (mk("Drums (session)", drums), mk("Harmony (session)", harm))
}

fn mk_samp(name: &str, sr: u32, data: Vec<f32>) -> Arc<Sample> {
    let peaks = peaks_3band(&data, 1, 64);
    Arc::new(Sample {
        name: name.into(),
        sr,
        ch: 1,
        bpm: 120.0,
        peaks: peaks.into(),
        path: format!("builtin:{name}"),
        data,
    })
}

fn build_pad_banks(sr: u32) -> Vec<[Arc<Sample>; 16]> {
    let kit_kinds = [0u8, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0, 1, 2, 3];
    let kit_pitch = [
        1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.84, 1.12, 1.26, 0.9, 0.75, 1.2, 0.7, 1.35, 1.5, 0.62,
    ];
    let perc_kinds = [5u8, 5, 3, 3, 2, 4, 5, 2, 3, 4, 5, 2, 4, 3, 5, 2];
    let perc_pitch = [
        0.7, 0.85, 1.0, 1.2, 1.4, 0.8, 1.1, 1.6, 0.65, 0.95, 1.3, 1.8, 0.55, 1.15, 0.78, 1.45,
    ];
    let hit_kinds = [4u8, 3, 1, 0, 4, 3, 1, 5, 4, 0, 3, 1, 4, 5, 0, 2];
    let hit_pitch = [
        0.5, 0.7, 0.9, 0.6, 0.8, 1.1, 1.3, 0.75, 0.45, 1.0, 1.25, 0.85, 0.55, 1.4, 0.95, 1.7,
    ];
    let mk_bank = |kinds: [u8; 16], pitch: [f32; 16], prefix: &str| {
        std::array::from_fn(|i| {
            let raw = synth_drum(kinds[i], sr);
            let data = resample_mono(&raw, pitch[i]);
            mk_samp(&format!("{prefix}-{i}"), sr, data)
        })
    };
    vec![
        mk_bank(kit_kinds, kit_pitch, "kit"),
        mk_bank(perc_kinds, perc_pitch, "perc"),
        mk_bank(hit_kinds, hit_pitch, "hit"),
    ]
}

fn build_kit(sr: u32) -> [Arc<Sample>; 6] {
    let names = ["Kick", "Snare", "Hat", "Clap", "OpenHat", "Tom"];
    std::array::from_fn(|i| {
        let data = synth_drum(i as u8, sr);
        let peaks = peaks_3band(&data, 1, 128);
        Arc::new(Sample {
            name: names[i].into(),
            sr,
            ch: 1,
            bpm: detect_bpm(&data, 1, sr),
            peaks: peaks.into(),
            path: format!("builtin:{names}", names = names[i]),
            data,
        })
    })
}

pub(crate) enum InitialSession {
    Empty,
    Project { state: project::State, media: Vec<Arc<dsp::Sample>> },
}
impl InitialSession {
    /// Prepare an empty or validated saved graph before backend ownership starts.
    /// `sample_rate` sets DSP storage; returns a stopped native graph or an error.
    pub(crate) fn prepare(self, sample_rate: u32) -> Result<project::Prepared, project::Error> {
        match self { Self::Empty => project::Prepared::empty(sample_rate), Self::Project { state, media } => project::Prepared::from_state(state, media, sample_rate) }
    }
}

pub struct Engine {
    pub(crate) audible: audible::Handle,
    pub(crate) routing: audio::routing::input::Pipe,
    pub undo: undo::Handle,
    pub project: project::Handle,
    pub cmd: CommandPort,
    pub ui_requests: ui_requests::Receiver,
    pub snap: Arc<Mutex<Snapshot>>,
    pub midi: midi::MidiHub,
    pub(crate) initial_builtin: bool,
    pub(crate) initial_playback: [Option<load_receipt::Receipt>; DECKS],
    pub(crate) performance_history: Option<history_measurement::control::Handle>,
    pub(crate) sampler_assets: crate::sampler_bank::assets::Owner,
    // Production owns the audio manager; it may retain a stopped graph after
    // backend failure while still serving project Save/Open/Close.
    _audio: Option<audio::AudioOut>,
}

impl Engine {
    pub fn start() -> anyhow::Result<Self> {
        Self::start_with_settings(&crate::preferences::Profile::defaults(std::path::Path::new("/tmp")))
    }

    pub fn start_with_settings(settings: &crate::preferences::Profile) -> anyhow::Result<Self> {
        Self::start_with_session(settings, None)
    }

    pub(crate) fn start_with_session(settings: &crate::preferences::Profile, initial: Option<InitialSession>) -> anyhow::Result<Self> {
        settings.validate().map_err(anyhow::Error::msg)?;
        let (tx, rx) = CommandPort::channel(256);
        if settings.startup.performance_mode { tx.performance().set_enabled(true)?; }
        tx.midi_learn().configure(settings.midi_learn.clone()).map_err(anyhow::Error::msg)?;
        let ui_requests = tx.take_ui_receiver().expect("fresh GUI request receiver");
        let snap = Arc::new(Mutex::new(Snapshot::default()));
        let mut rt = RtEngine::try_new(48000.0, rx, snap.clone()).map_err(anyhow::Error::msg)?;
        let initial_builtin = initial.is_none();
        if let Some(initial) = initial {
            let mut prepared = initial.prepare(48000).map_err(anyhow::Error::msg)?;
            prepared.swap_into(&mut rt);
        }
        let undo = rt.enable_undo()?;
        let project = rt.project.clone();
        let initial_playback = std::array::from_fn(|deck| rt.decks[deck].load_receipt.clone());
        let routing = rt.routing_pipe.clone();
        let sampler_assets = rt.sampler_assets.clone();
        let performance_history = rt.history_measurement.as_ref().map(|history| history.handle());
        let audible = rt.audible.handle();
        let audio = audio::start_with_settings(rt, &settings.audio)?;
        let midi = midi::MidiHub::start_with_routing(tx.clone(), snap.clone(), settings.midi_inputs.clone(),settings.midi_routing.clone())?;
        Ok(Self {
            audible,
            undo,
            project,
            cmd: tx,
            ui_requests,
            snap,
            midi,
            initial_playback,
            initial_builtin,
            performance_history,
            routing,
            sampler_assets,
            _audio: Some(audio),
        })
    }

    /// A real offline owner services project requests. This is not the test
    /// headless constructor: no caller owns or manually pumps the render graph.
    pub fn start_safe() -> anyhow::Result<Self> {
        let (cmd, rx)=CommandPort::channel(256);
        let ui_requests=cmd.take_ui_receiver().expect("fresh GUI request receiver");
        let snap=Arc::new(Mutex::new(Snapshot::default()));
        let mut rt=RtEngine::try_new(48000.0,rx,snap.clone()).map_err(anyhow::Error::msg)?;
        // Start empty and stopped. Builtin source generation is application
        // code; no external media/project is opened or automatically resumed.
        for track in &mut rt.tracks {track.clips=(0..SCENES).map(|_|Clip::empty()).collect();}
        for deck in &mut rt.decks {
            if let Some(receipt)=&deck.load_receipt {receipt.supersede();}
            *deck=DeckRt::new(48000.0);
        }
        let initial_playback=std::array::from_fn(|deck|rt.decks[deck].load_receipt.clone());
        let undo=rt.enable_undo()?;
        let project=rt.project.clone();
        let routing=rt.routing_pipe.clone();
        let sampler_assets=rt.sampler_assets.clone();
        let performance_history=rt.history_measurement.as_ref().map(|history|history.handle());
        let audible=rt.audible.handle();
        let audio=audio::owner::start_safe(rt)?;
        Ok(Self{audible,undo,project,cmd,ui_requests,snap,midi:midi::MidiHub::without_devices(),initial_playback,initial_builtin:false,performance_history,routing,sampler_assets,_audio:Some(audio)})
    }
    pub fn safe_mode(&self)->bool {self._audio.as_ref().is_some_and(|audio|audio.handle.safe_mode())}

    #[cfg(test)]
    pub(crate) fn headless_for_test(sample_rate: u32, capacity: usize) -> (Self, RtEngine) {
        Self::headless_with_sampler_owner_for_test(sample_rate, capacity, None)
    }
    /// Build an independent sampler editor fixture.
    /// Takes sample rate and command capacity; returns the app engine and renderer under the production asset limits.
    #[cfg(test)]
    pub(crate) fn headless_isolated_sampler_for_test(sample_rate: u32, capacity: usize) -> (Self, RtEngine) {
        let owner = crate::sampler_bank::assets::Owner::isolated_for_test(crate::sampler_bank::assets::Budget::limits());
        Self::headless_with_sampler_owner_for_test(sample_rate, capacity, Some(owner))
    }
    /// Build an app fixture with one asset account.
    /// Takes sample rate, command capacity and optional fixture owner; returns the complete engine and renderer.
    #[cfg(test)]
    fn headless_with_sampler_owner_for_test(
        sample_rate: u32, capacity: usize, owner: Option<crate::sampler_bank::assets::Owner>,
    ) -> (Self, RtEngine) {
        let (cmd, rx) = CommandPort::channel(capacity);
        let ui_requests = cmd.take_ui_receiver().expect("fresh GUI request receiver");
        let snap = Arc::new(Mutex::new(Snapshot::default()));
        let mut rt = RtEngine::try_new_with_sampler_owner(sample_rate as f32, rx, snap.clone(), owner)
            .expect("prepare app fixture");
        let undo = rt.enable_undo().expect("undo worker");
        let project = rt.project.clone();
        let initial_playback = std::array::from_fn(|deck| rt.decks[deck].load_receipt.clone());
        let routing = rt.routing_pipe.clone();
        let sampler_assets = rt.sampler_assets.clone();
        let performance_history = rt.history_measurement.as_ref().map(|history| history.handle());
        let audible = rt.audible.handle();
        (
            Self {
                audible,
                undo,
                project,
                cmd,
                ui_requests,
                snap,
                midi: midi::MidiHub::without_devices(),
                initial_playback,
                initial_builtin: true,
                performance_history,
                routing,
            sampler_assets,
                _audio: None,
            },
            rt,
        )
    }

    pub fn send(&self, c: Command) -> Result<SubmissionOutcome, SubmissionError> {
        self.cmd.send(c)
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.snap.lock().clone();
        s.timeline_seconds = self.project.timeline_seconds();
        s.performance = self.cmd.performance().status();
        s.audio = self.cmd.audio_metrics();
        s.cpu = s.audio.last_callback.and_then(|sample| sample.render_cpu_fraction()).map(|value| value as f32);
        s
    }

    pub fn output_info(&self) -> Option<audio::OutputInfo> { self._audio.as_ref().and_then(|output| output.handle.status().active.clone()) }
    pub fn audio_handle(&self) -> Option<audio::owner::Handle> { self._audio.as_ref().map(|output|output.handle.clone()) }

    pub fn sr(&self) -> u32 {
        self.project.sample_rate()
    }
    pub(crate) fn input_handle(&self) -> Option<audio::routing::input::Handle> { self._audio.as_ref().map(|audio| audio.input.clone()) }
    /// Request a physical-channel tone.
    /// Takes a zero-based channel; returns acceptance only for a running native output with stopped transports and protection off.
    pub(crate) fn test_output(&self, channel: u16) -> Result<(), String> {
        let output = self.output_info().ok_or("No native output is running")?;
        let snap = self.snapshot();
        if channel >= output.plan.channels || channel >= 64 { return Err("Test channel is absent from the active output".into()); }
        if snap.playing || snap.recording || snap.decks.iter().any(|deck| deck.playing || deck.touching) || snap.performance.protected { return Err("Stop all transports and leave performance protection before testing outputs".into()); }
        self.routing.shared.probe.compare_exchange(0, u32::from(channel) + 1, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).map(|_| ()).map_err(|_| "A channel test is already running".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_makes_sound() {
        let (tx, rx) = crossbeam_channel::bounded(16);
        let snap = Arc::new(Mutex::new(Snapshot::default()));
        let mut rt = RtEngine::new(48000.0, rx, snap);
        drop(tx);
        rt.apply(Command::LaunchScene { scene: 0 });
        rt.playing = true;
        let mut buf = vec![0.0f32; 48000 * 2];
        rt.process(&mut buf);
        let energy: f32 = buf.iter().map(|x| x * x).sum();
        assert!(energy > 10.0, "expected audible demo, energy={energy}");
    }

    #[test]
    fn xfader_extremes() {
        let (a, b) = xfader_gains(0.0, 0.35);
        assert!(a > 0.9 && b < 0.05);
        let (a, b) = xfader_gains(1.0, 0.35);
        assert!(b > 0.9 && a < 0.05);
    }

    #[test]
    fn demo_stems_are_audible() {
        let (a, b) = demo_stems(48000, 124.0);
        let ea: f32 = a.data.iter().map(|x| x * x).sum();
        let eb: f32 = b.data.iter().map(|x| x * x).sum();
        assert!(ea > 10.0, "drum stem silent {ea}");
        assert!(eb > 1.0, "harmony stem silent {eb}");
        assert!(!a.peaks.is_empty());
    }

    fn engine() -> RtEngine {
        let (_tx, rx) = crossbeam_channel::bounded(8);
        RtEngine::new(48000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
    }

    fn zcr(buf: &[f32]) -> f32 {
        buf.chunks(2)
            .zip(buf.chunks(2).skip(1))
            .filter(|(a, b)| a[0] * b[0] < 0.0)
            .count() as f32
    }

    fn deck_b_at(lock: bool, pitch: f32) -> Vec<f32> {
        let mut rt = engine();
        rt.xfader = 1.0;
        rt.decks[0].playing = false;
        rt.decks[1].playing = true;
        rt.decks[1].pitch_range = 2;
        rt.decks[1].pitch = pitch;
        if lock {
            rt.apply(Command::DeckKeylock { deck: 1 });
        }
        let mut buf = vec![0.0f32; 48000];
        rt.process(&mut buf);
        buf
    }

    /// C1: keylock at two fader rates keeps zero-crossing rate closer than unlocked.
    #[test]
    fn contract_pitch_lock_preserves_pitch() {
        let u_lo = zcr(&deck_b_at(false, 0.15));
        let u_hi = zcr(&deck_b_at(false, 0.85));
        let l_lo = zcr(&deck_b_at(true, 0.15));
        let l_hi = zcr(&deck_b_at(true, 0.85));
        let unlocked_ratio = (u_lo / u_hi.max(1.0) - 1.0).abs();
        let locked_ratio = (l_lo / l_hi.max(1.0) - 1.0).abs();
        assert!(
            locked_ratio < unlocked_ratio * 0.7 || locked_ratio < 0.12,
            "keylock should keep pitch closer: lock={locked_ratio} free={unlocked_ratio} zcr lock={l_lo}/{l_hi} free={u_lo}/{u_hi}"
        );
        assert!(l_lo > 20.0 && l_hi > 20.0, "keylock produced silence zcr={l_lo}/{l_hi}");
    }

    /// C2: match arms unfavored rate to favored pitched BPM and does not jump favored pos.
    #[test]
    fn contract_match_follows_favored_bpm() {
        let mut rt = engine();
        rt.xfader = 0.2;
        rt.decks[0].playing = true;
        rt.decks[0].pitch_range = 2;
        rt.decks[0].pitch = 0.7;
        rt.decks[1].playing = true;
        let fav_pos = rt.decks[0].pos;
        let fav_bpm = rt.decks[0].audio.as_ref().unwrap().bpm * rt.decks[0].pitch_rate();
        rt.apply(Command::DeckMatch);
        assert!(rt.decks[1].sync);
        assert!(
            (rt.decks[1].sync_bpm - fav_bpm).abs() < 0.5,
            "sync_bpm={} favored pitched={}",
            rt.decks[1].sync_bpm,
            fav_bpm
        );
        assert!((rt.decks[0].pos - fav_pos).abs() < 1.0, "favored playhead jumped");
        rt.xfader = 0.8;
        rt.apply(Command::DeckMatch);
        let fav_b = rt.decks[1].audio.as_ref().unwrap().bpm * rt.decks[1].pitch_rate();
        assert!((rt.decks[0].sync_bpm - fav_b).abs() < 0.5);
    }

    /// C3: three banks of 16 distinct samples; switching banks changes pad 0's buffer.
    #[test]
    fn contract_banks_sixteen_distinct() {
        let banks = build_pad_banks(22050);
        assert_eq!(banks.len(), 3);
        assert!(banks[0][0].name.starts_with("kit"));
        assert!(banks[1][0].name.starts_with("perc"));
        assert!(banks[2][0].name.starts_with("hit"));
        for (bi, bank) in banks.iter().enumerate() {
            assert_eq!(bank.len(), 16);
            let e0: f32 = bank[0].data.iter().map(|x| x * x).sum();
            let e7: f32 = bank[7].data.iter().map(|x| x * x).sum();
            assert!(e0 > 0.1 && e7 > 0.1, "bank {bi} silent");
            assert!(
                (bank[0].data.len() as i32 - bank[7].data.len() as i32).abs() > 10,
                "bank {bi} pads 0 and 7 look like the same buffer"
            );
        }
        let mut rt = engine();
        rt.apply(Command::SamplerPad { pad: 0, on: true });
        let kit = rt.pad_voices[0].as_ref().unwrap().audio.name.clone();
        rt.apply(Command::SamplerPad { pad: 0, on: false });
        rt.pad_voices[0] = None;
        rt.apply(Command::SamplerBank(1));
        rt.apply(Command::SamplerPad { pad: 0, on: true });
        let perc = rt.pad_voices[0].as_ref().unwrap().audio.name.clone();
        assert_ne!(kit, perc, "bank switch must change pad 0 sample ({kit} vs {perc})");
    }

    /// C4: down=on / up=off; octave doubles sample rate and transposes held notes.
    #[test]
    fn contract_held_pads_and_octave() {
        let mut rt = engine();
        rt.apply(Command::SamplerPad { pad: 0, on: true });
        assert!(rt.pad_voices[0].is_some(), "sample pad down must start a voice");
        let r0 = rt.pad_voices[0].as_ref().unwrap().rate;
        rt.apply(Command::SamplerPad { pad: 0, on: false });
        assert!(rt.pad_voices[0].is_some(), "one-shot may ring after release");
        rt.apply(Command::SamplerOct(1));
        assert!((rt.pad_voices[0].as_ref().unwrap().rate / r0 - 2.0).abs() < 1e-4);

        let mut rt = engine();
        rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
        rt.apply(Command::SamplerPad { pad: 3, on: true });
        assert!(
            rt.sampler_poly.voices.iter().any(|v| matches!(v.env.stage, 1 | 2 | 3)),
            "instrument pad down must hold a voice"
        );
        let note = rt
            .sampler_poly
            .voices
            .iter()
            .find(|v| matches!(v.env.stage, 1 | 2 | 3))
            .unwrap()
            .note();
        rt.apply(Command::SamplerOct(1));
        let note2 = rt
            .sampler_poly
            .voices
            .iter()
            .find(|v| v.env.active())
            .unwrap()
            .note();
        assert_eq!(note2, note + 12);
        rt.apply(Command::SamplerPad { pad: 3, on: false });
        assert!(
            rt.sampler_poly
                .voices
                .iter()
                .all(|v| v.env.stage == 0 || v.env.stage == 4),
            "instrument pad up must release, not stay gated"
        );
    }

    /// C5: shift-click (ComposeArm) arms compose; pads write into that empty cell.
    #[test]
    fn contract_pad_writes_empty_clip() {
        let mut rt = engine();
        assert!(!rt.tracks[4].clips[3].occupied());
        rt.apply(Command::SamplerPad { pad: 0, on: true });
        assert!(
            !rt.tracks[4].clips[3].occupied(),
            "pads must not scribble until compose is armed"
        );
        rt.apply(Command::ComposeArm { track: 4, scene: 3 });
        assert_eq!(rt.tracks[4].clips[3].kind, ClipKind::Midi);
        rt.apply(Command::SamplerPad { pad: 2, on: true });
        assert!(!rt.tracks[4].clips[3].notes.is_empty());
    }

    /// C6: spread makes L != R; balance at 0 is left-heavy; EQ3/5/8 differ; arp steps a chord.
    #[test]
    fn contract_spread_and_balance_are_stereo() {
        let mut chain = fx::FxChain::new(48000.0);
        chain.slots.push(fx::FxSlot::new(fx::FxId::Spread, 48000.0));
        chain.slots[0].p[0] = 1.0;
        chain.slots[0].mix = 1.0;
        let (l, r) = chain.tick_stereo(0.8, 48000.0);
        assert!((l - r).abs() > 1e-6, "spread should decorrelate L/R");
        let mut chain = fx::FxChain::new(48000.0);
        chain.slots.push(fx::FxSlot::new(fx::FxId::Balance, 48000.0));
        chain.slots[0].p[0] = 0.0;
        chain.slots[0].mix = 1.0;
        let (l, r) = chain.tick_stereo(0.8, 48000.0);
        assert!(l > r, "balance 0 should be left-heavy, got L={l} R={r}");
    }

    #[test]
    fn contract_eq_bands_differ() {
        fn energy(id: fx::FxId, hz: f32) -> f32 {
            let mut chain = fx::FxChain::new(48000.0);
            let mut slot = fx::FxSlot::new(id, 48000.0);
            slot.p = [1.0, 0.0, 1.0, 0.5];
            slot.mix = 1.0;
            chain.slots.push(slot);
            let mut acc = 0.0f32;
            for n in 0..2048 {
                let x = (n as f32 * hz / 48000.0 * std::f32::consts::TAU).sin();
                let y = chain.tick(x, 48000.0);
                acc += y * y;
            }
            acc
        }
        let e3 = energy(fx::FxId::Eq3, 2000.0);
        let e5 = energy(fx::FxId::Eq5, 2000.0);
        let e8 = energy(fx::FxId::Eq8, 2000.0);
        assert!(
            (e3 - e5).abs() > 1.0 && (e5 - e8).abs() > 1.0,
            "EQ3/5/8 must differ at 2kHz: {e3} {e5} {e8}"
        );
    }

    #[test]
    fn contract_arp_steps_chord() {
        let mut rt = engine();
        rt.tracks[2].fx.slots.push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
        rt.apply(Command::LaunchClip { track: 2, scene: 0 });
        let mut seen = std::collections::BTreeSet::new();
        let sixteenth = (rt.sr as f64 * 60.0 / rt.bpm as f64 / 4.0) as usize;
        // Observe within each step as well as at its end: the chord's
        // 0.45-beat gate ends before the second sixteenth finishes.
        let mut buf = [0.0f32; 128];
        for _ in 0..(sixteenth * 6).div_ceil(64) {
            rt.process(&mut buf);
            if let Some(n) = rt.tracks[2].arp_note {
                seen.insert(n);
            }
        }
        assert!(
            seen.len() >= 2,
            "arp should step overlapping chord notes, got {seen:?}"
        );
    }

    /// C7: builtin crate reloads a stem onto a deck, including when already loaded.
    #[test]
    fn contract_builtin_reload() {
        let mut rt = engine();
        rt.apply(Command::DeckUnload { deck: 0 });
        assert!(rt.decks[0].audio.is_none());
        rt.apply(Command::LoadBuiltin { deck: 0, stem: 0 });
        assert!(rt.decks[0].audio.is_some());
        assert!(rt.decks[0].title.contains("Drums"));
        rt.decks[0].pos = 9999.0;
        rt.apply(Command::LoadBuiltin { deck: 0, stem: 0 });
        assert!(rt.decks[0].pos < 1.0, "reload must re-seek the stem");
        rt.apply(Command::LoadBuiltin { deck: 1, stem: 1 });
        assert!(rt.decks[1].title.contains("Harmony"));
    }
}

/// Normalize imported clip values as well as admitted editor controls.
fn clip_gain(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.5) } else { 1.0 }
}

#[cfg(test)]
mod clip_gain_tests;
#[cfg(test)]
mod drum_borrow_tests;


#[cfg(test)]
mod drum_velocity_tests;

#[cfg(test)]
mod mixer_gain_tests;

#[cfg(test)]
mod license_content_tests;

#[cfg(test)]
pub(crate) mod performance_workload_tests;

#[cfg(test)]
mod deck_load_lock_tests;
