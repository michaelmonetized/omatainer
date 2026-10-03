# Scalable, high-contrast layouts: issue 118

Source freeze: `350f14e15cdf7d45d94f115ae8f84941ae8635bb`.

Preferences now save Theme, Dark or Light contrast, reduced motion, waveform
contrast and signal-activity contrast alongside the existing UI scale and font
size. Version-8 storage migrates older profiles without changing audio, MIDI or
startup choices. Preview / Apply, reopening, Cancel and external-file conflict
paths run through the actual Preferences widgets and worker.

Text has an 11-pixel physical floor; custom and standard controls have a
24-pixel target floor. Crate row virtualization uses the enlarged row stride.
Deck, sampler, sequencer and MIDI-editor geometry follows those sizes. Oversized
layouts scroll, keyboard focus scrolls immediately into view, and small logical
viewports combine the existing Safety, File, Setup and FX controls into scrollable
menus. Cue numbers remain visible at large fonts; complete custom cue names stay
in native help and accessibility labels.

Checked outlines, text and symbols distinguish enabled controls, held pads,
selection, mute/solo, queued clips and playing clips. A stopped transport no
longer carries the new playing symbol merely because a scene remains assigned.
Warning text uses the selected palette, including pending-audio and worker
failure notices. Reduced motion removes platter decoration and animated
scrolling while keeping transport, position, waveform and signal activity live.
Waveform and activity settings change paint contrast, preserving signal geometry
and audio. The activity strip uses the existing smoothed deck meter; it is not a
calibrated peak, loudness or clipping measurement.

## Qualification

Linux aarch64, Rust/Cargo 1.98.0, locked local builds.

- All **1,275 ordinary tests pass**: 1,274 in the serial suite, zero failures,
  29 opt-in ignored, 266.19 seconds; the exhaustive scene-index check passes
  separately in 256.06 seconds. Independent processes use the same immutable
  executable on separate CPUs.
- **128 optimized checks pass**: display geometry/paint/audio 4, theme 2,
  Preferences GUI 17, storage 9, accessibility 16, clip gain 3, compose 2,
  play-time tooltips 6, piano roll 7, virtualized crates 5, project GUI 35,
  template GUI 7, template engine 2, timing GUI 5, dependency GUI 5 and
  portable GUI 3.
- Actual App / AccessKit fixtures exercise 1280×720 at scale 0.5/font 8,
  1366×768 at 1.25/18, 3840×2160 at 2/32 and 1280×720 at 3/48. Focused
  Deck B pitch bounds remain inside each physical reference viewport. Custom
  button targets and primary cue text fit their bounds. Theme floors also pass
  all combinations of seven supported scale samples and five font samples.
- Dark/Light text colors exceed 4.5:1 against all five palette surfaces.
  Marker fallback exceeds 3:1. The same text checks pass severity-1 protan,
  deutan and tritan simulations using the published
  [Colour Machado dataset](https://github.com/colour-science/colour/blob/develop/colour/blindness/datasets/machado2010.py).
  The ratios and target size use W3C's
  [text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html),
  [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)
  and [target size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html)
  as engineering references. These fixtures do not establish human color-vision
  usability or WCAG certification.
- The display comparison renders 16 blocks of 512 real, nonzero audio samples
  with two native fixture renderers and confirms bit-for-bit equal output.
  Waveform and activity geometry remain equal while their colors change.
- Standard gate: eight workloads, three repeats, zero callback allocations and
  frees. Original audio hashes remain producer `26f84beea80f5ec5`, composer
  `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`, hybrid `adc9540dab057fed`.
  Policy is unchanged. The controlled run is pinned to CPU 6, after ordinary and
  optimized checks finish. Independent checking of the preserved production
  executable passes before and after the validation commit.
- Native AT-SPI API preflight: 158 actions, 255 visited nodes, 573 App/renderer
  frames. Eight license/package fixtures pass. Inventory retains 471
  source/build/gate files and 330 license entries.

The first complete attempt found a focus-scrolling regression, two pointer
fixtures that searched the old clip labels and a tooltip fixture whose second
row lay below the enlarged row height. Immediate focus scrolling fixes the
actual reachability failure; label and viewport fixtures now match the shipped
geometry. A painted warning test also covers the corrected light palette. The
complete final run above passes; the first failed attempt remains in local logs.

## Artifact bindings

Preserved in `/home/michael/Projects/omatainer-work/issue-118-qualified-release`:

- Production: `0c5d8399e73ef59bf6f114bab7bc4f11edc01e86add0d3f1daeb47d1664f58f9`
- Release tests: `5420acc817840a6a03877920dbf900083255ec3e65713b9bc80cb7c6ea7a747b`
- License manifest: `9fb05cdf879c775594e923cc87b980e5281053fd25857b0719cfabfce40f9be7`
- Ordinary tests: `73195d42aade10e06c7dbf1b74e24a8cdd5c52af8d98a1474689f35165606c2c`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Local receipts share `/home/michael/Projects/omatainer-work/issue-118-`:
`qualified-ordinary-tests.json`, `qualified-suite.log`,
`qualified-scene-boundary.log`, `release-focused-final.json`,
`release-focused-final.log`, `performance.json`, `performance.raw.json`,
`performance.log` and `license-tests-qualified.log`.
The controlled gate ran from `2026-10-03T02:06:27.188336Z` to
`2026-10-03T02:09:17.417384Z` (October 2 local time).

This qualifies the actual App/renderer, native accessibility API and local
persistence/worker paths. No desktop window, Orca, physical display, human
color-vision study or physical audio/MIDI device QA is claimed. The inherited
issue-107 supplemental quiet-host wall limit remains a separate unresolved
requirement; this standard gate does not replace it.
