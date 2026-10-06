//! Prepared channel routing shared by native capture, rendering and project state.
pub(crate) mod input;
pub(crate) mod model;
pub(crate) mod mic_aux;
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod ns7_native_tests;
pub(crate) mod prepared;
pub(crate) mod probe;
pub(crate) mod record;
mod render;
#[cfg(test)]
mod tests;
