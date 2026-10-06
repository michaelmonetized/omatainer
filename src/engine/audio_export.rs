use super::*;
impl RtEngine {
    /// Release export sources while keeping their native decay.
    /// Takes the independently owned renderer; stops clip/deck excitation without clearing effect or one-shot histories.
    pub(crate) fn stop_export_sources(&mut self) {
        self.apply(Command::Stop);
        for deck in &mut self.decks {
            deck.playing = false;
            deck.release_performance_controls();
            deck.fade_from_last_output(self.sr);
        }
    }
}
