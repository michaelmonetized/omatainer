# Omatainer control contract

Every labeled control must change audio or project state the way the label says.
Handlers that only flip a light, a string, or `return y` are bugs.

These clauses are enforced by `cargo test` (`engine::tests::contract_*`):

| Clause | Test |
| --- | --- |
| C1 Pitch lock | `contract_pitch_lock_preserves_pitch` |
| C2 Match | `contract_match_follows_favored_bpm` |
| C3 Banks | `contract_banks_sixteen_distinct` |
| C4 Held pads / octave | `contract_held_pads_and_octave` |
| C5 Compose | `contract_pad_writes_empty_clip` |
| C6 Spread / balance | `contract_spread_and_balance_are_stereo` |
| C6 EQ 3/5/8 | `contract_eq_bands_differ` |
| C6 Arp | `contract_arp_steps_chord` |
| C7 Builtin crate | `contract_builtin_reload` |

## C1. Pitch lock (L)

- **Off:** pitch fader changes tempo and pitch together (resample).
- **On:** pitch fader changes tempo only. Played pitch stays that of the file
  (OLA grains at 1×, playhead advances at the fader rate).
- Toggling L is audible at a non-center fader, not just a LED.

## C2. Match (⇄)

- Favored deck = xfader ≤ 0.5 → A, else B.
- Unfavored deck’s playback rate is set so its **effective BPM** equals the
  favored deck’s **pitched** BPM (`file_bpm * pitch_rate`), not session BPM.
- If both decks are playing, only the unfavored playhead is phase-aligned.
  Favored time does not jump.
- Works whenever the favored deck has audio; the unfavored deck can be stopped
  (rate is armed for when it plays).

## C3. Sample banks (Kit / Perc / Hits)

- Three banks, **16 samples each**.
- Switching banks changes which buffer pad N triggers.
- Visible sample pad N dispatches zero-based bank slot N−1. Bottom-row pads are
  1–8 (identities 0–7); top-row pads are 9–16 (identities 8–15). Labels, tooltips,
  and sampler commands share those identities. Piano mode preserves the lower
  natural notes and upper accidental positions; its three blank cells are inert.
- Pointer down captures that cell's identity and pointer up releases the same
  identity, including outside the cell or after a mode switch makes it blank.
  Mode changes while the pointer is held never create a new gate. A rejected
  press does not create a held gate or an unmatched release.
- Samples in a bank are not all the same buffer.

## C4. Held pads + instrument + octave

- Pointer **down** = note/sample on, **up** = off. No click-on-then-off.
- `samples`: pads fire the current bank (one-shots may ring after release).
- `analog` / `keys` / `pad`: pads hold their distinct named synth voices until
  release. Drums are available through the sample banks; there is no drums synth
  choice until a distinct implementation exists.
- Changing the instrument affects new pad presses. Held voices keep their
  original sound, envelope and mixer destination until their matching release.
- Octave ^/v transposes held instrument notes by 12 semitones, and sample
  playback rate by `2^(oct-3)`.

## C5. Compose (shift-click cell)

- Shift-click a sequencer cell explicitly arms it as the compose target. The
  sampler also has Arm selected cell and Disarm compose controls, and displays
  the renderer-confirmed armed destination or disarmed state.
- If empty, the armed target becomes a MIDI clip. Browsing and playback/scene
  launches cannot change that destination; only another explicit arm can.
- Pads monitor and write into that destination even when not recording. Held
  pad voices and capture releases retain their original destination.
- Disarm finalizes current pad captures without cutting held monitor voices.
  Stop (including transport toggle-off) disarms composition. Restarting cannot
  re-arm it. Disarmed pads do not edit clips unless Record is deliberately on.
- Plain selection only browses; it neither creates a clip nor arms composition.

## C6. FX: spread, balance, arp, EQ 3/5/8

- **Spread:** stereo width (Haas). L and R differ when spread ≠ noon.
- **Balance:** left/right mix.
- **Arp:** on a MIDI clip, overlapping notes are stepped as 16ths instead of
  held as a chord.
- **EQ3 / EQ5 / EQ8:** different crossover counts (3 / 5 / 8 bands).
- Chain sliders expose only implemented controls, with names, units, ranges,
  current values and explanations. Reverb and Chorus expose wet mix only;
  Delay exposes wet mix and feedback, with time explicitly fixed at 250 ms.
- Arp is an upstream MIDI event effect with on/off only. It has no wet mix or
  generic parameters and cannot be added to a scene's audio bus.

## C7. Builtin crate

- “Drums (session)” / “Harmony (session)” load that stem onto the chosen deck
  (→A / →B / double-click), including when it is already on a deck.

## Clip stop and voice tails

- Stopping or replacing a clip releases that clip's synth notes, including its
  arpeggiator. This also applies to scene/transport stop, single-shot completion,
  and replacing the note list of a playing clip.
- Synth voices finish their envelope release instead of being cut off. Finite
  drum one-shots and effect tails may finish naturally; no stopped clip may keep
  a synth voice held indefinitely.
- Live notes are separate from clip notes, including at the same pitch. Clip
  note-offs and stops preserve live notes and unaffected tracks; live note-offs
  preserve clip notes.

## Mute and solo

- Mute and exclusion by solo silence a track's output. Its note scheduling,
  synth envelopes, one-shot sample positions and FX histories keep advancing.
- A muted drum hit finishes naturally in silence; unmuting never resumes a
  frozen hit. Notes whose release occurred while muted cannot remain held.
- Unmuting restores the sound at the current musical position, including any
  still-current natural effect tail. It does not replay elapsed material.

## Quantized clip launches

- A queued clip starts on the next selected beat grid while transport runs;
  launching from stopped transport starts immediately. Every clip in one scene
  launch shares the same scheduled start.
- Before that start, the incoming clip emits no events. The sequencer shows it
  as queued with zero progress. Existing release envelopes, finite drum hits,
  effect tails, and unrelated live input retain the stop/tail policy above.
- The first event fires once, and a one-shot gets its complete clip length from
  that scheduled start. Loops and retriggers use the same sample boundary rule.
- The transport beat stored during rendering is the end of the current sample
  interval: an event exactly at that endpoint belongs to the following sample.
  A tiny beat tolerance absorbs clock accumulation error at exact grids.

## Library scanning

- Startup presents the built-in crate immediately. Filesystem discovery,
  metadata reconciliation, and sorting run on one background worker.
- Scan shows entry/file progress and an explicit completion or failure state.
  Cancel keeps the existing crate; another scan starts only after the current
  worker has acknowledged cancellation. Filesystem calls already in progress
  finish before cancellation can be observed.
- A completed crate replaces the old one atomically. Selection follows the
  same typed media source, valid cached metadata survives unchanged files, and
  play-history edits made during a scan remain visible. A scan error preserves
  the existing crate and selection rather than exposing a partial result.

## Pad monitoring and recorded playback

- Each pad press captures one destination track. The selected sampler instrument
  or sample bank supplies one source voice, which passes through that track's
  stereo EQ/FX, gain, pan, mute, and solo gate. Selection changes do not move a
  held voice or its release/one-shot tail to another track.
- Recording/composition writes the visible note immediately while the physical
  input supplies monitoring. That new note does not create a second clip voice
  in its initial pass or while the physical gate remains held, even after Record
  is switched off.
- After release, a captured note joins a later loop at its next eligible onset;
  a one-shot capture plays on a subsequent launch. Stored pitch, velocity, start,
  and duration remain unchanged. Other clip notes, including the same pitch,
  continue playing normally. Explicit note-list replacement or relaunch starts
  normal playback of the resulting clip.

## Scene FX ownership and routing

- Each scene owns its device settings and processor histories. Opening another
  scene panel changes only the edit target, never the audio route.
- A track enters its clip's scene bus when that clip actually starts. Pending
  launches keep the previous bus; stopped tracks retain their last bus for
  release envelopes, live input and track FX tails. New sessions use scene 1.
- Added scenes can play on different tracks at once. Each scene processes its
  own stereo sum, and those outputs are combined before decks and master FX.
- Starting another clip moves that track's complete output to the new scene,
  including any remaining track-level release tail. The old scene FX history
  stays on its original bus and continues receiving zero if no track remains.
  It decays according to its own feedback settings; changing panels/scenes
  never copies or clears it. Device bypass fades to dry, then freezes that slot's
  history according to the FX bypass policy below.

## Typed local control requests

Requests are JSON objects. `op` is required and case-sensitive; `id` is optional
(null, signed/unsigned 64-bit integer, or UTF-8 string of at most 128 bytes).
No operation accepts undeclared fields.

| Wire opcode | Required arguments beyond `op` | Meaning |
| --- | --- | --- |
| `ping`, `status` | none | Read current status |
| `follow` | none | Subscribe to bounded status frames |
| `play`, `stop`, `togglePlay`, `record`, `tap` | none | Existing transport/record/tap action |
| `scene` | `n`: JSON integer 0–7 | Launch that zero-based scene |
| `deckPlay`, `deckCue` | `deck`: JSON integer 0–1 | Act on exactly deck A or B |

Missing, null, boolean, string, fractional, negative and out-of-range target
values fail before command admission. Errors name `n` or `deck`; no default,
clamp, truncation or modulo selects another target. Unknown operations/fields
fail before mutation. CLI scene numbers remain one-based 1–8 and are converted
once before encoding the request.

## Local control connection limits

- A request may contain at most 4096 bytes before its newline. Idle reads expire
  after 500 ms, and each complete line has a 2-second total read budget.
- At most eight clients are handled concurrently; excess connections receive a
  bounded `server_busy` rejection where possible. An ordinary connection closes
  after 32 requests. Responses are at most 8192 bytes before their newline and
  have a 200 ms total write budget.
- Diagnostics do not echo request bodies. Status metadata is capped, with
  `state_truncated` indicating omitted text or device entries. If a snapshot is
  unavailable after 50 ms, an accepted command still receives its receipt;
  a status query receives a temporary-unavailability rejection.
- Violations and disconnects release the client slot. Server shutdown interrupts
  and joins client workers before removing its owned endpoint.

## Media load replacement and cancellation

- Each deck's new file selection receives a unique increasing request identity.
  Unload and built-in replacement invalidate older pending, active, completed
  and already-queued file results. Obsolete errors cannot replace newer status.
- One decoder worker has at most one active job and one pending/result slot per
  deck. Pending work coalesces to the latest selection; decks dispatch fairly.
- Token validity is checked at decoder boundaries, UI publication and renderer
  application. Cancellation does not interrupt a filesystem syscall in progress;
  it takes effect at the next safe boundary without blocking UI teardown.

## Controller library loads

- Controller load buttons capture the GUI-published typed selection and target
  deck when the MIDI dispatch worker admits the request. Browsing or rescanning
  afterward does not substitute another source or move the GUI selection.
- A separate bounded queue holds 16 library requests; the GUI dispatches at most
  eight per frame through the same built-in/file loading path as crate buttons.
  Raw MIDI callbacks and the audio renderer never decode media or resolve paths.
- Empty selections, unavailable GUI receivers and full request queues report
  visible failures. An uncaptured legacy renderer request fails explicitly;
  it cannot infer a later GUI selection. File loading uses the existing pending,
  result and diagnostic status, with request-token cancellation rules above.

## Crate browsing

- Crate rows use a fixed-height virtual viewport. Only visible rows and egui's
  bounded overscan are painted or formatted; unchanged visible cells are reused.
- Filtered indices follow the scan worker's sorted crate and rebuild only when
  the query or immutable library metadata changes. Controller selection uses
  the same index cache. Play-history changes refresh only visible history cells.
- Selection follows its typed media source across filtering and publication;
  the scroll anchor follows its source across reordered scan results. If a
  selected source disappears, selection clamps to a valid neighboring row.
- Clicking a row focuses the crate. Up/Down, Page Up/Down and Home/End move and
  reveal selection while that list has focus. Search-field arrows edit text.

## Clip gain

- Supported MIDI, arpeggiated and drum clips capture clip gain independently at
  each note/hit onset, before track EQ/FX and track gain. Zero silences that
  clip source; unity keeps its original level. Live input and pads use unity
  clip gain and remain subject to their ordinary track controls.
- Gain edits apply to new notes/hits. Held voices, finite drum hits, release
  envelopes and existing downstream effect tails retain their captured level,
  including after a different clip launches. Track gain still scales the whole
  track output.
- Alt-click an occupied clip to edit its gain from zero to 1.5, with explicit
  zero/unity controls. Snapshots and serialized clips expose the same value;
  nonfinite edits are rejected. Audio-clip playback remains a separate feature.

## Deck filter sweep

- Center (`0.47` through `0.53`) is transparent. Leftward motion monotonically
  increases low-pass attenuation; rightward motion independently increases
  high-pass attenuation. Each channel has independent filter history.
- The active response meets bypass continuously, with a bounded 5 ms control
  slew for full-range jumps. Returning to center clears old branch history.

## Drum velocity

- Live MIDI and MIDI-clip drum hits capture normalized velocity independently
  from clip gain. Overlapping hits retain both values until their one-shots end.
- Velocity-zero note-on follows the release path and never creates or steals a
  drum voice. Existing finite drum hits continue under the usual release policy.
- Arpeggiated drum steps use the largest velocity among active, visible notes
  of the selected pitch. Membership boundaries, edits and loops refresh that
  value; a zero-velocity step is silent. Synth arp velocity is unchanged.

## Neutral FX and bypass

- Empty racks, noon Spread/Balance, flat EQ, and the audio side of Arp pass each
  channel through exactly. Common dry/wet processing returns exact dry samples
  at zero mix; enabled processor histories continue advancing at zero mix.
- A slot's initial enabled/disabled state takes effect immediately. Subsequent
  bypass changes crossfade processed and dry audio linearly for 5 ms at the
  output sample rate. Reversing a change continues from the current fade level.
- Processor history advances during the audible fade, freezes once fully dry,
  and resumes during the enable fade. Flat EQ keeps history warm; neutral Spread
  never activates a delay tap. Sample-rate changes rebuild history and restart
  at the slot's configured enabled/disabled state.

## Time-effect slot mix

- Delay, Reverb and Chorus produce fully wet processor output. The slot applies
  one linear dry/wet interpolation: dry × (1 − mix) + wet × mix.
- Zero mix emits dry input while processor history keeps advancing; full mix
  emits only the time-effect signal. Intermediate mix changes do not scale the
  stored feedback history or apply the wet coefficient a second time.

## Metronome timing

- Each transport beat starts a 20 ms click with a 1 ms attack and a decay to
  zero. Beats divisible by four use 1200 Hz at amplitude 0.20; other beats use
  800 Hz at amplitude 0.12, before existing master/cue processing.
- Beat scheduling uses half-open sample intervals. Oscillator phase and envelope
  belong to the click voice, so callback block sizes cannot change the sound.
- Disabling the metronome or stopping clears its voice immediately. Enabling or
  resuming between beats waits for the next boundary; starting on a boundary
  starts that beat's click. A sample-rate change clears and rebuilds the voice.

## Mixer control gains

- Crossfader and track pan/gain coefficients are cached at audio-block boundaries.
  Unchanged controls perform no per-sample powers or square roots. The original
  crossfader law remains `A=(1-x)^(1+2.5c)`, `B=x^(1+2.5c)` for position `x` and
  curve `c`; the existing square-root pan law is unchanged at steady settings.
- After the first rendered frame, crossfader, curve, track gain and pan changes
  ramp from the currently audible gain pair to the exact new pair over 5 ms
  (rounded to the nearest sample). A reversal starts from the current pair.
  These are linear ramps in gain space; steady curve endpoints remain exact.
- Muted/solo-excluded tracks still advance ramps. The stopped sample-rate reset
  establishes current target gains directly at the next callback. Initial
  controls also start at their exact target without an unnecessary fade-in.

## Spread and Balance slot mix

- Every Spread and Balance instance processes its own position in the serial
  stereo chain. Its mix applies once: dry × (1 − mix) + full effect × mix.
  Zero mix is exact stereo identity; full mix keeps the existing effect law.
- Spread above noon delays the right channel; below noon it blends toward the
  delayed stereo midpoint. Noon is exact identity and does not advance a delay
  tap. Balance keeps the existing square-root attenuation of the opposite side.
- Nonneutral enabled Spread history advances even at zero mix, so increasing
  mix exposes its current history. Per-slot bypass retains the 5 ms fade/freeze
  policy; each duplicate instance owns its history and mix independently.

## Three master FX controls

- Each of the three legacy master FX Wet controls addresses its own supported
  stereo slot. Initial types are Echo, Reverb and Filter, all at zero wet.
- The third control (slot index 2) initially blends a two-pole 1 kHz low-pass:
  minimum is dry audio, maximum is the filtered signal on both channels.
  Its existing NS7FX channel-1 CC `0x32` mapping has those same endpoints.
- Select cycles Echo → Reverb → Filter in that slot. Snapshot type and wet
  values describe the actual processor and mix, including the third slot.
- Every factory FX binding must resolve to a supported slot and observable
  renderer state; adding a controller label cannot silently create an inert FX.

## Persistent status followers

- `ctl status` and `ctl follow` use the same typed, bounded JSON request encoder
  and fresh process/counter correlation IDs. Follow opens one read-only `follow`
  subscription rather than sending bare text or repeating ordinary status polls.
  Its initial request
  retains the 4096-byte, 500 ms idle and 2-second total read limits. It then
  receives at most four current-state frames per second; no updates are queued
  and later bytes on that connection cannot submit commands.
- A subscription consumes one of the existing eight tracked client slots. Its
  lifetime is deliberately independent of the ordinary 32-request cap. Each
  frame retains the 8192-byte and 200 ms write bounds; a failed write retires the
  worker. Shutdown closes sockets and joins workers, including subscriptions.
- Follow frames carry the subscription ID and null `accepted`/`command_status`:
  they describe published state, never command admission or execution. Snapshot
  contention reports temporary unavailability without replacing the connection.
- The server compares only bounded display metadata and scalar telemetry. It
  reuses the encoded frame while those fields are unchanged; tracks, clips and
  waveform data are neither copied nor serialized for status. Changed metrics
  still produce a fresh bounded frame.
- The CLI has a 2-second initial request write and per-frame read budget,
  nonblocking connection attempts, and reconnect delays of 250 ms, 500 ms, 1 s,
  2 s, then at most 4 s.
  A valid frame resets backoff. Closed output exits successfully; process exit
  closes the socket. A reconnect never retries a musical control command.
- Client follow failures retain the state-unavailable JSON shape and include
  `error_code`: malformed JSON/UTF-8, mismatched IDs, invalid state envelopes or
  oversized frames are `protocol_error`; absent/refused sockets are
  `not_running`; disconnects and other I/O failures are `transport_error`.
  Valid server state errors such as `snapshot_unavailable` pass through with
  their original code and keep the connection.
## Shell scene arguments

- `Service.scene(n)` and public shell IPC `omatainer.scene` use one-based scene
  numbers 1 through 8, matching `omatainer ctl scene <n>`. The CLI alone converts
  them to the protocol's zero-based `n` and engine scene indexes 0 through 7.
- Every queued shell scene command captures its own separate CLI argument.
  Its receipt and final result retain that argument, even when several different
  scenes are waiting. Command acceptance still does not mean audio application.
- Invalid scene inputs return an explicit rejected result without queuing or
  starting a control process. Fractions, nonnumeric strings, booleans, missing
  values and out-of-range numbers never collapse to scene 1.
## Crate duration

- Files start with an explicitly unknown duration. Once a decoded file becomes
  the current deck load, the crate records its playable frame count divided by
  its own sample rate, independently of mono/stereo channel count or output rate.
- Duration belongs to the source path and matching file fingerprint. Unchanged
  rescans preserve it; replaced or changed files return to unknown until a valid
  result for those bytes becomes current. Rejected, cancelled and stale loads
  cannot update another row or supply an old file's duration.
- The existing metadata worker merges duration and publishes the crate atomically.
  Selection follows its source and visible cells refresh. Unknown displays as
  `unknown`; a measured zero is distinct and displays as `0:00`. Display rounds
  down to whole seconds while the cached value retains fractional seconds.
## Play history

- A deck earns one last-play update when its current, successfully loaded source
  first renders a valid frame while Play is active. A rendered pause or EOF ends
  that episode; resume earns a new update. Continuous playback and loop wraps do
  not repeatedly update it. Initial built-in decks follow the same rule.
- This records source playback before deck gain and crossfader routing: valid
  silence and a deck mixed out still count. Paused scratching, retained transition
  tails, browsing, load admission, loaded-but-paused media, failed loads and
  cancelled unapplied requests do not count.
- The renderer stamps the event on the load's application receipt. Dismissing its
  status or replacing/unloading the deck before a GUI poll cannot lose a playback
  that already occurred. Filtering and later selections cannot retarget history.
- File history belongs to a typed pathname plus the verified decode fingerprint;
  replacement content at the same path and moved paths do not inherit it. Files
  whose identity changed during decoding receive no unverified path-only credit.
  Built-in stems have stable typed identity. History uses an in-session overlay until the DJ library worker commits it;
  unchanged identities can reappear after a scan or restart without losing their timestamp.

## Crate history across scans

- Unchanged files retain their existing history only when both the typed source
  path and verified filesystem fingerprint match. Built-in stems use their
  stable typed identity. Unknown or changed file identity never inherits a
  timestamp solely because its pathname matches an old row.
- Session-only crates omit removed files. A persistent DJ catalog retains its
  imported locations across scans; unavailable local files fail visibly on load.
  Renderer-confirmed playback history is keyed by source and fingerprint, so an
  unchanged file temporarily excluded from scan roots can recover its history.
  A renamed/moved path is a new identity; no move discovery or history transfer
  is inferred. Replacing bytes at the same path is also a new identity.
- Selection follows the same source while it remains in the filtered crate;
  if it disappears, selection clamps to a valid row. History arriving during a
  scan remains overlaid on the published crate. Cross-restart history follows the durable DJ library contract below.

## MIDI connection lifecycle

- Device rows distinguish discovered, connecting, connected, failed (with a
  reason), and disconnected. Only a successful backend connection can publish
  connected; completion cannot erase a failed attempt or revive an old one.
- Keyboard and mouse remain available regardless of hardware state. Ctrl+M
  exposes shared status and **Retry / rescan MIDI**, without opening a log.
- One management worker owns discovery, connection attempts and teardown. Retry
  has one bounded request slot and coalesces while work is running; successful
  connections are retained. Explicit rescans retry failed ports, discover new
  ones, and release input ownership for ports no longer reported by the backend.
- Raw MIDI callbacks retain their fixed input-handoff work and never publish
  status, discover ports or wait for connection management. Closing the UI does
  not wait for a blocked OS call: its worker retains connection ownership and
  cleans up after that call returns. No automatic OS hotplug detection is claimed.

## Deck selection and crate destination

- The crate exposes Deck A/B load-target selectors. A pointer press on either
  deck's controls or waveform selects that deck; hovering and releases elsewhere
  do not. Covered or clipped controls cannot change the target. Within a frame,
  pointer-event order determines the final target, not widget traversal order.
- Crate double-click and F capture the latest accepted target. Explicit → A/→ B
  buttons keep their direct destination without changing that selection.
- The selected deck has a renderer-confirmed outline. A queued selector is marked
  until the renderer publishes its matching request revision. Fast A/B changes
  cannot mistake an old equal-valued snapshot for acknowledgment. Rejected requests
  leave the previous target and use the existing visible submission-error path.

## Theme and font reload

- One background worker reads colors and the sibling `shell.toml`, and resolves
  fontconfig's current `monospace` selection on every check. Checks occur 800 ms
  after the previous pass; fontconfig-only and shell-only edits do not depend on
  a color-file timestamp. Configured source paths are preserved across reloads.
- The GUI receives at most one latest complete theme/font snapshot and applies
  styles and font definitions there. Discovery, file reads, validation, process
  execution and obsolete pending snapshot retirement run on the worker.
- Invalid/interim files keep the last valid settings for that source and are
  retried on later checks. Standard text styles honor finite shell base sizes
  from 4 through 96 points. Selected font bytes or face changes reinstall fonts;
  size-only changes reuse the font data and update text styles.
- Text inputs are capped at 64 KiB and font data at 32 MiB. Fontconfig has a
  500 ms deadline and 4096-byte limits on each output stream. Diagnostics are
  emitted only when they change. A slow filesystem never blocks GUI teardown.

### Explicit reload receipt

- `reload-theme` is a strict typed IPC operation with no target or extra fields.
  It uses a producer-only GUI port, never an audio command or callback-owned
  request state. Eight pending requests and one active worker transaction are
  the fixed bounds; full/unavailable endpoints reject explicitly. The existing
  eight IPC workers, request/reply byte limits and finite writes remain intact.
- Each force re-reads and validates colors, shell size and selected font bytes,
  including unchanged font identities. All requested resources must be valid.
  A failed force preserves the prior bundle and prevents partial automatic
  publication until a fully valid bundle is available again. Automatic checks
  otherwise retain their existing per-source last-good behavior.
- Completion is correlated to the request and follows the actual next GUI frame
  that installs font definitions and zoom. The latest applied preference profile
  remains authoritative: follow-theme off retains the default appearance while
  refreshing the cached desktop bundle; size and scale overrides remain active.
- Admission-to-receipt is bounded to 3 seconds; the reload CLI uses a separate
  4-second reply budget. Cancellation before GUI application prevents that
  request from applying. The independent watcher may later publish the current
  valid resources normally, so cancelling a ticket does not disable theme watching.
  Cancellation after application begins reports an unknown
  outcome, not rollback. Server shutdown cancels waiting handlers in at most a
  20 ms wait slice; GUI teardown does not join a blocked resource worker.
- Fontconfig retains its 500 ms subprocess limit. A kernel filesystem call can
  outlive the receipt deadline on its sole background worker; no additional jobs
  accumulate behind it and neither GUI nor audio waits for that call.

## Selected font and glyph fallback

- Fontconfig's resolved installed `monospace` face is first in both proportional
  and monospace UI text families. Its exact file bytes and face index are used;
  there is no fixed JetBrains override. Fontconfig may choose an installed
  substitute when the configured family is unavailable.
- All bundled glyph fallbacks remain available in both families, preserving each
  family's existing fallback order. This includes transport symbols such as
  `⇄` that some selected fonts and the default proportional chain lack.
- Before a valid selection is available, bundled Ubuntu/Hack/emoji fonts provide
  the fallback. If a later resolution or file validation fails, the last loaded
  valid font remains active until recovery. Discovery and font reads retain the
  background-worker and resource limits of the theme-reload contract.

## Last-play time display

- Unknown playback time remains `—`. Known times show `Just now`, elapsed whole
  minutes/hours/days, or an ISO calendar date explicitly labeled UTC after seven
  days. Hovering the crate row shows the precise timestamp including fractional
  seconds and UTC; the displayed date never wraps modulo epoch seconds.
- A timestamp later than the computer's current clock shows `Future time` with
  its precise timestamp and clock-relative annotation. Pre-1970 timestamps retain
  their actual UTC date. Values outside the platform calendar range keep an exact
  signed Unix-epoch offset instead of panicking or pretending to be unknown.
- Only visible cached history cells refresh when their timestamp changes, an age
  boundary passes, or the civil clock moves backwards. Aging history neither
  rebuilds the filtered crate nor formats offscreen rows. Far-future repaint
  deadlines are bounded before conversion to a native timer.

## Native project workflow

- Project New/Open/Recent/Save/Save As/Save Copy operate on versioned `.omat`
  documents containing the supported engine model, embedded decoded media and
  persistent UI view. Fixed factory mapping schema 1 is recorded and validated;
  no mutable mapping configuration, plugin host or automation editor is implied.
- A save captures one acknowledged engine revision plus its UI baseline. Save/As
  mark only that captured version clean and set the current path; later edits
  remain dirty. Copy changes neither path nor clean baseline. Complete engine and
  view validation precedes atomic publication. A committed directory-sync warning
  reports saved-with-warning; a precommit failure/cancellation preserves the file.
- File reads/writes, recent-path persistence, capture waits, preparation, and
  retired graph/media destruction stay on the project worker. The GUI never joins
  that worker. Cancellation respects safe boundaries; an in-progress OS call can
  complete first, and a committed file/install remains authoritative.
- Open/New requires a second GUI baseline check after preparation. Media-load
  intents invalidate that authorization even before audio admission. The renderer
  seals creative admission, drains earlier commands and rejects replacement if
  revision or pending controller GUI requests changed. Controls wait for a
  snapshot carrying the applied project revision before using restored selection.
- Reopening starts stopped, preserving deck positions and remembered clip launch
  targets for explicit resume. Physical gates/connections and DSP histories are
  not restored. Verified source fingerprints accompany matching captured deck
  receipts; embedded path strings alone never credit replacement-file history.
- Clean/saved window close seals creative admission through the terminal close
  command. Current-frame Cancel wins before terminal close receipt processing.
  Late edits reopen Save/Discard/Cancel. A failed clean-close handshake never
  auto-closes; explicit Discard also permits exit when the renderer or project
  worker is unavailable, without claiming pending changes were saved.

## Creative undo history

- Undo/Redo uses renderer-authoritative inverse transactions for supported
  creative state. It preserves unrelated live voices, physical gate identities
  and transport. No whole-session replacement is used for an ordinary Undo.
- Continuous pointer/held-arrow edits carry producer gesture IDs. Only adjacent
  compatible edits group; another source's edit forms an ordering boundary.
  A multi-object operation must validate all inverse targets before changing any.
- History retains referenced media and verified playback identities until their
  entries are retired. Owned payload destruction runs on a worker. Capacity and
  memory-limit rejection is visible and preserves the prior musical state.
- The Edit menu, named History panel and documented shortcuts use the same
  actions. Queue acceptance is not presented as renderer application. Text and
  modal controls retain their keyboard ownership.
- Saves record a coherent content checkpoint. Undo to that content can become
  clean; later values within the same gesture and persistent nonhistory view
  changes remain dirty. Replacement authorization still uses a monotonic revision
  so an edit-and-undo cannot silently authorize a stale Open/New/Close decision.
- Successful New/Open begins a new process-local history epoch. Saved projects
  retain the history-panel view, not old undo payloads or pending physical input.
## Deck time and runout display

- Each platter's time menu independently chooses elapsed source time or
  estimated remaining wall time, with a visible mode indicator. Remaining uses
  the renderer's captured actual rate, including sync and smoothing, and applies
  the source sample rate once. It does not reconstruct rate from a pitch fader.
- Each deck's lead is configurable from 0 (off) to 300 seconds; the default is
  30. At or below that estimate a playing forward deck shows RUNOUT plus a red
  outline/readout, in either time mode. Paused, scratching, stationary, reverse
  and absent/invalid media states do not warn.
- A finite, media-contained repeating loop longer than one frame suppresses the
  warning only while the playhead is inside it. LOOP identifies playing loop
  suppression; its tooltip explains that the file-end estimate ignores repeats.
  An invalid loop flag cannot hide a real runout warning.
## Persistent DJ library

- The DJ catalog is separate from DAW project/performance state. It assigns a
  durable 128-bit track ID to each typed location (random for media locations,
  reserved stable IDs for the two built-in stems) and stores title, artist,
  BPM/provenance, key hint, decoded duration, renderer-confirmed last play, main
  cue, eight hot cues, and the saved loop range/arming state. Cue/loop positions
  use source seconds; loading restores preparation while remaining paused.
- Built-in, absolute local file, removable volume/relative path, and provider/ID
  namespaces cannot collide. Local files and built-ins use the existing loader.
  Removable/provider resolution is explicitly unavailable; importing their
  metadata never fetches media or treats IDs as local paths.
- Replacing bytes at a local path keeps the track ID but creates a fresh
  fingerprint-qualified version. Old preparation/history remains archived and
  cannot apply to the replacement. Moves are not inferred; a different typed
  path gets a different ID. No editable beat-grid format is invented.
- One background metadata worker owns store locking, parsing, merging, migration,
  serialization and fsync. Scan/import publication is atomic and retains source
  selection. Renderer receipts publish preparation in twelve fixed atomic words;
  terminal receipts remain attributable even after replacement before a GUI poll.
- Schema 2 is written; strict flat schema 1 input migrates without changing IDs or
  preparation. Unknown schemas/fields, malformed stores and a missing primary
  with a preserved backup fail closed. Scans and startup defaults cannot replace
  them. Import conflicts reject the import while unrelated pending edits persist.
- Writes use a same-directory private temporary file, file sync, a prior-version
  backup, atomic rename and directory sync. A post-rename sync failure explicitly
  reports unconfirmed durability. The last committed catalog remains readable
  after an interrupted writer; stale temporary files are never loaded.
- The crate shows opening/saving/saved or a persistent error. Normal close waits
  asynchronously for a FIFO renderer fence and durable save receipt. A failed or
  delayed save permits Keep working, Retry, or an explicit Close without saving.
  Filesystem syscalls already in progress cannot be forcibly cancelled.

## Factory content and license records

- Content & licenses opens an offline index of factory sound/preset identities,
  embedded fonts, resolved Rust components and supplied integration files. Each
  entry links its source records, commercial-use/redistribution summary and full
  licensor notices. Missing records fail visibly; they cannot imply permission.
- Factory sounds are procedural originals. No impulse-response assets, ML models
  or proprietary reference-product content/SDKs are bundled. User media and
  system-selected fonts retain their own terms.
- The release packager and transactional installer reject unmanifested, missing
  or altered artifacts, source/dependency drift and stale embedded license
  records. Every retained installation includes that release's records and
  executable SHA-256 receipt. Refreshing records requires a rebuild.

## Keyboard and native accessibility

- Custom deck, waveform, mixer, sampler, crate and sequencer controls expose
  semantic names, roles, numeric ranges/values, state, focus and actions through
  AccessKit. Native effect sliders keep their actual bar bounds and physical
  units; associated value editors retain their native text editing behavior.
- Tab/Shift+Tab traversal shows focus and reveals clipped controls in both axes.
  Numeric arrows, fine Shift steps, Home/End and validated F2 entry use the same
  command handlers as pointer edits. Nonfinite numeric accessibility requests
  are discarded before native widgets process them. Focused activation never
  also toggles global transport, and text/modal shortcut ownership is retained.
- Pointer modifiers and right-click operations have named Shift+F10 alternatives.
  Linux's current AccessKit adapter lacks custom-action export, so a visible
  ordinary Actions button also opens those operations as native clickable menu
  entries. It retains the focused control identity, retires unavailable targets,
  and never routes a stale result to a different control.
- Sampler pointer, Space, Enter and assistive holds have separate local ownership
  bits around one accepted gate per pad. Only the last release emits note-off;
  failed admission never creates a local hold. Keyboard focus loss and window
  loss retire applicable holds. Instrument changes preserve original voice
  release identity; inactive piano gaps cannot acquire a new gate.
- The virtual crate exposes its entire filtered range through the selection
  control while naming/rendering only visible rows. Keyboard, native Value and
  ordinary menu actions do not require materializing all row widgets.
- Automated native evidence uses a private D-Bus accessibility bus and actual
  App output/renderer commands. It does not change the user's desktop bus or
  preferences, and is not a claim of Orca, human, or physical-controller QA.

### User preferences and setup profiles (#88)

- Preferences are strict versioned data at `$XDG_CONFIG_HOME/omatainer/preferences.json`
  when XDG_CONFIG_HOME is absolute, otherwise `$HOME/.config/omatainer/preferences.json`.
  Studio and Performance are editable defaults; Performance disables startup scanning.
  Profile identity, supported audio/MIDI choices, library roots, appearance, performance
  shortcut overrides and startup panel/scan choices round-trip without hidden fields.
- The settings window edits a draft. Preview discovers audio configurations and checks
  library folders on a worker. Apply saves the exact previewed draft before changing
  live settings. Cancel never submits a MIDI policy or changes live settings before
  commit. A save that already committed still reports success after late cancellation.
- Audio selection is consumed when starting the actual CPAL stream. An explicit
  device name must match one device; rate/channels/buffer must match advertised
  supported configurations. Unknown backend buffer limits are labeled, and startup
  errors do not silently pick another route. Main stereo uses channels 1/2; mono sums
  L/R; additional channels are silent. Independent cue routing is not implemented.
  Saved changes remain pending until a confirmed live audio change (#94) or restart. Explicit recovery can use system-default
  audio for one launch without changing the saved profile.
- MIDI startup uses the exact saved policy. Live policy changes use the manager's
  generation-qualified requested/applied receipt. Preview and Cancel have no MIDI
  side effects; allowed source identities/holds persist, excluded sources retire
  through the existing source-owned release path. Availability, missing exact names,
  pending work and manager errors remain visible and distinct.
- Preference IO/discovery runs outside App::update and all audio/MIDI callbacks.
  Jobs/results are bounded to one outstanding operation. Cancellation is cooperative
  at IO boundaries; blocked filesystem/driver calls are not claimed interruptible.
- Files are bounded to 1 MiB, regular and non-symlink; staging and exports are private
  0600 files. Saves validate, sync, recheck destination identity and atomically publish.
  Export refuses existing destinations. Observed external edits require Reload; this
  is optimistic conflict detection, not a filesystem transaction with arbitrary editors.
  Postcommit sync/identity warnings cannot be reported as failed saves.
- Unknown versions/fields, duplicate profile/shortcut keys, invalid values, unknown
  shortcut actions and effective shortcut collisions are rejected without rewriting
  the file. The supported version-1 single-profile form migrates losslessly in memory;
  an explicit save writes version 3; version-2 profiles migrate with default calibration/format fields. Recovery Reset preserves prior bounded regular
  file bytes in a private sibling backup before replacing them with defaults.
- Export contains only the typed preference model. No credentials, environment,
  connection handles or runtime tokens are serialized. Device names and explicit
  library paths remain visible/exported so cross-machine setup is reviewable.
- New settings controls use native accessible widgets and the shared numeric editor.
  Private Linux AT-SPI tests exercise actual Preferences/Preview/Apply/Cancel and
  persisted appearance values. Hardware route quality, controller compatibility,
  native-window behavior and human assistive-technology QA remain separate.

## Contextual help and lessons

- Every shipped control's help describes its actual handler, units, target and
  implemented limits. Widget hover text, accessibility descriptions, focused
  help and the offline manual share typed definitions; dynamic metadata remains
  visible alongside help. Disabled actions still explain their purpose.
- Guides observe renderer/project/receipt state and preserve original target
  identity. Admission, waiting, or an unrelated document cannot complete a step.
  Held capture and released capture are distinct. Restored cues are not a new
  cue edit; opening a project is not Save As.
- Starting/cancelling a guide cannot mutate musical state, bypass unsaved-work
  prompts, or stop an audition. Physical checks are explicitly self-reported;
  software steps never certify hardware. Progress remains session-local.
- In-app shortcuts reflect active preferences. The checked offline manual uses
  the canonical defaults and the same control/lesson catalogue; its golden test
  must pass when handlers, metadata or default bindings change.

### Professional audio devices and qualified latency (#94)

- Preferences version 3 persists the exact backend/device names, output format/rate/
  channels/buffer and optional calibration input/route/level. Unknown fields/formats
  fail closed. Versions 1/2 migrate in memory; migration never rewrites a file alone.
  CPAL exposes logical names rather than stable hardware serial identities; missing
  or duplicate explicit names fail visibly. Preview enumerates both directions and
  only offers supported choices within the application's validated limits.
- Saving audio intent does not switch a stream. A separate preview and confirmation
  stops decks/clips/captured and physical gates before applying the current advertised
  target. Exact preview drift is rejected; failed opens restore the prior exact plan.
  Cancel before the atomic activation decision rolls back; a later cancellation cannot
  misreport an already applied output. Neither success nor rollback resumes playback.
- Linux ALSA streams are created/played/dropped on one audio-owner thread. CPAL 0.15.3
  ALSA Stream Drop wakes and joins the callback worker. A unique graph lease returns
  through a preallocated one-slot channel; callbacks never share or lock a renderer.
  Other backend ownership contracts are not assumed supported. A missing return fails
  closed; blocking driver calls are not claimed interruptible. Shutdown independently
  silences managed output and probe callbacks, including a delayed opening stream.
- Audio operations share the project admission seal; physical releases remain accepted.
  On double failure the owner retains a stopped graph, rejects creative input via an
  independent offline flag, and services zero-frame project work for capture/Save,
  install/New/Open and Close. The audio guard releases only its own seal. Project DSP
  preparation reads the current logical rate; an obsolete-rate install is rejected.
- The displayed stream configuration is backend-accepted logical configuration, not
  physical negotiation. Observed callback frames and CPAL's output scheduling estimate
  are separate. Requested input/output buffer durations form a labeled partial roundtrip
  estimate; unknown buffer sizes make that estimate unavailable. Exact physical rate,
  converter/driver roundtrip and dropped-buffer counts are not fabricated.
- Loopback runs only after explicit cable/route/level confirmation, with the session
  stopped. Its selected input is temporary and never monitored into output. Three
  distinct coded probes stay between -60 and -24 dBFS on one chosen output; capture,
  timestamps and duration are bounded. Signal matching and quality checks run off audio.
  Silence/noise/ambiguous echoes/clipping/nonfinite data/missing stamps/backend failure/
  timeout/inconsistent repeated timing yield no measurement. Cancellation invalidates
  prior evidence. Accepted evidence names its profile and exact input/output plans.
- The measured quantity is common-host callback-entry-to-callback-entry loopback return
  time, with nominal callback resolution at the logical rate and repeat spread. It is
  not converter-only latency, and does not subtract unrelated CPAL stream clocks.
  No physical probe or controller qualification was performed for this change.

## Local release qualification

- Packaging and installation require source-, policy-, toolchain- and binary-bound
  passing local workload evidence. Raw timing samples and unresolved limits ship
  with the artifact; verification recomputes distributions and golden checks.
- A draft policy, missing or stale report, failed workload, missing native AT-SPI
  preflight or unreviewed target stops publication. Native builds and benchmarks
  remain local; no GitHub compute is used.
- Headless callback/render/UI budgets do not claim hardware deadlines, XRUN
  freedom, physical controller compatibility, compositor FPS or Orca testing.

## Performance protection (#96)

Show protection is shared by every CommandPort producer and renderer. It protects
active deck replacement and destructive edits while preserving mixing, composing,
recording and essential Save/catalog/history/retirement work. Safety requests use
a fixed priority mailbox; all-notes-off finalizes captures before release, and
recovery requires explicit input-release acknowledgment plus drained old work.
Raw MIDI epochs prevent buffered onsets replaying after recovery. Emergency output
uses a 2 ms ramp and stays muted until an explicit audio-owner stopped DSP reset;
quiet observation never unmutes. Failed reset retains mute. Project/device and
optional commit permits serialize mode entry without stealing a CloseGuard.
Optional cancellation never rewrites an already committed/applied outcome, and
pre-mode scan/import visibility cannot leak through later essential metadata
publication. Preferences persist startup protection only, not emergency state.
