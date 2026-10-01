//! One identity for a sampler bank slot and its piano position.
//! Bottom-row identities are 0..8; top-row identities are 8..16. Mode changes
//! change the label/source, never the identity retained by a held pointer gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PadIdentity(u8);

impl PadIdentity {
    pub fn new(index: u8) -> Self {
        assert!(index < 16, "sampler pad index must be in 0..16");
        Self(index)
    }
    pub fn index(self) -> usize {
        self.0 as usize
    }
    pub fn number(self) -> u8 {
        self.0 + 1
    }
    pub fn sample_label(self) -> &'static str {
        const LABELS: [&str; 16] = [
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        ];
        LABELS[self.index()]
    }
    pub fn piano_label(self) -> &'static str {
        const LABELS: [&str; 16] = [
            "A", "B", "C", "D", "E", "F", "G", "A", "A#", "", "C#", "D#", "", "F#", "G#", "",
        ];
        LABELS[self.index()]
    }
    pub fn midi_note(self, octave: i8) -> u8 {
        // Disabled accidental gaps retain the historical natural pitch for
        // internal commands; the GUI never sends a new gate from those cells.
        const OFFSETS: [i32; 16] = [0, 2, 3, 5, 7, 8, 10, 12, 1, 2, 4, 6, 7, 9, 11, 12];
        (21 + octave as i32 * 12 + OFFSETS[self.index()]).clamp(0, 127) as u8
    }
}
