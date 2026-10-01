# Issue 102 — local support reports and safe startup

The ordered implementation and final source-bound release qualification are recorded below.

Support evidence is an explicit typed allowlist: build/source-manifest identity and selected linked crate versions, numeric requested/backend-accepted routes, bounded structured event codes, existing atomic callback/command/MIDI counters, and opaque exact recovery references. Runtime driver/converter details and audio plugin hosting remain unavailable. No media, waveform, project content, notes, paths/titles, device or port names, raw error/panic text, environment, credentials, core dump or upload client is collected.

The private support root is `$XDG_STATE_HOME/omatainer/support`, falling back to `$HOME/.local/state/omatainer/support`. Directories are 0700 and files 0600. Reports are at most 4 MiB, with 512 event rows, 120 performance samples and eight recovery references. One bounded 256-observation lane feeds the collector; rejected observations have a separate counter. Eight inactive runs and 40 MiB of logical files, including staging, are the retention/cap limits. Unknown entries are preserved rather than pruned. Creation/retirement uses resumable namespaces under a nonblocking root mutation lock. Active run locks prevent reading or pruning another live process's evidence.

A durable fixed marker precedes engine startup. The panic hook writes one fixed state byte using the preopened file descriptor; it never serializes payload/backtrace or performs fsync. It is best effort: abrupt termination or write failure can leave an unclean marker, which does not identify a cause. Normal shutdown explicitly records clean completion; startup failures are distinct. Reports persist periodically and can lag observations by five seconds plus I/O delay. The displayed confirmed persistence time is the evidence boundary. A missing report leaves build identity unknown rather than attributing the current reader's build to an earlier crash.

Recovery references contain the full SHA256 of the application-owned session identity, document epoch, confirmed durable sequence/revisions and capture/commit times. Lookup verifies an exact retained record and its sidecars under the session lock. It never substitutes a newer record. Retired, pruned, active or corrupt references remain explicit. The regular recovery preview and unsaved Save/Discard/Cancel flow own restoration; support export contains no recovery assets.

`--safe-mode` runs a real stopped renderer owner without constructing CPAL or MIDI devices. Its project capture/install service remains available for Open, recovery, Save and close. It starts empty, uses built-in appearance, does not apply startup preferences, and defers external theme/library/recovery discovery. Saved files/settings are preserved. Live playback and device transactions are centrally refused. An explicit normal restart must complete the ordinary unsaved decision and close flow.

`--safe-mode --startup-check` uses that exact engine helper without opening a window, emits bounded sanitized JSON, and exits cleanly. It is **headless startup evidence**, not native GUI, audible-device, controller or human accessibility QA. Unknown, duplicate and conflicting startup arguments fail. Safe startup refuses an existing owned instance without focusing, controlling or replacing it.

Validation evidence:

- Focused support checks passed (36 groups in the final filtered run, plus two guarded/opt-in child entries). Their private child fixture executes an actual panic-abort and SIGKILL, with core dumps disabled, then verifies marker classification and exact real durable recovery linkage. The guarded child entry point is not a standalone test.
- Production build passed for the initial checkpoint.
- `scripts/check-safe-startup.py --binary …` passed against that built binary: real offline capture, zero callbacks, malformed preferences preserved, strict flags, an already-owned runtime refused without focus, and read-only support storage reported without a false clean-marker claim.

- Seven actual App workflow groups passed: inspect/review/export/reopen and invalid-file preservation; Cancel/Discard/Save before restart; optional-work cancellation/protection; actual IPC parser rejection; actual controlled audio-owner fault; real recovery ENOSPC injection followed by confirmed durable linkage; exact linked preview and stopped untitled restore. The audio fixture drives the real callback/owner with a controlled backend, not physical hardware.
- Actual warmed `OutputCallback` rendering under an attached, saturated support collector measured zero Rust allocations/frees. No new callback logging or filesystem path is introduced.
- Private Linux AT-SPI support workflow passed: 244 observed native nodes, nine native actions, zero accepted/rejected engine submissions, actual safe App/offline project owner, review/consent/export/reopen/restart request, and zero backend callbacks. No native window or actual process exec is exercised by that bridge. The shipped CLI fixture separately covers startup construction and shutdown.
- The pinned AccessKit AT-SPI adapter reports disabled custom Buttons as Enabled. Actual egui nodes are disabled, the native slider exposes its disabled state, and native button attempts cannot activate playback or pads. This dependency state discrepancy is disclosed; no Orca/usability fix is claimed.

The full ordinary suite passed **822 tests / 17 opt-in or maintainer entries ignored** in 95.69 seconds with two test threads. After the narrow route-schema clarification, 36 focused groups passed again; the production build, real private startup CLI, fresh private native support workflow, license inventory validation and exact embedded manifest/notices comparison passed. Combined ordered-stack/release qualification remains with the parent task. No physical device, plugin-host containment, network upload, or human screen-reader testing is claimed.

Reproduction (private Cargo target; no hardware required):

```sh
cargo test -- --test-threads=2
cargo test support -- --test-threads=2
cargo build
python3 scripts/check-safe-startup.py --binary /path/to/debug/omatainer
python3 scripts/check-accessibility.py --support --test-binary /path/to/debug/deps/omatainer-TEST_HASH
python3 scripts/license-manifest.py check
```

MIDI sample counters are ordered `received, queued, dispatched, coalesced,
dropped, source resets, oversized, disconnected`; all are cumulative numeric
observations. Callback wall time and DSP thread CPU retain their distinct units
and unavailable values from the existing telemetry contract. Requested routes
identify system-default intent separately from the resolved, redacted backend
route; mono sum / main stereo pair / remaining silent channels are explicit.

The full-suite result above precedes one isolated route-schema regression and a
native fixture assertion that disabled button attempts cause no command
submission; the final focused/native/build runs include both. No unchanged
performance policy was rerun on this private preparation branch, and it is not
publication/package evidence for the final assembled stack.

## Final ordered-stack qualification

The assembled source `7155ff7`, based on final issue101 `48e6a49`, includes exact
preview consent, linked recovery action identity and repeated-reference native
coverage. The final ordinary suite passed **892 tests**, with **17 opt-in or
maintainer entries ignored**, in **45.67 seconds** with four test threads.
`issue-102-final-full-v2.log` retains the run. The actual offline-manual generator
also passed; the license inventory was regenerated from the final source.

The unchanged local performance gate passed from `2026-10-01T10:41:15.985368+00:00` to
`2026-10-01T10:44:54.811437+00:00` with other agents' builds/tests paused. Fresh locked offline
release/application-test builds and all eight fixed workloads across three
sessions passed. Native general preflight covered **123 actions, 242 nodes and
1,099 App frames**. The exact release binary SHA-256 is
`82014f64453895b0414274c966c7d9db6e310564e2b40c2641a87d1496db803d`.

Worst callback wall p99 / maximum across three sessions, in milliseconds:
producer **0.831381 / 1.389134**, composer **0.811672 / 1.186258**, live DJ
**0.092876 / 0.165793**, hybrid **0.843214 / 1.194257**. Measured callback and
recording allocations/frees, rejected commands and MIDI drops were zero.
Worst large-crate frame p99 was **3.259063 ms**; multi-controller frame p99
**4.866448 ms**, IPC round-trip p99 **8.549723 ms**, MIDI dispatch p99
**3.192271 ms**. These are local fixture timings, not hardware/device latency.

The separate final-source private native safe-support workflow passed
**9 actions, 245 nodes and 163 App frames**: inspect, explicit consent,
exact local export, reopen and normal-restart request, with zero callbacks or
accepted/rejected engine commands. It exercises the actual safe App and offline
project owner through Linux AT-SPI, without a native window, Orca, devices or
actual process exec. Report: `issue-102-final-native-support.log`.

The actual release `check-safe-startup.py` passed headless safe construction,
offline project service, malformed-preferences preservation, strict arguments,
existing-instance refusal and read-only diagnostics. Independent performance
report checking, release packaging/package verification and all **7 CLI**,
**6 follow-protocol** and **5 runtime-isolation** groups passed. Retained evidence
under `/home/michael/Projects/omatainer-work` includes `issue-102-final-package`,
`issue-102-final-performance.{json,raw.json,log}` and `issue-102-stack-gate.log`.

Plugin failure is a typed simulated diagnostic observation; no audio plugin host
exists to qualify containment. There is no upload or network provider client.
The disabled native custom-button state discrepancy described above remains
explicit; actions are inert, but no screen-reader usability qualification is
claimed. Physical producer/composer/live-DJ controller, hardware latency and
human listening/accessibility acceptance remain for the user's final runs.
No issue is closed and no PR is merged by these checks.
