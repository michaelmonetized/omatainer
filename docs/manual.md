# Omatainer offline manual

Generated from the in-app help catalogue. Hardware observations remain separate from software confirmation.

## Audio setup

Start with the output quiet. Audio devices and latency reports the backend-accepted logical output, observed callback sizes and active project routing; physical negotiated rate and converter latency are unavailable through CPAL. Projects without explicit routing use outputs 1/2, mono sums them, and additional outputs are silent. Audio routing defines stable mono/stereo/multichannel aliases, independent tracks, buses, deck taps and record sources. Review and confirm changes while stopped; Save and Open retain them. Physical channel meters and quiet channel tests help verify connections. Live inputs require separate preview and confirmation. Settings profiles separate saved preferences from running audio: Apply and save persists audio choices without changing the stream. Preview saved audio, Use saved audio now, then Stop and change output explicitly stops performance and changes the stream, or restart to use the saved setup. A failed change restores the prior output when possible; if rollback also fails, the retained session can still Save, New/Open and Close. Playback never resumes automatically. Optional Measure loopback requires a suitable line-level cable/interface route and explicit confirmation; it reports qualified host callback-to-callback return timing only after three reliable probes, not physical converter roundtrip. Cancellation, missing loopback and corrupt evidence produce no current measurement. The main mixer is stereo; independent routing can address additional physical outputs. the cue blend does not provide a separate headphone output. Load a built-in stem, play it, then verify sound at your physical output. Connection and meter evidence cannot prove that speakers or headphones are audible.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Inspect the running output in Preferences, then open Diagnostics and wait for a completed audio callback. If saved audio changes are pending, preview and explicitly confirm them in Audio devices and latency, or restart. A saved profile is not the running output.
2. Load Drums (session) or Harmony (session) onto a deck and play it. Wait for renderer-confirmed playback of that accepted load.
3. At a quiet level, verify the sound at your physical speakers/headphones. Mark only your own listening observation; the app cannot measure it.

## Record a held note

Choose an empty clip cell and use its Arm compose action (Shift-click or Shift+F10), or Arm selected cell. The armed destination is displayed above the sampler and stays fixed while browsing. Choose Keys, Analog or Pad; hold a pad and release it to record its actual duration. Stopped composition starts at local beat zero. Playing clips capture launch-relative positions. Disarm finalizes captures; Stop also disarms. Launch the resulting clip to hear it later. This is MIDI/pad note capture, not microphone or external audio recording. Open Piano roll on the selected empty or MIDI cell to draw, select, move, resize, transpose, duplicate, mute or delete notes. Apply commits one captured-target MIDI edit; Cancel preserves the current clip. Triplet/free grids, numeric clip/loop bounds and note values, scale folding, zoom and scroll are available. Audition is independent and never records.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Choose an empty clip and explicitly Arm compose (Shift-click or alternate action). Watch the armed track/scene label; browsing alone does not arm.
2. Choose Keys, Analog or Pad. Hold a sampler pad long enough to observe the held capture. Wait for a new note in the original armed cell.
3. Release the same pad. Wait for its recorded duration to finalize; changing selection cannot redirect the release.
4. Disarm compose. The recorded note remains in its original cell.
5. Launch that original clip. Wait until it is playing, not merely queued. Stopped note entry used beat zero.

## Edit and undo

Open a clip's alternate actions and choose Edit clip gain. Gain changes future note onsets; already held notes retain their onset gain. Use Edit → Undo / Redo, or the active shortcut bindings, to compare. A continuous slider gesture is one named history entry. History is bounded; admission failures or truncation are shown explicitly. Transport and physical held gates are not creative edits. Piano roll offers note drawing, pointer/keyboard edits and explicit clip/loop ranges with stable note identities. It uses the same session History after Apply. This build offers frame-based picture placement alongside Session transport; it does not offer an Arrange timeline, warp editor, automation lanes or plugins.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Select a filled clip before starting this lesson. Open Edit clip gain from that cell's alternate actions (or Alt-click). Empty cells cannot demonstrate gain editing.
2. Change the target clip's gain and release the gesture. Wait for the renderer-confirmed value and its history entry.
3. Use Edit → Undo. Wait for the original target gain and the matching history position to return.
4. Use Edit → Redo. Wait for the changed gain to return. Unrelated held inputs and transport remain owned by their sources.

## Mix a session

Launch a scene, then adjust a track's gain. Track headers toggle mute; alternate actions provide solo and its effect rack. Muted tracks keep advancing, so unmuting returns to the current musical position. Scene effects process track audio assigned to that scene; monitoring and tails follow the track’s scene bus. Master effect slots process the stereo bus. Observe meters and callback diagnostics while adding load. Render CPU, callback elapsed time and UI frame time are different measurements; deadline overruns are not a backend XRUN count.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Launch a populated scene or clip. Wait for session playback and a clip past its queued boundary.
2. Adjust the gain of the track selected when this lesson started. Its output meter and effect chain remain on that track.
3. Toggle that track's mute using its header or alternate action. Observe the renderer-confirmed mute state.
4. Restore that track's original mute state. Playback kept advancing while muted; no elapsed hits are replayed.

## Prepare a DJ deck

Select a crate row and explicitly load A or B. Built-in Drums (session) and Harmony (session) work without media files. Wait for Loaded: Queued only confirms admission. The filename BPM/key hints and heuristic analyzed BPM have distinct provenance and are not a verified beat grid. Use the grid editor to set the downbeat, slip beat lines, stretch tempo or correct half/double ambiguity in a draft preview. Apply creates one manual uniform four-beat grid; it does not overwrite analyzed BPM or move cues. Cancel or Escape discards unapplied drafts. Set a main cue or hot cues, set loop in/out, then audition. Pitch lock separates pitch from tempo. Match follows the crossfader-favored deck's effective tempo. Remaining time is an estimate and repeating loops suppress runout alerts. Preparation persists in the library for the exact file identity.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Load a built-in stem or your file onto the deck selected when this lesson started. Wait for Loaded; decode/queue errors do not count.
2. Start that deck and wait for actual source playback. The accepted media receipt must remain current.
3. Set a previously empty hot cue, or remove and re-set one. The deck's cue indicator must change from the initial state.
4. Pause the same deck. Its preparation stays attached to this media identity; next try loop in/out and pitch lock using the reference.

## Connect a controller

Open the MIDI window and Retry / rescan MIDI. Settings can request all inputs, selected exact port names or disabled inputs. Check requested versus applied policy and per-port errors before playing. The native callback only queues bounded input; overflow releases that source's gates and reports counters. Factory profiles are limited, documented mappings; a matching device name is not physical compatibility proof. NS7 motorized platter and NS7II support remain unverified. MIDI clock input is an observable tick hook, not tempo synchronization or clock output. There is no editable MIDI-learn mapping UI.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Open MIDI. Inspect connections and errors; use Retry / rescan MIDI after connecting your controller. Missing hardware is not a passed check.
2. Operate a supported control and wait for new handled MIDI input. Input traffic only proves dispatch, not the meaning of a physical control.
3. Verify the intended on-screen/audio action and matching release on your actual device. Mark only your own observation. Unsupported controls and motorized NS7II behavior remain unverified.

## Stop and recover

Enable performance mode to protect playing/touched deck replacement, destructive edits and project/device changes across GUI, MIDI and IPC. Continuous mixing, live composition/recording, stopped-deck loads and Save remain available; optional scans/imports/analysis/theme/export work is refused or deferred. Safe stop deliberately stops the session and both decks, finalizes held captures and releases input notes; finite sample and effect tails continue naturally. Emergency silence additionally fades output to latched mute over 2 ms. Wait for renderer acknowledgment: request acceptance is not silence. Release physical keys, pads and touches, then explicitly acknowledge input recovery. This is your report, not a hardware health check. Emergency mute remains until a deliberate stopped DSP reset through the audio owner; a two-second −80 dBFS tail observation cannot prove bypassed/delayed history is empty. Failed reset keeps mute; you can acknowledge inputs and remain muted to Save and deliberately leave protection or close. No action automatically resumes playback. Ordinary Session Stop retains its narrower session-only behavior. Read Diagnostics queue/reset counters and explicit failures before recovery; physical hardware QA remains separate.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. With an audition already playing, identify session and deck playback separately. This lesson never starts audio for you; Cancel is safe at any time.
2. Stop the session, pause every playing deck, and disarm compose. Release physical keys/pads/touches separately; session Stop preserves unrelated held live gates.
3. Open Diagnostics and read callback/queue errors. Then verify that your physical inputs are released and the output is safe; tails may finish naturally.

## Save and reopen

Project → New, Open and window close protect unsaved work with Save changes / Discard changes / Cancel. Save project as chooses a .omat path; replacing an existing file requires the checkbox. Save copy leaves the current path and unsaved baseline unchanged. Native projects embed playable media and creative state. Reopen restores playback stopped and excludes physical held keys, connections and DSP tails. Later edits during saving remain dirty. Cancel before commit preserves the old document; a completed commit is reported honestly. Autosave and recovery keeps separate full edit-state batches about every two seconds and compacts checkpoints at the saved profile interval. Capture and disk delays can increase the loss window; read the last confirmed durable age. At restart, preview an inactive recovery and restore as a stopped, unsaved untitled copy without overwriting the original saved version. Full storage stops new writes visibly while preserving existing recovery. Project dependencies checks every embedded source and unavailable device; moved sources require explicit verified choices. Portable project lists samples, saved device and preset identities, retained application dependency notices and unverified audio rights. Export includes all embedded PCM and copies only explicitly selected originals into one checksum-verified .ompack archive. Import verifies and prepares the session in a new folder, refuses existing destinations, and rebinds collected originals locally. Opening the imported session stays stopped and uses the unsaved-work guard. Use a private test destination for the guided example.

Guided example (use Start this lesson in Help; Next requires observed evidence):

1. Use Project → Save project as… and a private new .omat path. Wait for a successful clean save; Save copy intentionally does not satisfy this step.
2. Use Project → New. Respond to any unsaved-work prompt deliberately. Wait for the empty replacement; cancellation or failed replacement does not count.
3. Open the exact saved file through Project → Open or Recent projects. Wait for the clean document with playback stopped; its stored media needs no original files.

## Offline operation

Omatainer starts without an account or cloud session. Locally available media, native projects with embedded PCM, user sampler banks, built-in instruments/effects, Help and license notices work locally. Reusable bank definitions reference local sources: retain those files or use a verified library relocation; missing sources remain explicit. Native projects embed their playable audio. Free To Use search and licensed non-premium previews require explicitly enabled internet access. Provider offline storage is unavailable. Audio plugin hosting, cloud transfer, account authentication and credential storage are not implemented. No account tokens are collected. Preferences, project envelopes and support reports reject unknown credential fields; project names, paths and audio are intentional user content, not redacted documents. Support export is a reviewed local file with an allowlisted schema, never an upload. License Source record links explicitly hand off to an external browser; that browser and host Unix proxies are outside the network-denied application test. Network filesystems, remote display/audio servers and source-build package downloads also need their own availability. Safe mode starts stopped without audio/MIDI devices; it can Open, Recover and Save locally. Network-denied fixtures qualify local software paths, not physical controllers, OS audio dropouts or perceived quality.


## Control reference

### Prepare upcoming tracks

4096 tracks and 2 MiB of captured references

Queue a selected track or filtered crate while other decks play. Reorder or remove upcoming tracks, load their captured file version through ordinary deck protection, and preview a loaded paused deck on its existing output. Stop preview restores its position. Remove-after-play requires a complete 10 ms window of confirmed digital main output from a playing deck; loads, paused previews, silence and uncertain measurements retain tracks. Retain keeps them until manual removal. Save creates one ordered manual crate; explicit restore uses its current catalog versions. Shift with deck A Load queues the selected track; shift with deck B Load queues the published filtered crate on profiles with those controls. Physical controller mappings still need device validation.

Workflow: Prepare a DJ deck.

### Library media health

Captured read-only validation; at most 4096 rows per queue

Validate a selected row or the filtered crate without replacing live decks. The existing decoder worker reads each complete file and discards packet PCM. Status distinguishes missing, unreadable, unsupported, corrupt and changed sources. Read-only audio remains playable and uses sidecar preparation. Bad tags are reported separately when audio still decodes. Results describe the last checked version; rescan and validate after media changes. Cancel stops remaining work; earlier results stay visible. Performance protection or an explicit media load stops validation. Closing the panel leaves an active queue running.

Workflow: Prepare a DJ deck.

### Interface language

English, Español or Deutsch; saved per profile

Select the display language for this profile. Preview and Cancel retain the applied language; Apply saves it and changes labels on the next frame. Musical values and stored names/paths remain unchanged. Advanced text without a translation uses English. Mixed-direction shaping is limited by the native renderer.

Workflow: Audio setup.

### Preparation locks

Reviewed grid, BPM and metadata protection

Capture current library versions, select lock fields, Review and Save. Grid locks prevent manual grid edits and Undo until unlocked. BPM and metadata locks exclude automatic replacement, including forced analysis and tag refresh. Reviewed manual tag edits remain available; unselected locked fields stay intact. Preview analysis replacements in the analysis panel and actual tag refresh replacements in the tag editor before refreshing. Locks are catalog sidecars and survive restart. Replacement content starts with fresh preparation.

Workflow: Prepare a DJ deck.

### Track ratings, colors and annotations

Stable track metadata; ratings 0–5

Capture selected or filtered stable IDs, select changed fields, Review and Apply. Ratings, colors, groups, tags and multiline notes are stored in library metadata; audio bytes and loaded sound are unchanged. Unselected fields preserve each track’s own values. Search also matches annotations; explicit filters include rating>=4, tag:clean, color:#FF6600, group:peak and note:request. Cancel can stop a pending write before its publication claim; closing keeps admitted writes running.

Workflow: Prepare a DJ deck.

### Smart crate rules

All or Any typed conditions; inclusive numeric ranges

Select an empty named crate and open Smart crate rules. Choose supported text, key, BPM, duration, rating or confirmed-played conditions. Preview current results before Save. Membership is updated on the catalog worker when current metadata changes. Refresh recomputes saved membership explicitly. Cancel stops a preview without saving; Remove leaves an empty manual crate.

Workflow: Prepare a DJ deck.

### Automatic annotation crate rules

Saved predicates over current library annotations

Select an empty named crate, choose minimum rating, optional color, group substring, exact tag and notes substring, then save its rule. Conditions combine and membership updates with annotations after reload or restart. Manual member edits are unavailable on a rule crate. Removing the rule leaves an empty manual crate; populated manual crates are protected from conversion.

Workflow: Prepare a DJ deck.

### Import local picture

FFmpeg/ffprobe; supported constant frame rates

Import one local SDR video after checking every decoded timestamp. Variable rate, rotation, unsupported codecs or missing tools produce an error. Audio from the picture is excluded. Save the native project to retain its external picture reference.

Workflow: Edit and undo.

### Picture preview

Resizable native preview and detached viewport

Picture follows the per-callback audio seconds clock. A saved preview latency adjustment (-2000 to 2000 ms; negative delays picture) can account for audio hardware; it does not shift exports. Outside trim is black; decoder latency shows a waiting frame rather than stale picture. Performance protection suspends optional decoding.

Workflow: Edit and undo.

### Picture trim and placement

Zero-based video frames; exclusive trim end

Apply a valid source trim and project placement. Timecode offset is signed frames; 29.97 and 59.94 support drop-frame timecode. Seek ends clip recording and releases clip notes while preserving physical input ownership.

Workflow: Edit and undo.

### Picture locators

128 named project-frame locators

Add or seek saved frame positions. Names contain 1–128 control-free UTF-8 bytes. Native project reopen verifies the same picture source before decoding.

Workflow: Edit and undo.

### Render selected scene against picture

New output folder; 48 kHz stereo float score

Render the selected Session scene from project zero with fresh DSP state, including native mixer/effects. Publish score.wav, lossless FFV1/PCM picture.mkv and alignment.json together. Picture audio is excluded. WAV is limited to 2 GiB; source and live project remain intact.

Workflow: Edit and undo.

### Cancel video work

Pending decoder or render

Stop and reap the owned decoder on its worker. Cancellation before publication removes staging output. A directory already published retains its committed result. Existing output folders are never overwritten.

Workflow: Edit and undo.

### Licensed music providers

Explicit non-commercial consent; one original-pitch preview

Enable the public Free To Use API only after accepting its current non-commercial license and attribution requirement. Search current metadata, copy credits and preview non-premium music. Premium and commercial use need a paid license for this app; none is configured. The adapter supports no offline storage, stems, recording, export or DJ decks. Close or Stop cancels preview audio. Network and decoding run on a bounded worker; safe and performance modes block these jobs.

Workflow: Prepare a DJ deck.

### Audio routing

Saved aliases and explicit channel maps

Inspect and edit project routes for tracks, scene effects, decks, buses, inputs, outputs and record sources. Choose pre FX, post FX or post mixer taps. Missing physical channels stay silent; cycles, invalid maps and changed project revisions reject the entire draft. Stop transports before confirmation. Apply is undoable and saved with the project. The pinned Linux backend advertises up to 32 live channels; saved addresses through 64 remain available for absent-channel preservation.

Workflow: Audio setup.

### Live routed input

Exact device at the active output nominal rate

Saved input choices never open automatically. Preview fresh advertised capabilities, then explicitly stop and enable input. Faults, missing callbacks and output changes disable input and require fresh confirmation. A bounded queue reports missing frames and overflow; separate device clocks are not resampled. Safe mode disables device access.

Workflow: Audio setup.

### Capture a record source

1–26 channels; float WAV; 128 MiB maximum

Apply and refresh routes, choose a record alias and new WAV path, then capture on a worker. Stop keeps completed audio; Cancel removes the partial file. Missing frames, overflow, invalid samples and file failures prevent incomplete publication. Existing files are preserved. Routing, project, audio and emergency-stop changes finish the current capture.

Workflow: Record a held note.

### Test a physical output

One second at −40 dBFS; stopped transports

Play a ramped 997 Hz tone on one active physical channel. Starting playback, cancellation or performance protection ends it. The tone and every independently routed output obey emergency silence. Digital meters and private software loopback do not prove hardware converter or interface behavior.

Workflow: Audio setup.

### Automation API

Versioned local and loopback control

Inspect the private local socket, stable session IDs and API results. Command acceptance is pending until its job reports applied. The native inspector uses the same typed API as scripts.

Workflow: Audio setup.

### Read API state

Stable IDs and reviewed revision

Read a bounded state page from automation version 1. Names may be shortened here; native project data stays complete.

Workflow: Audio setup.

### Schedule API action

Absolute quarter-note beat

While transport plays, schedule selected-track gain on a future musical beat. Stops, safety changes and project replacement invalidate pending actions. Audio executes from fixed storage without network IO.

Workflow: Audio setup.

### Scheduled API gain

0–1.5

Choose the selected track gain for an explicitly submitted future action. Moving this slider does not change audio.

Workflow: Audio setup.

### API action beat

Absolute quarter-note beat

Choose a future beat within 16384 quarter notes. Next beat fills a conservative upcoming boundary; past or stopped-transport scheduling is rejected.

Workflow: Audio setup.

### API track name

Atomic reviewed edit

Rename the selected native track through the version-1 API using its stable ID and the displayed project revision. Stale concurrent edits reject; an applied rename is undoable.

Workflow: Audio setup.

### Refresh API job

Pending/applied/rejected/cancelled

Read the last submitted job. Receipts survive controller reconnect while retained in this launch. Pending jobs are never evicted; completed results may expire as new jobs arrive.

Workflow: Audio setup.

### Cancel API job

Before audio claims the commit

Cancel a pending job atomically. Completed or claimed commits refuse cancellation rather than reporting an undo.

Workflow: Audio setup.

### Show OSC access token

Current loopback adapter only

Reveal the current 128-bit access token for an explicitly enabled loopback client. It changes after disable/re-enable or app restart and is never saved in preferences.

Workflow: Audio setup.

### Enable loopback OSC

127.0.0.1 UDP; saved profile

Preview and Apply explicitly to enable the authenticated OSC adapter. It binds only loopback, requires the current token and carries typed API JSON in an OSC blob. No LAN listener is opened. Cancel preserves the live listener.

Workflow: Audio setup.

### OSC port

0 automatic; otherwise 1024–65535

Save the loopback UDP port. Bind failure reports the error and preserves the previous listener. The actual selected port is shown in Automation.

Workflow: Audio setup.

### Retry OSC settings

Saved intent versus actual listener

Retry the saved listener intent after a bind or worker failure. Existing live configuration remains until a replacement is ready.

Workflow: Audio setup.

### Compact control menus

Small viewports and large text

Safety (Guard, STOP or MUTE when active), File, Setup and FX menus retain the full controls in scrollable menus when four toolbar rows would crowd the performance surface. Tab focus scrolls performance controls into view. Use Setup or Ctrl+, for Preferences.

Workflow: Offline operation.

### Display contrast

Desktop, dark or light palette

Choose a high contrast palette for dark rooms or bright stages. State cues use text and shapes as well as color. Preview and Apply save the choice; Cancel keeps the current display.

Workflow: Offline operation.

### Reduced motion

Decorative animation

Stop decorative platter rotation and UI transitions while retaining live transport values, waveform positions and audio. No playback command is sent.

Workflow: Offline operation.

### Waveform contrast

1 = original paint; 3 = foreground

Raise trace visibility without changing audio amplitude, seek positions or waveform geometry.

Workflow: Prepare a DJ deck.

### Level contrast

1 = original paint; 3 = foreground

Raise the visibility of the smoothed signal activity strip. It is not a calibrated peak, loudness or clipping measurement; changing its contrast does not change audio.

Workflow: Prepare a DJ deck.

### Templates

Native editable templates

Open native project and track templates. New template files and duplicates never replace an existing file.

Workflow: Save and reopen.

### Project versions

Named snapshots and alternatives

Save a named snapshot with revision notes and shared immutable audio. Compare tracks, clips, routing and devices before restoring an unsaved copy or branching a new project. Preview unreferenced assets before pruning.

Workflow: Save and reopen.

### Project import

Reuse selected project material

Browse a native project, select tracks and scenes, review timing and device conflicts, then apply one undoable import. Source files and existing tracks remain intact.

Workflow: Save and reopen.

### Template file paths

Native editable templates

Choose an absolute .omtemplate file, a new backup archive or a new import folder.

Workflow: Save and reopen.

### Save template

Native editable templates

Capture project state or selected-track devices and routing. Track templates exclude notes, clips and launch state.

Workflow: Save and reopen.

### Inspect template

Native editable templates

Validate the native file and retain its exact fingerprint, dependency manifest and desired hardware names.

Workflow: Save and reopen.

### Use template

Native editable templates

Create a fresh stopped unsaved project, or replace only selected-track configuration. Track application preserves music, stops playback and resets undo history. Hardware requires separate explicit review.

Workflow: Save and reopen.

### Duplicate template

Native editable templates

Write a new named template without changing the source file or active session.

Workflow: Save and reopen.

### Review template hardware

Native editable templates

Copy exact requested endpoints into Preferences draft. Preview and Apply explicitly; Cancel preserves current connections. Missing or ambiguous ports are reported without substitution.

Workflow: Save and reopen.

### Template backup and import

Native editable templates

Export embedded playable audio and metadata in a checksum-verified portable archive. Import into a new folder; existing destinations are refused.

Workflow: Save and reopen.

### Cancel template operation

Native editable templates

Cancel before publication or commit to preserve existing files and the current session. Completed publication is reported honestly.

Workflow: Save and reopen.

### Startup session

Native editable templates

Choose Demo, Empty or a native project template for next launch. Invalid templates preserve settings and offer an explicit empty session for this launch. Hardware remains on the active profile until reviewed.

Workflow: Save and reopen.

### Portable project

Native session and dependency manifest

Open the portable project workflow or a published imported session. Opening a session uses the existing unsaved-work guard and never starts playback automatically. Global device preferences and mappings are unchanged.

Workflow: Save and reopen.

### Inspect portable dependencies

Every sample and device/preset identity

Capture the native session, verify external sources and list every embedded sample, saved device ID and preset/settings checksum. Application dependency records and full distribution notices are retained in the archive. External audio and plugin redistribution rights remain unverified; no plugin binary is copied.

Workflow: Save and reopen.

### Collect original source

One explicit selection per asset

Choose a verified original source to copy into the archive under a relative SHA-256 filename. All embedded audio is included regardless of selection. Selected original reads are limited to 1 GiB; duplicate bytes occupy one archive entry. Originals are never changed.

Workflow: Save and reopen.

### Portable project paths

Absolute archive path and new folder

Choose an absolute archive path or an absolute new imported folder with an existing parent. Existing files and folders are refused. Choose a new name after a conflict.

Workflow: Save and reopen.

### Export portable project

Checksum-verified .ompack archive

Export the inspected musical revision with embedded audio, controls, notes, automation, unknown device state and selected originals. Changed revisions require inspection again. Export leaves the current session and its saved baseline intact.

Workflow: Save and reopen.

### Review portable archive

Untrusted metadata preview

Read the bounded dependency manifest before import. Preview does not prove the payload. Import verifies the complete archive, every entry, embedded PCM, selected source audio and device identities.

Workflow: Save and reopen.

### Import portable project

New private folder; no replacement

Verify and prepare the native session privately, bind collected originals to the new folder and publish it atomically without replacing an existing path. Unavailable effects bypass; unavailable instruments remain silent unless clips retain rendered audio. Uncollected missing originals remain reported, while embedded audio stays playable.

Workflow: Save and reopen.

### Cancel portable operation

Preserve current session and existing destinations

Cancel copying, verification or staging before publication. A package already published remains complete and reports success even if cancellation arrives later. Close waits for the worker result; temporary staging is cleaned up.

Workflow: Save and reopen.

### Project dependencies

Embedded audio and retained device state

Inspect source availability and unavailable devices. Native projects keep embedded audio, clips, automation and device state. Choose moved sources explicitly and save the project after relinking.

Workflow: Save and reopen.

### Check project dependencies

Captured project revision

Check every embedded source against its exact decoded audio identity and report unavailable device IDs and saved state. Disk work runs in a cancellable worker. Changed projects refuse stale results.

Workflow: Save and reopen.

### Search moved sources

1–64 absolute folders

Search selected folders for exact decoded audio, including renamed files. Unsupported/non-audio files are counted separately. Symlinks, nested foreign mounts, damaged audio and read errors produce a partial result; additional matches may exist. Successful decodes charge their actual PCM size; failed decodes retain conservative work credit.

Workflow: Save and reopen.

### Review replacement source

One explicit choice per asset

Compare candidate paths and choose each replacement. Duplicate matches remain unresolved until you choose. Keep current source reference clears an unapplied choice.

Workflow: Save and reopen.

### Apply reviewed relinks

Verified complete batch

Recheck every chosen file, decoded audio and mount before updating project source aliases as one batch. Failure or cancellation preserves all current references. Save the project to retain completed relinks.

Workflow: Save and reopen.

### Cancel dependency operation

Preserve current source references

Cancel worker inspection, search or verification before its result is applied. Completed relinks remain in the project.

Workflow: Save and reopen.

### Close dependency report

Explicitly retain or discard pending review

Close the report when idle. Pending choices and operations require explicit discard before close or project replacement. Discard cancels pending work and preserves previously applied source aliases. Choose the project action again after resolving the review.

Workflow: Save and reopen.

### Tempo and meter

Shared quarter-note timeline

Open the native timing draft. Tempo ramps rise or fall linearly in BPM to the next point. Meter markers and pickups change bar labels without moving musical events. Apply is one guarded History entry; save the project to retain it.

Workflow: Edit and undo.

### Tempo and meter points

Ordered beats on the source PPQN grid

Tempo rows contain beat, BPM and step/ramp. Meter rows contain beat and numerator/denominator. Both lists start at zero and allow up to 4096 points. Tempo is 40–240 BPM; the final tempo point uses step. Meter denominators are powers of two through 128. Invalid rows refuse the complete edit.

Workflow: Edit and undo.

### Pickup and click settings

Quarter-note pickup; meter-beat subdivisions; full count-in bars; linear gains

A pickup is shorter than the first bar and precedes its first meter change. Subdivisions are 1, 2 or 4. Count-in is 0–4 full bars at the starting meter and tempo, holding clips and recording while decks continue. Accent and beat gains are 0–2; zero silences that click voice.

Workflow: Record a held note.

### Apply timing

Atomic renderer-confirmed edit

Prepare and validate the whole map on the worker, then apply it as one History entry. Changed project identities, stale timing, active recording, held notes, count-in and performance protection refuse the edit. Queued is not Applied. Save retains exact ramps; MIDI export samples ramps at each MIDI tick with a 131072-tick bound.

Workflow: Edit and undo.

### Cancel timing operation

Cancellation before renderer ownership

Cancel preparation or a queued edit. An already claimed edit reports its actual completed outcome and remains in History. Unapplied drafts survive cancellation and failure.

Workflow: Edit and undo.

### Close or discard timing draft

Explicit unapplied-work decision

Close, Escape, application exit and project replacement preserve dirty drafts until Keep or Discard. Discard requests cancellation and closes only after pending work settles. Choose the project action again afterward. Completed edits remain in History; reopen the editor to inspect current timing.

Workflow: Edit and undo.

### Performance history

Independent saved DJ sessions

Inspect renderer-confirmed digital main-output contribution. Session history is independent of project Save and Undo. Hardware delivery and human listening require separate verification.

Workflow: Prepare a DJ deck.

### Start performance session

Actual output boundary

Begin only when the audio output callback acknowledges the request. Already loaded decks are included as unplayed until measurable main-output contribution. Start never starts playback.

Workflow: Prepare a DJ deck.

### End performance session

Actual output boundary and durable save

Finish pending measurement windows at the renderer boundary and save the session. Playback continues. A save failure remains pending; ending alone is not proof of persistence.

Workflow: Prepare a DJ deck.

### Select performance session

Stable session identity

Inspect one saved or active session without changing playback. Interrupted sessions retain their last saved prefix and are marked incomplete.

Workflow: Prepare a DJ deck.

### Played status override

Played / unplayed / measured

Set a manual assertion on this captured session and entry, or restore automatic status. Measured duration is preserved. Performance protection excludes manual history edits.

Workflow: Prepare a DJ deck.

### External track assertion

Title and artist; at most 1024 UTF-8 bytes each

Add a track played outside this engine. It is explicitly external with no measured deck or duration. A nonempty title is required. Performance protection excludes this optional edit.

Workflow: Prepare a DJ deck.

### Export performance session

New local JSON file

Export labels, opaque catalog IDs, boundaries and measurement counts. Media paths and fingerprints are omitted. Review user-supplied labels before sharing. Existing files are never overwritten. Performance protection excludes exports.

Workflow: Prepare a DJ deck.

### Retry history persistence

Pending essential save or close

Retry buffered history writes without discarding measurements or changing an already confirmed session end. Recording and automatic saves remain available during performance protection.

Workflow: Prepare a DJ deck.

### Keep working after history close

Cancel exit

Cancel exit and reopen history control after any pending boundary completes. An already ended session stays ended; explicitly start another session to record again.

Workflow: Prepare a DJ deck.

### Close without confirmed history save

Explicit loss warning override

Exit despite incomplete history persistence. Recent measurements and edits may be lost. Previously saved history is retained, and an active saved session will be marked interrupted on restart.

Workflow: Prepare a DJ deck.

### Named ordered crates

Saved collections; All tracks is a virtual view

Choose a named crate or All tracks. Named crates contain only their direct members, in manual order; nested children are separate views. Browse is available during performance protection, while edits require Studio. Closing this manager does not cancel an admitted edit.

Workflow: Prepare a DJ deck.

### Crate name

1–256 UTF-8 bytes

Use a trimmed name without control characters, unique among siblings. Rename changes only the saved collection name. IDs and audio files are preserved.

Workflow: Prepare a DJ deck.

### Create root or child crate

At most 4096 crates and 32 levels

Create an empty root or a child of the selected crate. The catalog owner generates a stable identity; Queued is not Saved. Wait for the durable result before another edit.

Workflow: Prepare a DJ deck.

### Delete crate subtree

Explicit membership-only confirmation

Review the exact named subtree and membership count, then confirm or keep it. This deletes collections and membership references only, never catalog tracks, source audio, sampler references or project PCM.

Workflow: Prepare a DJ deck.

### Manual crate and member order

Stable identity anchors

Move sibling crates up or down, move a child out, or reorder selected members before an unfiltered row. Last row plus one appends. Selected members retain their relative order as one group; selecting their own anchor is rejected. Filtering never sorts a named crate.

Workflow: Prepare a DJ deck.

### Copy, move and nesting destination

An explicitly selected saved crate

Choose a crate and Use as destination, then select a source. Copy keeps memberships in both crates; Move removes them from the source and inserts them in the destination. Nest moves a collection under the destination; cycles are rejected. No action moves an audio file.

Workflow: Prepare a DJ deck.

### Add captured library tracks

At most 4096 tracks per edit

Add the selected track or filtered view to the chosen destination crate, appending in visible order. Existing members are retained without duplicates or reordering. Capture uses stable catalog TrackIds. Unsaved rows and oversized batches are refused without partial changes.

Workflow: Prepare a DJ deck.

### Select or remove direct memberships

At most 4096 selected TrackIds

Checkboxes select exact track identities for a later operation. Remove changes only the current crate, leaving the track in other crates and the library. Child memberships are separate.

Workflow: Prepare a DJ deck.

### Cancel or retry a crate edit

Actual catalog outcome

Cancel requests cancellation before publication. If publication already won, wait for the truthful result. Retry is available only for a rejected edit and keeps its captured identities. Committed or unknown outcomes must not be repeated blindly.

Workflow: Prepare a DJ deck.

### Support and crash reports

Local redacted evidence

Inspect bounded structured events, numeric routing and performance counters. No uploader or audio plugin host exists. Original media, paths, titles, device names, raw errors, credentials and panic payloads are excluded.

Workflow: Stop and recover.

### Inspect current support report

Snapshot of bounded collected evidence

Build a reviewable JSON preview on a worker. Collection may lag the engine; confirmed persistence time is separate from current observations.

Workflow: Stop and recover.

### Support report categories

Explicit included fields

Choose routing, structured events, performance counters and opaque recovery references. Changing inclusion invalidates the old preview and requires another review before export.

Workflow: Stop and recover.

### Find previous support runs

Private retained reports

Read inactive run markers and their last bounded report. An observed Rust panic differs from an unclean exit; a kill or power loss does not identify its cause. Active or unreadable records are reported explicitly.

Workflow: Stop and recover.

### Support report file

Local JSON destination or source

Choose a local .omasupport.json path. Export creates a new private file and refuses existing destinations. Reopen validates bounds and schema without applying project data.

Workflow: Stop and recover.

### Reopen support report

Validated local JSON

Inspect an existing redacted report. Malformed, unsupported, oversized, symlink and non-regular inputs are rejected; the current project is unchanged.

Workflow: Stop and recover.

### Review before local export

User decision

Confirm that you reviewed this exact preview before creating a local report. There is no network upload; sharing a file remains a separate explicit user action.

Workflow: Stop and recover.

### Export reviewed support report

Local create-new publication

Write exactly the reviewed report on a cancellable worker. A completed publication is reported truthfully even after late cancellation; durability warnings stay visible.

Workflow: Stop and recover.

### Cancel support work

Pending optional work

Cancel inspection or export before publication. A committed local file remains exported. Live audio and the current project are unchanged.

Workflow: Stop and recover.

### Find exact linked recovery

Opaque session digest and durable sequence

Verify the exact retained journal record without substituting a newer one. Preview it in Recovery, then use the normal Save/Discard/Cancel restoration workflow. A pruned or corrupt record remains explicitly unavailable.

Workflow: Save and reopen.

### Restart normally

Explicit exit from safe mode

Safe mode opens no audio or MIDI devices and leaves startup settings unapplied. Save, discard or cancel unsaved work before restarting. Only a completed close starts normal device setup; playback does not resume automatically.

Workflow: Stop and recover.

### Close analysis panel

Hide the window only

Close the inspector while an admitted queue continues. Use Cancel analysis queue to stop remaining work; hiding the window never relabels a committed result.

Workflow: Prepare a DJ deck.

### Read and edit audio metadata

Embedded tags and durable library sidecars

Inspect a selected track or capture a filtered batch. Actual embedded title, artist, BPM and key carry tag provenance; missing fields explicitly use filename fallbacks. User sidecars take precedence. Loaded audio and manual beat grids keep their current performance values.

Workflow: Prepare a DJ deck.

### Inspect captured audio tags

1–4096 saved current local tracks

Read actual tag values and notices off the GUI. Batch review captures exact sources and versions; later browsing, filtering or selection cannot retarget them. Imperfect, unsupported or read-only media remain available for sidecar editing.

Workflow: Prepare a DJ deck.

### Choose changed metadata fields

At most 4096 UTF-8 bytes per field

Check only fields to change. Unchecked keeps each track's value; checked and empty clears it. BPM must be empty or a finite number greater than 1. Review the exact changed values before applying them to the captured tracks.

Workflow: Prepare a DJ deck.

### Embedded tags or sidecar values

MP3, FLAC, WAV and AIFF embedded writes up to 128 MiB

Enable supported embedded rewrites after complete audio and unrelated-metadata preservation checks. Larger, read-only, hard-linked, protected or unsupported metadata uses a durable library sidecar. Fractional BPM unsupported by an integer tag stays precise in the sidecar; no value is silently rounded.

Workflow: Prepare a DJ deck.

### Review metadata changes

Captured fields and media identities

Freeze the selected field changes before applying them. Editing a field or changing storage mode invalidates that review. Inspect the captured source paths and actual embedded values, especially before a batch clear or replacement.

Workflow: Prepare a DJ deck.

### Apply reviewed metadata edits

One durable track transaction at a time

Recheck each captured file and catalog identity. Stage supported rewrites privately, prove unchanged audio and retained unrelated metadata, then journal and atomically replace the file. Retain the original until the library confirms its save. A batch may finish partly; earlier saved tracks remain saved after cancellation.

Workflow: Prepare a DJ deck.

### Cancel metadata work

Truthful partial-batch outcome

Cancel inspection or remaining edits before their file claim. A file already replaced still completes its essential library save, even after Performance protection or close. Failed or unconfirmed writes retain recovery records and originals.

Workflow: Prepare a DJ deck.

### Retry metadata recovery

Durable tag journals and original backups

Retry catalog persistence and inspect pending tag journals on the filesystem owner. Complete verified installed edits or retire verified uninstalled staging. Conflicting external edits are preserved and reported; recovery never blindly overwrites another file.

Workflow: Stop and recover.

### Background track analysis

Local source/version cache

Open the bounded background analysis queue. Opening this panel neither decodes nor changes playback. Key detection is unavailable; existing filename hints and manual keys are not analysis results.

Workflow: Prepare a DJ deck.

### Analysis fields

BPM / duration / waveform

Choose which measured fields to prepare. Each result stays qualified by source identity, file fingerprint, digest and algorithm version. Manual BPM and locked preparation retain precedence. Waveforms store bounded low/mid/high band mean magnitudes, not playable PCM.

Workflow: Prepare a DJ deck.

### Force selected fields

Reanalysis of chosen fields only

Recompute the chosen fields even if verified cache entries exist. Unselected analysis fields and manual preparation remain unchanged. A missing or corrupt waveform is recomputed without trusting that cached blob. Force never overrides BPM or metadata locks.

Workflow: Prepare a DJ deck.

### Analyze selected row

One captured local source

Capture the selected row's identity and prepare missing chosen fields on the shared media worker. A deck or sampler load preempts background decoding. Ready means prepared, not persisted; wait for the catalog receipt.

Workflow: Prepare a DJ deck.

### Analyze filtered crate

1–4096 captured rows

Capture the current filtered view without cloning the crate. Process one source at a time and wait for each actual save result before advancing. Later sorting, filtering, scanning or selection does not retarget the queue. Narrow larger crates; no silent truncation.

Workflow: Prepare a DJ deck.

### Inspect selected cache

Verified cached record and waveform

Use Preview selected/filtered analysis changes to inspect the fields that will refresh and the current values protected by locks, without decoding or saving. Read the selected source/version's saved record and verify the waveform on the metadata worker. Missing fields are reported; inspection alone never starts source decoding. A changed source cannot inherit another version's result.

Workflow: Prepare a DJ deck.

### Retry current analysis

Explicit resume after failure or preemption

Retry the captured current source after a foreground load, protection or storage failure. This preserves earlier commits and never silently substitutes the currently selected row. Optional work is refused while performance protection is active.

Workflow: Prepare a DJ deck.

### Skip current analysis

Advance the captured queue

Leave this source's saved values unchanged and proceed to the next captured row. Earlier results remain saved; source files are never edited by analysis.

Workflow: Prepare a DJ deck.

### Cancel analysis queue

Cooperative cancellation

Stop remaining work and request cancellation at safe decode, hash and publication boundaries. An OS read may still finish. A save whose publication claim already won reports its actual committed result instead of being relabeled cancelled.

Workflow: Prepare a DJ deck.

### Help and lessons

Offline reference

Open task guidance, focused-control context, active shortcuts and observed-state lessons. Opening help never changes audio.

Workflow: Audio setup.

### Choose help workflow

Reference only

Read a task chapter without changing your active lesson or musical state. Start this lesson deliberately captures a fresh baseline.

Workflow: Audio setup.

### Search control reference

Text filter

Find implemented control descriptions by name, units or purpose. Filtering help does not filter the crate.

Workflow: Audio setup.

### Control reference entry

Expandable offline description

Read this control's purpose and units. The entry does not perform the named musical action.

Workflow: Audio setup.

### Active shortcut reference

Current profile bindings

Expand the effective shortcut table and keyboard/assistive input instructions. Customize bindings in Preferences; this reference alone does not change them.

Workflow: Audio setup.

### Start guided lesson

Observes current document

Start or restart this task guide. Lessons never reset your document, start audio, bypass unsaved-work prompts or submit musical edits for you.

Workflow: Audio setup.

### Next lesson step

Observed completion required

Advance only after the current step has published evidence. Queue acceptance is insufficient. For physical checks the separate checkbox records only your self-reported observation.

Workflow: Audio setup.

### Cancel lesson

Guide only

Stop this guide without undoing your work, stopping audio or releasing a held gate. Use the actual session/deck controls to stop playback.

Workflow: Audio setup.

### Physical observation

Self-reported; not software proof

Confirm only what you personally heard or tested on hardware. This does not certify compatibility, routing or lack of dropouts.

Workflow: Audio setup.

### Preferences and profiles

Saved versus running configuration

Edit a profile, preview resolved changes, then Apply and save. Draft changes do not affect playback. Saving audio choices does not switch the running stream. Use the separate Audio devices confirmation or restart; live MIDI policy has its own requested/applied result.

Workflow: Audio setup.

### Profile to edit

Studio / performance / named copies

Select which saved profile to edit. Selecting the editor profile alone does not activate it.

Workflow: Audio setup.

### Activate profile

Selected preference profile

Choose the profile intended for activation on Apply and save. Audio differences remain saved intent until explicit device confirmation or restart; MIDI and appearance report actual application separately.

Workflow: Audio setup.

### Clone profile

Independent named copy

Create an editable copy in the draft. Nothing is persisted until Apply and save.

Workflow: Audio setup.

### Profile name

Nonempty unique text

Rename the draft profile. Validation reports conflicts before saving.

Workflow: Audio setup.

### Output device

System default or exact device name

Save the exact output device name, or follow the system default. Preview resolves availability and rejects ambiguous names. Apply saves the choice; a separate Audio devices confirmation or restart changes the stream.

Workflow: Audio setup.

### Requested sample rate

8000–384000 Hz; advertised options or device default

Save an advertised output rate. The common 44.1/48/96/192 kHz choices appear only when advertised. Preview validates the whole configuration; saved intent and backend-accepted logical settings are distinct from physical negotiation.

Workflow: Audio setup.

### Requested buffer frames

16–32768 frames; optional device default

Save an advertised output buffer request, or let the backend choose. Apply saves it without switching audio. Requested buffer duration, observed callback size and backend scheduling estimates differ; smaller requests do not prove fewer dropouts.

Workflow: Audio setup.

### Audio devices and latency

Saved intent / active stream / observed timing

Inspect the active output and preview saved choices. Opening this window neither changes devices nor emits a probe. Save edits in Preferences before previewing them here.

Workflow: Audio setup.

### Audio backend

System backend or advertised backend name

Save the backend used to resolve input and output device names. An unavailable saved backend fails visibly rather than silently choosing another route.

Workflow: Audio setup.

### Output sample format

Advertised integer or floating-point PCM

Save a format supported by the selected output layout and rate. Device default lets the backend select; unsupported combinations fail preview instead of silently falling back.

Workflow: Audio setup.

### Calibration input device

System default or exact input name

Save the input used only for explicit loopback calibration. This does not enable input monitoring or general external-audio recording. Calibration uses the currently active output's logical rate.

Workflow: Audio setup.

### Calibration input channels

1–64 advertised interleaved channels

Save the temporary input stream's channel count. Select the actual capture channel separately; all channel numbers are interleaved positions, not verified connector labels.

Workflow: Audio setup.

### Calibration input sample format

Advertised integer or floating-point PCM

Save the temporary input format. Preview checks it against the active output rate and chosen input layout before any probe can run.

Workflow: Audio setup.

### Calibration input buffer

16–32768 advertised frames or device default

Save the temporary input buffer request. Together with an explicit output buffer this permits a buffer-only estimate; driver and converter time are excluded.

Workflow: Audio setup.

### Calibration capture channel

One-based channel 1–64

Choose which input channel captures the loopback probe. Preview rejects a channel outside the selected input stream. No captured input is monitored to output.

Workflow: Audio setup.

### Loopback probe output channel

One-based channel 1–64

Choose which active output channel emits the probe. Other probe-stream channels are silent. Preview rejects channels outside the active output stream; verify the physical route yourself.

Workflow: Audio setup.

### Loopback probe level

−60 to −24 dBFS

Set the digital level of three short coded probes. This does not control external amplifier volume. Use a suitable line-level route, disable monitoring and turn down speakers before explicit confirmation.

Workflow: Audio setup.

### Preview saved audio

Read-only device discovery

Resolve the saved profile against current input/output capabilities and the active calibration route. Preview changes no stream and plays no probe. Save draft preference edits first.

Workflow: Audio setup.

### Use saved audio now

Opens disruptive-change confirmation

Review the proposed output before choosing Stop and change output. This first button alone does not stop playback or activate the device.

Workflow: Audio setup.

### Reconnect retained output

Explicit physical-identity recovery

After device loss, reconnect only the retained physical identity and accepted route. Default/server aliases need explicit preview and fallback confirmation. Playback remains stopped, emergency mute remains latched and physical inputs need acknowledgment before Play. Native transport starts ramp output over 2 ms.

Workflow: Audio setup.

### Stop and change output

Explicit disruptive operation

Stop decks, clips, recording and held notes, then activate the previewed output. Failure attempts the previous output; double failure retains the session for Save, New/Open and Close. Playback remains stopped even after success or rollback.

Workflow: Audio setup.

### Keep current audio

Dismiss confirmation

Dismiss the device-change or probe confirmation without submitting it. Saved preferences remain saved; the current output is unchanged.

Workflow: Audio setup.

### Measure loopback

Opens physical-route confirmation

Review the chosen input, active output, channel numbers and probe level. This button alone emits no sound. A suitable physical cable or interface loopback is required.

Workflow: Audio setup.

### Cable ready: stop and measure

Explicit line-level loopback probe

Confirm your chosen route, disable input monitoring and turn down external speakers. Stop performance, emit three low-level probes, capture up to three seconds, then restore output without resuming. Missing, noisy, clipped, ambiguous or inconsistent evidence produces no measurement.

Workflow: Audio setup.

### Cancel audio operation

Cooperative cancellation

Request cancellation at safe worker boundaries; operating-system calls may still finish. Cancellation before output activation restores the prior route; a change already committed is reported honestly. Cancelled calibration produces no current measurement.

Workflow: Audio setup.

### Audio operation status

Pending / applied / rollback / offline

Read the operation's actual result. Dismiss hides only the notice. An offline retained session still supports Save, New/Open and Close; queue acceptance is not proof that an output is running.

Workflow: Audio setup.

### Advertised device capabilities

Backend ranges; not physical qualification

Expand input/output device ranges for formats, channels, rates and buffers. Exact names are not portable serial identities; duplicates are rejected. Discovery errors and truncation are explicit, and unknown buffer limits are not invented.

Workflow: Audio setup.

### Backend-accepted output

Logical device configuration

Read the configuration accepted by the backend. CPAL does not report the physical negotiated sample rate or converter latency; these logical settings are not a physical hardware measurement.

Workflow: Audio setup.

### Observed callback timing

Frames / milliseconds

Read completed callbacks for the current stream. Callback duration uses its logical sample rate; backend output scheduling estimates are separate from driver/converter roundtrip and measured dropouts.

Workflow: Audio setup.

### Roundtrip buffer estimate

Requested input plus output frames / logical rate

Estimate only the sum of explicitly requested buffers. Driver and converter time are excluded. If either buffer is backend-selected this estimate is unavailable.

Workflow: Audio setup.

### Measured loopback return

Host callback-to-callback timing / resolution / repeat spread

Read a result matched to the exact current profile and preview. Three reliable recorded probes establish host callback-to-callback return timing, not converter-only physical roundtrip. Old, cancelled or unrelated measurements are not shown as current.

Workflow: Audio setup.

### Dismiss preferences notice

Message only

Hide this notice without retrying, saving or changing the running profile.

Workflow: Audio setup.

### Delete profile

Inactive draft profile

Remove this inactive profile from the draft. The active profile cannot be deleted; Apply and save commits the draft.

Workflow: Audio setup.

### Reset profile to defaults

Selected draft profile

Replace this profile's draft values with defaults. Cancel discards the draft; Apply and save commits it.

Workflow: Audio setup.

### Reload saved preferences

Local saved file

Read the saved preferences back into the editor. Unsaved draft changes may be replaced; validate and apply deliberately.

Workflow: Audio setup.

### Preserve old file and reset preferences

Explicit recovery

Keep the unreadable/changed file as a backup, then create default preferences through the recovery worker. Failure remains visible rather than destroying the only copy.

Workflow: Audio setup.

### Requested output channels

1–64 advertised channels or device default

Save an advertised output stream channel count. Main uses channels 1/2, mono sums them and additional channels are silent; this does not create extra mixer or headphone buses.

Workflow: Audio setup.

### Retry saved MIDI policy

Saved policy to live connection worker

Request the saved profile’s policy again. Inspect applied generation and missing/failed ports; retry admission is not connection success.

Workflow: Connect a controller.

### MIDI input policy

All / selected exact names / disabled

After Apply and save, request the live connection policy. Check requested/applied generation, missing ports and errors; saving alone is not connection success.

Workflow: Connect a controller.

### Selected MIDI input names

One exact port name per line

Enter explicitly allowed inputs for Selected policy. Missing names remain missing; they never fall back to enabling all ports.

Workflow: Connect a controller.

### Explicit track MIDI routing

Saved studio/performance profile

In Preferences, choose exact input/output ports and track mappings. Preview and Cancel preserve live routing; Apply and save requests the saved routing. Inspect the applied generation and errors in MIDI. The input policy must also permit each input. Leaving explicit routing off keeps established controller maps and selected-track keyboard notes.

Workflow: Connect a controller.

### MIDI destination track

One route per stable track slot1–128

Assign each physical port/channel to a fixed track independently of browsing. Several inputs may feed a track and the same input may feed several tracks. Held notes release their original destination. Track routing belongs to the active profile.

Workflow: Connect a controller.

### MIDI route device and port

Exact case-sensitive name and optional backend id

Choose a discovered port or enter its exact name. Use a backend id to disambiguate identical names. Missing or ambiguous outputs are refused. Configured or active input devices/clients cannot also be an output destination. Select controller inputs explicitly and deselect the output device in Input policy. No first-port fallback is used.

Workflow: Connect a controller.

### MIDI route channels

Input channel mask; output1–16 or Preserve

Choose accepted input channels1–16 and optionally rewrite outgoing channel messages. Preserve retains each source channel; complete SysEx has no channel and is not rewritten.

Workflow: Connect a controller.

### Monitor routed MIDI notes

Internal instrument monitoring

Play accepted notes on this route's fixed internal track. Bank/program, controllers, pressure, bend and SysEx are external MIDI messages; internal monitor does not interpret them as synth controls.

Workflow: Connect a controller.

### Live MIDI thru

Explicit external output only

Send accepted live messages to the route's external port. Controller mappings remain separate. Configured and active input/output device overlap is refused, including controller-map inputs. Review physical MIDI cable/thru connections separately; packet equality cannot distinguish a returned echo from a legitimate repeated note.

Workflow: Connect a controller.

### MIDI route message filters

Notes, CC, bank, program, pressure, bend, SysEx

Filter each type independently. CC0/32 use Bank rather than ordinary CC. Pressure includes polyphonic and channel pressure. SysEx is off by default and accepts complete7-bit-data F0…F7 packets of at most256 bytes; fragmented or larger packets are refused. External clip output plays source notes and channel lanes before the internal arpeggiator/audio mix. Output has2048 queue slots and at most256 renderer messages per audio block; overload resets outputs and refuses that clip until a new launch, clip edit or routing change. Concurrent owners at one port/channel/pitch share one physical gate, retaining the first onset velocity until every owner releases. Sustain combines active owners; disconnect/clip stop releases only its owner. Other channel controls use received order when tracks share an output channel. Past controllers are not chased after a seek/route change.

Workflow: Connect a controller.

### Cancel pending MIDI routing

Before publication claim

Cancel keeps the previous applied routing when it wins before the worker's commit claim. A backend open/send already in progress must return before its receipt. A change that won the claim reports Applied; closing the panel does not undo a committed route.

Workflow: Connect a controller.

### All notes off / reset MIDI outputs

Explicit output-owner reset

Request all-sound-off, all-notes-off, reset controllers, centered bend and zero channel pressure on every connected output channel. Queue overflow and route/device changes also invalidate older outgoing events and request reset. Reset also stops the currently running external clip streams until relaunch, clip edit or routing apply; internal audio transport remains running. Backend errors report unconfirmed reset; this is not proof of physical hardware silence.

Workflow: Stop and recover.

### Library locations

One local path per line

Choose locations for background scan. A changed active profile cancels/restarts discovery while keeping the visible crate until complete results arrive.

Workflow: Prepare a DJ deck.

### UI scale

50–300%

Scale the interface after a successful Apply. Draft values alone do not change the live window.

Workflow: Audio setup.

### Font size

8–48 points; optional desktop/default size

Override the text size after Apply or keep the selected theme's size. Font selection remains the desktop font policy.

Workflow: Audio setup.

### Follow desktop theme and font

On / off

Use validated live desktop theme/font updates when enabled. Disabling selects the app's default appearance; invalid updates retain last good settings.

Workflow: Audio setup.

### Startup options

Scan / help / MIDI windows

Choose which background scan and panels start next time. Saving this option does not close the current project.

Workflow: Audio setup.

### Shortcut override

Exact key and modifiers / disabled

Override a stable named action in the active profile. Preview/Apply validate conflicts and reserved editing chords. In-app help shows effective bindings.

Workflow: Audio setup.

### Preview changes

Read-only background resolution

Resolve devices/routes and validate the draft without saving or changing the running engine. Missing hardware and unsupported routes remain explicit.

Workflow: Audio setup.

### Apply and save preferences

Validated atomic local file

Persist the validated draft before applying appearance/library/MIDI changes. Audio choices remain saved intent until the separate Audio devices confirmation or restart. Failure keeps running preferences unchanged. Configure panel layout copies named workspaces, reorders/hides/resizes panels and opens secondary windows. Use current window sizes before Preview and Apply. Cancel retains the applied layout; closing a panel window returns it to the main window for this session.

Workflow: Audio setup.

### Cancel preference changes

Draft / pending operation

Cancel pending work before commit or discard the draft and close the editor. A save already committed is reported as committed.

Workflow: Audio setup.

### Import preferences

Validated Omatainer preference JSON

Read a preference file into the draft on a worker. It does not activate or overwrite running preferences until Apply and save.

Workflow: Audio setup.

### Export preferences

Local preference JSON

Export validated draft preferences to the chosen local file. Existing-destination policy is explicit; exporting does not activate a profile.

Workflow: Audio setup.

### Preference import/export path

Local filesystem path

Choose a source/destination for portable preferences. No network transfer occurs.

Workflow: Audio setup.

### Autosave and recovery

Independent recovery copies

Open the recovery status and inactive-session list. Batched full edit-state records normally capture dirty work about every two seconds, including untitled projects. Confirmed durable age is the actual loss-window evidence; explicit project files are never overwritten.

Workflow: Save and reopen.

### Last confirmed durable recovery

Seconds and revision

Captured-state age advances only after that state has a confirmed durable journal commit. The separately labeled commit acknowledgment may be newer after slow I/O; edits accepted during the write are not included automatically. Busy capture, full storage and I/O failure keep the prior confirmed state. Newer edits may be lost until the next confirmed record; a committed durability warning is not a durable acknowledgment.

Workflow: Save and reopen.

### Refresh recovery list

Inactive sessions

Discover bounded, validated inactive recovery candidates on the worker, including sidecar hashing. Performance protection defers this work and invalidates old verified lists; leave protection and Refresh for a fresh selection. Active sessions remain locked. A malformed tail stops replay at its valid prefix and is reported; incomplete or corrupt media is never silently omitted.

Workflow: Save and reopen.

### Journal current edits now

Background capture

Request the next coherent edit-state record without changing playback. This is an asynchronous request, not confirmation of durability. Read the resulting durable timestamp and warnings.

Workflow: Save and reopen.

### Preview recovery

Read-only validation

Validate the selected record and embedded media on the worker and display its notes, media and recovery report. Preview does not replace the current project or write its explicit saved path. Performance protection rejects or cancels this optional PCM verification; a cheap protected-startup availability notice and automatic durability remain available. Full discovery is deferred until you leave protection and Refresh.

Workflow: Save and reopen.

### Restore as untitled copy

Stopped, unsaved document

Use the ordinary Save / Discard / Cancel protection before installing the selected recovery. The recovered project starts stopped as a dirty untitled copy; Save As chooses a new explicit file. The original path is informational and is not overwritten.

Workflow: Save and reopen.

### Delete recovery session

Every generation in one session

After explicit confirmation, permanently discard this inactive session's journal, checkpoints and sidecars. A session lock and candidate identity prevent deleting active or changed recovery. Explicit project files are unaffected. Performance protection excludes this optional explicit deletion; automatic committed-checkpoint cleanup remains essential durability.

Workflow: Save and reopen.

### Cancel recovery operation

Before commit

Cancel pending preview, discovery or deletion without changing the current project. A deletion or durable write that already committed remains truthful; cancellation cannot undo a committed filesystem operation.

Workflow: Save and reopen.

### Retry recovery retirement

Intentional close

Retry recording that this document epoch was intentionally closed. Failure leaves a visible warning and may offer the recovery copy again after restart; it never changes the explicit project Save decision.

Workflow: Save and reopen.

### Keep working after recovery close

Cancel exit

Cancel the exit and release its existing project close guard. Recovery restarts in a fresh session, including when the retirement marker already committed. Playback does not resume automatically.

Workflow: Save and reopen.

### Close and retain possible recovery

Explicit warning override

Finish the authorized exit even when recovery retirement failed or its durability is uncertain. The remaining copy may be offered at startup. This does not label unsaved work as explicitly saved.

Workflow: Save and reopen.

### Autosave and recovery limits

Saved profile settings

Configure checkpoint compaction, retained generations per session and the global storage cap. Dirty edit-state journal batches remain separate from checkpoints. Apply and save persists these settings; it is not a journal durability acknowledgment.

Workflow: Save and reopen.

### Recovery checkpoint interval

5–3600 seconds

Choose how often the edit-state journal compacts into a checkpoint. Dirty state is batched about every two seconds independently; capture, queue pressure and disk I/O can increase the loss window. Read the observed last durable age in Recovery.

Workflow: Save and reopen.

### Recovery generations per session

2–20 generations

Retain checkpoint generations within each session, preserving a known-valid prior generation until a newer durable commit. This does not silently delete another session's recovery history.

Workflow: Save and reopen.

### Recovery storage limit

64–16384 MiB globally

Bound embedded media, journal, checkpoint and staging storage together. Full storage stops new durable writes and reports the failure. The only usable recovery is never removed automatically to make room; explicit project files are outside this store.

Workflow: Save and reopen.

### Close panel

View only

Hide this panel without changing the audio or deleting its data.

Workflow: Audio setup.

### Control actions

Focused control

Open the focused control's alternate actions. Shift+F10 provides the same choices; controls without alternatives disable the menu.

Workflow: Audio setup.

### Direct numeric entry

Control's displayed units and bounds

F2 opens a focused value editor. Apply accepts a finite in-range value; Cancel preserves the previous value. Arrows adjust and Shift makes fine adjustments.

Workflow: Edit and undo.

### Scrollable surface

Pixels

Scroll to controls outside the viewport. Focused controls reveal themselves; arrows and Page/Home/End work on the focused scroll view.

Workflow: Audio setup.

### Performance protection

Session safety state

Protect playing/touched deck loads, destructive edits and project/device changes centrally. Mixing, composing, recording, stopped-deck loads and Save remain available. Leaving requires a deliberate decision; opening a project never disables protection.

Workflow: Stop and recover.

### Safe stop

Session and both decks

Confirm stopping transports, finalizing held captures and releasing input notes. Natural sample/effect tails continue. Input recovery is explicitly acknowledged; transport never resumes automatically.

Workflow: Stop and recover.

### Emergency silence

All output

Confirm all-notes-off and a 2 ms output fade to latched mute. Quiet observation alone never unmutes delayed or bypassed history. Explicit stopped DSP reset is required to unmute.

Workflow: Stop and recover.

### Controlled recovery

User-confirmed physical release

Release physical controls and explicitly acknowledge. The renderer drains old queued onsets before reopening controls. This is a user report, not hardware qualification. Emergency output mute stays latched until deliberate DSP reset.

Workflow: Stop and recover.

### Reset stopped DSP and unmute

Audio-owner worker

Deliberately reclaim the stopped graph, erase voice/effect/filter histories off the callback and reopen the same output. Playback remains stopped. Failed recovery retains emergency mute and reports its status.

Workflow: Stop and recover.

### Keep safety state

Cancel pending decision

Close this confirmation without changing protection, transport, input recovery or output mute.

Workflow: Stop and recover.

### Dismiss admission error

Message only

Hide this rejection message. The rejected action was not applied; dismissal does not retry it.

Workflow: Stop and recover.

### Pitch lock

On / off; 0.50–1.50× forward rate

On preserves pitch while changing tempo within the supported range. L! means the actual rate is outside that range and tempo and pitch change together. L~ means platter touch temporarily uses direct scratch playback. On release, a playing deck resumes pitch lock if its rate is supported; a stopped deck stays stopped. Original-rate playback passes through directly. Stopped or empty decks show armed status. This is independent of deck tempo sync; Match or Sync can exceed the supported range. Listen before a performance: quality varies with material and rate.

Workflow: Prepare a DJ deck.

### Deck pitch

Percent; selected ±8, ±16 or ±50 range

Adjust playback rate around zero. Pitch lock determines whether the file's pitch follows the rate. Center resets to the original rate.

Workflow: Prepare a DJ deck.

### Pitch range

±8%, ±16%, ±50%

Cycle the pitch fader's range. Check the current value after changing range.

Workflow: Prepare a DJ deck.

### Manual beatgrid editor

Loaded track; source-time coordinates

Open a draft grid for this exact loaded track. Analysis BPM may seed an explicitly unverified preview; it never establishes a downbeat. Changing the loaded track closes the draft.

Workflow: Prepare a DJ deck.

### Grid downbeat position

Finite source seconds; negative positions allowed

Enter the source-time position of beat zero. Earlier source positions have negative beat coordinates. Editing changes the preview only; absolute cues and audio stay unchanged until the separate grid Apply, which still does not move cues.

Workflow: Prepare a DJ deck.

### Set grid downbeat at playhead

Published source playhead seconds

Place beat zero at the displayed renderer playhead, using the draft tempo. Enter a valid tempo first when no usable BPM hint exists. This edits only the preview.

Workflow: Prepare a DJ deck.

### Slip beatgrid

−10 / −1 / +1 / +10 milliseconds

Move all draft beat lines by the chosen source-time offset without changing tempo. Preview the alignment before Apply.

Workflow: Prepare a DJ deck.

### Stretch grid tempo

20–400 BPM

Change beat spacing while keeping the draft downbeat fixed. Enter a finite tempo; invalid text cannot be applied. This uniform grid does not implement changing meters or tempo maps.

Workflow: Prepare a DJ deck.

### Half grid tempo

Draft BPM divided by two; minimum 20

Correct double-tempo ambiguity in the preview while retaining the downbeat. Disabled when the result would fall below the supported range.

Workflow: Prepare a DJ deck.

### Double grid tempo

Draft BPM multiplied by two; maximum 400

Correct half-tempo ambiguity in the preview while retaining the downbeat. Disabled when the result would exceed the supported range.

Workflow: Prepare a DJ deck.

### Reset manual beatgrid

Preview removal

Preview removing the manual grid. Apply commits removal; Cancel keeps the current grid. Reset does not change analyzed BPM, audio or absolute cue positions.

Workflow: Prepare a DJ deck.

### Beatgrid preview

Solid applied / dashed draft; uniform four-beat bars

Inspect bounded source-time beat markers around the live playhead over a summed low/mid/high magnitude envelope, not raw sample peaks. Beat zero is the first downbeat; negative beat coordinates represent pickups. Colored cue markers keep their source positions. Preview does not alter playback.

Workflow: Prepare a DJ deck.

### Apply manual beatgrid

One receipt-qualified undoable edit

Submit the validated draft for the same loaded track. Queued means waiting for renderer preparation; actual application and library durability are reported separately. Manual preparation survives reanalysis without replacing analysis metadata. An accepted edit cannot be cancelled by closing this window.

Workflow: Prepare a DJ deck.

### Cancel or close beatgrid editor

Draft only; Escape

Discard unapplied preview changes and close the editor without submitting an audio or grid edit. Closing after Apply does not cancel a command already accepted by the renderer queue.

Workflow: Prepare a DJ deck.

### Cue names and colors

Eight applied hot cues

Open the loaded track's cue list. Rows show renderer-applied positions, names and colors; fields are drafts until Apply. Changing the track closes the editor.

Workflow: Prepare a DJ deck.

### Cue name

Up to 64 UTF-8 bytes

Draft a name for an existing hot cue. Apply submits the edit to the same loaded track. Empty names display the cue number; control characters are rejected.

Workflow: Prepare a DJ deck.

### Cue color

Opaque #RRGGBB or theme

Draft six hexadecimal color digits. An empty field uses the current theme's cue color. Apply updates pad, waveform marker and cue list together.

Workflow: Prepare a DJ deck.

### Apply cue name and color

One undoable cue edit

Apply this row's validated draft only if its cue and loaded track identity are still current. Queue acceptance is not proof of application or a durable library save.

Workflow: Prepare a DJ deck.

### Reload cue values

Discard editor drafts

Replace the editor's name and color drafts with the currently applied cue values. This changes no audio, cue position or saved library data.

Workflow: Prepare a DJ deck.

### Close cue editor

Discard unapplied drafts

Close the cue list and discard fields that were not applied. Already applied cue positions, names and colors remain in the session and follow the library save status.

Workflow: Prepare a DJ deck.

### Relocate a library track

Local file association

Choose the new path, or search replacement folders, for the selected library track after a file move or copy. The worker must verify identical content before retaining the stable track identity and preparation.

Workflow: Prepare a DJ deck.

### Relocated track path

Absolute local file path

Enter the moved file's path. The original content must have been verified while available, or the original file must still exist. Filenames alone are never a match.

Workflow: Prepare a DJ deck.

### Verify and relocate track

Background content verification and catalog save

Compare complete file hashes and stable file metadata before changing the library association. Different bytes, an occupied destination or a changed source are rejected. Loaded and undo-retained old receipts preserve the same prepared identity. No files are moved or deleted.

Workflow: Prepare a DJ deck.

### Replacement search folders

1–64 absolute folders, one per line

Search the selected track by complete content identity, including renamed files. A missing original needs its previously saved content digest. Symlinks and nested mounts are skipped; incomplete coverage is explicit. Changing these folders discards the previous review.

Workflow: Prepare a DJ deck.

### Search replacement folders

Background read-only discovery in Studio

Compare up to 100,000 files, one million entries, 4,096 directories, depth 64 and 64 GiB of hashed bytes. Show up to 256 byte-identical copies without automatically choosing one. Cue and history saves remain available while searching. A newly measured original digest is saved before relocation becomes available.

Workflow: Prepare a DJ deck.

### Cancel replacement search

Leave track associations unchanged

Cancel discovery cooperatively. Closing the relocation window or entering Performance protection also invalidates the search. Already committed catalog saves remain committed.

Workflow: Prepare a DJ deck.

### Choose a replacement match

Explicit location selection

Review the full path of the copy you want. Several identical copies are ambiguous locations and require your choice. Incomplete searches may have additional unseen matches. Choosing a row alone changes no association.

Workflow: Prepare a DJ deck.

### Verify and use selected replacement

Recheck identity and persist association

Rehash the reviewed file and recheck its captured filesystem location before saving. Preserve stable track ID, preparation, crate membership and history. Replaced files and stale versions are rejected. A failed pre-rename save retains the old association; a completed rename with an unconfirmed directory sync is reported separately.

Workflow: Prepare a DJ deck.

### Hot cue

Eight positions per deck

An empty slot stores the current position; a filled slot jumps there. Shift-click or Delete cue in alternate actions removes it. Loading and preparation follow the exact source identity.

Workflow: Prepare a DJ deck.

### Deck EQ

Bass / mid / treble: 0–340% linear gain

Drag to change the band's gain; click cuts the band, alternate action solos it. Center is unity; left/right channel histories are independent.

Workflow: Mix a session.

### Deck gain

0–120% linear gain

Set the deck's level before mixing. Click or alternate actions can cut/solo the gain path. Zero is silent.

Workflow: Mix a session.

### Deck platter

Playback / cue / jog

Click toggles play/pause; right-click or Cue action returns to the main cue; Shift-click or Unload removes media. Drag while touching to scratch; release ends that touch. Keyboard/assistive actions expose the same operations.

Workflow: Prepare a DJ deck.

### Deck waveform position

0–source duration in seconds

Seek within loaded media. Seeking resets grain history with a bounded transition; it does not change the stored file. A stopped deck stays stopped.

Workflow: Prepare a DJ deck.

### Deck filter (mapped MIDI)

−100% low-pass to +100% high-pass

Center is exact bypass. Moving toward either edge attenuates more of the opposite spectrum. Smooth transitions preserve independent stereo histories.

Workflow: Mix a session.

### Crossfader

0% A to 100% B

Blend the two decks. At or left of center A is favored for Match; right of center B is favored. This is independent of session track gains.

Workflow: Mix a session.

### Deck time display

Elapsed source seconds / remaining wall estimate

Choose elapsed or estimated remaining time. Rate changes affect the remaining estimate; this is not a measured hardware output latency.

Workflow: Prepare a DJ deck.

### Runout warning

0–300 seconds; 0 disables

Set a per-deck warning before file end. Repeating loops suppress the warning. The alert does not stop or switch the deck.

Workflow: Prepare a DJ deck.

### Quantize

On / off

Toggle session launch and supported deck cue/loop quantization. Pending clip launches display queued until the selected beat boundary.

Workflow: Prepare a DJ deck.

### Loop in / out

Source positions

Primary action sets loop in; right-click or Loop out action sets loop out. The loop needs a valid ordered region.

Workflow: Prepare a DJ deck.

### Double loop

×2 duration

Double the current loop region within source bounds.

Workflow: Prepare a DJ deck.

### Halve loop

½ duration

Halve the current loop region within source bounds.

Workflow: Prepare a DJ deck.

### Reloop

Four bars when creating a loop

Toggle the current loop; if none exists, create the supported four-bar region.

Workflow: Prepare a DJ deck.

### Match decks

Effective beats per minute

Match the unfavored deck to the favored deck's file BPM times pitch rate. If both play, align the unfavored phase. The favored position does not jump.

Workflow: Prepare a DJ deck.

### Arm compose target

One track and scene cell

Explicitly capture the selected cell as the pad-writing destination; an empty target becomes MIDI. Browsing does not retarget it. Arming itself does not play a note.

Workflow: Record a held note.

### Disarm compose

Armed / disarmed

Finalize current pad captures and stop automatic pad writes. Existing monitor voices stay held until their own releases. Session Stop also disarms.

Workflow: Record a held note.

### Import Standard MIDI File

SMF 0/1, PPQN 1–32767, 16 MiB

Inspect notes, velocities, release velocities, channels, all channel messages, standard text, key, tempo and time signature events. Map tracks or split channels to session cells. Review unsupported omissions before import. Apply preserves source ticks and creates one guarded Undo entry.

Workflow: Save and reopen.

### Export Standard MIDI File

Source clip coordinates, SMF 0 or 1

Select MIDI cells and an unused destination path. Export writes all selected clips at source beat zero, preserving trailing source silence. Choose muted-note inclusion and original or session conductor events. Clip loops, launch offsets and native mute flags are not SMF fields. Existing destinations produce an error.

Workflow: Save and reopen.

### MIDI file path

Regular local .mid or .midi file

Read and write file bytes on the MIDI worker. Inspection detects source replacement; import uses the inspected contents only while that source remains current. Export uses an atomic exclusive file publication after successful encoding and synchronization.

Workflow: Save and reopen.

### MIDI track and channel mapping

Up to 128 tracks by 512 scenes; 64 mapped/export clips per transaction; 8192 notes per clip, 65536 per session

Choose every destination or disable a source row. Several source rows mapped to one cell combine there. Channel splitting attaches file-track metadata to its first row. Replace contents or explicitly merge existing MIDI clips; audio destinations fail before any change. Gains remain unchanged.

Workflow: Save and reopen.

### MIDI conductor choice

Integer microseconds per quarter; meter denominators through 128

Keep the current session conductor or use the complete file map. Conflicting simultaneous values require an authoritative file track. Applying a conductor supports 40–240 BPM and at most 4096 points of each kind; other values remain in original source metadata when keeping the session conductor. Metronome beats follow the meter denominator and accent each bar. A manual tempo edit resumes constant tempo and 4/4 and can be undone.

Workflow: Save and reopen.

### MIDI tick precision

PPQN 1–32767

Original imported notes and events keep exact integer ticks. A different division or newly edited beat may require rounding. Exact export refuses unrepresentable values; allow nearest-tick rounding explicitly to accept at most half a destination tick per boundary. Export can refuse ambiguous overlapping same-channel/pitch note releases instead of changing durations.

Workflow: Save and reopen.

### Cancel MIDI file work

Worker cancellation and atomic renderer claim

Cancel inspection, preparation or encoding before publication. Cancel a queued import before the renderer claims it. After ownership or file publication wins, the actual completed result is reported. Close waits for pending work to settle.

Workflow: Save and reopen.

### MIDI piano roll

Captured empty or MIDI clip

Inspect the selected cell on the project worker. Drawing and numeric changes remain a draft until Apply. Browsing never retargets this editor. Every note has a stable identity and a virtualized accessible list entry.

Workflow: Record a held note.

### MIDI timing grid

Free, straight subdivisions or triplets

Snap drawing and pointer movement to the selected quarter-beat subdivision. Free retains continuous pointer timing. Keyboard movement uses the grid, or 1/64 beat in Free. Numeric fields accept explicit values independently of snapping.

Workflow: Record a held note.

### MIDI note actions

Pitch 0–127; up to 8192 notes

Draw on empty space; select a note, Shift-click to extend, drag its body to move or its right edge to resize. With Draw notes disabled, drag empty space for a rectangular selection. Arrows move or transpose; Shift+Left/Right resize; Ctrl+A selects all, Ctrl+D duplicates, M toggles mute and Delete removes. New and duplicate notes receive fresh IDs.

Workflow: Record a held note.

### MIDI note values

Source quarter-note beats and velocity 1–127

Enter pitch, start, length, velocity and mute. Add note creates a new identity; Set selected note values applies these values to each selected note while retaining its identity. Movement buttons change selected positions relatively.

Workflow: Record a held note.

### MIDI clip and loop bounds

0–262144 source quarter-note beats

Set clip start/end and loop start/end numerically or drag the distinct ruler markers. Start must not exceed loop start; loop end must not exceed clip end. Minimum span is 1/1024 beat. Intro notes play once; looping repeats its active region and gates close at explicit boundaries. Very dense short loops are rejected.

Workflow: Record a held note.

### MIDI pitch and time view

Pitch/time rulers, folding, zoom and scroll

Time scroll chooses the first visible source beat; Top pitch chooses the highest visible pitch. Zoom changes pixels per beat or pitch. Used/major/minor folding keeps existing off-scale notes visible. Hidden notes remain selectable in the native note list.

Workflow: Record a held note.

### Apply MIDI edit

One guarded session History transaction

Commit this captured draft only if its original MIDI content and project epoch still match. Active recording into the target, stale work, performance protection or unavailable History reject the request and retain the draft. Wait for the renderer outcome. Applied changes persist through native project Save and Undo/Redo.

Workflow: Record a held note.

### MIDI note audition

Independent non-recording voice

Audition the note value through the captured track. Stop note audition, focus loss or close releases its own gate without releasing other physical notes. Drum one-shots retain their natural decay. Performance protection excludes new audition presses but accepts releases.

Workflow: Record a held note.

### Cancel or close MIDI editor

Explicit draft discard and pending cancellation

Cancel/Escape and application Close preserve unapplied work until Keep editing or Discard. A pending Apply can be cancelled before renderer ownership; an already claimed edit completes and remains in History. Unconfirmed disconnected outcomes remain visible.

Workflow: Record a held note.

### Edit sampler banks

Captured project bank

Open a draft of the selected working bank. Factory originals are read-only: copy before editing. Later selection, Undo or project replacement never retargets an existing draft.

Workflow: Record a held note.

### Sampler bank name

1–128 UTF-8 bytes for reusable definitions

Working draft and reusable-save names are separate. Renaming does not replace another definition with the same name.

Workflow: Record a held note.

### Create empty bank

Sixteen empty slots

Prepare a new working-bank identity off the audio thread. It remains a preview until Apply; empty slots never receive default sounds.

Workflow: Record a held note.

### Copy sampler bank

New working identity

Copy immutable audio and controls into a new editable draft. Original Kit / Perc / Hits buttons remain available even after a project replaced the startup banks. Original banks and held voices keep their captured sources.

Workflow: Record a held note.

### Sampler slot

Slots 1–16

Choose the slot to inspect or edit. Changing the selected slot stops the independent audition; pad input remains separate.

Workflow: Record a held note.

### Assign selected local source

Exact crate source and file version

Capture the selected local-file or mounted removable-volume identity and decode it on the bounded shared worker. Changed or invalid media is rejected without replacing the current bank. Built-in deck stems are not file assignments.

Workflow: Record a held note.

### Clear sampler slot

Prepared edit

Prepare an empty slot. Apply is required before pads change; clearing never substitutes a factory sound.

Workflow: Record a held note.

### Retry sampler source or store

Explicit worker retry

Retry the referenced missing source or reopen the reusable store. Source identity must still match or have a verified library relocation; unrelated files are not substituted.

Workflow: Record a held note.

### Sampler slot gain

0–2 linear

Gain is captured at audition or pad onset. Prepare preview and Apply to change future pads; held voices retain their original gain.

Workflow: Record a held note.

### Sampler source range

Start inclusive / end exclusive in source seconds

Blank end uses the source end. Start must precede end within the decoded source. Prepare validates the range; source seconds are independent of output sample rate.

Workflow: Record a held note.

### Prepare sampler preview

Worker validation

Validate controls and register immutable audio off the renderer. A ready preview changes neither the working bank nor its physical gates.

Workflow: Record a held note.

### Audition sampler slot

One independent voice

Play the prepared range and gain through the captured selected track. This does not press a sampler pad or record a note. Existing mix/effect controls still affect output.

Workflow: Record a held note.

### Stop sampler audition

Reserved release

Stop the exact audition identity. Cancel, slot changes and replacement also request this release; admission failures remain visible and retry.

Workflow: Record a held note.

### Reusable sampler definition

Stable definition identity

Choose a saved reference definition. Selection alone changes no working bank; Import prepares sources and reports unavailable slots explicitly.

Workflow: Record a held note.

### Import reusable sampler bank

Local reference resolution

Prepare a new working identity from the selected definition. Missing or changed files stay missing with diagnostics. Apply is separate; native projects embed playable audio.

Workflow: Record a held note.

### Replace reusable definition

Explicit selected identity

Allow Save reusable to overwrite the selected definition ID. Matching a name alone never authorizes replacement.

Workflow: Record a held note.

### Save reusable bank

Reference file / explicit replacement

Persist the prepared draft as a named reusable definition. This does not Apply a working bank or save the native project. Reusable banks retain file identities, controls and ranges, not copies of local PCM.

Workflow: Record a held note.

### Apply sampler bank

One revision-qualified undoable edit

Queue the prepared bank for its original project epoch and bank revision. Wait for renderer Applied: admission is not completion. Cancellation can win only before renderer claim.

Workflow: Record a held note.

### Cancel sampler editor

Stop audition and discard unapplied draft

Cancel pending preparation and request audition release. Applied edits and committed reusable saves survive closing. An already claimed Apply reports its actual outcome.

Workflow: Record a held note.

### Sampler waveform

256 sampled PCM positions

Display bounded actual PCM samples in separate left and right lanes, plus prepared range markers. This sampled preview can miss narrow peaks and is not a complete peak analysis; audition checks the prepared sound.

Workflow: Record a held note.

### Sampler bank

Working banks; original Kit / Perc / Hits

Choose a working bank of sixteen slots. Edit banks creates, copies or imports reusable banks. New pad presses use the new bank; held voices retain their captured source.

Workflow: Record a held note.

### Sampler instrument

Samples / Analog / Keys / Pad

Choose the source for new pad presses. Synth choices have distinct sound/envelopes; held voices keep their original instrument. Drums remain sample banks, not a separate synth choice.

Workflow: Record a held note.

### Sampler octave

C1–C7; steps of 12 semitones

Transpose held instrument notes by an octave; sample rate changes by powers of two around octave 3. Matching releases retain the original input identity.

Workflow: Record a held note.

### Sampler pad

Sixteen sample identities / thirteen piano keys

Press holds a gate; release stops it, including outside the cell. Space/Enter hold the focused pad; assistive click toggles hold. Sample pads are 9–16 above 1–8; blank piano cells are inert. Armed compose writes one captured note.

Workflow: Record a held note.

### Search crate

Text filter · AND terms and explicit fields

Combine ordinary words or quoted field values with AND. Text fields: title, artist, key (exact), tag (exact), group and note. Use bpm:120..128, bpm>=120, length:3:00..4:00, rating>=4, played:yes/no or color:#RRGGBB. Length also accepts seconds; ranges include both ends. Unknown fields and malformed values show an error and no results. Played means renderer-confirmed playback of the current source/version. Text editing owns its keys; filtering never changes loaded media.

Workflow: Prepare a DJ deck.

### Search all library

Current crate / whole library

Broaden the current query without changing the named crate. Returning restores that crate's saved query, selected source and scroll. Changing the selected crate deliberately discards the old return context. This switch changes only library navigation.

Workflow: Prepare a DJ deck.

### Scan library

Background filesystem discovery

Scan configured watched folders with bounded traversal. Known removable roots resolve by UUID even at a new mountpoint; an offline volume is not a deleted file. Readable supported entries publish together; skipped reasons and incomplete coverage are visible. Cancellation keeps the prior crate, and the persistent catalog retains saved identities absent from a scan.

Workflow: Prepare a DJ deck.

### Cancel scan

Pending operation

Request cancellation and retain the existing crate. An in-progress filesystem call finishes before cancellation is acknowledged.

Workflow: Prepare a DJ deck.

### Crate row / selection

Filtered row number

Select with click or focused Up/Down/Page/Home/End. Double-click or Enter loads the selected deck; alternate actions choose A or B. BPM provenance, duration and history describe the exact source.

Workflow: Prepare a DJ deck.

### Selected deck

A / B

Choose the destination for F, Enter and controller Load selected. Explicit →A/→B always use their named deck. Browsing later does not redirect an already accepted load.

Workflow: Prepare a DJ deck.

### Lock playing deck

Per-deck replacement protection

Prevent mouse, keyboard, drop, MIDI and IPC load/eject while this deck plays or is touched. Native deliberate replacement captures the current track and chosen source for review; the existing track remains until decoding succeeds. Changing the track invalidates approval. Performance mode still requires pausing and releasing the deck. Lock state is a runtime safety setting; a restarted engine starts unlocked.

Workflow: Prepare a DJ deck.

### Load selected crate source

Deck A / B

Request the captured source on the named deck. Queued/decoding are not Loaded; wait for renderer confirmation. Failures retain existing media. Built-in stems bypass file decoding.

Workflow: Prepare a DJ deck.

### Edit session layout

128 tracks, 512 scenes; worker preparation and bounded Undo

Create audio/MIDI tracks and scenes; rename, reorder by display position, choose RGB colors, duplicate or delete. Go to track and Go to scene reveal any active cell, including offscreen rows and columns. All controls accept keyboard and assistive actions. Reorder preserves playing clips, automation, compose targets and stable controller slots. Duplicates share immutable embedded audio and copy notes/settings with fresh IDs, starting stopped. Delete ends that target's captures and voices; Undo restores content without inventing a held key or restarting playback. Limits are 65536 session notes, 8192 per clip, 16 MiB MIDI lanes, 64 MiB native metadata and 256 MiB prepared effect buffers, plus existing PCM/edit/undo budgets. A refused or cancelled edit preserves current work. MIDI routing Apply/Retry attaches a route to the current track identity after deletion/reuse or project change.

Workflow: Edit and undo.

### Track header

Up to 128 tracks

Click toggles mute; right-click or alternate actions toggle solo. Alternate actions also select the track or open its effect rack. Muted playback keeps advancing.

Workflow: Mix a session.

### Track gain

0–120% linear gain

Adjust track output level. Click toggles mute; right-click toggles solo; Control-click or alternate action opens effects. Held source ownership does not move with selection.

Workflow: Mix a session.

### Scene launch

Up to 512 scene rows

Primary action launches or stops the scene; Shift adds its clips alongside other scenes; right-click restarts. Alternate actions expose the same choices and the scene effect rack.

Workflow: Mix a session.

### Clip cell

Track × scene

Click launches once; right-click launches looping. Shift-click explicitly arms composition; Alt-click opens gain. Shift+F10 exposes named actions. Queued launch is distinct from playing.

Workflow: Record a held note.

### Clip gain

0–150% linear gain

Change gain for future note onsets. Already held synth/sample notes retain their onset gain; track gain still affects the whole output.

Workflow: Edit and undo.

### Add effect

Track or scene rack

Add the named implemented processor to the current rack. Arp is available only on a MIDI track's upstream events, not a scene audio bus. Capacity rejection is explicit.

Workflow: Mix a session.

### Effect enabled

On / off

Enable or bypass this slot with a bounded 5 ms transition. Arp changes MIDI event scheduling rather than blending audio.

Workflow: Mix a session.

### Effect parameter

Per-effect displayed units and range

Each slider names its implemented DSP control. The detailed label, current value and tooltip come from the same DSP metadata. Delay time is fixed at 250 ms; Arp has only enable/bypass.

Workflow: Mix a session.

### Close effect rack

View only

Return to the clip grid without removing or bypassing the rack's effects.

Workflow: Mix a session.

### Master effect type

Echo → Reverb → Filter

Cycle this stereo master slot's processor. Filter is a fixed 1 kHz low-pass. Each slot has independent left/right histories.

Workflow: Mix a session.

### Master effect wet

0–100%

Blend dry and processed output for this master slot. Zero preserves dry audio; 100% selects processed output.

Workflow: Mix a session.

### MIDI connections

Port status and input counters

Open the MIDI window to inspect actual connection state, queue/drop/reset counters and per-port errors. Device names alone do not prove mapping or hardware behavior.

Workflow: Connect a controller.

### MIDI learn

256 exact-port assignments; one captured gesture

Choose a performance action, target and compatible message type. Relative encoders require an explicit format and sensitivity. Capture consumes one gesture for review; unrelated controls retain their mapping. Conflicts require Replace. Test submits the captured action through ordinary control admission. Add/Replace/Edit/Remove affect this run; Save MIDI assignments retains the active profile after its durable preferences receipt. Remove restores any built-in action. Cancel, close, timeout and disconnect end learning; existing assignments remain. Input changes release older source gates and fence stale queued messages. Missing exact ports remain unavailable until recaptured.

Workflow: Connect a controller.

### Retry / rescan MIDI

Background connection request

Discover current ports and retry the requested input policy, then rescan saved output routing. Vanished outputs reset/retire and remain unavailable until an exact reconnect. No automatic hotplug detection is claimed. Busy means work is pending; missing/failed ports remain explicit. Keyboard and mouse remain available.

Workflow: Connect a controller.

### Project menu

Native .omat documents

Create, save or open a document. New/Open/Close protect unsaved work and restore playback stopped.

Workflow: Save and reopen.

### New project

Empty session

Replace the document with an empty session and factory sampler resources only after the unsaved-work decision. Use Cancel to keep working.

Workflow: Save and reopen.

### Open project

Native .omat file

Read and validate a project off the UI/audio thread before replacing the current document. Failed or unsupported files preserve the current session.

Workflow: Save and reopen.

### Recent projects

Local paths

Open a previously successful project path with the same validation and unsaved-work protection as Open. A listed path may no longer exist.

Workflow: Save and reopen.

### Save project

Current .omat path

Save a coherent renderer capture. Edits made after capture remain unsaved. An error does not mark the document clean.

Workflow: Save and reopen.

### Save project as

New current .omat path

Choose a destination and make it current after successful save. Explicit replacement permission is required for an existing destination.

Workflow: Save and reopen.

### Save copy

Independent .omat destination

Write a copy without changing the current project path or clean/dirty baseline.

Workflow: Save and reopen.

### Project path

Absolute or working-directory-relative path

Enter the local native project destination or source. No network upload occurs. Cancel preserves the current document.

Workflow: Save and reopen.

### Replace existing project file

Explicit overwrite permission

Permit replacement of this chosen destination for Save as/Copy. The file is staged and committed atomically; rejected saves preserve the previous file.

Workflow: Save and reopen.

### Discard unsaved changes

Destructive document decision

Authorize this pending New/Open/Close without saving the current unsaved work. This is not an undo-history operation.

Workflow: Save and reopen.

### Cancel project operation

Pending dialog or worker

Keep the current document by cancelling the pending decision/operation before commit. A commit already completed is reported as completed, not undone.

Workflow: Save and reopen.

### Undo

Previous named creative transaction

Restore the previous recorded creative edit while preserving unrelated transport and input gates. Availability is renderer-confirmed, bounded and separate from queue admission.

Workflow: Edit and undo.

### Redo

Next named creative transaction

Reapply the next recorded edit. A new edit clears the redo branch. Rejected history operations leave a visible reason.

Workflow: Edit and undo.

### Edit history

Bounded named transactions

Inspect applied/redo entries, target context and memory accounting. Continuous gestures group; unsupported future features are not represented.

Workflow: Edit and undo.

### Dismiss history message

Message only

Clear the renderer's history notice. It does not retry a rejected edit or recover evicted history.

Workflow: Edit and undo.

### Audio diagnostics

Measured CPU, wall time, queues and sampled load

Inspect callback elapsed time/deadlines separately from render-thread CPU, UI update time and reported output latency. Backend XRUN count remains unavailable when the backend supplies none.

Workflow: Stop and recover.

### Start diagnostic capture

Bounded local sample report

Capture measured counters and sampled render cost with bounded storage. No project audio or personal media paths are exported.

Workflow: Stop and recover.

### Stop diagnostic capture

Capture state

Finish collecting diagnostic samples and keep the captured report for review/export. Audio playback is unchanged.

Workflow: Stop and recover.

### Cancel diagnostic operation

Capture or file worker

Cancel the pending capture/file operation. The audio engine is unaffected; an already committed file is reported honestly.

Workflow: Stop and recover.

### Cancel diagnostic file operation

Pending worker

Cancel the pending report export or reopen before commit. A file already committed is reported as completed; live audio is unchanged.

Workflow: Stop and recover.

### Diagnostic file path

Local JSON path

Choose a local destination/source for a redacted diagnostic report. Export refuses an existing destination instead of overwriting it.

Workflow: Stop and recover.

### Export redacted diagnostics

Local JSON file

Write the bounded captured report on a worker. Wait for exported confirmation; no upload or automatic sharing occurs.

Workflow: Stop and recover.

### Reopen diagnostic capture

Validated local JSON file

Read a prior report for inspection without changing the live engine. Invalid/oversized reports are rejected.

Workflow: Stop and recover.

### Library catalog

Persistent preparation and media identity

Inspect catalog save status or import an Omatainer catalog. The library is separate from native projects and never substitutes another file's preparation by path alone.

Workflow: Prepare a DJ deck.

### Catalog import path

Local Omatainer catalog JSON

Enter the catalog to validate and merge. This does not import arbitrary third-party library formats.

Workflow: Prepare a DJ deck.

### Import catalog

Validated background merge

Merge compatible identities and preparation off the UI thread. Malformed/newer/conflicting data is rejected; unrelated pending edits remain preserved.

Workflow: Prepare a DJ deck.

### Music paths

Explicit file and folder inputs

Enter one absolute local music file or folder per line. The background worker reads at most 64 inputs, 64 folder levels, one million entries and 100,000 library rows. Unsupported, unreadable and truncated entries have visible reasons.

Workflow: Prepare a DJ deck.

### Import music files/folders

Background music discovery

Wait for the current catalog save, then merge readable audio files/folders without unloading either deck. Overlapping inputs are deduplicated; mounted removable media keeps UUID/relative-path identity. Cancellation before publication keeps the prior crate.

Workflow: Prepare a DJ deck.

### Manage music folders

Saved library root preferences

Open Preferences to edit and apply watched music folders. After a scan, changes coalesce on the filesystem worker; a full-scan hint every 30 idle seconds covers missed notifications. Scans wait for Studio and current catalog work; filesystem work can delay completion. Known volumes reconnect by UUID; offline or ambiguous media stays explicit. Removing a root deletes only its bookmark, never tracks or audio.

Workflow: Prepare a DJ deck.

### Retry library save

Persistent preparation

Retry a failed catalog publication. Keep the visible failure until durable saving is confirmed.

Workflow: Stop and recover.

### Close without saving library

Explicit data-loss decision

Exit despite unsaved library preparation after a save failure. Keep working or Retry preserves a chance to save it.

Workflow: Stop and recover.

### Licenses and notices

Installed build records

Read bundled notices for this build offline. The native manifest identifies the packaged components and exact source records.

Workflow: Audio setup.

### Search license notices

Text filter

Filter local component/notice entries. No network lookup occurs.

Workflow: Audio setup.

### Keep working

Cancel pending exit

Cancel the pending close while library saving is pending or failed and return to the current document. Unsaved library edits remain available for retry.

Workflow: Stop and recover.

### Notice source link

External upstream URL

Open the recorded upstream source in your browser. This leaves the offline viewer; the bundled notice remains available without network access.

Workflow: Audio setup.

### Full license notice

Bundled text

Expand the complete license or attribution text recorded for this build. Reading does not change audio or project state.

Workflow: Audio setup.

### License entry

Component record / notice text

Select or expand a bundled notice to read its attribution and terms.

Workflow: Audio setup.

### Retry media load

Captured source and destination

Retry the failed source on its original deck. Later crate selection does not redirect Retry. Existing playing media stays until accepted replacement is applied.

Workflow: Prepare a DJ deck.

### Dismiss media status

Message only

Hide this load status. It does not unload the deck, cancel playback or discard an already rendered play-history event.

Workflow: Prepare a DJ deck.

## Shortcut reference

The in-app Help window shows the active bindings. Defaults follow; text fields and dialogs own their keys before performance shortcuts.

- Space: Play / stop session
- 1: Launch scene 1
- 2: Launch scene 2
- 3: Launch scene 3
- 4: Launch scene 4
- 5: Launch scene 5
- 6: Launch scene 6
- 7: Launch scene 7
- 8: Launch scene 8
- Q: Play / pause deck A
- A: Cue deck A
- W: Toggle deck A tempo sync
- P: Play / pause deck B
- L: Cue deck B
- O: Toggle deck B tempo sync
- [: Crossfader fully to A
- ]: Crossfader fully to B
- F: Load selected crate item onto selected deck
- ? (Shift+/): Show / hide contextual help and lessons
- F1: Show / hide contextual help and lessons
- Ctrl+M: Show / hide MIDI window
- Escape: Close effect chain
- Ctrl+Z: Undo last creative edit
- Ctrl+Shift+Z: Redo next creative edit
- Ctrl+Y: Redo next creative edit
