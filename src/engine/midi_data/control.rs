use crate::midi_file::Message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ControlKind {
    Cc {
        controller: u8,
    },
    Bend,
    Pressure,
    Program {
        bank_msb: u8,
        bank_lsb: u8,
        program: u8,
    },
}
impl ControlKind {
    pub fn valid(&self) -> bool {
        match *self {
            Self::Cc { controller } => controller < 120,
            Self::Program {
                bank_msb,
                bank_lsb,
                program,
            } => bank_msb < 128 && bank_lsb < 128 && program < 128,
            _ => true,
        }
    }
    pub fn name(&self) -> String {
        match *self {
            Self::Cc { controller } => format!(
                "CC {controller} {}",
                match controller {
                    0 => "Bank MSB",
                    1 => "Modulation",
                    2 => "Breath",
                    7 => "Volume",
                    10 => "Pan",
                    11 => "Expression",
                    32 => "Bank LSB",
                    64 => "Sustain",
                    74 => "Brightness",
                    _ => "",
                }
            )
            .trim_end()
            .into(),
            Self::Bend => "Pitch bend".into(),
            Self::Pressure => "Channel pressure".into(),
            Self::Program {
                bank_msb,
                bank_lsb,
                program,
            } => format!("Bank {bank_msb}:{bank_lsb} · program {program}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Label {
    pub channel: u8,
    pub control: ControlKind,
    pub name: String,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct StatePoint {
    pub message: Message,
    pub banks: [Option<u8>; 2],
}
#[derive(Clone, Debug)]
pub(crate) struct StateLane {
    pub key: u16,
    pub points: Vec<StatePoint>,
}
impl StateLane {
    pub fn before(&self, tick: f64) -> Option<StatePoint> {
        let index = self
            .points
            .partition_point(|p| (p.message.tick as f64) < tick);
        index.checked_sub(1).map(|i| self.points[i])
    }
}
/// Index controller state on the preparation worker.
/// Takes ordered source messages; returns compact per-channel histories for logarithmic seek lookup.
pub(crate) fn index(messages: &[Message]) -> Vec<StateLane> {
    let mut banks = [[None; 2]; 16];
    let mut lanes = BTreeMap::<u16, Vec<StatePoint>>::new();
    for &message in messages {
        let channel = usize::from(message.bytes[0] & 15);
        let kind = match message.bytes[0] & 0xf0 {
            0xb0 => {
                if message.bytes[1] == 0 {
                    banks[channel][0] = Some(message.bytes[2]);
                }
                if message.bytes[1] == 32 {
                    banks[channel][1] = Some(message.bytes[2]);
                }
                u16::from(message.bytes[1])
            }
            0xe0 => 128,
            0xd0 => 129,
            0xc0 => 130,
            _ => continue,
        };
        lanes
            .entry(channel as u16 * 256 + kind)
            .or_default()
            .push(StatePoint {
                message,
                banks: banks[channel],
            });
    }
    lanes
        .into_iter()
        .map(|(key, mut points)| {
            points.shrink_to_fit();
            StateLane { key, points }
        })
        .collect()
}

/// Restore controller state in the order required by MIDI devices.
/// Takes indexed state lookup and a bounded packet sink; restores reset, original program banks, pending banks and ordinary controls before notes without replaying parameter transactions.
pub(crate) fn chase(mut point:impl FnMut(u16)->Option<StatePoint>,mut emit:impl FnMut([u8;3],u8,u64)){
    for channel in 0..16u16{
        let reset=point(channel*256+121);if let Some(reset)=reset{emit(reset.message.bytes,3,u64::from(channel));}
        let program=point(channel*256+130);
        if let Some(program)=program{for(i,cc)in[0,32].into_iter().enumerate(){if let Some(value)=program.banks[i]{emit([0xb0|channel as u8,cc,value],3,32+u64::from(channel)*4+i as u64);}}emit(program.message.bytes,2,34+u64::from(channel)*4);}
        for kind in(0..120u16).chain([128,129]){
            if matches!(kind,6|38|96..=101){continue;}
            let Some(value)=point(channel*256+kind)else{continue};
            if !matches!(kind,0|7|10|32)&&reset.is_some_and(|r|(value.message.tick,value.message.order)<(r.message.tick,r.message.order)){continue;}
            if matches!(kind,0|32)&&program.is_some_and(|p|p.banks[usize::from(kind==32)]==Some(value.message.bytes[2])){continue;}
            emit(value.message.bytes,value.message.length,128+u64::from(channel)*256+u64::from(kind));
        }
    }
}
