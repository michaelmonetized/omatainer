use crate::{engine::session::{Id, Layout}, plugin_host::{self, Saved}};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_PLUGINS: usize = 32;
pub(crate) const MAX_PARAMETERS: usize = crate::plugin_host::MAX_CONTROL_PARAMETERS;

/// Bound the on-disk processor portion before admitting a graph.
/// Takes validated instances; returns refusal when JSON state expansion would consume more than half the native metadata budget.
pub(crate) fn validate_encoded(instances: &[Instance]) -> Result<(), String> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes:&[u8]) -> std::io::Result<usize> {
            self.0=self.0.checked_add(bytes.len()).filter(|n|*n<=crate::project_file::DEFAULT_METADATA_LIMIT/2).ok_or_else(||std::io::Error::other("Plugin metadata exceeds 32 MiB after encoding"))?; Ok(bytes.len())
        }
        fn flush(&mut self)->std::io::Result<()> {Ok(())}
    }
    serde_json::to_writer(Budget(0),instances).map_err(|e|e.to_string())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Instance {
    pub id: u64,
    pub name: String,
    pub saved: Saved,
    pub inputs: Vec<u8>,
    pub outputs: Vec<u8>,
    pub midi_track: Option<Id>,
    pub scene_track: Option<Id>,
    pub instrument: bool,
    pub bypass: bool,
    pub latency: u32,
    pub parameters: Vec<Parameter>,
    pub automation: Vec<Automation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Parameter {
    pub id: u32,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Automation {
    pub id: u32,
    pub points: Vec<[f64; 2]>,
}
impl Automation {
    /// Read a normalized linear parameter envelope.
    /// Takes absolute song beats; returns its held or interpolated value without allocating.
    pub(crate) fn at(&self, beat: f64) -> f64 {
        let next = self.points.partition_point(|p| p[0] <= beat);
        if next == 0 { return self.points[0][1]; }
        let old = self.points[next - 1];
        self.points.get(next).map_or(old[1], |p| old[1] + (p[1] - old[1]) * ((beat - old[0]) / (p[0] - old[0])))
    }
}
impl Instance {
    /// Validate a saved processor independently of its installed binary.
    /// Takes retained session identity; returns a storage or identity refusal while preserving missing devices.
    pub(crate) fn validate(&self, layout: &Layout) -> Result<(), String> {
        self.saved.validate()?;
        let width = |b: &[u8]| b.len() <= plugin_host::MAX_BUSES && b.iter().all(|c| (1..=32).contains(c)) && b.iter().map(|c| usize::from(*c)).sum::<usize>() <= 32;
        if self.name.trim().is_empty() || self.name.len() > 128 || self.name.chars().any(char::is_control)
            || !width(&self.inputs) || !width(&self.outputs) || self.outputs.is_empty()
            || self.latency > 384_000 || self.parameters.len() > MAX_PARAMETERS || self.automation.len() > MAX_PARAMETERS
            || self.midi_track.is_some_and(|id| !layout.tracks.iter().any(|t| t.id == id))
            || self.scene_track.is_some_and(|id| !layout.tracks.iter().any(|t| t.id == id))
            || self.instrument && self.midi_track.is_none()
            || self.unavailable.as_ref().is_some_and(|s| s.len() > 4096)
        { return Err("Plugin name, buses, MIDI source or storage limits are invalid".into()); }
        let mut ids = std::collections::BTreeSet::new();
        if self.parameters.iter().any(|p| !ids.insert(p.id) || !p.value.is_finite() || !(0.0..=1.0).contains(&p.value)) { return Err("Plugin parameters require unique IDs and normalized values".into()); }
        ids.clear();
        let mut points = 0;
        for lane in &self.automation {
            points += lane.points.len();
            if !ids.insert(lane.id) || lane.points.is_empty() || points > 16384
                || lane.points.iter().any(|p| !p[0].is_finite() || !(0.0..=1.0e9).contains(&p[0]) || !p[1].is_finite() || !(0.0..=1.0).contains(&p[1]))
                || lane.points.windows(2).any(|p| p[0][0] >= p[1][0])
            { return Err("Plugin automation requires unique lanes and ordered bounded song positions".into()); }
        }
        Ok(())
    }
    pub(crate) fn input_width(&self) -> usize { self.inputs.iter().map(|n| usize::from(*n)).sum() }
    pub(crate) fn output_width(&self) -> usize { self.outputs.iter().map(|n| usize::from(*n)).sum() }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.name.capacity() + self.saved.state.capacity() + self.unavailable.as_ref().map_or(0,String::capacity)
            + self.saved.class_id.capacity() + self.saved.plugin_version.capacity() + self.saved.state_codec.capacity()
            + self.saved.binary.bundle.as_os_str().len() + self.saved.binary.binary.as_os_str().len() + self.saved.binary.sha256.capacity() + self.saved.binary.arch.capacity()
            + self.inputs.capacity() + self.outputs.capacity() + self.parameters.capacity() * std::mem::size_of::<Parameter>()
            + self.automation.capacity() * std::mem::size_of::<Automation>() + self.automation.iter().map(|a| a.points.capacity() * 16).sum::<usize>()
    }
}
