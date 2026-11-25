use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Values {
    pub pitch: f32,
    pub pressure: u8,
    pub timbre: u8,
}
impl From<crate::engine::mpe::Values> for Values {
    fn from(values: crate::engine::mpe::Values) -> Self {
        Self {
            pitch: values.pitch,
            pressure: values.pressure,
            timbre: values.timbre,
        }
    }
}
impl Values {
    fn valid(self) -> bool {
        self.pitch.is_finite()
            && (-120.0..=120.0).contains(&self.pitch)
            && self.pressure < 128
            && self.timbre < 128
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Event {
    On {
        id: i32,
        pitch: u8,
        velocity: u8,
        values: Values,
    },
    Values {
        id: i32,
        values: Values,
    },
    Off {
        id: i32,
        pitch: u8,
    },
}
impl Event {
    pub(crate) fn id(self) -> i32 {
        match self {
            Self::On { id, .. } | Self::Values { id, .. } | Self::Off { id, .. } => id,
        }
    }
    pub(crate) fn valid(self) -> bool {
        self.id() > 0
            && match self {
                Self::On {
                    pitch,
                    velocity,
                    values,
                    ..
                } => pitch < 128 && (1..128).contains(&velocity) && values.valid(),
                Self::Values { values, .. } => values.valid(),
                Self::Off { pitch, .. } => pitch < 128,
            }
    }
    /// Deliver a voice with its own native identity.
    /// Takes the isolated plugin, exact sample offset and musical position; returns the native event result without using shared channel controllers.
    pub(crate) fn send(
        self,
        plugin: &mut vst3_host::Plugin,
        offset: i32,
        beat: f64,
    ) -> Result<(), String> {
        use vst3_host::{NoteExpressionType, NoteId, PluginEvent, PluginEventData};
        if !self.valid() {
            return Err("Per-note plugin event is invalid".into());
        }
        let event = |data| PluginEvent {
            bus_index: 0,
            sample_offset: offset,
            ppq_position: beat,
            flags: 0,
            data,
        };
        let values = match self {
            Self::On {
                id,
                pitch,
                velocity,
                values,
            } => {
                plugin
                    .send_plugin_event(event(PluginEventData::NoteOn {
                        channel: 0,
                        pitch: i16::from(pitch),
                        tuning: 0.0,
                        velocity: f32::from(velocity) / 127.0,
                        length: 0,
                        note_id: id,
                    }))
                    .map_err(|e| e.to_string())?;
                Some((id, values))
            }
            Self::Values { id, values } => Some((id, values)),
            Self::Off { id, pitch } => {
                plugin
                    .send_plugin_event(event(PluginEventData::NoteOff {
                        channel: 0,
                        pitch: i16::from(pitch),
                        tuning: 0.0,
                        velocity: 0.0,
                        note_id: id,
                    }))
                    .map_err(|e| e.to_string())?;
                None
            }
        };
        if let Some((id, values)) = values {
            for (kind, value) in [
                (
                    NoteExpressionType::Tuning,
                    f64::from(values.pitch) / 240.0 + 0.5,
                ),
                (
                    NoteExpressionType::Expression,
                    f64::from(values.pressure) / 127.0,
                ),
                (
                    NoteExpressionType::Brightness,
                    f64::from(values.timbre) / 127.0,
                ),
            ] {
                plugin
                    .send_note_expression_at(NoteId::from_raw(id), kind, value, offset)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Routed {
    Midi([u8; 3]),
    Expression(Event),
}
