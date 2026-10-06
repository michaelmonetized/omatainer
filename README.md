# omatainer

Omarchy-native DAW + live DJ surface. One clock, one mixer, configurable workspaces:

- **Session** clip grid with quantized scene and clip launches
- **Compose** a MIDI piano roll and explicitly armed pad-note capture, with stable note identities and undo
- **Two decks** with spinning platters, Serato-style waveforms, hot cues, loops, vinyl jog, sync, EQ, filter, and a crossfader

Hardware is optional. USB class-compliant MIDI is first-class. Profiles ship for Akai APC Mini / APC40 / MPK / MPD232, Pioneer DDJ-SP1 / DDJ-FX / FLX / SB / 400, and original Numark NS7, with a legacy partial NS7FX map. The [native original NS7 driver](drivers/ns7/README.md) supplies ALSA audio and MIDI on Linux. Controller feedback runs outside the audio callback. See the [connected hardware evidence](docs/validation/hardware-resurrection.md) for qualified I/O and remaining physical checks. Unmapped musical inputs can play notes where the profile allows; MIDI learn retains exact-port assignments in the active profile after Save. Keyboard and pointer controls remain available.

It reads the current Omarchy theme (`~/.local/state/omarchy/current/theme/colors.toml`) and font, registers in the keybind menu, and drops a Quickshell bar chip next to the rest of the shell.

## Licensed music providers

Open **Music providers** in Setup. Network access starts only after you accept
Free To Use's current non-commercial license and attribution requirement.
Search its public catalog, copy credits and preview non-premium music at its
original pitch. No API key or account is required. Premium and commercial use
need a paid license for Omatainer; none is configured.

This adapter supports one transient preview voice and no offline storage, stems,
recording/export or DJ decks. Remote tracks retain provider IDs and never enter
local crates or saved project media. Close the panel or press **Stop provider
preview** to cancel playback. See the [provider contract](docs/music-providers.md).

## Install on this machine

Open **Setup → Background jobs** to inspect measured progress and cancel one
captured request. Decode, analysis, indexing, provider requests, video work and
media exports share bounded worker admission. At most one deck decoder and two
optional workers run together, with 3 GiB of declared active reservations.
An explicit deck load gets the next worker turn and cancels optional work when
its reservation would otherwise block decoding. Linux workers use ordinary CPU
scheduling, nice 10 or lower priority, and idle I/O priority. Existing asset and
decoder limits still apply; the reservation total is not process memory usage.
See the [background work limits](docs/background-jobs.md).

```bash
~/Projects/omatainer/scripts/install-omarchy.sh
```

Then `SUPER+O` launches or focuses it. Right-click the bar chip to play/stop without opening the window.

The installer stages and validates the complete integration before publishing it.
Failed publication or desktop validation restores the prior files. Each successful
install prints a recovery journal under `~/.local/state/omatainer/installations`;
restore its saved state with `scripts/install-omarchy.sh --recover /path/to/journal.json`.
Running app instances retain their executable until you close and relaunch them.

## Score to video

**Setup → Video** imports one local picture using installed `ffmpeg` and
`ffprobe`. The preview follows the audio callback's project clock and can open
in a separate resizable window. Frame-based trim, placement, named locators and
timecode offsets persist in native projects; reopening verifies the same source.
29.97/59.94 support drop-frame timecode. A saved preview latency adjustment
accounts for your audio hardware and leaves exported timing unchanged.

Scrub by project frame or use previous/next frame and locator actions. A seek
ends clip recording and releases clip notes. Optional decoding pauses during
Performance protection; out-of-range or unavailable picture shows an explicit
black/waiting state. Video audio is excluded.

**Render selected scene against picture** loops the chosen Session scene from
project zero with fresh native mixer/effect state. A new output folder receives
48 kHz stereo float `score.wav`, lossless FFV1/PCM `picture.mkv` and
`alignment.json`, including exact frame/sample boundaries. Publication preserves
existing output, and Cancel removes unpublished staging. The live session stays
intact. **Remove picture reference** clears saved picture metadata while retaining
its source file. Portable archives currently require a saved copy with that
external picture reference cleared.

Supported SDR sources include H.264, HEVC, ProRes, VP8/VP9, FFV1 and MPEG-4 in
allowlisted local MOV/MP4, Matroska/WebM, AVI and MPEG-TS containers. Rates are
24/25/30/50/60 and 24000/1001, 30000/1001 or 60000/1001. Import checks every decoded
timestamp and bounds sources to one million frames and 4096×2160. Variable rate,
rotation, nonsquare pixels, tagged HDR and unsupported formats report errors.
Preview scales to 1280×720; WAV render is bounded to 2 GiB. FFmpeg is a system
dependency, not bundled software. See the [video workflow and validation boundary](docs/video-scoring.md).

## Preferences and profiles

Open **Preferences** in the status bar or press **Ctrl+,** outside an editor.
Choose Studio/Performance, duplicate or rename a profile, and select **Use this
profile** to stage a switch. **Preview changes** shows the requested audio route,
MIDI input policy and folder availability. **Apply and save** commits the draft;
**Cancel changes** keeps the current setup.

Audio device, sample format, rate, channel count and buffer size are saved separately
from the running output. Device loss retains the project and stops playback.
See [Recovering lost audio](docs/audio-recovery.md) for reconnect, fallback and
pending physical qualification. **Audio routing** adds stable channel aliases, independent track/deck/bus taps, confirmed input activation and worker-written WAV record sources. See [audio routing](docs/audio-routing.md) for supported widths and verification limits. Open **Audio devices and latency**, select **Preview saved
audio**, then **Use saved audio now** and **Stop and change output** to apply without
restarting. This stops performance and tries to restore the prior output on failure;
it never resumes playback automatically. If both devices fail, the stopped session
still supports Save, New/Open and Close while you recover an output. An unavailable
startup setup offers **Use system default audio this time** without changing the
saved profile. Projects without explicit routing send main left/right to outputs 1/2; mono sums both channels and extra
channels are silent. The original NS7's native four-channel ALSA output adds its
HEADPHONE MIX, HEADPHONE MODE and headphone volume controls on outputs 3/4.
Headphone volume starts at zero until the knob moves. Explicit output aliases
retain their channel assignments.

The audio window lists advertised input/output capabilities and distinguishes the
backend-accepted logical configuration from observed callback sizes. Physical
negotiated rate and converter latency are unavailable through CPAL. Optional
**Measure loopback** requires a suitable line-output-to-line-input cable/interface
route and explicit **Cable ready: stop and measure** confirmation. It plays three
low-level probes, never monitors input, rejects weak/ambiguous/corrupt evidence,
and reports qualified host callback-to-callback return timing, not converter-only
roundtrip latency. See [audio settings validation](docs/validation/issue-94-audio-settings.md).

Linux profiles also support **JACK**, using an existing JACK server or PipeWire's
JACK library. Named output/input ports, exact saved links and explicit reconnect
share the existing audio-owner workflow. The server owns rate, quantum and callback
scheduling. See [Linux audio backends](docs/linux-audio.md) for setup, saved routes,
supported bounds and software qualification limits.

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

**Setup → Automation** exposes the versioned local API and native job inspector. Use `omatainer ctl api` for discovery, state subscriptions, commands, atomic edits and musical scheduling. Optional authenticated loopback OSC is disabled by default. See the [automation protocol and examples](docs/automation-api.md).

The Library window also exports a fixed saved DJ catalog, optionally collects explicitly authorized local music, verifies the complete backup and restores into a new directory. Import remains explicit and rejects conflicts. See [library backup](docs/validation/issue-143-library-backup.md).

The [offline manual](docs/manual.md) is generated from the same catalogue. This build has no Arrange timeline, external audio recording, warp editor, automation lane editor or plugin host. Native projects, piano roll, pad-note capture, clip gain, history, mixer/FX and DJ preparation are implemented workflows.

## Play without files

The default session is a four-clip house sketch (drums, bass, keys, pad). Press **space**. Scenes **1** and **2** are filled.

Drop wav/mp3/flac onto a platter, or put tracks in `~/Music` and load with **F** / **→ A** / **→ B**.

### Reuse another project's material

**Project → Import from another project…** browses a native `.omat` without
replacing the current session. Select source tracks and scenes, choose whether
to copy clips/controller lanes and native instruments/effects, then Review and
Apply. The import appends new tracks and scenes as one Undo/Redo transaction.
Existing processors, audio playback and save path remain in place. Source files
are read only; all referenced clip and drum PCM accompanies the copied material.

Review keeps destination tempo/meter, preserves source beat/tick coordinates,
converts native playback to the current output sample rate and lists unavailable
devices that retain serialized state while bypassed. Imported tracks are disarmed,
not soloed and not auto-launched. Selected scene buses map to new source scenes;
other buses map to the destination's selected scene. Hardware profiles, decks,
global sampler banks, picture and the project-wide conductor are outside this
track selection. Source aliases are not installed into destination preferences.

The storage bounds remain 128 tracks and 512 scenes, including inactive slots.
Preparation stays on a cancellable worker. Source-file changes and creative
changes to the reviewed destination reject Apply; review again to use new state.


## Named project versions

Project → Named versions saves named snapshots and revision notes with shared immutable audio. Compare musical changes before restoring an unsaved copy or branching a new native project. Preview unreferenced assets before pruning. See [the version workflow](docs/project-versions.md).

## Native projects

**Project → Next live set…** preflights one embedded project while the current
mix continues. Standard stereo master routes use outputs 1/2; a four-channel
device can cue the next set on outputs 3/4. Review the fade and any unsaved edits
before **Transition to next set**. Both complete graphs render through the
linear fade, including outgoing effect tails, then the worker retires the old
graph. Cancel before commit preserves the current set. Safety stop and silence
remain available after commit.

Preload accepts up to 256 MiB of embedded audio, 32 MiB of metadata and the
existing 256 MiB processor bound. It refuses missing sampler audio and
unavailable processors. Leave performance protection deliberately before
preloading. Cue playback starts remembered clips and loaded decks; transition
continues from the preview position. Custom routing and physical transition
qualification remain outside this first path.

Use **Project → Save project as…** to choose a `.omat` path. Native projects embed
the session's media alongside clips/notes, instruments, mixer/effects, sampler
banks, deck cues/loops/positions, selections and the crate/panel view. The current
controller mapping schema is recorded. Global hardware preferences and controller
profiles remain local to each machine.

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
connections and DSP tails are transient. Default limits are 64 MiB metadata,
256 embedded media entries and 1 GiB PCM; unsupported or corrupt projects are
rejected without replacement. See the [project workflow evidence](docs/validation/issue-82-ui.md)
and [file format and atomic save rules](docs/validation/issue-82-codec.md).

**Project → Portable project…** lists every embedded sample, saved instrument,
effect and sampler preset, including exact settings hashes and unavailable device
identifiers. The application dependency catalogue and full distribution notices
travel with the archive. Audio redistribution rights remain unverified. Inspect, select each
original source to collect, then export to a new absolute `.ompack` path. Embedded
PCM is always included; selected originals use relative SHA-256 filenames and
duplicate source bytes occupy one entry. Original files and the current project's
saved baseline stay intact. Selected original reads are limited to 1 GiB and the
complete session/source payload to 3 GiB, with 64 MiB of manifest metadata.

Review an archive, choose a new imported folder with an existing parent, then
import. Checksums, embedded audio identities, device/preset identities and copied
source audio are verified before atomic publication. Existing paths are refused;
choose a new name after a conflict. Collected sources are rebound to that folder.
**Open imported project** uses the unsaved-work guard and starts stopped. Missing
effects bypass; unavailable instruments retain their state and notes, and clips
with rendered audio remain playable. Uncollected missing originals remain visible
in **Project dependencies…**. Cancel removes unpublished staging; a completed
publication is reported even if cancellation arrives afterward.

**Project → Project and track templates…** saves reusable `.omtemplate` files.
Project templates retain the complete creative session. Track templates retain
one track's devices, mixer and exact scene-bus alias while excluding song content.
Inspect before using or duplicating; all new files refuse replacement. Creating a
project uses the unsaved-work guard and a fresh session identity. Track application
keeps clips and other tracks, stops playback and starts fresh undo history.
The live project never uses the template as its Save destination.

Templates retain requested audio and MIDI names and optional port IDs. Missing
and ambiguous endpoints stay explicit. **Review template hardware in Preferences**
creates a draft for Preview / Apply; active connections stay on the current profile
until that deliberate step. Cancel changes preserves them. Track routing merges
only into the selected track and refuses stale or deleted target identities.

Preferences → Startup chooses **Demo session**, **Empty session** or a project
template for the next launch. Invalid or missing templates preserve saved settings
and offer **Start an empty session this time**. Safe mode ignores external templates.
Templates can be backed up to `.ompack` and imported into a new folder on another
profile or machine; embedded PCM remains playable without original source files.
Archives preserve native device state, desired hardware names and dependency
notices; existing destinations are refused and unpublished staging is cancelled.

## DJ library

Imports, scans and native deck loads read embedded title, artist, BPM and key.
Use **tags…** to inspect one selected track or a captured filtered batch of up
to 4,096 tracks. The review shows actual tag values, their source and explicit
filename fallbacks. Check only fields to change; a checked empty field clears
that value. Review the changes, then apply to those captured tracks. Browsing
another filter afterward does not change the batch.

MP3, FLAC, WAV and AIFF support guarded embedded writes when their original tags
and complete audio can be preserved. Unsupported, read-only, hardlinked or
otherwise unsafe media keep their bytes and use library sidecars. Fractional BPM
that a container cannot represent is retained exactly in the sidecar. Embedded
rewrites are limited to 128 MiB per file; larger files can still use sidecars.
User values and explicit clears take precedence over tags, analysis and filename
hints. A tagged BPM is supplied metadata, not a measured tempo or key estimate.

Each file completes separately. Cancellation retains earlier saved edits and
reports skipped or unconfirmed outcomes. Performance protection stops optional
work; a media write that already committed still receives its catalog receipt.
An interrupted embedded write retains its original and a private recovery record
until the catalog save is confirmed. Startup reconciles these records; conflicts
remain visible and preserve both versions. Repair/retry is available in **tags…**.

Use **analyze…** beside the crate search to prepare BPM, duration and a bounded
waveform for selected local files or mounted removable libraries. **Analyze selected row** captures one source;
**Analyze filtered crate** captures the current filtered order, up to 4,096 rows.
Later filtering or scanning does not retarget that queue. Choose individual
fields and **Force selected fields** to reanalyze them; manual BPM and locked
preparation remain authoritative. Musical-key analysis stores a separate,
version-bound estimate. The key column shows conventional and harmonic notation;
saved user or embedded keys take precedence. Inspect the cache for profile
correlation, ambiguity margin and unknown results, or compare against another
source key. Scores are not probabilities. Use Audio metadata to review a
correction; an explicit clear stays cleared. Pitch changes are not included in
the source-key comparison.

The shared media worker hashes and decodes one source at a time. Explicit deck
or sampler loads preempt background analysis; **Retry current analysis** resumes
the captured source. Cancel stops remaining work at safe boundaries, while a
publication already claimed reports its actual outcome. Prepared progress is
separate from catalog persistence. **Inspect selected cache** verifies stored
values and the waveform without starting a source decode. Missing/corrupt cache
or changed source versions remain explicit. Closing the panel leaves its queue
running. Optional analysis is unavailable during Performance protection.


Use **All tracks ▾** to open the **Named crates** manager. Search crate names independently of tracks, filter **Favorite crates only**, or **Find crates containing selected track**. Pin/unpin through the catalog save owner. Selecting a discovery result preserves the original query, track and scroll for **Return to previous crate view**. Learn **Browse crates** on a relative encoder and **Return to previous crate view** on a note button; captured IDs prevent queued input from retargeting another result.

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

**annotations…** stores ratings (0–5), colors, performance groups, tags and notes
against stable track IDs. Capture the selected track or the filtered batch,
select the fields to replace, review, then apply. Unselected fields keep each
track's values. Clearing a selected field is explicit. Edits use the catalog
save receipt and never write audio files or change loaded sound.

The crate table includes these fields. Ordinary search includes groups, tags and
notes; combine `rating>=4 tag:clean color:#FF6600 group:peak note:request` for
annotation filters. Each field accepts one predicate; search terms after a colon
are single words. Invalid predicates show an error. Saved annotation rules on
empty named crates update membership from the published library automatically.
The rule editor accepts phrases and preserves existing manual crates. Ratings,
notes and rules survive restart, relocation and library metadata export/import.

**library… → Import music files/folders** accepts one absolute path per line,
including individual files and nested folders. It merges readable supported audio
without stopping either deck. Progress shows skipped reasons and incomplete
coverage: at most 64 inputs, 64 folder levels, one million visited entries and
100,000 rows; the first 32 skipped paths are shown. Cancellation retains the
prior crate. The worker does not follow descendant symlinks.

**layout…** saves named library layouts in the active profile. Show or hide columns, move them with the editor’s arrows and resize their widths. Choose Compact, Comfortable or Artwork rows; Artwork reads real embedded PNG/JPEG covers for visible scanned local tracks. Missing artwork stays explicit, and **Retry artwork** refreshes the bounded cache after access changes.

Choose a primary and secondary sort with independent directions. Header clicks sort by that column; Shift-click adds a different secondary column. Missing metadata remains last, equal values keep their published or manual crate order, and sorting preserves selected-source and scroll anchors. **Manual order / none** restores the direct saved crate order. **Preview layout** changes the browser locally; **Save layouts** persists through the preference owner; **Discard layout preview** restores saved layouts. Preferences version 13 migrates older profiles without writing until explicit save. [Layout qualification](docs/validation/issue-144-library-layouts.md) records the software checks.

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
worker. Version 13 is the current format; versions 1–12 migrate without changing
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
**Deck grid…** edits a manual downbeat and up to 64 ordered tempo anchors independently of analyzed
BPM. Map each anchor's beat number to its source time; insert, replace or delete
anchors while previewing. Each segment must stay between 20 and 400 BPM.
Preview Set at playhead, slip, stretch, half/double tempo and Reset before
Apply; Cancel/Escape discards the draft. Applied grids are undoable, survive
library/project/recovery reloads, and guide Match phase and quantized loops.
Negative beat coordinates represent pickups; deck bars currently contain four beats.
Loops and synchronization follow local tempo segments. Cue positions stay in
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
| `A` | Hold Cue audition deck A |
| `W` | Toggle deck A tempo sync |
| `P` | Play / pause deck B |
| `L` | Hold Cue audition deck B |
| `O` | Toggle deck B tempo sync |
| `[` | Crossfader fully to A |
| `]` | Crossfader fully to B |
| `F` | Load selected crate item onto selected deck |
| `? (Shift+/)` | Show / hide contextual help and lessons |
| `Shift+[` | Beat jump backward on selected deck |
| `Shift+]` | Beat jump forward on selected deck |
| `Alt+[` | Smaller beat jump on selected deck |
| `Alt+]` | Larger beat jump on selected deck |
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

Hold the visible deck CUE button, focused Space/Enter or A/L to audition from pause. Release returns to the main cue; Play during the hold continues playback. Cue while playing stops and returns. MIDI Learn exposes Deck hold Cue audition separately from the existing one-shot Set/return Cue. Independent local and controller holds release separately. [Cue audition details](docs/validation/cue-audition.md).

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

Unmapped live musical notes route to their captured selected track where the profile allows. MIDI clock reception reports ticks; tempo synchronization and clock output are not implemented. APC grids launch clips. Pioneer relative jog bindings decode forward and reverse movement using their documented centered value. Original NS7 wheel positions use a stateful wrapping decoder; its first position establishes a reference without moving a deck. Motor and platter-touch control remain unimplemented. DDJ-SP1 has its own deck, pad, browser and paired 14-bit FX addresses. MPD232 preserves all programmable pad notes and velocities, with MIDI learn for its configurable controls; it does not assume an eight-pad MPK layout.

The APC40 original and mkII use separate protocol-based input profiles for eight
track faders, the master fader, clip grid and five scene buttons. Other APC40
buttons are ignored; record-arm and track-select do not operate unrelated
controls. Device-name selection and synthetic MIDI tests do not establish
physical compatibility. Connected mkII identity and feedback are qualified; physical fader and pad checks remain pending. See the
[APC40 mapping evidence](docs/validation/issue-38-apc40-profiles.md).

Ctrl+M shows each MIDI port's discovery/connection state and failure reason.
**Retry / rescan MIDI** checks current ports in a background worker and retries
failed connections without reopening working ones. Keyboard and mouse remain
available throughout. While performance protection permits device changes, a background rescan every 1.5 seconds discovers arrivals and removals without reopening working inputs. Protected performances retain their active connections; use the explicit rescan when device changes are permitted.
See the [connection lifecycle validation](docs/validation/issue-73-midi-connections.md).

**MIDI learn** selects an action and target, captures one compatible control, and previews its exact device/channel/message. Add or deliberately Replace an address, test its action through normal admission, edit/remove existing assignments, and **Save MIDI assignments** after review. Relative encoders require an explicit format and sensitivity. Cancel, Escape, close, timeout and disconnect restore normal input; stale capture-period packets cannot trigger later actions. Saved mappings require the exact port. See [learn behavior and qualification limits](docs/validation/issue-148-midi-learn.md).

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

**Edit sampler banks** saves independent Trigger, Hold or Toggle modes per slot. Trigger starts at the cue on every press and continues after release. Hold stops on the final release. Toggle stops on the next press. Repeat wraps to the captured cue or trim start. Prepare validates cue/range changes before Apply; sounding samples retain their captured source, mode and range across bank edits or selection changes. Project schema 12 and reusable-bank schema 2 retain these settings. Older valid banks migrate without writing until explicit Save.

**Stop slot** uses an independent reserved queue lane. Right-click a pad, press Shift+Escape on its focused pad, use the editor’s selected-slot stop, or learn a Note button for an exact sample slot. Other slots and transport keep playing. Release any held physical input before triggering it again. [Sampler qualification](docs/validation/issue-153-sampler-playback.md) separates software checks from pending USB input and listening.

Select an empty or MIDI clip cell and open **Piano roll**. Draw notes, drag their
bodies to move them and right edges to resize them, or use the named note fields
and action buttons. The focused roll supports arrows for movement/transposition,
Shift+Left/Right for length, Ctrl+A, Ctrl+D, M and Delete. Timing grids include
triplets and Free; note values use quarter-note beats. Pitch/time rulers, folding,
zoom and scroll have keyboard controls, and every note is available in the
accessible note list even when outside the painted viewport.

Clip start/end and loop start/end are explicit source-beat coordinates. Intro
notes play once before the repeating region; note gates close at explicit loop
or clip ends. **Apply MIDI edit** commits one captured-target History transaction.
Changed targets, active recording, project replacement and performance protection
reject it while retaining the draft. **Audition note** uses an independent voice
without recording; stop it with **Stop note audition**, loss of focus or close.
Cancel and Escape ask before discarding unapplied work. Native project saves
retain note identity, pitch, timing, velocity, mute and clip ranges. Existing
projects migrate identities deterministically and retain their original playback.

**Project → Import MIDI file…** inspects Standard MIDI Files before changing the
session. SMF 0 and 1 with PPQN 1–32767 are supported; format 2, SMPTE and RMID
require conversion. Map file tracks or individual channels into MIDI cells,
ignore rows, replace or merge notes, and keep the session conductor or choose
the complete file or an authoritative conductor track. Conflicting simultaneous
tempo/meter events require a choice. Applying a file conductor supports 40–240
BPM; keeping the session retains other source tempos for subsequent export.
Review unsupported SysEx/proprietary data before accepting its omission.

Native projects retain exact integer source ticks, note channels and on/off
velocities, controller/program lanes, text, key, tempo, meter and trailing file
silence. The native instrument uses the track's selected sound; controller and
program lanes are preserved for file interchange rather than changing that
sound. The piano roll shows exact source coordinates and retains them through
pitch/velocity/mute edits; moving or resizing a note replaces its source timing.
File conductor playback follows integer microseconds per quarter, updates
tempo-dependent master effects, and displays the current meter/bar/beat. The
metronome follows its denominator beats and bar accents. Manual tempo changes return to a constant conductor;
Undo restores the map. An entire mapped import is one Undo operation.

**Project → Export MIDI file…** selects up to 64 MIDI cells, SMF 0/1, PPQN,
muted-note inclusion and either the session or original source conductor.
Exports use source coordinates; clip launch/loop transforms are not flattened.
An inexact PPQN conversion requires explicit nearest-tick rounding. Exports
publish to an unused path atomically and never replace an existing file.
Inspection, preparation and publication run on a cancellable worker. Limits are
16 MiB per SMF, 128 file tracks, 262144 events, 8192 notes per cell, 65536 session
notes, 16 MiB of session MIDI lanes and 64 MiB of native project metadata.

**Edit session** manages audio/MIDI tracks and scenes: create, rename, reorder,
color, duplicate and delete, with Undo/Redo. Sets support up to 128 tracks and 512
scenes. **Go to track** and **Go to scene** use display order and reveal offscreen
cells; the larger grid renders only its visible rows and columns. Reordering
preserves playing clips, automation, controller slots and compose targets.
Duplication keeps musical settings and embedded audio but starts stopped.
Deleted or reused targets invalidate pending edits and MIDI routes; Apply/Retry
reattaches routing to the current track. Limits and refusals are visible. These
track types use the existing MIDI/embedded-audio paths; microphone recording
remains outside the current feature set.

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

Use **Lock playing deck A/B** to protect each live deck from accidental loading or ejecting. Review an override to identify the exact target and keep its current audio playing until the replacement is ready. See [deck load protection](docs/deck-load-protection.md) for the controls and IPC requests.

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
or verified as relocated. Explicitly enabled Free To Use search and licensed
non-premium previews require internet access; provider offline storage is
unavailable. Plugin hosting, account authentication and cloud transfer are
not implemented. No credential store is
advertised or stubbed. Support exports are reviewed local files, not uploads.

License **Source record (external browser)** links deliberately leave the local
viewer; bundled notices stay available offline. Remote filesystems, display or
audio servers and host Unix proxies are separate dependencies. Building from
source can require previously obtained packages. See Help → Offline operation
and the [network-denied qualification boundary](docs/validation/issue-103-offline.md).

Interface language and Unicode search are described in [international text](docs/localization.md). Preferences retain English, Spanish or German per profile; untranslated technical text has an English fallback.

Use [Commands and shortcut bindings](docs/shortcuts.md) to search native actions, capture logical keys, reset defaults and export bindings without other profile settings.

[Touch and pen input](docs/touch-and-pen.md) supports independently held performance controls, reported pad pressure and cancellation. The Linux backend pressure and physical-device limits are documented.

Use [Workspaces and panel windows](docs/workspaces.md) to save panel visibility, order, heights and secondary-window sizes through Preferences.

Deck waveforms offer linked 2/4/8/16-bar zoom, saved cue/loop markers and phase against the selected deck. Save waveform view retains it in preference schema 14. The output position estimate uses the backend’s reported time; unknown timing, changed media, key-lock and fades show renderer position explicitly. [Waveform qualification and limits](docs/validation/issue-135-waveform-timing.md).
