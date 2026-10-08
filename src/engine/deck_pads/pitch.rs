/// Resolve one chromatic cue pad's equal-tempered offset.
/// Takes the selected low/center/high range and fixed zero-based pad; returns a supported offset containing original key in every range.
pub(crate) fn semitones(range: u8, pad: u8) -> Option<i8> {
    let first = [-6, -3, -1].get(usize::from(range))?;
    (pad < 8).then_some(first + pad as i8)
}

#[cfg(test)]
mod tests;
