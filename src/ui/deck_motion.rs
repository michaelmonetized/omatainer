use crate::engine::DeckSnap;
use std::time::Instant;

/// Present motion between completed renderer snapshots.
/// Takes a captured deck and monotonic display time; updates its display copy only, with a bounded prediction.
pub(super) fn present(deck: &mut DeckSnap, now: Instant) {
    let Some(at) = deck.captured_at else { return; };
    let elapsed = now.saturating_duration_since(at).as_secs_f64().min(0.025);
    if let Some((turns, rate)) = &mut deck.platter { *turns += elapsed * f64::from(*rate); }
    if !(deck.playing || deck.platter.is_some_and(|p| p.1.abs() > 0.001)) || deck.touching || deck.source_sample_rate == 0 { return; }
    deck.pos += elapsed * f64::from(deck.playback_rate) * f64::from(deck.source_sample_rate);
    if deck.loop_on && deck.loop_len > 1.0 && deck.pos >= deck.loop_start + deck.loop_len {
        deck.pos = deck.loop_start + (deck.pos - deck.loop_start).rem_euclid(deck.loop_len);
    }
    deck.pos = deck.pos.clamp(0.0, deck.frames.max(0.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn motion_advances_between_snapshots_but_stale_and_stopped_positions_are_bounded() {
        let at = Instant::now();
        let mut deck = DeckSnap { captured_at: Some(at), pos: 48000.0, frames: 480000.0, source_sample_rate: 48000, playing: true, playback_rate: 1.0, platter: Some((0.25, 0.5)), ..Default::default() };
        present(&mut deck, at + Duration::from_millis(10));
        assert_eq!(deck.pos, 48480.0); assert_eq!(deck.platter.unwrap().0, 0.255);
        present(&mut deck, at + Duration::from_secs(4));
        assert_eq!(deck.pos, 49680.0);
        deck.playing = false; deck.platter.as_mut().unwrap().1 = 0.0;
        present(&mut deck, at + Duration::from_secs(5));
        assert_eq!(deck.pos, 49680.0);
    }
}
