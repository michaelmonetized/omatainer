//! Persistent musical state, deliberately separate from oscillator histories,
//! physical key ownership, worker handles and device connections.
use super::super::*;

pub const STATE_VERSION: u32 = 35;
pub const MAX_BANKS: usize = 16;
pub const MAX_FX_PER_RACK: usize = 128;
pub const MAX_NOTES_PER_CLIP: usize = 8192;
pub const MAX_TOTAL_NOTES: usize = 65536;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_MEDIA_REFS: usize = session::MAX_TRACKS * (session::MAX_SCENES + 6) + DECKS + 2 + MAX_BANKS * 16 + arrangement::MAX_SOURCES;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    #[serde(default = "note_variation::default_seed", skip_serializing_if = "note_variation::seed_is_default")]
    pub(crate) note_seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) musical_context: Option<musical_context::Context>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) sampler_scale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sync_leader: Option<deck_sync::Leader>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub(crate) navigation: Option<song_navigation::Saved>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub(crate) arrangement: Option<Arc<arrangement::Model>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<Arc<audio::routing::model::Model>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) migration: Option<Arc<crate::ableton::Migration>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_aux: Option<audio::routing::mic_aux::Configuration>,
    #[serde(default)]
    pub session: Option<session::Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scene_timing: Option<scene::Timing>,
    pub bpm: f32,
    #[serde(default)]
    pub(crate) conductor: Option<Arc<midi_data::Conductor>>,
    pub beat: f64,
    pub timeline_seconds: f64,
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
    pub tracks: Vec<Track>,
    pub decks: [Deck; DECKS],
    pub scene_fx: Vec<Vec<Effect>>,
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
    #[serde(default = "note_variation::default_seed")]
    note_seed: u64,
    #[serde(default)]
    musical_context: Option<musical_context::Context>,
    #[serde(default)]
    sampler_scale: bool,
    #[serde(default)]
    sync_leader: Option<deck_sync::Leader>,
    #[serde(default)]
    navigation: Option<song_navigation::Saved>,
    #[serde(default)]
    arrangement: Option<Arc<arrangement::Model>>,
    #[serde(default)]
    routing: Option<Arc<audio::routing::model::Model>>,
    #[serde(default)]
    migration: Option<Arc<crate::ableton::Migration>>,
    #[serde(default)]
    mic_aux: Option<audio::routing::mic_aux::Configuration>,
    #[serde(default)]
    session: Option<session::Layout>,
    #[serde(default)]
    scene_timing: Option<scene::Timing>,
    bpm: f32,
    #[serde(default)]
    conductor: Option<Arc<midi_data::Conductor>>,
    beat: f64,
    #[serde(default)]
    timeline_seconds: Option<f64>,
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
    tracks: Vec<Track>,
    decks: [Deck; DECKS],
    scene_fx: Vec<Vec<Effect>>,
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
        let version = raw["version"].as_u64().unwrap_or(0);
        if version < 35 && raw.get("decks").and_then(serde_json::Value::as_array).is_some_and(|decks| decks.iter().any(|deck| deck.get("channel_effect").is_some())) {
            return Err(serde::de::Error::custom("Selectable channel effects require project state version 35"));
        }
        if version < 31
            && raw
                .get("arrangement")
                .and_then(|a| a.get("sources"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|sources| sources.iter().any(|s| s.get("audio_clock").is_some()))
        {
            return Err(serde::de::Error::custom(
                "Aligned render clocks require project state version 31",
            ));
        }
        if version < 30 && raw.get("migration").is_some() {
            return Err(serde::de::Error::custom(
                "Ableton migration requires project state version 30",
            ));
        }
        note_variation::reject_legacy_fields(&raw).map_err(serde::de::Error::custom)?;
        if version < 33 && (raw.get("musical_context").is_some() || raw.get("sampler_scale").is_some()
            || raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten()
                .flat_map(|track| track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten())
                .chain(raw.get("arrangement").and_then(|arrangement| arrangement.get("sources")).and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|source| source.get("clip")))
                .any(|clip| clip.get("properties").is_some_and(|properties| properties.get("context").is_some()))) {
            return Err(serde::de::Error::custom("Song and clip scales require project state version 33"));
        }
        if version < 29 && raw.get("routing").is_some_and(|r| r.get("plugins").is_some()) { return Err(serde::de::Error::custom("Native plugins require project state version 29")); }
        if version < 28 && raw.get("decks").and_then(serde_json::Value::as_array).is_some_and(|decks| decks.iter().any(|deck| deck.get("key_shift").is_some())) { return Err(serde::de::Error::custom("Independent key shift requires project state version 28")); }
        if version < 26 && (raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten().flat_map(|t|t.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten()).chain(raw.get("arrangement").and_then(|a|a.get("sources")).and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|s|s.get("clip"))).any(|c|c.get("audio_region").is_some_and(|r|r.get("fades").is_some())) || raw.get("arrangement").and_then(|a|a.get("instances")).and_then(serde_json::Value::as_array).into_iter().flatten().any(|i|["fades","fade_link","crossfade"].into_iter().any(|f|i.get(f).is_some()))) { return Err(serde::de::Error::custom("Audio fades and crossfade links require project state version 26")); }
        if version < 25 && (raw.get("sync_leader").is_some() || raw.get("decks").and_then(serde_json::Value::as_array).is_some_and(|decks| decks.iter().any(|deck| deck.get("sync_phase").is_some()))) { return Err(serde::de::Error::custom("Sync leaders and phase modes require project state version 25")); }
        if version < 23 && (raw.get("scene_timing").is_some() || raw.get("session").is_some_and(|layout| ["tracks", "scenes"].into_iter().flat_map(|axis| layout.get(axis).and_then(serde_json::Value::as_array).into_iter().flatten()).any(|item| item.get("scene").is_some()))) { return Err(serde::de::Error::custom("Scene properties require project state version 23")); }
        if version < 22 && raw.get("navigation").is_some() { return Err(serde::de::Error::custom("Song sections require project state version 22")); }
        if version<21 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten().flat_map(|track|track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten()).chain(raw.get("arrangement").and_then(|song|song.get("sources")).and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|source|source.get("clip"))).any(|clip|clip.get("properties").is_some_and(|p|p.get("launch").is_some())){return Err(serde::de::Error::custom("Clip launch policy requires project state version 21"));}
        if version<20 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten().flat_map(|track|track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten()).chain(raw.get("arrangement").and_then(|song|song.get("sources")).and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|source|source.get("clip"))).any(|clip|clip.get("properties").is_some()){return Err(serde::de::Error::custom("Clip properties require project state version 20"));}
        if version<19 && raw.get("arrangement").is_some(){return Err(serde::de::Error::custom("Arrangement sources require project state version 19"));}
        if version < 18 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten().flat_map(|track|track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten()).any(|clip|clip.get("audio_region").is_some()) {return Err(serde::de::Error::custom("Audio clip source regions require project state version 18"));}
        if version < 17 && raw.get("mic_aux").is_some() { return Err(serde::de::Error::custom("Mic/aux controls require project state version 17")); }
        if version < 16 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten().any(|track| track.get("input_monitor").is_some()) {
            return Err(serde::de::Error::custom("Input monitoring requires project state version 16"));
        }
        if version < 15 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .flat_map(|track| track.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten())
            .any(|clip| clip.get("lanes").is_some_and(|lanes| lanes.get("labels").is_some())) {
            return Err(serde::de::Error::custom("MIDI device labels require project state version 15"));
        }
        if version < 14 && raw.get("decks").and_then(serde_json::Value::as_array).into_iter().flatten().any(|deck| deck.get("source_gain").is_some()) {
            return Err(serde::de::Error::custom("Source gain requires project state version 14"));
        }
        if version < 7 && raw.get("session").is_some() { return Err(serde::de::Error::custom("Legacy projects cannot contain session identity metadata")); }
        if version < 27 && raw.get("decks").and_then(|v|v.as_array()).is_some_and(|decks|decks.iter().any(|deck|deck.get("saved_loops").is_some_and(|bank|bank.get("cue_loops").is_some()))) { return Err(serde::de::Error::custom("Cue-loop associations require state version 27")); }
        if version < 24 && raw.get("decks").and_then(|v| v.as_array()).is_some_and(|decks| decks.iter().any(|deck| deck.get("saved_loops").is_some())) { return Err(serde::de::Error::custom("Saved loop banks require state version 24")); }
        if (7..=u64::from(STATE_VERSION)).contains(&version) && !raw.get("session").is_some_and(serde_json::Value::is_object) { return Err(serde::de::Error::custom("Supported versions 7 and newer require session identity metadata")); }
        if version < 8 && raw.get("conductor").and_then(serde_json::Value::as_object).is_some_and(|c| c.contains_key("native") || c.get("tempos").and_then(serde_json::Value::as_array).is_some_and(|points| points.iter().any(|p| p.get("ramp").is_some()))) {
            return Err(serde::de::Error::custom("Legacy projects cannot contain native tempo ramps or timing options"));
        }
        if version < 9 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .flat_map(|track| track.get("fx").and_then(serde_json::Value::as_array).into_iter().flatten())
            .chain(raw.get("scene_fx").and_then(serde_json::Value::as_array).into_iter().flatten()
                .flat_map(|rack| rack.as_array().into_iter().flatten()))
            .any(|effect| effect.get("state").is_some()
                || effect.get("id").cloned().and_then(|id| serde_json::from_value::<fx::FxId>(id).ok())
                    .is_none_or(|id| id == fx::FxId::Unavailable))
        {
            return Err(serde::de::Error::custom("Legacy projects cannot contain unavailable devices or serialized device state"));
        }
        if version < 9 && raw.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .filter_map(|track| track.get("synth")).chain(raw.get("sampler_synth"))
            .any(|synth| synth.get("state").is_some() || synth.get("kind").cloned()
                .and_then(|kind| serde_json::from_value::<SynthInstrument>(kind).ok()).is_none()) {
            return Err(serde::de::Error::custom("Legacy projects cannot contain unavailable instruments or serialized instrument state"));
        }
        if version < 9 && raw.get("banks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .filter_map(|bank| bank.get("settings")).filter_map(|settings| settings.get("slots").and_then(serde_json::Value::as_array)).flatten()
            .any(|slot| slot.get("source").and_then(|source| source.get("kind")).and_then(serde_json::Value::as_str) == Some("project")) {
            return Err(serde::de::Error::custom("Legacy projects cannot contain relinked project sources"));
        }
        if version < 10 && raw.get("timeline_seconds").is_some() { return Err(serde::de::Error::custom("Legacy projects cannot contain sample-based timeline positions")); }
        if (10..=u64::from(STATE_VERSION)).contains(&version) && !raw.get("timeline_seconds").is_some_and(serde_json::Value::is_number) { return Err(serde::de::Error::custom("Project schema 10 requires a timeline position")); }
        if version < 11 && raw.get("routing").is_some() { return Err(serde::de::Error::custom("Legacy projects cannot contain audio routing metadata")); }
        if version < 12 && raw.get("banks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .filter_map(|bank| bank.get("settings")).filter_map(|settings| settings.get("slots").and_then(serde_json::Value::as_array)).flatten()
            .any(|slot| slot.get("playback").is_some()) {
            return Err(serde::de::Error::custom("Legacy projects cannot contain sampler playback modes"));
        }
        if version < 13 && raw.get("decks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .filter_map(|deck| deck.get("grid")).any(|grid| grid.get("anchors").is_some()) {
            return Err(serde::de::Error::custom("Tempo anchors require project state version 13"));
        }
        let wire: StateWire = serde_json::from_value(raw).map_err(serde::de::Error::custom)?;
        Ok(Self {
            version: wire.version,
            musical_context: wire.musical_context, note_seed: wire.note_seed, sampler_scale: wire.sampler_scale,
            sync_leader: wire.sync_leader,
            navigation: wire.navigation,
            arrangement: wire.arrangement,
            routing: wire.routing,
            migration: wire.migration,
            mic_aux: wire.mic_aux,
            session: wire.session,
            scene_timing: wire.scene_timing,
            bpm: wire.bpm,
            beat: wire.beat,
            timeline_seconds: wire.timeline_seconds.unwrap_or_else(|| wire.conductor.as_ref().map_or(wire.beat * 60.0 / f64::from(wire.bpm), |map| map.seconds_at(wire.beat))),
            conductor: wire.conductor,
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
    pub clips: Vec<SavedClip>,
    pub scene_bus: usize,
    // A remembered launch is an explicit resume target. Opening never emits
    // notes or auto-starts transport; Play resumes these targets together.
    pub launch: Option<Launch>,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_monitor: Option<input_monitor::Mode>,
    pub kind: u8,
    pub synth: Synth,
    pub eq: [f32; 3],
    pub drums: [usize; 6],
    pub fx: Vec<Effect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedClip {
    #[serde(default,skip_serializing_if="clip_management::Properties::is_default")]
    pub(crate) properties: clip_management::Properties,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub(crate) audio_region: Option<audio_clip::Region>,
    #[serde(default)]
    pub(crate) lanes: Option<Arc<midi_data::Lanes>>,
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
    pub scene: u16,
    pub start_beat: f64,
    pub looping: bool,
}

#[derive(Clone, Debug)]
pub struct Synth {
    pub kind: SynthInstrument,
    pub voices: usize,
    pub cutoff: f32,
    pub tuning_hz: f32,
    pub(crate) offline: Option<Arc<fx::OfflineDevice>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SynthWire {
    kind: String,
    voices: usize,
    cutoff: f32,
    tuning_hz: f32,
    #[serde(default)]
    state: Option<fx::DeviceState>,
}
impl Serialize for Synth {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let known = match self.kind { SynthInstrument::Analog => "analog", SynthInstrument::Keys => "keys", SynthInstrument::Pad => "pad" };
        let kind = self.offline.as_ref().map_or(known, |device| device.identifier.as_str());
        let state = self.offline.as_ref().and_then(|device| device.state.as_ref());
        let mut out = serializer.serialize_struct("Synth", 4 + usize::from(state.is_some()))?;
        out.serialize_field("kind", kind)?; out.serialize_field("voices", &self.voices)?;
        out.serialize_field("cutoff", &self.cutoff)?; out.serialize_field("tuning_hz", &self.tuning_hz)?;
        if let Some(state) = state { out.serialize_field("state", state)?; }
        out.end()
    }
}
impl<'de> Deserialize<'de> for Synth {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SynthWire::deserialize(deserializer)?;
        let known = serde_json::from_value::<SynthInstrument>(wire.kind.clone().into()).ok().filter(|_| wire.state.is_none());
        let (kind, offline) = if let Some(kind) = known { (kind, None) } else {
            (SynthInstrument::Analog, Some(Arc::new(fx::OfflineDevice::new(wire.kind, wire.state).map_err(serde::de::Error::custom)?)))
        };
        Ok(Self { kind, voices: wire.voices, cutoff: wire.cutoff, tuning_hz: wire.tuning_hz, offline })
    }
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub id: fx::FxId,
    pub on: bool,
    pub mix: f32,
    pub p: [f32; 4],
    pub(crate) offline: Option<Arc<fx::OfflineDevice>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectWire {
    id: String,
    on: bool,
    mix: f32,
    p: [f32; 4],
    #[serde(default)]
    state: Option<fx::DeviceState>,
}
impl Serialize for Effect {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let id = self.offline.as_ref().map_or_else(
            || if self.id == fx::FxId::Dist { "dist" } else { self.id.name() },
            |device| device.identifier.as_str());
        let state = self.offline.as_ref().and_then(|device| device.state.as_ref());
        let mut out = serializer.serialize_struct("Effect", 4 + usize::from(state.is_some()))?;
        out.serialize_field("id", id)?;
        out.serialize_field("on", &self.on)?;
        out.serialize_field("mix", &self.mix)?;
        out.serialize_field("p", &self.p)?;
        if let Some(state) = state { out.serialize_field("state", state)?; }
        out.end()
    }
}
impl<'de> Deserialize<'de> for Effect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = EffectWire::deserialize(deserializer)?;
        let known = serde_json::from_value::<fx::FxId>(serde_json::Value::String(wire.id.clone()))
            .ok().filter(|id| *id != fx::FxId::Unavailable && wire.state.is_none());
        let (id, offline) = if let Some(id) = known { (id, None) } else {
            (fx::FxId::Unavailable, Some(Arc::new(fx::OfflineDevice::new(wire.id, wire.state).map_err(serde::de::Error::custom)?)))
        };
        Ok(Self { id, on: wire.on, mix: wire.mix, p: wire.p, offline })
    }
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
    #[serde(default, skip_serializing_if = "key_shift::is_zero")]
    pub key_shift: i8,
    pub sync: bool,
    #[serde(default, skip_serializing_if = "deck_sync::Phase::is_none")]
    pub(crate) sync_phase: deck_sync::Phase,
    pub gain: f32,
    pub eq: [f32; 3],
    #[serde(default, skip_serializing_if = "crate::track_gain::Policy::is_off")]
    pub source_gain: crate::track_gain::Policy,
    pub filter_morph: f32,
    pub filter_amt: f32,
    #[serde(default, skip_serializing_if = "channel_fx::Kind::is_filter")]
    pub(crate) channel_effect: channel_fx::Kind,
    pub pfl: bool,
    pub hotcues: [Option<f64>; HOTCUES],
    #[serde(default)]
    pub cue_styles: [crate::engine::cue_metadata::Style; HOTCUES],
    #[serde(default)]
    pub grid: Option<crate::engine::beatgrid::Grid>,
    #[serde(default, skip_serializing_if = "saved_loops::Bank::is_default")]
    pub(crate) saved_loops: saved_loops::Bank,
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
    /// Keep space for transport metadata that can grow without an edit revision.
    /// Takes this validated import destination; returns native limits with room for
    /// every launch, clock/deck scalar and up to 256 held-note duration changes.
    pub(crate) fn import_metadata_limits(&self) -> crate::project_file::Limits {
        let notes: usize = self.tracks.iter().flat_map(|t| &t.clips).map(|c| c.notes.len()).sum();
        let reserve = 96 * self.tracks.len()
            + 32 * (4 + DECKS * 32 + notes.min(super::super::recording::CAPTURES));
        let mut limits = crate::project_file::Limits::default();
        limits.max_metadata_bytes -= reserve;
        limits
    }
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
    /// Create a stopped empty native session with valid unused drum bindings.
    /// Takes no source content; returns editable state and a silent attachment for unused legacy drum slots.
    pub(crate) fn empty() -> Result<(Self, Vec<Arc<Sample>>), String> {
        let mut state = Self::blank();
        state.banks = vec![Bank {
            name: "Empty bank".into(),
            media: [None; 16],
            instance: Some(crate::sampler_bank::BankId::new()?),
            settings: Some(Arc::new(crate::sampler_bank::resident::Settings::empty(
                "Empty bank".into(),
            )?)),
        }];
        let media = vec![Arc::new(Sample {
            name: "Unused drum silence".into(),
            peaks: Arc::new(vec![]),
            spectrum: None,
            bpm: 120.,
            data: vec![0.; 64],
            sr: 48000,
            ch: 1,
            path: "native-unused-drum-silence".into(),
        })];
        Ok((state, media))
    }
    pub(crate) fn blank() -> Self {
        Self {
            version: STATE_VERSION,
            musical_context: None, note_seed: note_variation::DEFAULT_SEED, sampler_scale: false,
            sync_leader: None,
            navigation: None,
            scene_timing: None,
            arrangement: None,
            routing: None,
            migration: None,
            mic_aux: None,
            session: Some(session::Layout::legacy((0..TRACKS).map(|_| String::new()), SCENES)),
            conductor: None,
            bpm: 124.0,
            beat: 0.0,
            timeline_seconds: 0.0,
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
            tracks: (0..TRACKS).map(|_| Track {
                name: String::new(),
                clips: (0..SCENES).map(|_| SavedClip {
                    properties: Default::default(),
                    audio_region: None, lanes: None,
                    region: None,
                    kind: ClipKind::Empty,
                    name: String::new(),
                    bars: 1.0,
                    notes: Vec::new(),
                    gain: 1.0,
                    audio: None,
                }).collect(),
                scene_bus: 0,
                launch: None,
                gain: 0.8,
                pan: 0.0,
                mute: false,
                solo: false,
                armed: false,
                input_monitor: None,
                kind: 0,
                synth: Synth {
                    kind: SynthInstrument::Analog,
                    voices: 8,
                    cutoff: 700.0,
                    tuning_hz: 440.0,
                    offline: None,
                },
                eq: [1.0; 3],
                drums: [0; 6],
                fx: Vec::new(),
            }).collect(),
            decks: std::array::from_fn(|_| Deck {
                audio: None,
                pos: 0.0,
                cue_pos: 0.0,
                pitch: 0.5,
                vinyl: true,
                keylock: false,
                key_shift: 0,
                sync: false,
                sync_phase: deck_sync::Phase::None,
                gain: 0.85,
                source_gain: crate::track_gain::Policy::Off,
                eq: [1.0; 3],
                filter_morph: 0.5,
                filter_amt: 0.5,
                channel_effect: channel_fx::Kind::Filter,
                pfl: false,
                hotcues: [None; HOTCUES],
                cue_styles: [crate::engine::cue_metadata::Style::default(); HOTCUES],
                grid: None,
                saved_loops: Default::default(),
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
            scene_fx: (0..SCENES).map(|_| Vec::new()).collect(),
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
                offline: None,
            },
            banks: Vec::new(),
            builtin: [None; 2],
        }
    }

    /// Check before constructing delay/reverb buffers. PCM has its own loader limits.
    pub(crate) fn validate_processor_storage(&self, sr: u32) -> Result<(), String> {
        const LIMIT: usize = session::MAX_PROCESSOR_BYTES;
        let synth_bytes = self.tracks.iter().map(|track| &track.synth).chain(std::iter::once(&self.sampler_synth))
            .map(|synth| synth.offline.as_ref().map_or(0, |device| device.bytes())).sum::<usize>();
        let bytes = synth_bytes + self.tracks.iter().flat_map(|t| &t.fx).chain(self.scene_fx.iter().flatten())
            .map(|effect| fx::FxSlot::required_storage(effect.id, sr as f32)
                + effect.offline.as_ref().map_or(0, |device| device.bytes())).sum::<usize>();
        if bytes > LIMIT { return Err("Session processor storage exceeds 256 MiB; remove effects or use a lower output rate".into()); }
        Ok(())
    }
    pub fn validate(&self, media: &[Arc<Sample>]) -> Result<(), String> {
        let fail = |name: &str| Err(format!("invalid project {name}"));
        if let Some(migration) = &self.migration {
            if self.version < 30 {
                return fail("migration in legacy state");
            }
            if self.version<32 && migration.schema>=2 {return fail("Pack metadata in legacy migration state");}
            migration.validate(media.len())?;
        }
        if self.version < 17 && self.mic_aux.is_some() {return fail("mic/aux controls in a legacy state");}
        if let Some(cfg)=self.mic_aux {cfg.validate(self.routing.as_deref()).map_err(str::to_owned)?;}
        if self.version < 16 && self.tracks.iter().any(|track| track.input_monitor.is_some()) { return fail("input monitoring in a legacy state"); }
        if self.version < 9 && (self.sampler_synth.offline.is_some() || self.tracks.iter().any(|track| track.synth.offline.is_some())
            || self.tracks.iter().flat_map(|track| &track.fx).chain(self.scene_fx.iter().flatten()).any(|effect| effect.offline.is_some())) {
            return fail("unavailable device in a legacy state");
        }
        if self.tracks.is_empty() || self.tracks.len() > session::MAX_TRACKS || self.scene_fx.is_empty() || self.scene_fx.len() > session::MAX_SCENES || self.tracks.iter().any(|t| t.clips.len() != self.scene_fx.len()) { return fail("session dimensions (1–128 tracks, 1–512 scenes)"); }
        if self.version >= 7 && self.session.is_none() {return fail("missing session identity metadata");}
        if let Some(timing) = self.scene_timing { timing.validate()?; if self.version < 23 || self.conductor.is_some() { return fail("scene timing version or conductor conflict"); } }
        if self.version < 34 && self.note_seed != note_variation::DEFAULT_SEED { return fail("note random seed requires project state version 34"); }
        if self.musical_context.is_some_and(|context| !context.valid()) || self.version < 33 && (self.musical_context.is_some() || self.sampler_scale) { return fail("song scale or scale-aware sampler version"); }
        if self.version < 23 && self.session.as_ref().is_some_and(|layout| layout.scenes.iter().chain(&layout.tracks).any(|item| !item.scene.is_default())) { return fail("scene properties version"); }
        if self.version < 7 && (self.tracks.len() != TRACKS || self.scene_fx.len() != SCENES || self.session.is_some()) { return fail("legacy session dimensions or identity"); }
        if let Some(layout) = &self.session {
            layout.validate()?;
            if layout.tracks.len() != self.tracks.len() || layout.scenes.len() != self.scene_fx.len() { return fail("session identity and storage disagree"); }
            if !layout.tracks[self.selected_track.min(layout.tracks.len()-1)].active || !layout.scenes[self.selected_scene.min(layout.scenes.len()-1)].active { return fail("inactive session focus"); }
        }
        if let Some(routing) = &self.routing {
            if self.version < 11 { return fail("routing metadata in a legacy state"); }
            if self.version < 29 && !routing.plugins.is_empty() { return fail("plugins in a legacy state"); }
            routing.order(self.session.as_ref().ok_or("Routing requires retained session identities")?)?;
        }
        if !(1..=STATE_VERSION).contains(&self.version) {
            return Err(format!(
                "unsupported project state version {}",
                self.version
            ));
        }
        if !(40.0..=240.0).contains(&self.bpm)
            || !finite_range(self.beat, 0.0, 1.0e12)
            || !finite_range(self.timeline_seconds, 0.0, 1.0e12)
            || !finite_range(self.quant as f64, 0.0, 64.0)
            || !unit(self.xfader)
            || !unit(self.xfader_curve)
            || !finite_range(self.master as f64, 0.0, 1.5)
            || !unit(self.cue_mix)
            || self.selected_track >= self.tracks.len()
            || self.selected_scene >= self.scene_fx.len()
            || self.selected_deck >= DECKS
            || !(self.fx_view == -1
                || (0..self.tracks.len() as i16).contains(&self.fx_view)
                || (if self.version < 7 {100} else {session::SCENE_FX_BASE} .. if self.version < 7 {100} else {session::SCENE_FX_BASE} + self.scene_fx.len() as i16).contains(&self.fx_view))
            || self.fx_wet.iter().any(|v| !unit(*v))
        {
            return fail("timing, view or mixer controls");
        }
        if self.banks.is_empty()
            || self.banks.len() > MAX_BANKS
            || self.sampler_bank >= self.banks.len()
            || !(-4..=8).contains(&self.sampler_oct)
            || !valid_synth(&self.sampler_synth)
        {
            return fail("sampler settings");
        }
        if self.version < 6 && self.conductor.is_some() { return fail("legacy conductor"); }
        if let Some(conductor) = &self.conductor {
            conductor.validate()?;
            if self.version < 8 && (conductor.native.is_some() || conductor.tempos.iter().any(|p| p.ramp)) { return fail("legacy native timeline"); }
        }
        let mut midi_bytes = self.conductor.as_ref().map_or(0, |c| c.bytes());
        let reference = |index: usize| index < media.len();
        let optional = |index: Option<usize>| index.is_none_or(reference);
        let mut note_count = 0usize;
        for track in &self.tracks {
            if !text_ok(&track.name)
                || track.scene_bus >= self.scene_fx.len()
                || track.kind > 4
                || !valid_synth(&track.synth)
                || !finite_range(track.gain as f64, 0.0, 1.5)
                || !finite_range(track.pan as f64, -1.0, 1.0)
                || !valid_eq(track.eq)
                || track.drums.iter().any(|i| !reference(*i))
                || !valid_fx(&track.fx, false)
            {
                return fail("track controls or media reference");
            }
            if let Some(launch) = track.launch {
                if launch.scene as usize >= self.scene_fx.len()
                    || !finite_range(launch.start_beat, -1.0e12, 1.0e12)
                {
                    return fail("clip resume target");
                }
            }
            for clip in &track.clips {let (notes,bytes)=clip.validate(self.version,media)?;note_count+=notes;midi_bytes+=bytes;}
        }
        if let Some(navigation)=&self.navigation{if self.version<22{return fail("song sections in legacy state");}navigation.validate()?;}
        if let Some(arrangement)=&self.arrangement{if self.version<19{return fail("arrangement in legacy state");}if self.version<26 && (arrangement.instances.iter().any(|i|i.fades.is_some()||i.fade_link!=0||i.crossfade.is_some())||arrangement.sources.iter().any(|s|s.clip.audio_region.is_some_and(|r|!r.fades.is_default()))){return fail("audio fades in a legacy arrangement");}arrangement.validate(media,self.session.as_ref().ok_or("Arrangement requires track identities")?)?;let (n,b)=arrangement.midi_storage();note_count+=n;midi_bytes+=b;}
        if midi_bytes > midi_data::MAX_LANE_BYTES { return fail("MIDI metadata exceeds 16 MiB"); }
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
            if self.version < 9 && bank.settings.as_ref().is_some_and(|settings| settings.slots.iter().any(|slot| matches!(slot.source, Some(crate::sampler_bank::Source::Project { .. })))) { return fail("relinked source in a legacy state"); }
            if self.version < 4 {
                if bank.instance.is_some() || bank.settings.is_some() || bank.media.iter().any(Option::is_none) {
                    return fail("legacy sample bank must contain embedded media only");
                }
            } else {
                let (Some(instance), Some(settings)) = (bank.instance, &bank.settings) else { return fail("sample bank identity or settings"); };
                if !bank_ids.insert(instance) || settings.definition == Some(instance) || settings.name != bank.name || settings.validate().is_err() { return fail("sample bank identity or settings"); }
                if self.version < 12 && settings.slots.iter().any(|slot| slot.playback != crate::sampler_bank::Playback::default()) {
                    return fail("sampler playback modes in a legacy state");
                }
                for (slot, index) in settings.slots.iter().zip(bank.media) {
                    if index.is_some_and(|i| slot.frames(media[i].sr, media[i].frames()).is_err()) { return fail("sample bank source range"); }
                }
            }
        }
        if self.builtin.iter().any(|i| !optional(*i)) {
            return fail("built-in sample reference");
        }
        for (index, deck) in self.decks.iter().enumerate() {
            if self.sync_leader.and_then(deck_sync::Leader::deck) == Some(index) && !deck.sync_phase.is_none() { return Err("A sync leader cannot also follow phase".into()); }
            if !deck.sync_phase.is_none() && (!deck.sync || self.sync_leader.is_none()) || self.version < 25 && (!deck.sync_phase.is_none() || self.sync_leader.is_some()) { return Err("Invalid sync intent or older project header".into()); }
            if !optional(deck.audio)
                || !text_ok(&deck.title)
                || !unit(deck.pitch)
                || !(-key_shift::MAX_SEMITONES..=key_shift::MAX_SEMITONES).contains(&deck.key_shift)
                || self.version < 28 && deck.key_shift != 0
                || !finite_range(deck.gain as f64, 0.0, 1.5)
                || !deck.source_gain.valid()
                || self.version < 14 && !deck.source_gain.is_off()
                || !valid_eq(deck.eq)
                || !unit(deck.filter_amt)
                || !unit(deck.filter_morph)
                || deck.pitch_range > 2
                || !(-1..=3).contains(&deck.eq_solo)
                || deck
                    .eq_store
                    .iter()
                    .any(|v| !finite_range(*v as f64, 0.0, 16.0))
                || self.version < 13 && deck.grid.is_some_and(|grid| !grid.anchors().is_empty())
                || !finite_range(deck.bpm as f64, 1.0, 1000.0)
                || !finite_range(deck.sync_bpm as f64, 1.0, 1000.0)
                || !finite_range(deck.pos, -1.0e12, 1.0e12)
                || !finite_range(deck.cue_pos, -1.0e12, 1.0e12)
                || !finite_range(deck.loop_start, -1.0e12, 1.0e12)
                || !finite_range(deck.loop_len, 0.0, 1.0e12)
                || !deck.saved_loops.valid()
                || self.version < 27 && !saved_loops::Bank::cue_loops_empty(&deck.saved_loops.cue_loops)
                || deck.saved_loops.cue_loops.iter().enumerate().any(|(cue, id)| id.is_some_and(|id| deck.hotcues[cue].is_none_or(|position| deck.audio.and_then(|index|media.get(index)).is_none_or(|audio| deck.saved_loops.slots[usize::from(id - 1)].is_none_or(|slot| (position - slot.start * f64::from(audio.sr)).abs() > 1e-6)))))
                || self.version < 24 && !deck.saved_loops.is_default()
                || deck.saved_loops.slots.iter().flatten().any(|slot| deck.audio.and_then(|index|media.get(index)).is_none_or(|audio| slot.length * f64::from(audio.sr) + 1e-6 < 64.0 || (slot.start + slot.length) * f64::from(audio.sr) > audio.frames() as f64 + 1e-6))
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
        self.media_indices(|index| *index = remap[*index]);
        *media = unique;
    }

    fn media_indices(&mut self, mut visit: impl FnMut(&mut usize)) {
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                if let Some(index) = &mut clip.audio { visit(index); }
            }
            for index in &mut track.drums {
                visit(index);
            }
        }
        for deck in &mut self.decks {
            if let Some(index) = &mut deck.audio { visit(index); }
        }
        for bank in &mut self.banks {
            for index in bank.media.iter_mut().flatten() {
                visit(index);
            }
        }
        for index in self.builtin.iter_mut().flatten() { visit(index); }
        if let Some(arrangement)=&mut self.arrangement{for source in &mut Arc::make_mut(arrangement).sources{if let Some(index)=&mut source.clip.audio{visit(index);}}}
    }

    /// Number media in the same attachment order as a native renderer capture.
    /// Takes validated state and owned media; preserves samples while matching save metadata indices.
    pub(crate) fn capture_media_order(&mut self, media: &mut Vec<Arc<Sample>>) -> Result<(), String> {
        self.validate(media)?;
        let previous = std::mem::take(media);
        self.media_indices(|index| { media.push(previous[*index].clone()); *index = media.len() - 1; });
        self.deduplicate(media);
        Ok(())
    }

    /// Keep and reindex only audio referenced by validated native state.
    /// `media` is compacted off audio; duplicate shared samples use one entry.
    pub(crate) fn compact_media(&mut self, media: &mut Vec<Arc<Sample>>) -> Result<(), String> {
        self.validate(media)?;
        let mut used = vec![false; media.len()];
        self.media_indices(|index| used[*index] = true);
        let mut remap = vec![0; media.len()];
        let mut retained = Vec::new();
        for (index, sample) in media.iter().enumerate() {
            if used[index] { remap[index] = retained.len(); retained.push(sample.clone()); }
        }
        self.media_indices(|index| *index = remap[*index]);
        *media = retained;
        self.deduplicate(media);
        Ok(())
    }

    /// Capture one track's settings and audio dependencies without song content.
    /// `track` is a live storage slot; returned state has one empty clip and bus.
    pub(crate) fn track_configuration(&self, media: &[Arc<Sample>], track: usize) -> Result<(Self, Vec<Arc<Sample>>), String> {
        self.validate(media)?;
        let mut selected = self.tracks.get(track).ok_or("Template track is unavailable")?.clone();
        if self.session.as_ref().is_some_and(|layout| !layout.tracks[track].active) {
            return Err("Template track is inactive".into());
        }
        let mut state = Self::blank();
        selected.clips = vec![state.tracks[0].clips[0].clone()];
        selected.launch = None;
        selected.scene_bus = 0;
        state.session = Some(session::Layout::fresh([selected.name.clone()], 1));
        state.tracks = vec![selected];
        state.scene_fx = vec![Vec::new()];
        state.banks = vec![Bank { name: "Empty bank".into(), media: [None;16], instance: Some(crate::sampler_bank::BankId::new()?),
            settings: Some(Arc::new(crate::sampler_bank::resident::Settings::empty("Empty bank".into())?)) }];
        let mut audio = media.to_vec();
        state.compact_media(&mut audio)?;
        Ok((state, audio))
    }

    /// Apply a track configuration while retaining clips and unrelated state.
    /// `target` is an active storage slot and `bus` must name one active scene.
    /// Returns an owned replacement with rebound audio; source data stays intact.
    pub(crate) fn apply_track_configuration(mut self, mut media: Vec<Arc<Sample>>, configuration: &Self,
        source_media: &[Arc<Sample>], target: usize, bus: &str) -> Result<(Self, Vec<Arc<Sample>>), String> {
        self.validate(&media)?;
        configuration.validate(source_media)?;
        if configuration.tracks.len() != 1 || configuration.scene_fx.len() != 1
            || configuration.arrangement.as_ref().is_some_and(|s|!s.sources.is_empty()||!s.instances.is_empty())
            || configuration.tracks[0].launch.is_some()
            || configuration.tracks[0].clips.iter().any(|clip| clip.kind != ClipKind::Empty || !clip.notes.is_empty() || clip.audio.is_some() || clip.lanes.is_some()) {
            return Err("Track template contains song content or multiple tracks".into());
        }
        let layout = self.session.as_ref().ok_or("Current session has no bus aliases")?;
        if !layout.tracks.get(target).is_some_and(|track| track.active) {
            return Err("Template target track is unavailable".into());
        }
        let mut buses = layout.scenes.iter().enumerate().filter(|(_, scene)| scene.active && scene.name == bus);
        let destination = buses.next().map(|(index, _)| index).ok_or("Template scene-bus alias is unavailable; no routing was changed")?;
        if buses.next().is_some() { return Err("Template scene-bus alias is ambiguous; no routing was changed".into()); }
        let generation = layout.generation.checked_add(1).ok_or("Session generation exhausted")?;
        let mut replacement = configuration.tracks[0].clone();
        let offset = media.len();
        for index in &mut replacement.drums { *index += offset; }
        replacement.clips = self.tracks[target].clips.clone();
        replacement.launch = self.tracks[target].launch;
        replacement.scene_bus = destination;
        media.extend(source_media.iter().cloned());
        self.tracks[target] = replacement;
        let layout = self.session.as_mut().unwrap();
        layout.tracks[target].name = self.tracks[target].name.clone();
        layout.generation = generation;
        self.compact_media(&mut media)?;
        Ok((self, media))
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
fn valid_synth(s: &Synth) -> bool {
    (1..=64).contains(&s.voices)
        && finite_range(s.cutoff as f64, 0.0, 20000.0)
        && finite_range(s.tuning_hz as f64, 20.0, 20000.0)
}
fn valid_fx(rack: &[Effect], scene: bool) -> bool {
    rack.len() <= MAX_FX_PER_RACK
        && rack.iter().all(|f| {
            unit(f.mix) && f.p.iter().all(|p| unit(*p)) && (!scene || f.id != fx::FxId::Arp)
                && (f.id == fx::FxId::Unavailable) == f.offline.is_some()
        })
}

#[cfg(test)]
mod routing_schema_tests {
use super::*;

#[test]
fn routing_schema_requires_native_identity_and_rejects_legacy_injection() {
    let (_, rt) = Engine::headless_for_test(48000, 256);
    let mut frame = super::super::capture::Frame::new();
    for _ in 0..4 { frame.capture(&rt); if frame.complete { break; } frame.prepare(); }
    assert!(frame.complete);
    let mut state = frame.state;
    state.routing = Some(std::sync::Arc::new(audio::routing::model::Model::default()));
    let saved = serde_json::to_vec(&state).unwrap();
    let reopened: crate::engine::project::State = serde_json::from_slice(&saved).unwrap();
    reopened.validate(&frame.media).unwrap();
    assert_eq!(reopened.routing, state.routing);
    let mut legacy: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    legacy["version"] = 10.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(legacy).is_err());
    let mut model = audio::routing::model::Model::default();
    model.connections[0].source.group = audio::routing::model::Group::Track(state.session.as_ref().unwrap().scenes[0].id);
    assert!(model.order(state.session.as_ref().unwrap()).is_err());
}

}

impl SavedClip{
    /// Create an unused clip slot.
    /// Takes no content; returns neutral editable metadata with no note, audio or launch attachments.
    pub(crate) fn empty() -> Self {
        Self {
            properties: Default::default(),
            audio_region: None,
            lanes: None,
            region: None,
            kind: ClipKind::Empty,
            name: String::new(),
            bars: 1.,
            notes: Vec::new(),
            gain: 1.,
            audio: None,
        }
    }

    /// Validate one retained clip without constructing a project graph.
    /// Takes the schema and shared media; returns its note and lane budgets or the same native source refusal used by project validation.
    pub(crate) fn validate(&self,version:u32,media:&[Arc<Sample>])->Result<(usize,usize),String>{
        if self.notes.iter().any(|note| note.variation.is_some()) && (version < 34 || self.kind != ClipKind::Midi || self.region.is_none()) { return Err("Note variation requires a MIDI region and project state version 34".into()); }
        note_variation::validate(&self.notes)?;
        if self.properties.context.is_some_and(|context| !context.valid()) || version < 33 && self.properties.context.is_some() { return Err("Clip scale requires a valid tonic and project state version 33".into()); }
        if version<21 && !self.properties.launch.is_default(){return Err("Clip launch policy requires project state version 21".into());}
        if version<20 && !self.properties.is_default(){return Err("Clip properties require project state version 20".into());}
        let clip=self;let fail=|name:&str|Err(format!("invalid project {name}"));let reference=|index:usize|index<media.len();let optional=|index:Option<usize>|index.is_none_or(reference);let mut note_ids=std::collections::HashSet::new();let mut midi_bytes=0;let mut note_count=0;

                if let Some(lanes) = &clip.lanes {
                    if version < 6 || clip.kind != ClipKind::Midi { return fail("legacy or non-MIDI lanes"); }
                    if version < 15 && !lanes.labels.is_empty() { return fail("MIDI labels in a legacy state"); }
                    lanes.validate()?; midi_bytes += lanes.bytes();
                }
                if let Some(region)=clip.audio_region {if version<26 && !region.fades.is_default(){return fail("audio fades in a legacy clip");}if version<18||clip.kind!=ClipKind::Audio{return fail("audio source region in a legacy or non-audio clip");}let source=clip.audio.and_then(|i|media.get(i)).ok_or_else(||"Audio clip region has no embedded source".to_owned())?;let plan=region.prepare(source).map_err(str::to_owned)?;if clip.bars!=(plan.duration_beats/4.0)as f32{return fail("audio clip duration disagrees with its source region");}}
                note_ids.clear();
                if !text_ok(&clip.name)
                    || !finite_range(clip.bars as f64, if clip.audio_region.is_some() || version >= 30 && clip.kind == ClipKind::Audio && clip.audio.is_none() && clip.properties.disabled {0.0000001}else if clip.region.is_some() { 1.0 / 4096.0 } else { 0.25 }, 65536.0)
                    || !finite_range(clip.gain as f64, 0.0, 1.5)
                    || !optional(clip.audio)
                    || clip.notes.len() > MAX_NOTES_PER_CLIP
                    || clip.region.is_some_and(|region| !region.allows(&clip.notes) || clip.kind != ClipKind::Midi || clip.bars != (region.end / 4.0) as f32)
                    || version < 5 && clip.region.is_some()
                {
                    return fail("clip controls or media reference");
                }
                note_count += clip.notes.len();
                for note in &clip.notes {
                    if !note.interchange_valid()
                        || version<6 && (note.channel!=0 || note.release_vel!=64 || note.source_timing.is_some())
                        || note.pitch > 127
                        || note.vel > 127
                        || !finite_range(note.start as f64, 0.0, 262144.0)
                        || !finite_range(note.len as f64, 0.0, 262144.0)
                        || version >= 5 && (!note.id.valid() || !note_ids.insert(note.id))
                        || version < 5 && note.muted
                    {
                        return fail("MIDI note");
                    }
                }

        Ok((note_count,midi_bytes))
    }
}
