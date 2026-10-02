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
- **On, forward rate 0.50–1.50×:** original MIT stereo-linked waveform-similarity
  overlap-add preserves pitch while the playhead advances at the requested
  tempo. Source sample rate and output sample rate remain distinct.
- At exactly 1×, direct source playback avoids overlap coloration. Platter
  touch uses direct scratch playback (`L~`); release resets overlap history
  with the existing bounded transition. Seek/cue/load transitions cannot
  resurrect audio from before the explicit destination.
- Reverse, stopped-rate transitions and rates beyond the supported interval
  use direct resampling. A playing deck outside the range shows `L!`; stopped
  and empty decks show armed status. Match/Sync can exceed the supported range.
- Resident PCM supplies look-ahead without an output FIFO. Search and overlap
  can still move content relative to the transport; this displacement must be
  measured separately from stream/device latency. No converter or hardware
  latency, transparent listening result or vendor parity is implied.
- Toggling L is audible at a non-center fader, not just a LED.

## C2. Match (⇄)

- Favored deck = xfader ≤ 0.5 → A, else B.
- Unfavored deck’s playback rate is set so its **effective BPM** equals the
  favored deck’s **pitched** BPM (`manual_grid_or_source_bpm * pitch_rate`), not session BPM. Manual grids also define source downbeat phase for Match.
- If both decks are playing, only the unfavored playhead is phase-aligned.
  Favored time does not jump.
- Works whenever the favored deck has audio; the unfavored deck can be stopped
  (rate is armed for when it plays).

## C3. Sample banks (factory and user banks)

- Kit, Perc and Hits remain original factory banks, **16 samples each**. The
  editor can always create an editable copy, even after opening a project that
  replaced all startup banks. A session supports at most 16 working banks.
- Named reusable definitions contain 16 slots with local catalog identity or an
  explicit factory source. At most 64 definitions are stored in schema 1,
  bounded to 4 MiB. Loading/copying creates a fresh working identity; replacing
  a reusable definition requires its exact ID, never a matching display name.
- Slot gain is 0–2. Start and exclusive end are source seconds, validated against
  resident PCM without clamping; blank end means the source end. A draft can be
  prepared and auditioned without applying it. Apply targets the captured bank,
  revision and project epoch, and creates one Undo entry after an atomic claim.
- Assignment, decoding and reusable-store work run off the GUI/audio callbacks.
  The existing decoder has two deck lanes and one sampler lane, with one active
  job and one pending/result per lane. A source is hashed and decoded through
  the same descriptor, then its fingerprint/path identity is rechecked. Only
  fresh successful decode proofs, acknowledged Applied, qualify catalog hashes.
- Missing/changed reusable files remain explicitly unavailable; no factory sound
  is substituted. Verified same-content catalog relocation preserves TrackId.
  Native projects embed available PCM independently of reusable file references,
  so missing originals do not silence valid embedded project audio.
- Held sample voices retain their onset's PCM, source range, gain and destination
  across edits, Undo/Redo and bank changes. User/project PCM keeps its native
  rate across output-rate changes. Only explicitly marked startup factory data
  is regenerated; State 1–3 banks migrate as embedded regardless of their names.
- Engine State 4 persists sparse bank slots, controls, sources and identities.
  Generic project container/recovery formats remain unchanged. Unknown future
  states fail before installation. Independent background pins keep final PCM,
  settings and bank destruction off the callback, including surviving captures;
  ownership has fixed 1 GiB PCM/8 MiB metadata and object-count limits. Capacity
  refusal is explicit. Unexpected owner failure refuses new preparation until
  restart while keeping existing callback references safe.
- Performance protection rejects new preparation/store work and bank edits at
  producer admission and renderer consumption. Audition has a guaranteed FIFO
  release, separate from pad/recording gates; completion is observable even when
  a short preview finishes between snapshots. Cancellation before an edit claim
  is inert; cancellation after the claim reports the actual terminal outcome.
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

- Startup presents the built-in crate immediately. Filesystem discovery and
  scan sorting run on one filesystem worker; catalog reconciliation and
  persistence stay on the existing sole metadata writer.
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
  A renamed/moved path is a new identity unless explicit verified relocation
  associates identical bytes with the existing track. No move is inferred.
  Replacing bytes at the same path creates a new fingerprint identity.
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

- MIDI piano-roll documents are coherent project-worker captures, never mutable
  note lists borrowed from the renderer. Apply validates the captured slot,
  project epoch and exact original MIDI content before capturing an inverse.
  Clip gain and unrelated mixer edits are retained. Recording into the target
  rejects Apply. Owned request data and acknowledgement references retire on
  the existing worker; renderer application/rejection/Undo/Redo do no heap work.
- Schema 6 persists stable per-clip note IDs, velocity and mute, and optional
  source-beat clip/loop ranges. IDs survive move, resize and value changes;
  drawing/duplication/recording create distinct IDs. Legacy IDs migrate
  deterministically; new fields, including null, are rejected in older schemas.
  Absent ranges retain legacy playback. Explicit ranges chase crossing start
  notes, play the intro once, close gates at boundaries and optionally repeat
  the loop. A compensated beat accumulator preserves exact long-clip boundaries.
- The editor supports pointer/keyboard note actions, numeric note/range values,
  triplet/free grids, pitch/time rulers, scale folding, zoom and scroll. Painted
  work depends on visible geometry; a virtualized native list exposes every
  note through stable IDs. Audition has a distinct, non-recording input owner
  and reserved release admission. Focus loss/close stop it. Pending or unapplied
  work postpones application close until its outcome or explicit discard.
- MIDI clips remain bounded at 8192 notes, source beats at 262144, and explicit
  clip/loop spans at a minimum 1/1024 beat. Repeating active note density cannot
  exceed 8192 per beat, including recording additions. This is the existing
  legacy note limit at its minimum supported period, not an unlimited event rate.

- Standard MIDI File interchange supports formats 0/1, PPQN 1–32767, integer
  note start/duration, channels and on/off velocity, standard channel messages,
  tempo, meter, key and standard text bytes. Format 2, SMPTE and RMID refuse;
  SysEx/proprietary data is listed and requires explicit omission approval.
  Trailing silence and canonical event order survive native save/reopen/export.
  Unsupported data is never silently approximated into musical events.
- Import maps tracks/channels into 1–64 MIDI cells, with ignore/replace/merge
  choices and session/file/authoritative-track conductor choices. Audio targets,
  ambiguous merged note pairing, stale targets, held target recording and
  changed conductor baselines refuse before inverse capture. Applying a map
  also refuses held project notes or recording. One atomic import uses up to
  65 preallocated inverse patches; apply/reject/Undo/Redo do no callback heap work.
- Original ticks remain authoritative only while their visible projections
  match. Musical scheduling uses the exact source values. Controller/program
  lanes persist for file interchange; the native track instrument retains its
  selected sound. This feature does not introduce device/channel output routing.
- File conductor playback uses integer microseconds, with at most 4096 tempo
  and 4096 meter points, 40–240 BPM, and meter denominators up to 128. Manual
  tempo edits remove the map with a complete inverse. Kept-session imports retain
  source conductor metadata even when outside playback limits.
  Tempo-dependent master effects update at map transitions; metronome beats
  use the current meter denominator and accent each bar or explicit meter change.
- Files are bounded at 16 MiB/128 tracks/262144 events; native sessions permit
  65536 notes and 16 MiB of MIDI lanes. Prospective native project serialization
  is checked against its 64 MiB metadata bound before import, reserving 128 KiB
  for the GUI envelope. Schemas before 6 reject presence, including null, of
  exact timing, channel, release velocity, source lanes or conductor fields.
- Export selects source cells and SMF form/division, muted-note inclusion and
  current-session or original-source conductor. Inexact conversion requires
  explicit rounding. Export never flattens clip loop/launch transforms. A new
  destination is claimed atomically; existing/racing destinations remain intact.
  Worker cancellation and changed source/directory paths leave no partial file
  or partial musical import. Parsing, encoding and adapters poll cancellation.

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
  cue, eight hot cue positions/names/optional RGB colors, a manual source-time beatgrid, and the saved loop
  range/arming state. Cue/loop positions
  use source seconds; loading restores preparation while remaining paused.
- Built-in, absolute local file, removable volume/relative path, and provider/ID
  namespaces cannot collide. Local files, built-ins and mounted removable volumes
  use the existing worker/loader. UUID and volume-relative identity remain on
  receipts; resolved mountpoints are transient I/O paths. Offline, missing,
  ambiguous and changed media remain distinct. Providers are unavailable locally;
  metadata imports never fetch media or treat provider IDs as local paths.
- Replacing bytes at a local path keeps the track ID but creates a fresh
  fingerprint-qualified version. Old preparation/history remains archived and
  cannot apply to the replacement. Moves are not inferred; a different typed
  path gets a different ID unless an explicit worker-verified, exact-byte
  relocation preserves it. Old receipts resolve through the captured path and
  fingerprint; only verified equivalent versions share cues, manual grids and history. Grid-only preparation qualifies for the same bounded optional content hash.
- One background metadata worker owns store locking, parsing, merging, migration,
  serialization and fsync. Scan/import publication is atomic and retains source
  selection. Renderer receipts publish preparation in 87 fixed atomic words;
  terminal receipts remain attributable even after replacement before a GUI poll.
- Schema 7 is written; schemas 1–6 migrate without changing IDs or preparation.
  Schema 6 retains its forest and gains an empty local watched-root book; earlier
  catalogs gain both. Missing required fields or new fields smuggled into old
  schemas are rejected. Older relocated-source histories remain File-only.
  Older hot cues default to unnamed, theme-colored slots. Unknown schemas/fields, malformed stores and a missing primary
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
### Batched recovery journal and untitled recovery (#97)

- A separate recovery worker captures coherent engine state through the bounded
  project handoff. It batches dirty edit state about every two seconds, including
  untitled work, and coalesces pending GUI view updates into one latest value.
  Encoding, hashing, embedded-media writes, fsync and retirement run off GUI/audio.
  This is a full edit-state batch journal, not synchronous per-command durability.
- Confirmed durable captured-state time, revision and sequence identify coverage.
  The separate commit acknowledgment is not the age of its captured edits. Capture
  contention, unavailable rendering, cancellation, full storage and I/O failures
  can enlarge the loss window; warnings retain the last confirmed durable age.
  A committed directory-sync warning never becomes a false durability claim.
- Preferences version 5 migrates versions 1–4 without losing version 4 startup
  performance protection; migration alone never rewrites a saved file.
  Saved profile settings configure checkpoint compaction (5–3600 seconds, default
  30), generations retained per session (2–20, default 3), and a global storage cap
  (64 MiB–16 GiB, default 2 GiB). Media, journal/checkpoint and staging bytes count
  toward the cap. No automatic deletion of another session or sole useful copy
  makes space for a newer write. Replay is bounded by record and byte limits.
- Private locked sessions and checksummed record framing distinguish epochs and
  order. Embedded media is content-addressed and verified on restart; in-session
  Arc identity is only a cache. A malformed final tail yields a valid-prefix report.
  Missing provenance files are informational when verified PCM is embedded;
  missing/corrupt required media prevents that candidate from silently losing audio.
- Startup discovery and Preview never replace the active document. Restore reuses
  Save/Discard/Cancel and the revision-checked project installation handshake. It
  starts stopped as an unsaved untitled copy and never adopts or overwrites the
  recorded explicit save path. A changed baseline or cancellation preserves the
  current project. Performance protection remains enforced by the project API.
  Full discovery/PCM Preview and explicit session deletion are optional guarded
  work. Protected startup only inspects root metadata and shows a deferred-scan
  notice; it does not claim there are no recoverable copies. Automatic durability
  remains essential, and protection invalidates prior verified previews/lists.
- Automatic capture yields to explicit project work using cancellation and a bounded
  worker-only retirement fence. It does not acquire destructive project/audio seals.
  Intentional exit retires its epoch after the existing project/library decisions;
  retirement failure permits an explicit warning override. Cancel exit starts a
  fresh recovery session even if retirement already committed. A replaced epoch is
  retained until the newer epoch has a confirmed durable record.


### Manual beatgrid editing

- A validated grid stores a finite downbeat in source seconds and a 20–400 BPM
  constant period; negative beat coordinates support pickups. Four-beat bars
  are presentation only; changing meter and tempo maps remain outside this model.
- The GUI drafts Set, slip, stretch, half/double and Reset without renderer edits.
  Apply sends one receipt-qualified undoable command. Its own atomic Applied or
  Rejected acknowledgement survives later grid edits/Undo; unrelated preparation
  cannot complete it. A disconnected unconfirmed request has an unknown outcome.
- Analysis/source BPM remains separate from manual effective BPM, including Reset
  before snapshot refresh. Absolute cue positions never move with the grid.
- Catalog 4 and engine State 3 onward preserve grids. State 1/2 default to no manual grid;
  generic recovery journal 1/container 1 needs no format change. Future/invalid
  states fail before installation. Restored transport remains stopped.
- Performance protection rejects every grid edit at producer admission and again
  at consumption. Already-admitted rejected payloads acknowledge rejection and
  retire off the callback, without allocation/free in the warmed render path.


## Local support evidence and safe startup

- Support export is a typed allowlist, capped at 4 MiB, 512 structured events,
  120 numeric counter samples and eight exact durable recovery references.
  Media/project contents, paths/titles, port/device names, raw error/panic text,
  environment, credentials and core dumps are excluded. No upload or audio
  plugin host exists. Linked crate versions are distinct from unavailable
  runtime driver versions and hardware latency/XRUN measurements.
- A 256-observation nonblocking producer lane feeds a worker; JSON, filesystem
  work and retirement never run in the audio callback. Collection loss and
  confirmed persistence time are explicit. Eight inactive runs / 40 MiB logical
  storage are bounded, and unknown/corrupt entries are preserved on refusal.
- Private startup markers distinguish confirmed clean/startup failure, an
  observed Rust panic, and unexplained unclean termination. The panic hook uses
  a preopened descriptor and fixed byte only, without payload serialization or
  fsync; abrupt failure may leave only unclean evidence. Marker ownership stays
  locked through the last logical hook owner, independent of raw duplicated FDs.
- Support lookup matches full opaque session digest, epoch and sequence plus
  confirmed metadata; it never selects a newer record implicitly. Existing
  recovery validation, unsaved decisions and stopped untitled restoration apply.
- Safe mode opens no CPAL/MIDI backend and applies no startup preferences,
  external themes/catalog or automatic recovery discovery. Real stopped project
  capture/install supports Open, recovery and Save. Existing offline admission
  rejects engine controls. Normal restart is explicit and occurs only after
  successful close coordination, including Save/Discard/Cancel.

## Offline runtime contract

- Local media, embedded-PCM native projects, local sampler-bank definitions,
  built-in Rust instruments/effects, Help and bundled notices require no cloud
  session or online authorization. Missing source references fail explicitly;
  the application never substitutes a download or provider login.
- No audio plugin host, provider download/streaming, cloud transfer,
  authentication or credential acquisition/storage is implemented. A future
  authenticated integration requires its own OS secret-store implementation,
  opaque references in app state and separate offline/cache qualification; this
  contract does not invent that capability. Unknown credential fields are
  rejected by strict preferences/project/support schemas. User-authored project
  names, paths and PCM remain intentional document content, not sanitized data.
- Support export is a reviewed local allowlisted file, without environment,
  credentials or raw project/media/error content. External upstream source
  links explicitly request a browser. Reading bundled documentation/notices
  issues no external URL request; browser/portal activity is outside offline
  qualification.
- Linux qualification denies non-AF_UNIX socket creation and io_uring setup only
  in new same-EUID children/descendants. Negative controls are measured before
  exec and local Unix IPC remains enabled. No firewall, route, desktop network,
  service or identity is changed; existing IPC/runtime ownership checks remain.
  Private buses, no inherited network descriptors and an allowlisted child
  environment bound the tested surface. This is not a hostile-code sandbox or
  proof about a host Unix proxy, remote filesystem/audio/display or unavailable
  physical hardware.
- `scripts/check-offline.py` source-binds prebuilt executables and checks actual
  UI/local persistence, fresh-process reopen after source removal, safe startup,
  support and private native accessibility. Its separate performance phase uses
  unchanged #95 and #100 workloads/policies; actual deadline exceedances remain
  distinct from ceiling pass and backend XRUNs remain unavailable.
## Background track analysis

- Analysis records belong to an exact track version and verified source digest.
  BPM, duration and waveform each retain an algorithm version and measurement
  timestamp; completed unknown BPM is distinct from an unmeasured value. User
  BPM and manual beatgrids remain authoritative even after stale loader or scan
  publications. Existing source bytes and other versions are unchanged.
- The single media decoder admits one lowest-priority analysis job, behind deck
  and sampler loads. Hashing, decoding, tempo estimation and waveform work check
  cancellation cooperatively; prepared results contain no playable PCM. Source
  identity is checked on the same descriptor and path before publication.
- Waveforms are immutable SHA-addressed private cache blobs: at most 2,048 bands
  and 256 KiB per blob, within 1 GiB and 32,768 retained files. Publication uses
  no-overwrite links and verified directory/lock ownership. Corrupt blobs remain
  preserved; reanalysis can publish a new verified reference. Cache quota or
  ownership failures remain explicit and never evict unrelated files.
- A performance admission guard and a shared cancellation/publication claim
  cover the catalog save. Cancellation can win before that claim; afterward
  the receipt reports the actual save, including post-rename unconfirmed
  durability. A missing worker cannot turn an unknown result into success.

### Background analysis queue and inspection

- The GUI captures an immutable crate and filtered-index Arc in constant work,
  never a copied batch of sources. One selected row or at most 4,096 filtered
  rows is admitted; oversized batches are refused without silent truncation.
  Later filters, sort publications and selection do not retarget queued sources.
- Exactly one source inspection, shared-decoder token and catalog publication
  receipt is awaited in order. Prepared/Ready is not a saved result. Foreground
  preemption and failures pause the captured row with explicit Retry, Skip and
  Cancel. A committed result cannot be relabeled cancelled; unconfirmed
  durability is counted and displayed separately from successful saves.
- Selected BPM/duration/waveform fields reuse only qualified cached values;
  Force recomputes chosen fields. Inspection reads partial saved results and
  verifies waveform bytes on the metadata owner without initiating decode.
  Automatic key detection is unavailable, and existing hints are not promoted
  to measured keys. Manual/locked preparation remains authoritative.
- Captured large views and inspection payloads return through bounded retirement
  slots to the metadata owner. The GUI keeps ownership and retries if those
  slots are occupied. No analysis filesystem access or source decoding runs on
  the GUI or audio callback. Closing the panel hides it without cancelling work.


## Named and nested crates

- Collections store stable TrackId membership, never file moves or copied audio.
  Roots, children and direct members retain explicit manual order. A track may
  belong to several crates once each; children remain separate views. All tracks
  retains the library sort, and filtering a named crate preserves manual order.
- The catalog permits 4,096 crates, depth 32, names of at most 256 UTF-8 bytes,
  100,000 members per crate and 250,000 total memberships, subject to the existing
  catalog byte/track limits. One GUI edit accepts at most 4,096 selected members.
  Invalid names, cycles, duplicates, stale revision/anchors and bounds fail
  before mutation; exact no-op edits retain the collection revision.
- The existing metadata owner generates crate IDs, validates candidates and
  saves one explicit operation at a time, including its terminal receipt.
  Cancellation and performance protection may reject before publication claim.
  After the claim, actual durable, committed-unconfirmed or unknown outcomes
  remain distinct. Imports merge tracks and forest as one validated candidate;
  identity/order conflicts reject the complete import without guessing.
- Removing membership or deleting a confirmed subtree preserves source bytes,
  catalog tracks, preparation, analysis, history and sampler banks. Refresh,
  missing media and verified relocation preserve membership identity.
- The owner prepares row/track lookup tables for each exact immutable catalog
  and row publication. A different publication cannot reuse the old mapping.
  Superseded tables retire on the worker; the GUI virtualizes the tree/member
  list and retains only the bounded selected-member set.
- Controller views use the selected crate's filtered manual order and a new
  epoch after publication changes. Already admitted loads keep their captured
  source; obsolete Browse requests cannot redirect the new view. Project state
  saves the selected crate ID only, never the external forest. Missing saved
  crates fall back explicitly to All tracks without recreating collections.


## Music imports and watched removable libraries (#107)

- Explicit files/folders merge on the existing scanner. Limits are 64 inputs,
  64 folder levels, one million visits and 100,000 rows. No descendant symlink
  traversal; live reason counts and at most 32 bounded path/detail samples expose
  unreadable, unsupported, missing and incomplete inputs. Cancellation preserves
  the previous candidate. Imported catalogs never activate foreign watched roots.
- The scanner owns one nonblocking inotify descriptor and at most 4,096 directory
  watches. Notifications and mount observations coalesce into one hint. A
  30-second full-scan hint covers missed/new directories and unsupported watches.
  The GUI admits that scan against its current catalog/baseline only in Studio,
  outside Close and after pending persistence. Mount polling is worker-only.
  No second filesystem/catalog writer is introduced. Large summaries and
  enrollment batches retire on workers. Startup waits for the saved root book.
- The catalog writer atomically saves local root bookmarks with optional scan
  enrollment. At most 32 profiles / 64 roots each are retained. Offline bindings
  survive; removing a root removes only its bookmark. Rebinding a known UUID
  requires a confirmed removal and later enrollment. Essential cues and measured
  source proofs survive an optional scan cancelled before commit.
- Linux discovery uses bounded mountinfo and libudev block data. UUID is durable
  identity; namespace, mount ID/root/point and filesystem/block device are access
  guards. Duplicate UUIDs, ambiguous overmounts, foreign mounts, unsupported UUIDs
  and symlink traversal are refused. Btrfs access verifies its read-only FS_INFO
  UUID when inode and mountinfo device numbers differ. Existing UUID resolution does not depend on
  a later change in bus/removable classification. Unsupported unrelated block
  UUID formats do not disable local-file imports.
- Missing-file observations come from guarded inspection of that exact path on
  the visible mounted volume, never absence from a traversal. Offline/not-visible/
  unreadable/ambiguous/changed remain distinct; all records are retained. Selected
  rows display the last scan observation. No scan deletes media or saved tracks.
- Exact captured File locations enroll as UUID/relative sources while preserving
  TrackId, crate membership and archived aliases. Changed fingerprints create
  fresh versions; UUID/path alone never transfer preparation/history. Removable
  deck loads hash and decode one opened regular descriptor (8 GiB source limit),
  retain typed receipts and verify path/mount/descriptor afterward. Analysis and
  sampler preparation use the same guarded resolver. Only fresh same-byte proof
  restores archived preparation. Explicit verified relocation supports both
  local namespaces and retains old attributable references, without moving files.
- Software mount inventories, local block-filesystem resolver checks and converted
  callback comparisons do not qualify a physical removable-drive unplug, human
  listening, controller compatibility, backend XRUN freedom or Orca behavior.

## Reviewed missing-media relocation (#108)

- The selected track can use an explicit replacement path or search 1–64
  absolute roots. Complete SHA content identity, never a filename or size guess,
  qualifies a match. A missing original requires its previously saved digest;
  an available unqualified original is measured on the filesystem worker and
  its digest is persisted before a searched choice can commit.
- Search shares the existing scanner, leaving essential metadata saves free.
  Bounds are depth 64, one million visited entries, 100,000 inspected files,
  4,096 guarded directories, 64 GiB hashed bytes and 256 matching locations.
  Static symlinks and nested mounts are skipped. Unavailable inputs, errors and
  limits expose incomplete coverage, full observed reason counts and up to 32
  bounded samples. Cancellation and changed directory/mount guards discard
  publication. Two bounded worker pins retire immutable result rows off the GUI.
- All identical copies require explicit selection, including a single match.
  New searches and edited roots retire the prior review. Control identities
  capture input text, track and search/choice identity so an obsolete event
  cannot select a different displayed path. Closing cancels search; Performance
  protection invalidates old work even after a quick return to Studio.
- Commit rechecks the reviewed candidate's fingerprint, mount namespace/location
  and complete bytes using the sole catalog writer. The latest cue/grid edits,
  stable TrackId, crate memberships and history survive the path change, with
  the old source retained as an attributable alias. No media is moved/deleted.
  Pre-rename persistence failure rolls back the optional association while
  retaining essential edits for retry. A post-rename failure retains the actual
  committed association and reports unconfirmed durability.

## Embedded audio metadata and reviewed edits (#109)

- Import/scan receipts and renderer-accepted native loads carry guarded embedded
  title, artist, BPM and key observations. Primary tag precedence, duplicate or
  conflicting values, incomplete parsing and filename fallbacks remain explicit.
  User overrides and intentional clears win over tags, automatic analysis and
  filename hints. Supplied tag values are never called measured tempo/key.
- Inspection uses the existing filesystem/media owners, bounded reads and parser
  allocations, and cancellation at read/seek boundaries. Tolerant read-only
  observation cannot authorize rewriting a malformed or incomplete tag set.
  No filesystem parsing, hashing or container write runs on GUI/audio callbacks.
- The GUI captures one selected row or at most 4,096 filtered rows via immutable
  views. Review captures field changes, storage choice, exact track/version and
  performance generation. Edited drafts invalidate review; later browsing cannot
  retarget it. Unchecked fields are retained and checked empty values clear.
- Supported MP3/FLAC/WAV/AIFF writes stage a private same-filesystem replacement,
  verify requested values, preserve unknown metadata/artwork/ancillary bytes and
  compare complete playback payloads plus successful full decoding. Rewrites
  require an exact old fingerprint and whole-file digest. Payload equivalence
  authorizes only that measured within-track transaction; the new file has its
  own fingerprint/digest. Stable IDs, crates, preparation and history survive.
- Rewrites are limited to 128 MiB and refuse read-only, non-owned, hardlinked,
  special-mode or extended-attribute media. Unsupported/unsafe formats or field
  precision retain exact catalog sidecars, with actual bytes unchanged. An
  ambiguous transaction or installed recovery record cannot become a fallback
  sidecar that conceals an unknown media outcome.
- Durable intent precedes atomic exchange. The shared short commit claim orders
  cancellation/protection against that exchange. A write that won reports its
  installed state even after cancellation; original bytes remain recoverable
  until the sole catalog writer confirms persistence. External exchange races
  preserve both files and a conflict record, without speculative rollback.
- Startup recovery follows catalog ownership. Confirmed installed media receive
  essential catalog reconciliation, even during Close/protection; an unchanged
  original permits staged-copy cleanup. Unknown/conflicting journals are retained.
  UUID-qualified removable recovery requires fresh mount access and exact source
  bytes; local recovery cannot guess across changed device identities. Backup
  cleanup follows durable catalog confirmation. A stopped worker or post-rename
  sync failure reports unconfirmed outcome and remains explicitly retryable.
- Each reviewed row finishes filesystem, catalog and cleanup receipts before the
  next. Cancellation retains earlier saved edits; unchanged skips and unconfirmed
  outcomes have distinct counts. Tag work does not block essential cue/history
  saves while its filesystem operation waits. Already resident playing PCM is
  unchanged by tag edits; reload uses current effective metadata.
- Catalog schema 8 migrates earlier versions while refusing newer-field states
  falsely labeled as old schemas. A small pinned Symphonia RIFF patch excludes
  the AIFF SSND header from PCM and rejects incomplete/inconsistent frame extents;
  source media is unchanged by the decoder, and all modified MPL sources/notices
  are included in the release provenance/package.
