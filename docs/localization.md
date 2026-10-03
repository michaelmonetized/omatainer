# Interface languages and international text

Preferences → Appearance → Interface language offers English, Español and
Deutsch. Preview does not change the live language. Apply saves the selected
profile; its language appears on the next frame and after reopening. Cancel
keeps the applied language. Older profiles migrate to English in memory; Apply
writes preferences version 10. Unknown languages are refused without rewriting
the saved file. A language change does not alter tempo, notes, routing or gain.

The external UTF-8 catalogs in `locales/` use stable English message keys.
`en.json` holds complete templates for the cataloged UI and shared help definitions;
Spanish and German cover the principal project, preference, performance and
navigation controls. Untranslated advanced controls, help prose, technical
worker errors and the offline manual fall back to English. These catalogs are
embedded in the native binary and included in its source/license/performance
binding. They require no account, network or runtime translation service.

Formatted translations can reorder numbered placeholders but must retain every
value once. Parameter values such as names and paths retain their original
spelling. Display decimals use comma in Spanish/German; editable display values
accept comma or point and keep finite numbers. Transport/musical values and
native/JSON/OSC/MIDI file formats keep their existing numeric representation.
UTC history/version dates follow the selected date order; saved timestamps stay
absolute. Window IDs stay independent of their translated title. Preferences
use wrapping rows, scrolling and expandable windows for longer text.

Search keys use full locale-independent Unicode case folding followed by NFC.
For example, `CAFÉ` finds `Café` and `STRASSE` finds `Straße`. Search and ordering
keys are temporary: source paths, project names, tags and notes are never
normalized or transliterated on disk. Existing tag validation remains compatible.
The normalization and folding follow the pinned
[ICU4X normalization](https://docs.rs/icu_normalizer/2.3.0/icu_normalizer/struct.ComposingNormalizer.html)
and [case mapping](https://docs.rs/icu_casemap/2.3.0/icu_casemap/struct.CaseMapperBorrowed.html)
APIs.

This release retains left-to-right controls. The pinned egui 0.32 renderer does
not promise complete bidirectional ordering, Arabic shaping or cursor behavior
for mixed-direction text; [upstream tracks bidirectional support](https://github.com/emilk/egui/issues/1016).
Unicode codepoints and direction marks round-trip unchanged. A matching installed
font is needed for scripts outside the bundled glyph coverage; Font setup uses
the existing desktop font selection. English, Spanish and German layout tests
cover their bundled glyphs. Native project fixtures use non-Latin, combining and
mixed-script names under a removable-drive-style directory. Physical USB media,
input methods and a right-to-left OS window remain separate unverified paths.
