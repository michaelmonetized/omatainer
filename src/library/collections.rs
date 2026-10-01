//! Catalog integration keeps every membership tied to an existing stable track.
use super::*;

impl Catalog {
    pub(crate) fn edit_crates(
        &mut self,
        expected: u64,
        edit: &crates::Edit<TrackId>,
    ) -> Result<bool, String> {
        self.validate()?;
        let known: HashSet<_> = self.tracks.iter().map(|track| &track.id).collect();
        self.crates
            .apply(expected, edit, |id| known.contains(id))
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
