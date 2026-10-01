# omatainer

Omarchy-native DAW + live DJ surface. One clock, one mixer, one window:

- **Session** clip grid with quantized scene and clip launches
- **Compose** explicitly armed pad-note capture, with held durations and undo
- **Two decks** with spinning platters, Serato-style waveforms, hot cues, loops, vinyl jog, sync, EQ, filter, and a crossfader

Hardware is optional. USB class-compliant MIDI is first-class. Factory maps ship for Akai APC Mini / APC40 / MPK and Pioneer DDJ-FX / FLX / SB / 400, with legacy partial NS7 / NS7FX control maps. These profiles do not establish physical compatibility; NS7 motorized wheels and NS7II support remain unverified. Unmapped musical inputs can play notes where the profile allows; this build has no editable MIDI-learn mapping UI. Keyboard and pointer controls remain available.

It reads the current Omarchy theme (`~/.local/state/omarchy/current/theme/colors.toml`) and font, registers in the keybind menu, and drops a Quickshell bar chip next to the rest of the shell.

## Install on this machine

```bash
~/Projects/omatainer/scripts/install-omarchy.sh
```

Then `SUPER+O` launches or focuses it. Right-click the bar chip to play/stop without opening the window.

The installer stages and validates the complete integration before publishing it.
Failed publication or desktop validation restores the prior files. Each successful
install prints a recovery journal under `~/.local/state/omatainer/installations`;
restore its saved state with `scripts/install-omarchy.sh --recover /path/to/journal.json`.
Running app instances retain their executable until you close and relaunch them.

## Preferences and profiles

Open **Preferences** in the status bar or press **Ctrl+,** outside an editor.
Choose Studio/Performance, duplicate or rename a profile, and select **Use this
profile** to stage a switch. **Preview changes** shows the requested audio route,
MIDI input policy and folder availability. **Apply and save** commits the draft;
**Cancel changes** keeps the current setup.

Audio device, sample format, rate, channel count and buffer size are saved separately
from the running output. Open **Audio devices and latency**, select **Preview saved
audio**, then **Use saved audio now** and **Stop and change output** to apply without
restarting. This stops performance and tries to restore the prior output on failure;
it never resumes playback automatically. If both devices fail, the stopped session
still supports Save, New/Open and Close while you recover an output. An unavailable
startup setup offers **Use system default audio this time** without changing the
saved profile. Main left/right use outputs 1/2; mono sums both channels and extra
channels are silent. There is no separate headphone cue bus yet.

The audio window lists advertised input/output capabilities and distinguishes the
backend-accepted logical configuration from observed callback sizes. Physical
negotiated rate and converter latency are unavailable through CPAL. Optional
**Measure loopback** requires a suitable line-output-to-line-input cable/interface
route and explicit **Cable ready: stop and measure** confirmation. It plays three
low-level probes, never monitors input, rejects weak/ambiguous/corrupt evidence,
and reports qualified host callback-to-callback return timing, not converter-only
roundtrip latency. See [audio settings validation](docs/validation/issue-94-audio-settings.md).

MIDI input selection applies through the connection worker, with requested/applied
generations and missing-device errors visible. Appearance (desktop theme/font,
font size and scale), library roots and performance shortcut overrides apply live
following a successful save. Startup scanning and Help/MIDI panel choices apply
when launching. Navigation, focused-control keys and reserved project keys remain
available when performance shortcuts are disabled.

Settings live in `$XDG_CONFIG_HOME/omatainer/preferences.json`, or
`~/.config/omatainer/preferences.json` if XDG_CONFIG_HOME is unset/relative.
**Reset profile to defaults** stages a reset; **Import into draft** also needs
Preview and Apply. **Export draft** writes a new private file without overwriting
an existing one. It includes device names and library paths, but no credentials
or runtime handles. Invalid/newer files are preserved; recovery offers an explicit
backup-and-reset operation instead of silently replacing them.

## Offline help and guided workflows

Use **Help** or the active **F1** binding for focused-control explanations, accessible input, effective shortcuts and guided lessons. Hovered or focused controls include purpose and units; F1 remains available while editing text when it is bound to Help. Lessons observe renderer-confirmed steps and never replace a project or start audio automatically. Cancel lesson closes the guide only. Physical listening/controller checks are explicitly self-reported.

The [offline manual](docs/manual.md) is generated from the same catalogue. This build has no Arrange timeline, piano-roll note editor, external audio recording, warp editor, automation lanes or plugin host. Native projects, pad-note capture, clip gain, history, mixer/FX and DJ preparation are implemented workflows.

## Play without files

The default session is a four-clip house sketch (drums, bass, keys, pad). Press **space**. Scenes **1** and **2** are filled.

Drop wav/mp3/flac onto a platter, or put tracks in `~/Music` and load with **F** / **→ A** / **→ B**.

## Native projects

Use **Project → Save project as…** to choose a `.omat` path. Native projects embed
the session's media alongside clips/notes, instruments, mixer/effects, sampler
banks, deck cues/loops/positions, selections and the crate/panel view. The current
factory controller mapping schema is recorded; there is no editable mapping
configuration to save in this build.

**Project → Autosave and recovery…** shows separate recovery copies, including
untitled sessions. Dirty edit state is normally journaled about every two seconds;
the displayed last confirmed durable age is the actual recovery coverage, and
capture/disk delays can increase that loss window. Checkpoint interval, retained
generations and global storage cap are saved profile settings. After interruption,
Preview a candidate, then Restore as untitled copy: the ordinary unsaved-changes
decision protects your current session, playback starts stopped, and Save As chooses
a new explicit project file. The original saved project is never overwritten.

**Save project** updates the current file. **Save project as…** changes the current
path; **Save copy…** writes another file while keeping the current path and unsaved
state. Path dialogs accept absolute paths or paths relative to the application's
working directory. Replacing an existing destination in As/Copy requires the
explicit checkbox. The title marks unsaved edits; changes made during a save stay
unsaved after that captured version finishes writing.

**New project** creates an empty clip session with the factory sampler resources.
**Open project…** and **Recent projects** restore a saved session with playback
stopped. Press **Space** to resume remembered session clips; each deck's play
button resumes its saved position. New, Open and window close offer
**Save changes / Discard changes / Cancel** when needed. File operations run in a
worker and expose cancellation and visible errors; a failed open preserves the
current session. An unresponsive engine cannot silently authorize a clean close;
explicit Discard can still exit without saving.

The versioned file includes exact decoded audio, so original source files are not
required for playback after reopening. Physical held keys, scratch touches,
connections and DSP tails are transient. Default limits are 8 MiB metadata,
256 embedded media entries and 1 GiB PCM; unsupported or corrupt projects are
rejected without replacement. See the [project workflow evidence](docs/validation/issue-82-ui.md)
and [file format and atomic save rules](docs/validation/issue-82-codec.md).
## DJ library

Use **analyze…** beside the crate search to prepare BPM, duration and a bounded
waveform for selected local files or mounted removable libraries. **Analyze selected row** captures one source;
**Analyze filtered crate** captures the current filtered order, up to 4,096 rows.
Later filtering or scanning does not retarget that queue. Choose individual
fields and **Force selected fields** to reanalyze them; manual BPM and locked
preparation remain authoritative. Key detection is unavailable.

The shared media worker hashes and decodes one source at a time. Explicit deck
or sampler loads preempt background analysis; **Retry current analysis** resumes
the captured source. Cancel stops remaining work at safe boundaries, while a
publication already claimed reports its actual outcome. Prepared progress is
separate from catalog persistence. **Inspect selected cache** verifies stored
values and the waveform without starting a source decode. Missing/corrupt cache
or changed source versions remain explicit. Closing the panel leaves its queue
running. Optional analysis is unavailable during Performance protection.


Use **All tracks ▾** to open the **Named crates** manager. Create root or child
crates, rename them, reorder siblings or nest a crate in a chosen destination.
Crates hold references to tracks; deleting a confirmed subtree removes only its
crates and memberships. Source audio, saved preparation and sampler banks stay
intact. Names must be unique among siblings.

Choose **Use as destination**, then select All tracks or another crate to add
the selected track or up to 4,096 filtered tracks. Member checkboxes and the
numeric **Member row** control select tracks for copy, move, removal or manual
reordering. **Insert before row** uses the unfiltered membership order; its last
position appends. Filtering preserves that order. Children are separate views,
and All tracks keeps the normal library sort.

Changes wait for the catalog save receipt; cancellation cannot undo a published
edit, and unconfirmed durability stays visible. Membership survives restart,
refresh, missing media and verified relocation. The manager shows unavailable
members without substituting another source. Projects remember the selected
crate as a view preference; they never replace this independent catalog. A
missing saved crate falls back to All tracks. Collection edits are unavailable
during Performance protection; browsing existing crates remains available.

**library… → Import music files/folders** accepts one absolute path per line,
including individual files and nested folders. It merges readable supported audio
without stopping either deck. Progress shows skipped reasons and incomplete
coverage: at most 64 inputs, 64 folder levels, one million visited entries and
100,000 rows; the first 32 skipped paths are shown. Cancellation retains the
prior crate. The worker does not follow descendant symlinks.

**Manage music folders in Preferences** edits saved watched roots. After the
startup/manual scan, directory notifications coalesce into one pending rescan;
a full-scan hint every 30 idle seconds catches missed events and reconnects. Watches
cover at most 4,096 directories; the fallback scan covers further directories
within the traversal limits. Scans wait for Studio and for the current catalog
save; filesystem work can delay completion. Removing a root removes its bookmark, never tracks or source files.

Known removable roots retain their filesystem UUID across mountpoint changes.
An offline volume remains distinct from a missing file observed on the mounted
volume; scans never infer deletion from an unreadable or incomplete traversal.
Previous file records enroll without changing TrackId or named-crate membership.
New fingerprints start unprepared; only a freshly verified matching content hash
restores older cues/history. Duplicate UUIDs and foreign mounts are refused.
Importing another catalog does not activate its watched folders.

Tracks discovered by Scan or successfully loaded from a dropped file are stored
in `$XDG_DATA_HOME/omatainer/library.json` (normally
`~/.local/share/omatainer/library.json`). Stable track IDs, metadata, tempo
provenance, duration, playback history, cues and loop preparation survive restart.
This catalog is independent of DAW projects. Its saving/error status appears
below the crate controls; row tooltips include track ID and typed location.

**library… → Import catalog** imports an Omatainer catalog JSON on the background
worker. Version 7 is the current format; versions 1–6 migrate without changing
IDs or preparation. Source-qualified analysis results and their algorithm versions
are retained in the catalog; bounded waveform blobs live in the private cache. Conflicting identities or unknown
fields/formats are rejected rather than discarded. Mounted removable libraries
resolve by filesystem UUID and volume-relative path. Offline, ambiguous or changed
volumes fail explicitly; provider references remain unavailable locally. No
provider/network media is fetched.

The main cue, eight hot cues and saved loop range/arming state return on a later
load; playback stays paused. Replaced file bytes keep their location's ID but get
fresh preparation, with old fingerprint versions preserved in the catalog. A
new path receives a new identity unless the crate’s **relocate…** action
verifies identical file bytes and deliberately preserves the existing track ID.
Keep the original file available until its move-verification digest is saved.
**Deck grid…** edits a manual downbeat and uniform tempo independently of analyzed
BPM. Preview Set at playhead, slip, stretch, half/double tempo and Reset before
Apply; Cancel/Escape discards the draft. Applied grids are undoable, survive
library/project/recovery reloads, and guide Match phase and quantized loops.
Negative beat coordinates represent pickups; bars currently contain four beats
(changing meters and tempo maps are not implemented). Cue positions stay in
source seconds. Performance protection rejects grid edits; essential saving
continues, with optional relocation hashing deferred until Studio mode.

**Deck Actions → Edit cue names and colors** edits all eight hot cues with names
up to 64 UTF-8 bytes and optional RGB colors. Pads, waveform markers and the cue
list share those values; edits are undoable and the list shows whether they are
session-only or saved. [Cue storage and relocation details](docs/validation/issue-98-cue-metadata.md).

Saves publish atomically and retain `library.backup.json`. A malformed or newer
store is never reset to an empty library. Repair/restore it while the app is
closed, then restart; errors remain visible. Normal close waits without blocking
the UI for earlier deck edits and the background save. **Close without saving**
explicitly accepts any uncommitted changes being lost. See the
[storage schema, recovery policy and validation](docs/validation/issue-85-dj-library.md).

## Performance history

Open **History**, then **Start session** before your set and **End session**
afterward. Each boundary waits for the output renderer; it does not start or
stop playback. The history records deck and catalog identity plus measured
main-output contribution in 10 ms windows above −90 dBFS, including master
FX tails and this build's cue blend. Review incomplete or uncertain coverage
and the pending-save status. Hardware delivery and listening require your QA.

**Mark played**, **Mark unplayed**, and **Use measured status** keep measured
duration intact. Add named external tracks for material played outside the
engine; these have no measured duration. Sessions survive restart independently
of projects and Undo. Interrupted sessions retain their last saved prefix.
**Export session** writes a new local JSON file with labels and opaque catalog
IDs, omitting media locations and fingerprints. Review labels before sharing;
existing destinations are refused. Performance protection permits recording,
ending and automatic saves, and excludes manual edits and export.

Normal exit waits for the actual session end and its save. **Keep working**
cancels exit; an already ended session remains ended. The explicit close-without-
confirmed-save action accepts loss of recent history. See the
[history measurement and persistence evidence](docs/validation/issue-106-history.md).

## Deck pitch lock

The deck's **L** button preserves pitch during forward playback at 0.50–1.50×
the original rate. At the original rate, playback passes through directly.
**L~** shows direct scratch playback while touching the platter; release returns
to the supported playback mode, and a stopped deck stays stopped. **L!** means
the current rate is outside the supported range and both tempo and pitch change.
Match and Sync can request rates beyond the pitch fader's range. Hover or focus
the button for the actual renderer state and playback rate.

The implementation is original MIT stereo-linked waveform-similarity
overlap-add. Its resident-source look-ahead does not insert an output FIFO, but
can shift musical content relative to the transport. Quality varies with the
material and rate; the [qualification evidence](docs/validation/issue-100-keylock-quality.md)
separates objective render/CPU measurements from the pending blind listening
and physical performance checks.

## Keybinds (also under `?`)

Omarchy desktop bindings: `SUPER+O` launches/focuses the app;
`SUPER+SHIFT+SPACE` controls session playback while unfocused.

<!-- app-shortcuts:start -->
| Keys | Action |
| --- | --- |
| `Space` | Play / stop session |
| `1` | Launch scene 1 |
| `2` | Launch scene 2 |
| `3` | Launch scene 3 |
| `4` | Launch scene 4 |
| `5` | Launch scene 5 |
| `6` | Launch scene 6 |
| `7` | Launch scene 7 |
| `8` | Launch scene 8 |
| `Q` | Play / pause deck A |
| `A` | Cue deck A |
| `W` | Toggle deck A tempo sync |
| `P` | Play / pause deck B |
| `L` | Cue deck B |
| `O` | Toggle deck B tempo sync |
| `[` | Crossfader fully to A |
| `]` | Crossfader fully to B |
| `F` | Load selected crate item onto selected deck |
| `? (Shift+/)` | Show / hide contextual help and lessons |
| `F1` | Show / hide contextual help and lessons |
| `Ctrl+M` | Show / hide MIDI window |
| `Escape` | Close effect chain |
| `Ctrl+Z` | Undo last creative edit |
| `Ctrl+Shift+Z` | Redo next creative edit |
| `Ctrl+Y` | Redo next creative edit |
<!-- app-shortcuts:end -->

The bracket keys choose the crossfader endpoints, with the existing mixer ramp.
F uses the current filtered crate selection and selected deck. Tab navigation
between session/arrange/compose views is separate feature work.

Typing in search or another text field owns the keyboard, including the frame
that editing ends. Blocking dialogs and popups also suppress global shortcuts.
Outside those contexts, letter/space shortcuts require no modifiers; `?` uses
Shift+Slash, and Ctrl+M opens the MIDI window.

## Keyboard and assistive controls

Tab and Shift+Tab traverse controls; focused custom controls show an outline and
scroll into view. Space or Enter activates the focused button. A focused pad
holds its note until that key is released or focus leaves; Space on a focused
control does not also change the session transport.

Arrow keys adjust a focused numeric control in its displayed units. Shift makes
fine adjustments, Home/End select the limits, and F2 opens a direct value editor
with Apply/Cancel. Invalid values stay in the editor for correction. The crate's
**Crate selection** control supports arrows, Page Up/Down, Home/End and direct
one-based row numbers through F2, including rows outside the visible viewport.
Enter loads its selected row onto the selected deck.

Shift+F10 opens the focused control's alternatives: cue deletion, loop out,
clip launch mode, compose arm, gain editing, mute/solo, and pad Press/Release.
The **Actions for …** button exposes the same menu to assistive tools through
ordinary buttons. Focus a control with alternate actions first to choose that menu's target. Assistive
click on a pad toggles a hold; **Release pad** explicitly ends it. Window focus
loss releases local pad holds.

Linux accessibility uses the existing eframe/AccessKit AT-SPI bridge. Names
include deck, track, scene and effect context; values and states are exposed in
the native tree. The automated private-bus fixture verifies real AT-SPI queries
and actions; Orca and human workflow qualification remain to be performed.
See [accessibility validation](docs/validation/issue-87-accessibility.md).

Unmapped live musical notes route to their captured selected track where the profile allows. MIDI clock reception reports ticks; tempo synchronization and clock output are not implemented. APC grids launch clips. Pioneer relative jog bindings decode forward and reverse movement using their documented centered value. NS7 wheel input is deliberately unmapped: its absolute-position protocol cannot use the old guessed relative-CC/pitch-bend bindings. See the [jog decoder evidence and hardware limits](docs/validation/issue-42-relative-jog.md).

The APC40 original and mkII use separate protocol-based input profiles for eight
track faders, the master fader, clip grid and five scene buttons. Other APC40
buttons are ignored; record-arm and track-select do not operate unrelated
controls. Device-name selection and synthetic MIDI tests do not establish
physical compatibility. Controller QA remains pending; see the
[APC40 mapping evidence](docs/validation/issue-38-apc40-profiles.md).

Ctrl+M shows each MIDI port's discovery/connection state and failure reason.
**Retry / rescan MIDI** checks current ports in a background worker and retries
failed connections without reopening working ones. Keyboard and mouse remain
available throughout. A disconnected device is detected on an explicit rescan
or when its input worker ends; automatic hotplug detection is not implemented.
See the [connection lifecycle validation](docs/validation/issue-73-midi-connections.md).

MIDI Start and Stop use the transport handlers. Received MIDI Clock ticks expose
their accepted count and last source in `midi_clock` status; this reception hook
does not synchronize tempo or phase. See the [MIDI framing validation](docs/validation/issue-41-midi-realtime.md).

The sampler offers **samples**, **analog**, **keys**, and **pad**. Samples use
factory Kit/Perc/Hits banks or your own banks. Analog is a fast saw/square bass, Keys blend saw and sine with
a medium release, and Pad blends sine with a detuned saw and a longer envelope.
Changing the selection affects new presses; held notes keep their original sound.
Sample pads **1–8** occupy the bottom row and **9–16** the top row, preserving the
piano layout's natural/accidental positions. Number N triggers bank slot N−1.
Hover a pad for its slot or piano MIDI note; blank piano positions are inactive.

Open the **Sampler bank editor** to create an empty bank, copy an original factory
bank, or edit a user bank. Select a local crate file and assign it to one of 16
slots. Prepare its gain and source-second start/end range, inspect the sampled
PCM waveform, and audition it through the selected track before **Apply bank**.
The waveform samples a bounded set of actual PCM positions; it can miss narrow
peaks. Cancel discards unapplied drafts; an already-applied edit remains applied.
Existing voices retain the
source, range, gain and destination captured at their onset.

**Save reusable** stores a named reference definition outside the project. Loading
it creates a new working bank; duplicate names do not overwrite each other.
There are up to 64 reusable definitions and 16 simultaneous project banks, with
bounded shared PCM storage. Reusable definitions need their original files or a
verified same-content library relocation; missing slots are shown explicitly and
never replaced with defaults. Native **Project Save** embeds the available PCM,
so a project remains playable when its original files are missing. User/project
samples keep their source rate when the audio output rate changes. Preparation
and reusable-store operations are unavailable during Performance protection.

Theme colors, shell base font size, and fontconfig's current monospace font
reload in a background worker, including font-only edits. The next valid update
replaces the applied style; incomplete writes retain the last valid settings.
See the [theme reload validation](docs/validation/issue-78-theme-reload.md).

The installed face resolved by `fc-match monospace` is used first for both UI
text families. Fontconfig may supply an installed substitute for an unavailable
family. If resolution or font-file validation fails, Omatainer keeps the last
valid font, or uses its bundled Ubuntu/Hack/emoji fallback before the first valid
selection. Bundled symbol fallbacks remain available alongside the chosen font.
See the [selected-font and glyph validation](docs/validation/issue-79-selected-font.md).

## Layout

```
Project / Edit · Preferences · Help · MIDI · diagnostics
deck A controls | waveforms + crossfader | deck B controls
sampler instruments, banks and held pads
searchable crate and explicit deck load targets
session clip grid + track gains, or selected effect rack
```

Not Ableton plus Serato with a bridge — one document. Decks are extra mixer buses that stay live while clips launch.

## Paths

| | |
| --- | --- |
| Binary | `~/.local/bin/omatainer` |
| Control socket | `$XDG_RUNTIME_DIR/omatainer.sock` |
| Config | `~/.config/omatainer/` |
| Plugin | `~/.config/omarchy/plugins/omatainer/` |
| Theme | `~/.local/state/omarchy/current/theme/colors.toml` |

The runtime directory must belong to the effective user, have mode `0700`, and
have trusted directory ancestors without symlinks. If `XDG_RUNTIME_DIR` is unset,
the app and CLI use `${TMPDIR:-/tmp}/omatainer-<effective-UID>/omatainer.sock` in a
private `0700` directory and print a warning. Invalid directories or endpoint
collisions fail without changing their owners, permissions, or contents. The old
shared `/tmp/omatainer.sock` endpoint is never used or removed.

```bash
omatainer ctl status
omatainer ctl reload-theme
omatainer ctl togglePlay
omatainer ctl scene 1 # scene numbers are 1 through 8
omarchy-shell -q omatainer togglePlay
omarchy-shell omatainer scene 3 # shell scenes are also 1 through 8
```

`ctl scene` requires exactly one integer from 1 through 8. The control socket's
JSON scene operation uses zero-based indexes instead: `{"op":"scene","n":0}`
launches scene 1, and `n` must be an integer from 0 through 7. Invalid scene
requests return `{"ok":false,"error":"..."}` without changing the session.

The shell service's `scene(n)` and public `omatainer.scene` IPC method also take
one-based scene numbers 1 through 8. They validate before queuing, preserve the
number as a separate CLI argument, and report invalid requests as `rejected`
with `accepted: false` without launching a control process. For example,
`omarchy-shell omatainer scene 3` runs `omatainer ctl scene 3`, which sends
`{"op":"scene","n":2}`. Queued/result records include the captured `arguments`
array so multiple pending scene requests remain distinguishable.

Control commands return `accepted: true` and `command_status: "accepted"` when
queued for audio processing. The response's state fields are the latest published
snapshot and may precede execution; use `ctl status` or `ctl follow` for updates.
Full or disconnected queues return `ok: false`, `accepted: false`, and an error.
Status queries have null `accepted` and `command_status` fields.

`ctl reload-theme` forces a fresh colors, shell-size and selected-font read on the
background theme worker. It returns `event: "theme", status: "applied"` only after
the GUI installs the valid result. Saved follow-theme, font-size override and UI
scale preferences remain authoritative; the response reports that effective
appearance. Invalid resources return a correlated error and preserve the usable
style. The server waits at most 3 seconds and this CLI operation reads for at most
4 seconds (ordinary control/status still uses 800 ms). A timeout after application
begins reports an unknown outcome; it does not claim to undo a visible change.

`ctl follow` maintains one read-only status connection, with at most four updates
per second. It reconnects after a server restart with bounded backoff (up to four
seconds), and exits cleanly if its output consumer closes. Followers share the
eight-client IPC limit; normal command connections retain their 32-request cap.

Performance diagnostics are available from **Diagnostics** in the audio status
bar. The panel separates callback deadlines, render CPU, UI update wall time,
predicted output latency and queue pressure. Optional sampled track/device
costs include their coverage limits. Start a bounded 30-second capture, then
export a redacted private JSON file or reopen one for inspection. Exact backend
XRUN counts remain unavailable. See [diagnostic measurement and export
semantics](docs/validation/issue-81-diagnostics.md).

**Edit → Undo / Redo / History** reverses supported creative edits while keeping
transport and physical performance gates separate. Continuous drags form one
entry unless another source edits between them. History shows applied/redo
entries, memory use and explicit rejected edits; Ctrl+Z and Ctrl+Shift+Z (or
Ctrl+Y) use the same actions. Text fields keep their own Undo. Returning to saved
content restores a clean project indicator; New/Open start a new history.
See [history and save semantics](docs/validation/issue-83-history-ui.md).

### Content provenance and licenses

Open **Content & licenses** in the application for factory sound/preset IDs,
embedded-font terms, component sources, commercial-use/redistribution summaries
and complete offline notices. Original factory sounds are generated in code;
reference-product assets, proprietary SDKs, IR files and ML models are not
bundled. Imported media and fonts selected from your system retain their own
terms. See [the provenance records](licenses/manifest.json) and
[validation details](docs/validation/issue-86-content-license-manifest.md).

Maintainers refresh reviewed records with `python3 scripts/license-manifest.py
update`, then rebuild. `check` verifies the locked native source/component
inventory. Create a retained release with `package --binary PATH --destination
NEW_DIR`, and check it with `verify-package --destination DIR`. The installer
validates and retains the same records, including an executable hash receipt,
under `~/.local/share/omatainer/licenses` and its per-install recovery journal.
`omatainer licenses --manifest` and `omatainer licenses --notices` print the
records embedded in the executable without opening audio or the desktop.

Release packaging and installation also require a passing local workload report
for the exact artifact. Run `python3 scripts/performance-gate.py run` after
reviewing the benchmark policy and refreshing the license inventory; `check`
recomputes the retained raw evidence. The report travels with the package and
installation. Draft policies, missing/stale evidence and failed checks block
publication. See [the local release gate contract and reproduction steps](docs/validation/issue-95-local-release-gate.md).

### Performance protection

Use the visible **Performance** bar before a show. Protection keeps playing/touched
deck loads, destructive edits, project replacement and device changes from being
applied; mixing, composing, recording, stopped-deck loads and project Save remain
available. Optional scans, imports, analysis, theme reloads and exports are deferred
or refused explicitly. The startup preference can enable protection next launch.

**Safe stop…** stops session and both decks, finalizes captured notes and releases
input synth gates; finite hits and effects decay naturally. **Emergency silence…**
adds a 2 ms fade to latched mute. Both require deliberate confirmation and display
renderer acknowledgment. Release physical inputs, then confirm **Recover inputs…**;
playback stays stopped. Emergency mute requires a deliberate **Reset stopped DSP
and unmute…** through the audio owner and remains latched if reset fails. You may
retain mute to Save and deliberately leave/close. Quiet tail readings and device
names do not establish physical hardware health. See the
[protection policy and validation](docs/validation/issue-96-performance-mode.md).


**Support and crash reports** is in the Project menu. Inspect the bounded JSON,
choose its categories, and confirm review before exporting a new local
`.omasupport.json` file. There is no upload client. Media, project content,
paths, titles, device names, raw errors, credentials and panic payloads are
excluded. Retained reports distinguish an observed Rust panic from an unclean
exit whose cause is unknown; exact retained recovery references open the normal
recovery preview.

Run `omatainer --safe-mode` to use a stopped project service without opening audio
or MIDI devices, external theme/library startup, or automatically restoring a
project. Open, recovery and Save remain available; engine controls are offline.
**Restart normally** completes Save/Discard/Cancel before restarting with the
saved setup. `--safe-mode --startup-check` is a bounded headless startup diagnostic,
not native GUI or hardware QA. See [support/privacy and validation boundaries](docs/validation/issue-102-support.md).

**Offline operation:** installed Omatainer needs no account or cloud session for
local media, native projects, built-in DSP, sampler banks, Help or license
notices. Native projects embed playable PCM; reusable bank definitions keep
source references, so unavailable files remain visibly missing until restored
or verified as relocated. Plugin hosting, provider streaming/downloads,
authentication and cloud transfer are not implemented. No credential store is
advertised or stubbed. Support exports are reviewed local files, not uploads.

License **Source record (external browser)** links deliberately leave the local
viewer; bundled notices stay available offline. Remote filesystems, display or
audio servers and host Unix proxies are separate dependencies. Building from
source can require previously obtained packages. See Help → Offline operation
and the [network-denied qualification boundary](docs/validation/issue-103-offline.md).
