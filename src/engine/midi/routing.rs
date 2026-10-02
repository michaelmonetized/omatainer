//! Explicit MIDI device/port/channel routing, independently of GUI selection.
mod control;
mod model;
mod output;
pub(crate) mod playback;
pub(crate) use control::Owner;
pub(crate) use control::Shared;
pub(crate) use control::Sources;
pub(crate) use control::Summary;
pub(crate) use output::Manager;
pub use output::Status;
pub(crate) mod packet;
pub use model::{Endpoint, Filter, Input, Route, Routing};

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use output::tests::install_for_test;
