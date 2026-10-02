//! Persistent musical state, deliberately separate from oscillator histories,
//! physical key ownership, worker handles and device connections.
use super::super::*;

pub const STATE_VERSION: u32 = 5;
pub const MAX_BANKS: usize = 16;
pub const MAX_FX_PER_RACK: usize = 128;
pub const MAX_NOTES_PER_CLIP: usize = 8192;
pub const MAX_TOTAL_NOTES: usize = 65536;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_MEDIA_REFS: usize = TRACKS * (SCENES + 6) + DECKS + 2 + MAX_BANKS * 16;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub bpm: f32,
    pub beat: f64,
    pub quant: f32,
    pub quantize: bool,
    pub metronome: bool,
    pub view: View,
    pub xfader: f32,
    pub xfader_curve: f32,
    pub master: f32,
    pub cue_mix: f32,
    pub selected_track: usize,
    pub selected_scene: usize,
    pub selected_deck: usize,
    pub fx_view: i16,
    pub tracks: [Track; TRACKS],
    pub decks: [Deck; DECKS],
    pub scene_fx: [Vec<Effect>; SCENES],
    pub fx_kind: [FxKind; 3],
    pub fx_wet: [f32; 3],
    pub sampler_bank: usize,
    pub sampler_inst: SamplerInstrument,
    pub sampler_oct: i8,
    pub sampler_synth: Synth,
    pub banks: Vec<Bank>,
    pub builtin: [Option<usize>; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    version: u32,
    bpm: f32,
    beat: f64,
    quant: f32,
    quantize: bool,
    metronome: bool,
    view: View,
    xfader: f32,
    xfader_curve: f32,
    master: f32,
    cue_mix: f32,
    selected_track: usize,
    selected_scene: usize,
    selected_deck: usize,
    fx_view: i16,
    tracks: [Track; TRACKS],
    decks: [Deck; DECKS],
    scene_fx: [Vec<Effect>; SCENES],
    fx_kind: [FxKind; 3],
    fx_wet: [f32; 3],
    sampler_bank: usize,
    sampler_inst: SamplerInstrument,
    sampler_oct: i8,
    sampler_synth: Synth,
    banks: Vec<Bank>,
    builtin: [Option<usize>; 2],
}
impl<'de> Deserialize<'de> for State {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        midi_edit::reject_legacy_fields(&raw).map_err(serde::de::Error::custom)?;
        let wire: StateWire = serde_json::from_value(raw).map_err(serde::de::Error::custom)?;
        Ok(Self {
            version: wire.version,
            bpm: wire.bpm,
            beat: wire.beat,
            quant: wire.quant,
            quantize: wire.quantize,
            metronome: wire.metronome,
            view: wire.view,
            xfader: wire.xfader,
            xfader_curve: wire.xfader_curve,
            master: wire.master,
            cue_mix: wire.cue_mix,
            selected_track: wire.selected_track,
            selected_scene: wire.selected_scene,
            selected_deck: wire.selected_deck,
            fx_view: wire.fx_view,
            tracks: wire.tracks,
            decks: wire.decks,
            scene_fx: wire.scene_fx,
            fx_kind: wire.fx_kind,
            fx_wet: wire.fx_wet,
            sampler_bank: wire.sampler_bank,
            sampler_inst: wire.sampler_inst,
            sampler_oct: wire.sampler_oct,
            sampler_synth: wire.sampler_synth,
            banks: wire.banks,
            builtin: wire.builtin,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub name: String,
    pub clips: [SavedClip; SCENES],
    pub scene_bus: usize,
    // A remembered launch is an explicit resume target. Opening never emits
    // notes or auto-starts transport; Play resumes these targets together.
    pub launch: Option<Launch>,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub kind: u8,
    pub synth: Synth,
    pub eq: [f32; 3],
    pub drums: [usize; 6],
    pub fx: Vec<Effect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedClip {
    #[serde(default)]
    pub region: Option<midi_edit::Region>,
    pub kind: ClipKind,
    pub name: String,
    pub bars: f32,
    pub notes: Vec<MidiNote>,
    pub gain: f32,
    pub audio: Option<usize>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub scene: u8,
    pub start_beat: f64,
    pub looping: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Synth {
    pub kind: SynthInstrument,
    pub voices: usize,
    pub cutoff: f32,
    pub tuning_hz: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub id: fx::FxId,
    pub on: bool,
    pub mix: f32,
    pub p: [f32; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank {
    pub name: String,
    pub media: [Option<usize>; 16],
    #[serde(default)]
    pub instance: Option<crate::sampler_bank::BankId>,
    #[serde(default)]
    pub settings: Option<Arc<crate::sampler_bank::resident::Settings>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deck {
    pub audio: Option<usize>,
    pub pos: f64,
    pub cue_pos: f64,
    pub pitch: f32,
    pub vinyl: bool,
    pub keylock: bool,
    pub sync: bool,
    pub gain: f32,
    pub eq: [f32; 3],
    pub filter_morph: f32,
    pub filter_amt: f32,
    pub pfl: bool,
    pub hotcues: [Option<f64>; HOTCUES],
    #[serde(default)]
    pub cue_styles: [crate::engine::cue_metadata::Style; HOTCUES],
    #[serde(default)]
    pub grid: Option<crate::engine::beatgrid::Grid>,
    pub loop_on: bool,
    pub loop_start: f64,
    pub loop_len: f64,
    pub bpm: f32,
    pub title: String,
    pub eq_cut: [bool; 4],
    pub eq_solo: i8,
    pub eq_store: [f32; 4],
    pub pitch_range: u8,
    pub sync_bpm: f32,
}

impl State {
    pub(super) fn migrate_notes(&mut self) {
        if self.version >= 5 { return; }
        for (track, data) in self.tracks.iter_mut().enumerate() {
            for (scene, clip) in data.clips.iter_mut().enumerate() {
                for (index, note) in clip.notes.iter_mut().enumerate() {
                    if !note.id.valid() { note.id = midi_edit::NoteId::legacy(track, scene, index); }
                }
            }
        }
    }
    pub(super) fn blank() -> Self {
        Self {
            version: STATE_VERSION,
            bpm: 124.0,
            beat: 0.0,
            quant: 1.0,
            quantize: true,
            metronome: false,
            view: View::Session,
            xfader: 0.5,
            xfader_curve: 0.35,
            master: 0.85,
            cue_mix: 0.0,
            selected_track: 0,
            selected_scene: 0,
            selected_deck: 0,
            fx_view: -1,
            tracks: std::array::from_fn(|_| Track {
                name: String::new(),
                clips: std::array::from_fn(|_| SavedClip {
                    region: None,
                    kind: ClipKind::Empty,
                    name: String::new(),
                    bars: 1.0,
                    notes: Vec::new(),
                    gain: 1.0,
                    audio: None,
                }),
                scene_bus: 0,
                launch: None,
                gain: 0.8,
                pan: 0.0,
                mute: false,
                solo: false,
                armed: false,
                kind: 0,
                synth: Synth {
                    kind: SynthInstrument::Analog,
                    voices: 8,
                    cutoff: 700.0,
                    tuning_hz: 440.0,
                },
                eq: [1.0; 3],
                drums: [0; 6],
                fx: Vec::new(),
            }),
            decks: std::array::from_fn(|_| Deck {
                audio: None,
                pos: 0.0,
                cue_pos: 0.0,
                pitch: 0.5,
                vinyl: true,
                keylock: false,
                sync: false,
                gain: 0.85,
                eq: [1.0; 3],
                filter_morph: 0.5,
                filter_amt: 0.5,
                pfl: false,
                hotcues: [None; HOTCUES],
                cue_styles: [crate::engine::cue_metadata::Style::default(); HOTCUES],
                grid: None,
                loop_on: false,
                loop_start: 0.0,
                loop_len: 0.0,
                bpm: 124.0,
                title: String::new(),
                eq_cut: [false; 4],
                eq_solo: -1,
                eq_store: [1.0, 1.0, 1.0, 0.85],
                pitch_range: 0,
                sync_bpm: 124.0,
            }),
            scene_fx: std::array::from_fn(|_| Vec::new()),
            fx_kind: [FxKind::Echo, FxKind::Reverb, FxKind::Filter],
            fx_wet: [0.0; 3],
            sampler_bank: 0,
            sampler_inst: SamplerInstrument::Samples,
            sampler_oct: 3,
            sampler_synth: Synth {
                kind: SynthInstrument::Keys,
                voices: 8,
                cutoff: 1800.0,
                tuning_hz: 440.0,
            },
            banks: Vec::new(),
            builtin: [None; 2],
        }
    }

    pub fn validate(&self, media: &[Arc<Sample>]) -> Result<(), String> {
        let fail = |name: &str| Err(format!("invalid project {name}"));
        if !(1..=STATE_VERSION).contains(&self.version) {
            return Err(format!(
                "unsupported project state version {}",
                self.version
            ));
        }
        if !(40.0..=240.0).contains(&self.bpm)
            || !finite_range(self.beat, 0.0, 1.0e12)
            || !finite_range(self.quant as f64, 0.0, 64.0)
            || !unit(self.xfader)
            || !unit(self.xfader_curve)
            || !finite_range(self.master as f64, 0.0, 1.5)
            || !unit(self.cue_mix)
            || self.selected_track >= TRACKS
            || self.selected_scene >= SCENES
            || self.selected_deck >= DECKS
            || !(self.fx_view == -1
                || (0..TRACKS as i16).contains(&self.fx_view)
                || (100..100 + SCENES as i16).contains(&self.fx_view))
            || self.fx_wet.iter().any(|v| !unit(*v))
        {
            return fail("timing, view or mixer controls");
        }
        if self.banks.is_empty()
            || self.banks.len() > MAX_BANKS
            || self.sampler_bank >= self.banks.len()
            || !(-4..=8).contains(&self.sampler_oct)
            || !valid_synth(self.sampler_synth)
        {
            return fail("sampler settings");
        }
        let reference = |index: usize| index < media.len();
        let optional = |index: Option<usize>| index.is_none_or(reference);
        let mut note_count = 0usize;
        let mut note_ids = std::collections::HashSet::new();
        for track in &self.tracks {
            if !text_ok(&track.name)
                || track.scene_bus >= SCENES
                || track.kind > 4
                || !valid_synth(track.synth)
                || !finite_range(track.gain as f64, 0.0, 1.5)
                || !finite_range(track.pan as f64, -1.0, 1.0)
                || !valid_eq(track.eq)
                || track.drums.iter().any(|i| !reference(*i))
                || !valid_fx(&track.fx, false)
            {
                return fail("track controls or media reference");
            }
            if let Some(launch) = track.launch {
                if launch.scene as usize >= SCENES
                    || !finite_range(launch.start_beat, -1.0e12, 1.0e12)
                {
                    return fail("clip resume target");
                }
            }
            for clip in &track.clips {
                note_ids.clear();
                if !text_ok(&clip.name)
                    || !finite_range(clip.bars as f64, if clip.region.is_some() { 1.0 / 4096.0 } else { 0.25 }, 65536.0)
                    || !finite_range(clip.gain as f64, 0.0, 1.5)
                    || !optional(clip.audio)
                    || clip.notes.len() > MAX_NOTES_PER_CLIP
                    || clip.region.is_some_and(|region| !region.allows(&clip.notes) || clip.kind != ClipKind::Midi || clip.bars != (region.end / 4.0) as f32)
                    || self.version < 5 && clip.region.is_some()
                {
                    return fail("clip controls or media reference");
                }
                note_count += clip.notes.len();
                for note in &clip.notes {
                    if note.pitch > 127
                        || note.vel > 127
                        || !finite_range(note.start as f64, 0.0, 262144.0)
                        || !finite_range(note.len as f64, 0.0, 262144.0)
                        || self.version >= 5 && (!note.id.valid() || !note_ids.insert(note.id))
                        || self.version < 5 && note.muted
                    {
                        return fail("MIDI note");
                    }
                }
            }
        }
        if note_count > MAX_TOTAL_NOTES {
            return fail("note count (maximum 65536)");
        }
        if self.scene_fx.iter().any(|rack| !valid_fx(rack, true)) {
            return fail("scene rack");
        }
        let mut bank_ids = std::collections::HashSet::new();
        for bank in &self.banks {
            if !text_ok(&bank.name) || bank.media.iter().any(|i| !optional(*i)) {
                return fail("sample bank");
            }
            if self.version < 4 {
                if bank.instance.is_some() || bank.settings.is_some() || bank.media.iter().any(Option::is_none) {
                    return fail("legacy sample bank must contain embedded media only");
                }
            } else {
                let (Some(instance), Some(settings)) = (bank.instance, &bank.settings) else { return fail("sample bank identity or settings"); };
                if !bank_ids.insert(instance) || settings.definition == Some(instance) || settings.name != bank.name || settings.validate().is_err() { return fail("sample bank identity or settings"); }
                for (slot, index) in settings.slots.iter().zip(bank.media) {
                    if index.is_some_and(|i| slot.controls.frames(media[i].sr, media[i].frames()).is_err()) { return fail("sample bank source range"); }
                }
            }
        }
        if self.builtin.iter().any(|i| !optional(*i)) {
            return fail("built-in sample reference");
        }
        for deck in &self.decks {
            if !optional(deck.audio)
                || !text_ok(&deck.title)
                || !unit(deck.pitch)
                || !finite_range(deck.gain as f64, 0.0, 1.5)
                || !valid_eq(deck.eq)
                || !unit(deck.filter_amt)
                || !unit(deck.filter_morph)
                || deck.pitch_range > 2
                || !(-1..=3).contains(&deck.eq_solo)
                || deck
                    .eq_store
                    .iter()
                    .any(|v| !finite_range(*v as f64, 0.0, 16.0))
                || !finite_range(deck.bpm as f64, 1.0, 1000.0)
                || !finite_range(deck.sync_bpm as f64, 1.0, 1000.0)
                || !finite_range(deck.pos, -1.0e12, 1.0e12)
                || !finite_range(deck.cue_pos, -1.0e12, 1.0e12)
                || !finite_range(deck.loop_start, -1.0e12, 1.0e12)
                || !finite_range(deck.loop_len, 0.0, 1.0e12)
                || deck
                    .hotcues
                    .iter()
                    .flatten()
                    .any(|v| !finite_range(*v, -1.0e12, 1.0e12))
            {
                return fail("deck controls or media reference");
            }
        }
        Ok(())
    }

    /// Capture uses one cheap reference per attachment. Deduplicate off audio
    /// before persistence so shared kit/bank/deck samples occupy one file chunk.
    pub(super) fn deduplicate(&mut self, media: &mut Vec<Arc<Sample>>) {
        let mut by_pointer = std::collections::HashMap::new();
        let mut remap = Vec::with_capacity(media.len());
        let mut unique = Vec::new();
        for sample in media.drain(..) {
            let next = unique.len();
            let index = *by_pointer
                .entry(Arc::as_ptr(&sample) as usize)
                .or_insert_with(|| {
                    unique.push(sample);
                    next
                });
            remap.push(index);
        }
        let map = |v: &mut Option<usize>| {
            if let Some(index) = v {
                *index = remap[*index];
            }
        };
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                map(&mut clip.audio);
            }
            for index in &mut track.drums {
                *index = remap[*index];
            }
        }
        for deck in &mut self.decks {
            map(&mut deck.audio);
        }
        for bank in &mut self.banks {
            for index in bank.media.iter_mut().flatten() {
                *index = remap[*index];
            }
        }
        for index in &mut self.builtin {
            map(index);
        }
        *media = unique;
    }
}

fn finite_range(value: f64, lo: f64, hi: f64) -> bool {
    value.is_finite() && (lo..=hi).contains(&value)
}
fn unit(value: f32) -> bool {
    finite_range(value as f64, 0.0, 1.0)
}
fn text_ok(value: &str) -> bool {
    value.len() <= MAX_TEXT_BYTES && !value.contains('\0')
}
fn valid_eq(values: [f32; 3]) -> bool {
    values
        .into_iter()
        .all(|v| finite_range(v as f64, 0.0, 16.0))
}
fn valid_synth(s: Synth) -> bool {
    (1..=64).contains(&s.voices)
        && finite_range(s.cutoff as f64, 0.0, 20000.0)
        && finite_range(s.tuning_hz as f64, 20.0, 20000.0)
}
fn valid_fx(rack: &[Effect], scene: bool) -> bool {
    rack.len() <= MAX_FX_PER_RACK
        && rack.iter().all(|f| {
            unit(f.mix) && f.p.iter().all(|p| unit(*p)) && (!scene || f.id != fx::FxId::Arp)
        })
}
