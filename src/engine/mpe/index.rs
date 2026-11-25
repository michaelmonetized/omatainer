use super::{valid, State};
use crate::{engine::midi_data::StateLane, midi_file::Message};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
struct Point {
    tick: u64,
    order: u32,
    state: State,
}
#[derive(Debug)]
pub(crate) struct Index {
    points: Vec<Point>,
}
impl Index {
    /// Prepare zone and parameter history.
    /// Takes validated source messages and cancellation; returns an immutable index when the stream enables MPE.
    pub(crate) fn prepare(
        messages: &[Message],
        cancel: &mut impl FnMut() -> bool,
    ) -> Result<Option<Arc<Self>>, String> {
        let mut state = State::default();
        let mut points = Vec::new();
        let mut enabled = false;
        for (index, message) in messages.iter().enumerate() {
            if index % 1024 == 0 && cancel() {
                return Err("MPE preparation cancelled".into());
            }
            let bytes = &message.bytes[..usize::from(message.length)];
            if !valid(bytes) {
                continue;
            }
            let before = state;
            state.input(bytes);
            enabled |= state.lower != 0 || state.upper != 0;
            if bytes[0] & 0xf0 == 0xb0 && matches!(bytes[1], 6 | 98..=101 | 121) && state != before
            {
                if (points.len() + 1) * std::mem::size_of::<Point>()
                    > crate::engine::midi_data::MAX_LANE_BYTES
                {
                    return Err("MPE parameter history exceeds the MIDI lane memory limit".into());
                }
                points.push(Point {
                    tick: message.tick,
                    order: message.order,
                    state,
                });
            }
        }
        points.shrink_to_fit();
        Ok(enabled.then(|| Arc::new(Self { points })))
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.points.capacity() * std::mem::size_of::<Point>()
    }
    /// Restore expression at a seek boundary.
    /// Takes source ticks and prepared ordinary-control histories; returns zones and current controls through bounded binary searches without allocation.
    pub(crate) fn before(&self, tick: f64, lanes: &[StateLane]) -> State {
        self.before_event(tick, 0, lanes)
    }
    pub(crate) fn before_event(&self, tick: f64, order: u32, lanes: &[StateLane]) -> State {
        let checkpoint = self
            .points
            .partition_point(|point| (point.tick as f64, point.order) < (tick, order))
            .checked_sub(1)
            .map(|index| self.points[index]);
        let mut state = checkpoint.map_or_else(State::default, |point| point.state);
        for lane in lanes {
            if !matches!(lane.key % 256, 64 | 74 | 128 | 129) {
                continue;
            }
            let index = lane.points.partition_point(|point| {
                (point.message.tick as f64, point.message.order) < (tick, order)
            });
            let Some(point) = index.checked_sub(1).map(|index| lane.points[index]) else {
                continue;
            };
            if checkpoint.is_some_and(|checkpoint| {
                (point.message.tick, point.message.order) <= (checkpoint.tick, checkpoint.order)
            }) {
                continue;
            }
            state.input(&point.message.bytes[..usize::from(point.message.length)]);
        }
        state
    }
}

impl State {
    /// Restore zones, ranges and controls before chased notes.
    /// Takes a bounded wire sink; emits complete RPN transactions and the current expression without inventing note events.
    pub(crate) fn setup(&self, mut emit: impl FnMut([u8; 3], u8)) {
        for (manager, members) in [(0, self.lower), (15, self.upper)] {
            for bytes in [
                [0xb0 | manager, 101, 0],
                [0xb0 | manager, 100, 6],
                [0xb0 | manager, 6, members],
            ] {
                emit(bytes, 3);
            }
        }
        for channel in 0..16u8 {
            let current = self.channels[usize::from(channel)];
            if self.role(channel).is_some() {
                for bytes in [
                    [0xb0 | channel, 101, 0],
                    [0xb0 | channel, 100, 0],
                    [0xb0 | channel, 6, current.range],
                    [
                        0xe0 | channel,
                        (current.bend & 127) as u8,
                        (current.bend >> 7) as u8,
                    ],
                    [0xb0 | channel, 74, current.timbre],
                    [0xb0 | channel, 64, if current.sustain { 127 } else { 0 }],
                ] {
                    emit(bytes, 3);
                }
                emit([0xd0 | channel, current.pressure, 0], 2);
            }
            emit([0xb0 | channel, 101, current.rpn[0]], 3);
            emit([0xb0 | channel, 100, current.rpn[1]], 3);
        }
    }
}
