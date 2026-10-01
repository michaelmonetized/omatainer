# Issue 79 — selected installed font and safe glyph fallback

Issue 78 supplies the common background resolution/loading/reinstallation path
needed for font-only reloads. This layer completes and verifies its selected-font
contract, fixes a demonstrated missing-symbol fallback, and documents unavailable
font behavior. It does not introduce a second font discovery implementation.

The selected font is the installed face returned by `fc-match monospace`, using
its actual file bytes and face index. It takes precedence in both proportional
and monospace egui families. Discovery, bounded file reading and validation run
on the theme worker, with the time/byte limits documented for issue 78. Creating
bundled startup definitions does no subprocess or filesystem work.

The new actual UI tests initially reproduced missing `⇄` (the deck match button)
when Ubuntu was selected. egui's default proportional chain lacks Hack, although
Hack contains that symbol. Both family chains now append missing bundled
fallbacks without reordering their existing entries. The chosen font remains
first, so ordinary text metrics still follow the selected face. The complete
fallback is also installed at application startup before asynchronous discovery.

Fallback policy:

- If fontconfig substitutes an installed face for a missing configured family,
  the application uses that resolved face and records its actual family name.
- Before any selection has loaded successfully, bundled Ubuntu, Hack and emoji
  fonts remain usable in both text families.
- A missing, malformed, unsupported/oversized font file, invalid face index,
  fontconfig error, or resolution timeout retains the last valid loaded font.
  Later checks recover automatically; shell size changes can still be applied.
- Bundled fallback preserves supported transport symbols and emoji. It does not
  claim universal Unicode or font-format coverage.

Three actual egui fixture groups exercise the production background loader and
application adoption path:

1. Real fontconfig uses only a private font directory, private configuration and
   cache, containing controlled Hack and Ubuntu files. Switching between these
   two recognized installed faces changes the application's actual font bytes
   and rendered glyph widths in both families. Hack's i/W widths are equal;
   Ubuntu's differ, demonstrating real face selection rather than a label edit.
2. Required toolbar symbols `↻ ⇄ → ↓ ↑ ½ × … —` and the music emoji `🎵` remain
   renderable. Glyphs absent from the chosen face are checked against an isolated
   chosen-face-only egui context, where they are unavailable, proving that the
   combined context uses an actual fallback. Every original bundled fallback
   entry remains in its established order.
3. Cold-start missing-file behavior uses bundled defaults; a later missing
   selection retains the preceding installed font while size updates proceed;
   restoration switches to the new font. A configured nonexistent family through
   real fontconfig uses the installed substitute from the private fixture.

No user font, fontconfig file, desktop setting, live socket or installed binary
was changed. These are local fontconfig/egui fixtures, not a physical desktop
session claim.

Results: the three new font-selection groups pass, `cargo test` passes all 315
tests, and `cargo build` passes. Peer source review found no remaining blocker.
