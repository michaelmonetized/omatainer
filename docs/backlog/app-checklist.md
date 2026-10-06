# App completion checklist

Reviewed 2026-10-06. Source base: `6c06d18a41c1d49de5c338b0471b45087f9ccc8b`. Tracking: [#1](https://github.com/michaelmonetized/omatainer/issues/1).

Physical hardware qualification is paused at Michael's request. Existing captures are closed; software work continues without new physical gestures.

Each issue appears once, after its dependencies. The Code column checks off implemented code, including the existing stack, and does not claim complete acceptance. Accepted requires complete recorded acceptance. The eight provider integrations previously excluded by Michael stay outside this release. No open issue is automatically closed by this checklist.

127 existing stack · 21 implemented · 6 partial · 194 planned · 0 accepted · 8 outside release

- [x] High-resolution stereo source peaks with measured rainbow frequency bands, native-rate validation, project reopen and pixel-scaled rendering. [Receipt](../validation/rainbow-waveforms.md)

| Code | Issue | Capability | State | Depends on | Evidence / existing PRs |
| --- | --- | --- | --- | --- | --- |
| [x] | [#2](https://github.com/michaelmonetized/omatainer/issues/2) | Reject invalid scene indexes at CLI, IPC and engine boundaries | existing stack |  | [#358](https://github.com/michaelmonetized/omatainer/pull/358) |
| [x] | [#3](https://github.com/michaelmonetized/omatainer/issues/3) | Stop allocating and sorting arpeggiator chords every sample | existing stack |  | [#359](https://github.com/michaelmonetized/omatainer/pull/359) |
| [x] | [#4](https://github.com/michaelmonetized/omatainer/issues/4) | Release clip-owned voices when tracks or scenes stop or replace clips | existing stack |  | [#360](https://github.com/michaelmonetized/omatainer/pull/360) |
| [x] | [#5](https://github.com/michaelmonetized/omatainer/issues/5) | Bound control-command work per audio block | existing stack |  | [#361](https://github.com/michaelmonetized/omatainer/pull/361) |
| [x] | [#6](https://github.com/michaelmonetized/omatainer/issues/6) | Reset track-specific loop state when replacing deck audio | existing stack |  | [#362](https://github.com/michaelmonetized/omatainer/pull/362) |
| [x] | [#7](https://github.com/michaelmonetized/omatainer/issues/7) | Separate left and right deck EQ and filter histories | existing stack |  | [#363](https://github.com/michaelmonetized/omatainer/pull/363) |
| [x] | [#8](https://github.com/michaelmonetized/omatainer/issues/8) | Prevent GUI and IPC engine locks from zeroing audio buffers | existing stack |  | [#364](https://github.com/michaelmonetized/omatainer/pull/364) |
| [x] | [#9](https://github.com/michaelmonetized/omatainer/issues/9) | Give every stacked FX slot its own processing state | existing stack |  | [#365](https://github.com/michaelmonetized/omatainer/pull/365) |
| [x] | [#10](https://github.com/michaelmonetized/omatainer/issues/10) | Remove FX slot allocations from the sample loop | existing stack |  | [#366](https://github.com/michaelmonetized/omatainer/pull/366) |
| [x] | [#11](https://github.com/michaelmonetized/omatainer/issues/11) | Handle full control queues without silently dropping GUI commands | existing stack |  | [#367](https://github.com/michaelmonetized/omatainer/pull/367) |
| [x] | [#12](https://github.com/michaelmonetized/omatainer/issues/12) | Reset pitch-lock grain history after seeks, cues and media replacement | existing stack |  | [#368](https://github.com/michaelmonetized/omatainer/pull/368) |
| [x] | [#13](https://github.com/michaelmonetized/omatainer/issues/13) | Correct stereo timing and channel state in master delay and reverb | existing stack |  | [#369](https://github.com/michaelmonetized/omatainer/pull/369) |
| [x] | [#14](https://github.com/michaelmonetized/omatainer/issues/14) | Remove per-sample MIDI clip cloning and repeated full-note scans | existing stack |  | [#370](https://github.com/michaelmonetized/omatainer/pull/370) |
| [x] | [#15](https://github.com/michaelmonetized/omatainer/issues/15) | Advance note and drum lifecycles while tracks are muted or excluded by solo | existing stack |  | [#371](https://github.com/michaelmonetized/omatainer/pull/371) |
| [x] | [#16](https://github.com/michaelmonetized/omatainer/issues/16) | Route note-offs back to the track and voice that received note-on | existing stack |  | [#372](https://github.com/michaelmonetized/omatainer/pull/372) |
| [x] | [#17](https://github.com/michaelmonetized/omatainer/issues/17) | Render silence from stopped decks instead of a frozen sample | existing stack |  | [#373](https://github.com/michaelmonetized/omatainer/pull/373) |
| [x] | [#18](https://github.com/michaelmonetized/omatainer/issues/18) | Keep quantized clips pending until their scheduled start beat | existing stack |  | [#374](https://github.com/michaelmonetized/omatainer/pull/374) |
| [x] | [#19](https://github.com/michaelmonetized/omatainer/issues/19) | Record note positions relative to the selected clip and its actual length | existing stack |  | [#375](https://github.com/michaelmonetized/omatainer/pull/375) |
| [x] | [#20](https://github.com/michaelmonetized/omatainer/issues/20) | Record actual held note durations instead of fixed quarter-beat notes | existing stack |  | [#376](https://github.com/michaelmonetized/omatainer/pull/376) |
| [x] | [#21](https://github.com/michaelmonetized/omatainer/issues/21) | Reconfigure every rate-dependent processor for the output sample rate | existing stack |  | [#377](https://github.com/michaelmonetized/omatainer/pull/377) |
| [x] | [#22](https://github.com/michaelmonetized/omatainer/issues/22) | Route instrument pads through one deliberate mixer destination | existing stack |  | [#378](https://github.com/michaelmonetized/omatainer/pull/378) |
| [x] | [#23](https://github.com/michaelmonetized/omatainer/issues/23) | Preserve session stereo when processing scene FX | existing stack |  | [#379](https://github.com/michaelmonetized/omatainer/pull/379) |
| [x] | [#24](https://github.com/michaelmonetized/omatainer/issues/24) | Keep FX state and settings separate for each scene | existing stack |  | [#380](https://github.com/michaelmonetized/omatainer/pull/380) |
| [x] | [#25](https://github.com/michaelmonetized/omatainer/issues/25) | Publish audio snapshots without blocking on UI or IPC readers | existing stack |  | [#381](https://github.com/michaelmonetized/omatainer/pull/381) |
| [x] | [#26](https://github.com/michaelmonetized/omatainer/issues/26) | Replace the installed executable atomically | existing stack |  | [#382](https://github.com/michaelmonetized/omatainer/pull/382) |
| [x] | [#27](https://github.com/michaelmonetized/omatainer/issues/27) | Make multi-file installation transactional and recoverable | existing stack |  | [#383](https://github.com/michaelmonetized/omatainer/pull/383) |
| [x] | [#28](https://github.com/michaelmonetized/omatainer/issues/28) | Return errors for malformed JSON and unknown IPC operations | existing stack |  | [#384](https://github.com/michaelmonetized/omatainer/pull/384) |
| [x] | [#29](https://github.com/michaelmonetized/omatainer/issues/29) | Surface IPC startup failure instead of running without the control surface | existing stack |  | [#385](https://github.com/michaelmonetized/omatainer/pull/385) |
| [x] | [#30](https://github.com/michaelmonetized/omatainer/issues/30) | Bound IPC request size and idle connection lifetime | existing stack |  | [#386](https://github.com/michaelmonetized/omatainer/pull/386) |
| [x] | [#31](https://github.com/michaelmonetized/omatainer/issues/31) | Do not unlink a live instance socket after a slow response | existing stack |  | [#387](https://github.com/michaelmonetized/omatainer/pull/387) |
| [x] | [#32](https://github.com/michaelmonetized/omatainer/issues/32) | Keep the fallback control socket private to the current user | existing stack |  | [#388](https://github.com/michaelmonetized/omatainer/pull/388) |
| [x] | [#33](https://github.com/michaelmonetized/omatainer/issues/33) | Report shell control results and avoid replacing in-flight commands | existing stack |  | [#389](https://github.com/michaelmonetized/omatainer/pull/389) |
| [x] | [#34](https://github.com/michaelmonetized/omatainer/issues/34) | Load built-in crate entries through the built-in media path | existing stack |  | [#390](https://github.com/michaelmonetized/omatainer/pull/390) |
| [x] | [#35](https://github.com/michaelmonetized/omatainer/issues/35) | Report corrupt and incomplete audio decoding instead of silently accepting it | existing stack |  | [#391](https://github.com/michaelmonetized/omatainer/pull/391) |
| [x] | [#36](https://github.com/michaelmonetized/omatainer/issues/36) | Move filesystem scanning off the UI and startup rendering path | existing stack |  | [#392](https://github.com/michaelmonetized/omatainer/pull/392) |
| [x] | [#37](https://github.com/michaelmonetized/omatainer/issues/37) | Reject stale deck load completions and cancel obsolete decode jobs | existing stack |  | [#393](https://github.com/michaelmonetized/omatainer/pull/393) |
| [x] | [#38](https://github.com/michaelmonetized/omatainer/issues/38) | Remove duplicate APC40 fader bindings that alter every track | existing stack |  | [#394](https://github.com/michaelmonetized/omatainer/pull/394) |
| [x] | [#39](https://github.com/michaelmonetized/omatainer/issues/39) | Keep MIDI input callbacks from blocking on the engine queue | existing stack |  | [#395](https://github.com/michaelmonetized/omatainer/pull/395) |
| [x] | [#40](https://github.com/michaelmonetized/omatainer/issues/40) | Connect controller load commands to the actual library loader | existing stack |  | [#396](https://github.com/michaelmonetized/omatainer/pull/396) |
| [x] | [#41](https://github.com/michaelmonetized/omatainer/issues/41) | Parse one-byte MIDI real-time messages before channel-message length checks | existing stack |  | [#397](https://github.com/michaelmonetized/omatainer/pull/397) |
| [x] | [#42](https://github.com/michaelmonetized/omatainer/issues/42) | Decode relative jog movement with correct signed deltas | existing stack |  | [#398](https://github.com/michaelmonetized/omatainer/pull/398) |
| [x] | [#43](https://github.com/michaelmonetized/omatainer/issues/43) | Make pad composition arming explicit and keep its target stable | existing stack |  | [#399](https://github.com/michaelmonetized/omatainer/pull/399) |
| [x] | [#44](https://github.com/michaelmonetized/omatainer/issues/44) | Virtualize crate rows and cache unchanged filter/format work | existing stack |  | [#400](https://github.com/michaelmonetized/omatainer/pull/400) |
| [x] | [#45](https://github.com/michaelmonetized/omatainer/issues/45) | Show loading, failure and completion status to the user | existing stack |  | [#401](https://github.com/michaelmonetized/omatainer/pull/401) |
| [x] | [#46](https://github.com/michaelmonetized/omatainer/issues/46) | Suppress global transport shortcuts while editing text | existing stack |  | [#402](https://github.com/michaelmonetized/omatainer/pull/402) |
| [x] | [#47](https://github.com/michaelmonetized/omatainer/issues/47) | Reject malformed UTF-8 color values without panicking | existing stack |  | [#403](https://github.com/michaelmonetized/omatainer/pull/403) |
| [x] | [#48](https://github.com/michaelmonetized/omatainer/issues/48) | Measure the entire audio callback workload in CPU telemetry | existing stack |  | [#404](https://github.com/michaelmonetized/omatainer/pull/404) |
| [x] | [#49](https://github.com/michaelmonetized/omatainer/issues/49) | Apply the stored clip gain during clip playback | existing stack |  | [#405](https://github.com/michaelmonetized/omatainer/pull/405) |
| [x] | [#50](https://github.com/michaelmonetized/omatainer/issues/50) | Make the DJ low-pass sweep progress smoothly away from bypass | existing stack |  | [#406](https://github.com/michaelmonetized/omatainer/pull/406) |
| [x] | [#51](https://github.com/michaelmonetized/omatainer/issues/51) | Stop cloning drum sample Arcs on every output frame | existing stack |  | [#407](https://github.com/michaelmonetized/omatainer/pull/407) |
| [x] | [#52](https://github.com/michaelmonetized/omatainer/issues/52) | Apply MIDI velocity to drum voices | existing stack |  | [#408](https://github.com/michaelmonetized/omatainer/pull/408) |
| [x] | [#53](https://github.com/michaelmonetized/omatainer/issues/53) | Make neutral and empty FX chains sample-transparent | existing stack |  | [#409](https://github.com/michaelmonetized/omatainer/pull/409) |
| [x] | [#54](https://github.com/michaelmonetized/omatainer/issues/54) | Apply delay, reverb and chorus wet mix exactly once | existing stack |  | [#410](https://github.com/michaelmonetized/omatainer/pull/410) |
| [x] | [#55](https://github.com/michaelmonetized/omatainer/issues/55) | Route legacy hardware FX selection into the actual renderer | existing stack |  | [#411](https://github.com/michaelmonetized/omatainer/pull/411) |
| [x] | [#56](https://github.com/michaelmonetized/omatainer/issues/56) | Render a timed metronome click instead of one sample per beat | existing stack |  | [#412](https://github.com/michaelmonetized/omatainer/pull/412) |
| [x] | [#57](https://github.com/michaelmonetized/omatainer/issues/57) | Cache unchanged crossfader and pan gain calculations | existing stack |  | [#413](https://github.com/michaelmonetized/omatainer/pull/413) |
| [x] | [#58](https://github.com/michaelmonetized/omatainer/issues/58) | Map sampler instrument choices to the sounds named by the UI | existing stack |  | [#414](https://github.com/michaelmonetized/omatainer/pull/414) |
| [x] | [#59](https://github.com/michaelmonetized/omatainer/issues/59) | Share immutable waveform peaks instead of cloning them on every snapshot | existing stack |  | [#415](https://github.com/michaelmonetized/omatainer/pull/415) |
| [x] | [#60](https://github.com/michaelmonetized/omatainer/issues/60) | Honor wet mix and chain order for Spread and Balance | existing stack |  | [#416](https://github.com/michaelmonetized/omatainer/pull/416) |
| [x] | [#61](https://github.com/michaelmonetized/omatainer/issues/61) | Apply the SVF cutoff limit in the correct frequency domain | existing stack |  | [#417](https://github.com/michaelmonetized/omatainer/pull/417) |
| [x] | [#62](https://github.com/michaelmonetized/omatainer/issues/62) | Connect or explicitly reject the third hardware FX wet control | existing stack |  | [#418](https://github.com/michaelmonetized/omatainer/pull/418) |
| [x] | [#63](https://github.com/michaelmonetized/omatainer/issues/63) | Cache synth pitch increments between note and tuning changes | existing stack |  | [#419](https://github.com/michaelmonetized/omatainer/pull/419) |
| [x] | [#64](https://github.com/michaelmonetized/omatainer/issues/64) | Reuse a status connection instead of reconnecting and spawning a thread every poll | existing stack |  | [#420](https://github.com/michaelmonetized/omatainer/pull/420) |
| [x] | [#65](https://github.com/michaelmonetized/omatainer/issues/65) | Publish MIDI device status into the shared snapshot used by CLI and shell | existing stack |  | [#421](https://github.com/michaelmonetized/omatainer/pull/421) |
| [x] | [#66](https://github.com/michaelmonetized/omatainer/issues/66) | Forward the requested scene number through the shell service | existing stack |  | [#422](https://github.com/michaelmonetized/omatainer/pull/422) |
| [x] | [#67](https://github.com/michaelmonetized/omatainer/issues/67) | Reconcile decoded BPM with crate metadata and identify filename guesses | existing stack |  | [#423](https://github.com/michaelmonetized/omatainer/pull/423) |
| [x] | [#68](https://github.com/michaelmonetized/omatainer/issues/68) | Populate file duration in the existing crate length column | existing stack |  | [#424](https://github.com/michaelmonetized/omatainer/pull/424) |
| [x] | [#69](https://github.com/michaelmonetized/omatainer/issues/69) | Preserve accidentals when parsing filename key hints | existing stack |  | [#425](https://github.com/michaelmonetized/omatainer/pull/425) |
| [x] | [#70](https://github.com/michaelmonetized/omatainer/issues/70) | Update play history only after successful playback | existing stack |  | [#426](https://github.com/michaelmonetized/omatainer/pull/426) |
| [x] | [#71](https://github.com/michaelmonetized/omatainer/issues/71) | Preserve existing crate history across rescans | existing stack |  | [#427](https://github.com/michaelmonetized/omatainer/pull/427) |
| [x] | [#72](https://github.com/michaelmonetized/omatainer/issues/72) | Connect controller browse events to the visible crate selection | existing stack |  | [#428](https://github.com/michaelmonetized/omatainer/pull/428) |
| [x] | [#73](https://github.com/michaelmonetized/omatainer/issues/73) | Report MIDI devices as connected only after connection succeeds | existing stack |  | [#429](https://github.com/michaelmonetized/omatainer/pull/429) |
| [x] | [#74](https://github.com/michaelmonetized/omatainer/issues/74) | Restore documented shortcuts for implemented scene, sync, crossfader and load actions | existing stack |  | [#430](https://github.com/michaelmonetized/omatainer/pull/430) |
| [x] | [#75](https://github.com/michaelmonetized/omatainer/issues/75) | Render effect-specific controls instead of inert generic sliders | existing stack |  | [#431](https://github.com/michaelmonetized/omatainer/pull/431) |
| [x] | [#76](https://github.com/michaelmonetized/omatainer/issues/76) | Align displayed sample pad numbers with dispatched pad indexes | existing stack |  | [#432](https://github.com/michaelmonetized/omatainer/pull/432) |
| [x] | [#77](https://github.com/michaelmonetized/omatainer/issues/77) | Make deck selection control the crate double-click destination | existing stack |  | [#433](https://github.com/michaelmonetized/omatainer/pull/433) |
| [x] | [#78](https://github.com/michaelmonetized/omatainer/issues/78) | Reload font and shell theme settings when those sources change | existing stack |  | [#434](https://github.com/michaelmonetized/omatainer/pull/434) |
| [x] | [#79](https://github.com/michaelmonetized/omatainer/issues/79) | Use the current Omarchy font when installing application fonts | existing stack |  | [#435](https://github.com/michaelmonetized/omatainer/pull/435) |
| [x] | [#80](https://github.com/michaelmonetized/omatainer/issues/80) | Display last-play times as meaningful dates or elapsed time | existing stack |  | [#436](https://github.com/michaelmonetized/omatainer/pull/436) |
| [x] | [#81](https://github.com/michaelmonetized/omatainer/issues/81) | Expose audio deadlines, xruns, latency and device-level performance diagnostics | existing stack |  | [#437](https://github.com/michaelmonetized/omatainer/pull/437) |
| [x] | [#82](https://github.com/michaelmonetized/omatainer/issues/82) | Save and reopen complete native production and DJ projects | existing stack |  | [#438](https://github.com/michaelmonetized/omatainer/pull/438) |
| [x] | [#83](https://github.com/michaelmonetized/omatainer/issues/83) | Provide nondestructive undo history across all creative edits | existing stack |  | [#439](https://github.com/michaelmonetized/omatainer/pull/439) |
| [x] | [#84](https://github.com/michaelmonetized/omatainer/issues/84) | Add configurable elapsed, remaining and end-of-track deck warnings | existing stack |  | [#440](https://github.com/michaelmonetized/omatainer/pull/440) |
| [x] | [#85](https://github.com/michaelmonetized/omatainer/issues/85) | Create a persistent DJ library with stable track identity | existing stack |  | [#441](https://github.com/michaelmonetized/omatainer/pull/441) |
| [x] | [#86](https://github.com/michaelmonetized/omatainer/issues/86) | Track factory content, model and integration licensing in shipped assets | existing stack |  | [#442](https://github.com/michaelmonetized/omatainer/pull/442) |
| [x] | [#87](https://github.com/michaelmonetized/omatainer/issues/87) | Expose complete keyboard and assistive-technology operation | existing stack |  | [#443](https://github.com/michaelmonetized/omatainer/pull/443) |
| [x] | [#88](https://github.com/michaelmonetized/omatainer/issues/88) | Persist user preferences and named studio/performance profiles | existing stack |  | [#444](https://github.com/michaelmonetized/omatainer/pull/444) |
| [x] | [#89](https://github.com/michaelmonetized/omatainer/issues/89) | Provide contextual help, interactive lessons and complete workflow documentation | existing stack |  | [#445](https://github.com/michaelmonetized/omatainer/pull/445) |
| [x] | [#90](https://github.com/michaelmonetized/omatainer/issues/90) | Fail installation when Hyprland reload or configuration validation fails | existing stack | #27 | [#446](https://github.com/michaelmonetized/omatainer/pull/446) |
| [x] | [#91](https://github.com/michaelmonetized/omatainer/issues/91) | Send valid status JSON from ctl follow | existing stack | #28 | [#447](https://github.com/michaelmonetized/omatainer/pull/447) |
| [x] | [#92](https://github.com/michaelmonetized/omatainer/issues/92) | Validate IPC argument types instead of defaulting or wrapping targets | existing stack | #2, #28 | [#448](https://github.com/michaelmonetized/omatainer/pull/448) |
| [x] | [#93](https://github.com/michaelmonetized/omatainer/issues/93) | Make reload-theme actually reload theme resources | existing stack | #78 | [#449](https://github.com/michaelmonetized/omatainer/pull/449) |
| [x] | [#94](https://github.com/michaelmonetized/omatainer/issues/94) | Configure professional audio devices, sample rates and buffers | existing stack | #88 | [#450](https://github.com/michaelmonetized/omatainer/pull/450) |
| [x] | [#95](https://github.com/michaelmonetized/omatainer/issues/95) | Gate releases on production-scale audio and UI workloads | existing stack | #81 | [#451](https://github.com/michaelmonetized/omatainer/pull/451) |
| [x] | [#96](https://github.com/michaelmonetized/omatainer/issues/96) | Provide a performance mode that protects the active show | existing stack | #81 | [#452](https://github.com/michaelmonetized/omatainer/pull/452) |
| [x] | [#97](https://github.com/michaelmonetized/omatainer/issues/97) | Recover unsaved work after crashes and interrupted writes | existing stack | #82 | [#453](https://github.com/michaelmonetized/omatainer/pull/453) |
| [x] | [#98](https://github.com/michaelmonetized/omatainer/issues/98) | Name, color and persist per-track hot cues | existing stack | #85 | [#454](https://github.com/michaelmonetized/omatainer/pull/454) |
| [x] | [#99](https://github.com/michaelmonetized/omatainer/issues/99) | Create and edit persistent per-track beatgrids | existing stack | #85 | [#455](https://github.com/michaelmonetized/omatainer/pull/455) |
| [x] | [#100](https://github.com/michaelmonetized/omatainer/issues/100) | Qualify DJ time stretching for transparent key lock across tempo ranges | existing stack | #81 | [#456](https://github.com/michaelmonetized/omatainer/pull/456) |
| [x] | [#101](https://github.com/michaelmonetized/omatainer/issues/101) | Load user samples into persistent DJ sampler banks | existing stack | #85 | [#457](https://github.com/michaelmonetized/omatainer/pull/457) |
| [x] | [#102](https://github.com/michaelmonetized/omatainer/issues/102) | Export actionable support bundles and recovery-safe crash diagnostics | existing stack | #81 | [#458](https://github.com/michaelmonetized/omatainer/pull/458) |
| [x] | [#103](https://github.com/michaelmonetized/omatainer/issues/103) | Preserve and qualify offline operation as optional online features arrive | existing stack | #82 | [#459](https://github.com/michaelmonetized/omatainer/pull/459) |
| [x] | [#104](https://github.com/michaelmonetized/omatainer/issues/104) | Add persistent background track analysis with selective reanalysis | existing stack | #85, #81 | [#460](https://github.com/michaelmonetized/omatainer/pull/460) |
| [x] | [#105](https://github.com/michaelmonetized/omatainer/issues/105) | Add named ordered crates and nested playlist organization | existing stack | #85 | [#461](https://github.com/michaelmonetized/omatainer/pull/461) |
| [x] | [#106](https://github.com/michaelmonetized/omatainer/issues/106) | Record actual DJ performance history with session boundaries | existing stack | #85, #70, #71 | [#462](https://github.com/michaelmonetized/omatainer/pull/462) |
| [x] | [#107](https://github.com/michaelmonetized/omatainer/issues/107) | Import arbitrary folders, files and removable music libraries | existing stack | #85, #36 | [#463](https://github.com/michaelmonetized/omatainer/pull/463) |
| [x] | [#108](https://github.com/michaelmonetized/omatainer/issues/108) | Relocate missing music while preserving DJ preparation | existing stack | #85 | [#464](https://github.com/michaelmonetized/omatainer/pull/464) |
| [x] | [#109](https://github.com/michaelmonetized/omatainer/issues/109) | Read and edit real audio metadata instead of filename guesses | existing stack | #85 | [#465](https://github.com/michaelmonetized/omatainer/pull/465) |
| [x] | [#110](https://github.com/michaelmonetized/omatainer/issues/110) | Deliver a full MIDI piano-roll editor with keyboard-accessible editing | existing stack | #82, #83 | [#466](https://github.com/michaelmonetized/omatainer/pull/466) |
| [x] | [#111](https://github.com/michaelmonetized/omatainer/issues/111) | Import and export Standard MIDI Files without losing musical timing | existing stack | #82 | [#467](https://github.com/michaelmonetized/omatainer/pull/467) |
| [x] | [#112](https://github.com/michaelmonetized/omatainer/issues/112) | Route MIDI by device, port, channel and message type | existing stack | #88 | [#468](https://github.com/michaelmonetized/omatainer/pull/468) |
| [x] | [#113](https://github.com/michaelmonetized/omatainer/issues/113) | Support user-managed tracks and scenes beyond the fixed 8 by 8 grid | existing stack | #82, #83 | [#469](https://github.com/michaelmonetized/omatainer/pull/469) |
| [x] | [#114](https://github.com/michaelmonetized/omatainer/issues/114) | Implement tempo automation and changing time signatures | existing stack | #82 | [#471](https://github.com/michaelmonetized/omatainer/pull/471) |
| [x] | [#115](https://github.com/michaelmonetized/omatainer/issues/115) | Relink missing project media and restore unavailable devices | existing stack | #82 | [#472](https://github.com/michaelmonetized/omatainer/pull/472) |
| [x] | [#116](https://github.com/michaelmonetized/omatainer/issues/116) | Collect project media and package portable self-contained sessions | existing stack | #82 | [#474](https://github.com/michaelmonetized/omatainer/pull/474) |
| [x] | [#117](https://github.com/michaelmonetized/omatainer/issues/117) | Save reusable project, track and live-set templates | existing stack | #82 | [#476](https://github.com/michaelmonetized/omatainer/pull/476) |
| [x] | [#118](https://github.com/michaelmonetized/omatainer/issues/118) | Provide scalable, high-contrast and color-independent layouts | existing stack | #88 | [#478](https://github.com/michaelmonetized/omatainer/pull/478) |
| [x] | [#119](https://github.com/michaelmonetized/omatainer/issues/119) | Expose a versioned automation and remote-control API | existing stack | #82 | [#480](https://github.com/michaelmonetized/omatainer/pull/480) |
| [x] | [#120](https://github.com/michaelmonetized/omatainer/issues/120) | Add licensed music-provider integration with explicit capability contracts | existing stack | #85 | [#481](https://github.com/michaelmonetized/omatainer/pull/481) |
| [x] | [#121](https://github.com/michaelmonetized/omatainer/issues/121) | Add track ratings, colors and performance annotations | existing stack | #85 | [#483](https://github.com/michaelmonetized/omatainer/pull/483) |
| [x] | [#122](https://github.com/michaelmonetized/omatainer/issues/122) | Score to video with frame-accurate playback and timecode | existing stack | #82 | [#485](https://github.com/michaelmonetized/omatainer/pull/485) |
| [x] | [#123](https://github.com/michaelmonetized/omatainer/issues/123) | Import tracks, clips and device chains from another project | existing stack | #82, #83 | [#487](https://github.com/michaelmonetized/omatainer/pull/487) |
| [x] | [#124](https://github.com/michaelmonetized/omatainer/issues/124) | Manage named project versions and compare creative revisions | existing stack | #82 | [#489](https://github.com/michaelmonetized/omatainer/pull/489) |
| [x] | [#125](https://github.com/michaelmonetized/omatainer/issues/125) | Localize the interface and preserve international file and metadata handling | existing stack | #82 | [#492](https://github.com/michaelmonetized/omatainer/pull/492) |
| [x] | [#126](https://github.com/michaelmonetized/omatainer/issues/126) | Customize shortcuts and find commands through a command palette | existing stack | #88 | [#494](https://github.com/michaelmonetized/omatainer/pull/494) |
| [x] | [#127](https://github.com/michaelmonetized/omatainer/issues/127) | Support touch, pen and simultaneous performance gestures | existing stack | #87 | [#496](https://github.com/michaelmonetized/omatainer/pull/496) |
| [x] | [#128](https://github.com/michaelmonetized/omatainer/issues/128) | Save configurable workspaces and multiple-window layouts | existing stack | #88 | [#499](https://github.com/michaelmonetized/omatainer/pull/499) |
| [ ] | [#129](https://github.com/michaelmonetized/omatainer/issues/129) | Recover safely from audio-device loss and backend restarts | partial | #94, #82 | [receipt](../../docs/validation/issue-129-audio-recovery.md) |
| [ ] | [#130](https://github.com/michaelmonetized/omatainer/issues/130) | Route independent multichannel inputs and outputs through an audio graph | partial | #94 | [receipt](../../docs/validation/issue-130-audio-routing.md), [receipt](../../docs/validation/issue-146-prepare-queue.md) |
| [x] | [#131](https://github.com/michaelmonetized/omatainer/issues/131) | Protect audible DJ decks from accidental load and eject | implemented | #96 | [receipt](../../docs/validation/issue-131-deck-load-lock.md) |
| [x] | [#132](https://github.com/michaelmonetized/omatainer/issues/132) | Prioritize and cancel expensive background audio jobs | implemented | #96 | [receipt](../../docs/validation/issue-132-background-work.md), [receipt](../../docs/validation/remaining-backlog-qualification.md) |
| [x] | [#133](https://github.com/michaelmonetized/omatainer/issues/133) | Integrate low-latency PipeWire, JACK and ALSA workflows | implemented | #94 | [receipt](../../docs/validation/issue-133-linux-audio.md), [receipt](../../docs/validation/remaining-backlog-qualification.md) |
| [x] | [#134](https://github.com/michaelmonetized/omatainer/issues/134) | Support manually mapped tempo changes within a track | implemented | #99 | [receipt](../../docs/validation/issue-134-variable-tempo-grid.md) |
| [ ] | [#135](https://github.com/michaelmonetized/omatainer/issues/135) | Show aligned beatgrid, phase and phrase position on deck waveforms | partial | #99 | [receipt](../../docs/validation/issue-135-waveform-timing.md), [receipt](../../docs/validation/rainbow-waveforms.md) |
| [ ] | [#136](https://github.com/michaelmonetized/omatainer/issues/136) | Switch live sets without interrupting the outgoing mix | partial | #82, #96 | [receipt](../../docs/validation/issue-136-live-set.md) |
| [ ] | [#137](https://github.com/michaelmonetized/omatainer/issues/137) | Maintain a versioned professional capability and compatibility matrix | partial | #95 | [receipt](../../docs/capability-matrix.md) |
| [x] | [#138](https://github.com/michaelmonetized/omatainer/issues/138) | Deliver verified releases with safe updates and rollback | implemented | #82, #96 | [receipt](../../docs/validation/safe-updates.md) |
| [x] | [#139](https://github.com/michaelmonetized/omatainer/issues/139) | Add field-aware search and filtering across the music library | implemented | #85, #109 | [receipt](../../docs/validation/issue-139-library-search.md) |
| [x] | [#140](https://github.com/michaelmonetized/omatainer/issues/140) | Protect prepared beatgrids and metadata from bulk reanalysis | implemented | #104, #99 | [receipt](../../docs/validation/issue-140-preparation-locks.md) |
| [x] | [#141](https://github.com/michaelmonetized/omatainer/issues/141) | Analyze and recall safe per-track gain adjustment | implemented | #104 | [receipt](../../docs/validation/issue-141-track-gain.md) |
| [x] | [#142](https://github.com/michaelmonetized/omatainer/issues/142) | Analyze musical key and display harmonic compatibility | implemented | #104 | [receipt](../../docs/validation/issue-142-musical-key.md) |
| [x] | [#143](https://github.com/michaelmonetized/omatainer/issues/143) | Back up and move the complete DJ library with preparation data | implemented | #85, #108 | [receipt](../../docs/validation/issue-143-library-backup.md) |
| [x] | [#144](https://github.com/michaelmonetized/omatainer/issues/144) | Make library columns, sorting and view density configurable | implemented | #109 | [receipt](../../docs/validation/issue-144-library-layouts.md) |
| [x] | [#145](https://github.com/michaelmonetized/omatainer/issues/145) | Expose missing, corrupt, unsupported and read-only track status | implemented | #104 | [receipt](../../docs/validation/issue-145-library-health.md) |
| [x] | [#146](https://github.com/michaelmonetized/omatainer/issues/146) | Add a reorderable prepare queue for upcoming tracks | implemented | #105 | [receipt](../../docs/validation/issue-146-prepare-queue.md) |
| [ ] | [#147](https://github.com/michaelmonetized/omatainer/issues/147) | Transmit stable MIDI clock and transport to external instruments | planned | #112, #81 |  |
| [x] | [#148](https://github.com/michaelmonetized/omatainer/issues/148) | Deliver a usable MIDI learn editor for performance controls | implemented | #112, #39 | [receipt](../../docs/validation/issue-148-midi-learn.md) |
| [x] | [#149](https://github.com/michaelmonetized/omatainer/issues/149) | Edit MIDI CC, pitch-bend, program and channel-pressure data in clips | implemented | #110, #112, #111 | [receipt](../../docs/validation/midi-controller-lanes.md) |
| [ ] | [#150](https://github.com/michaelmonetized/omatainer/issues/150) | Record, edit and render per-note pitch, pressure and timbre expression | planned | #110, #112 |  |
| [x] | [#151](https://github.com/michaelmonetized/omatainer/issues/151) | Add cursor-based MIDI step recording and computer-keyboard note input | implemented | #110 | [receipt](../../docs/validation/midi-step-rhythm.md), [#510](https://github.com/michaelmonetized/omatainer/pull/510) |
| [ ] | [#152](https://github.com/michaelmonetized/omatainer/issues/152) | Add editable Session clip management and reusable clip presets | planned | #113, #82, #83 |  |
| [x] | [#153](https://github.com/michaelmonetized/omatainer/issues/153) | Add trigger, hold and toggle sample playback modes | implemented | #101 | [receipt](../../docs/validation/issue-153-sampler-playback.md) |
| [x] | [#154](https://github.com/michaelmonetized/omatainer/issues/154) | Add crate favorites, search and membership discovery | implemented | #105 | [receipt](../../docs/validation/issue-154-crate-discovery.md) |
| [ ] | [#155](https://github.com/michaelmonetized/omatainer/issues/155) | Manage music files and duplicates safely from the library | planned | #108 |  |
| [ ] | [#156](https://github.com/michaelmonetized/omatainer/issues/156) | Import standard playlists and existing local music-library exports | planned | #105 |  |
| [x] | [#157](https://github.com/michaelmonetized/omatainer/issues/157) | Export and optionally publish performed setlists | implemented | #106 | [receipt](../../docs/validation/setlist-exports.md) |
| [ ] | [#158](https://github.com/michaelmonetized/omatainer/issues/158) | Integrate an authorized Beatport streaming workflow | outside release | #120 |  |
| [ ] | [#159](https://github.com/michaelmonetized/omatainer/issues/159) | Integrate authorized SoundCloud catalog and playlists | outside release | #120 |  |
| [ ] | [#160](https://github.com/michaelmonetized/omatainer/issues/160) | Integrate an authorized TIDAL DJ account and catalog | outside release | #120 |  |
| [ ] | [#161](https://github.com/michaelmonetized/omatainer/issues/161) | Implement Link-compatible tempo, phase and start/stop synchronization | planned | #114 |  |
| [ ] | [#162](https://github.com/michaelmonetized/omatainer/issues/162) | Recover recent MIDI performances with retrospective capture | planned | #110, #112 |  |
| [ ] | [#163](https://github.com/michaelmonetized/omatainer/issues/163) | Recombine note properties and shape velocity using editable transformation tools | planned | #110 |  |
| [ ] | [#164](https://github.com/michaelmonetized/omatainer/issues/164) | Add quantize, stretch, reverse and tempo-curve MIDI transformations | planned | #110, #83 |  |
| [ ] | [#165](https://github.com/michaelmonetized/omatainer/issues/165) | Edit multiple MIDI clips together with explicit focus and ghost notes | planned | #110 |  |
| [ ] | [#166](https://github.com/michaelmonetized/omatainer/issues/166) | Add per-note chance, velocity ranges and expressive note properties | planned | #110 |  |
| [x] | [#167](https://github.com/michaelmonetized/omatainer/issues/167) | Generate editable rhythmic and Euclidean MIDI patterns | implemented | #110 | [receipt](../../docs/validation/midi-step-rhythm.md), [#510](https://github.com/michaelmonetized/omatainer/pull/510) |
| [ ] | [#168](https://github.com/michaelmonetized/omatainer/issues/168) | Make keys and scales shared musical context for editing and devices | planned | #110 |  |
| [ ] | [#169](https://github.com/michaelmonetized/omatainer/issues/169) | Exchange editable sessions using an open DAW interchange format | planned | #116 |  |
| [ ] | [#170](https://github.com/michaelmonetized/omatainer/issues/170) | Inspect project storage and safely clean unused media | planned | #124 |  |
| [ ] | [#171](https://github.com/michaelmonetized/omatainer/issues/171) | Integrate Apple Music only through an authorized DJ playback path | outside release | #120 |  |
| [ ] | [#172](https://github.com/michaelmonetized/omatainer/issues/172) | Integrate Spotify only if an authorized DJ service agreement is available | outside release | #120 |  |
| [ ] | [#173](https://github.com/michaelmonetized/omatainer/issues/173) | Support negotiated MIDI 2.0 and high-resolution expression | planned | #112 |  |
| [ ] | [#174](https://github.com/michaelmonetized/omatainer/issues/174) | Offer notation and MusicXML exchange for composer collaboration | planned | #111 |  |
| [ ] | [#175](https://github.com/michaelmonetized/omatainer/issues/175) | Offer optional cross-device project transfer with explicit conflict handling | planned | #116, #124 |  |
| [ ] | [#176](https://github.com/michaelmonetized/omatainer/issues/176) | Provide isolated cue/master headphone mixing and split cue | planned | #130 |  |
| [ ] | [#177](https://github.com/michaelmonetized/omatainer/issues/177) | Monitor live inputs with explicit In, Auto and Off modes | planned | #130 |  |
| [ ] | [#178](https://github.com/michaelmonetized/omatainer/issues/178) | Compensate device and routing latency across the complete graph | planned | #130 |  |
| [ ] | [#179](https://github.com/michaelmonetized/omatainer/issues/179) | Export offline and real-time master audio with professional format controls | planned | #82, #130 |  |
| [ ] | [#180](https://github.com/michaelmonetized/omatainer/issues/180) | Add live microphone and auxiliary DJ input channels | planned | #130 |  |
| [ ] | [#181](https://github.com/michaelmonetized/omatainer/issues/181) | Record live DJ performances to reliable audio files | planned | #130 |  |
| [ ] | [#182](https://github.com/michaelmonetized/omatainer/issues/182) | Make audio a first-class Session and Arrangement clip type | planned | #82, #130 |  |
| [ ] | [#183](https://github.com/michaelmonetized/omatainer/issues/183) | Host native VST3 instruments and audio effects | planned | #130 |  |
| [ ] | [#184](https://github.com/michaelmonetized/omatainer/issues/184) | Stream long recordings and deck media with bounded caches | planned | #132 |  |
| [ ] | [#185](https://github.com/michaelmonetized/omatainer/issues/185) | Add beat-jump transport and controller pad controls | planned | #134 |  |
| [ ] | [#186](https://github.com/michaelmonetized/omatainer/issues/186) | Implement hold-to-audition and stutter behavior for temporary cues | planned | #148 |  |
| [ ] | [#187](https://github.com/michaelmonetized/omatainer/issues/187) | Add assignable deck DJ-FX units with tempo controls | planned | #130, #55, #62 |  |
| [ ] | [#188](https://github.com/michaelmonetized/omatainer/issues/188) | Apply beatgrid quantization to deck cues and loop operations | planned | #134 |  |
| [ ] | [#189](https://github.com/michaelmonetized/omatainer/issues/189) | Support four independently controlled DJ decks | planned | #130 |  |
| [ ] | [#190](https://github.com/michaelmonetized/omatainer/issues/190) | Add precise loop boundary editing and beat-based loop movement | planned | #134 |  |
| [ ] | [#191](https://github.com/michaelmonetized/omatainer/issues/191) | Add a dedicated track preparation and audition workflow | planned | #85, #130 |  |
| [ ] | [#192](https://github.com/michaelmonetized/omatainer/issues/192) | Add deck performance-pad modes with per-deck selection | planned | #148, #98 |  |
| [ ] | [#193](https://github.com/michaelmonetized/omatainer/issues/193) | Separate tempo matching from beat and bar synchronization | planned | #134 |  |
| [ ] | [#194](https://github.com/michaelmonetized/omatainer/issues/194) | Qualify sustained browsing and preparation on professional-size libraries | planned | #85, #139, #104, #81, #36, #44 |  |
| [ ] | [#195](https://github.com/michaelmonetized/omatainer/issues/195) | Persist, import and export MIDI mapping presets | planned | #148 |  |
| [ ] | [#196](https://github.com/michaelmonetized/omatainer/issues/196) | Implement per-clip launch modes, launch quantization and legato switching | planned | #152 |  |
| [ ] | [#197](https://github.com/michaelmonetized/omatainer/issues/197) | Schedule independent tracks across cores with bounded real-time execution | planned | #130, #81 |  |
| [ ] | [#198](https://github.com/michaelmonetized/omatainer/issues/198) | Host CLAP instruments and effects with expression support | planned | #130 |  |
| [ ] | [#199](https://github.com/michaelmonetized/omatainer/issues/199) | Host LV2 instruments, effects and native Linux plugin UIs | planned | #130 |  |
| [ ] | [#200](https://github.com/michaelmonetized/omatainer/issues/200) | Add deliberate continuous playback from an ordered crate | planned | #105, #131 |  |
| [ ] | [#201](https://github.com/michaelmonetized/omatainer/issues/201) | Add optional digital-vinyl control using a supported control-signal format | planned | #130 |  |
| [ ] | [#202](https://github.com/michaelmonetized/omatainer/issues/202) | Add independent deck key shift and harmonic key sync | planned | #100, #142 |  |
| [ ] | [#203](https://github.com/michaelmonetized/omatainer/issues/203) | Add slip playback with configurable release timing | planned | #134 |  |
| [ ] | [#204](https://github.com/michaelmonetized/omatainer/issues/204) | Detect changing-tempo beatgrids automatically as a preview-comparison feature | planned | #134, #104 |  |
| [x] | [#205](https://github.com/michaelmonetized/omatainer/issues/205) | Add automatically maintained smart crates | implemented | #105, #139 | [receipt](../../docs/validation/issue-205-smart-crates.md) |
| [ ] | [#206](https://github.com/michaelmonetized/omatainer/issues/206) | Manage provider-authorized offline lockers and readiness | outside release | #158 |  |
| [ ] | [#207](https://github.com/michaelmonetized/omatainer/issues/207) | Build chord progression and voicing generation tools | planned | #110, #168 |  |
| [ ] | [#208](https://github.com/michaelmonetized/omatainer/issues/208) | Create shape-guided and seeded melodic MIDI generators | planned | #110, #168 |  |
| [ ] | [#209](https://github.com/michaelmonetized/omatainer/issues/209) | Generate arpeggios, strums, ornaments and articulated note repetitions | planned | #110, #168 |  |
| [ ] | [#210](https://github.com/michaelmonetized/omatainer/issues/210) | Support microtonal tuning systems and per-track tuning bypass | planned | #168, #150 |  |
| [ ] | [#211](https://github.com/michaelmonetized/omatainer/issues/211) | Support professional cross-platform collaboration and document native limits | planned | #116, #169 |  |
| [ ] | [#212](https://github.com/michaelmonetized/omatainer/issues/212) | Add a separately controlled booth monitor bus | planned | #130, #180 |  |
| [ ] | [#213](https://github.com/michaelmonetized/omatainer/issues/213) | Add deck-isolated output mode for external mixers | planned | #130, #189 |  |
| [ ] | [#214](https://github.com/michaelmonetized/omatainer/issues/214) | Implement transient detection and editable audio warp markers | planned | #182, #114 |  |
| [ ] | [#215](https://github.com/michaelmonetized/omatainer/issues/215) | Manage plugin presets, A/B states and reusable device chains | planned | #183, #82 |  |
| [ ] | [#216](https://github.com/michaelmonetized/omatainer/issues/216) | Scan, catalog and quarantine plugins outside the application process | planned | #183 |  |
| [ ] | [#217](https://github.com/michaelmonetized/omatainer/issues/217) | Add configurable crossfader assignments, curves and scratch cut-in | planned | #189 |  |
| [ ] | [#218](https://github.com/michaelmonetized/omatainer/issues/218) | Add selectable DJ waveform and library layouts | planned | #189 |  |
| [ ] | [#219](https://github.com/michaelmonetized/omatainer/issues/219) | Add safe pitch-fader pickup and temporary pitch bend | planned | #193 |  |
| [ ] | [#220](https://github.com/michaelmonetized/omatainer/issues/220) | Add named persistent loop banks for each track | planned | #85, #188 |  |
| [ ] | [#221](https://github.com/michaelmonetized/omatainer/issues/221) | Enforce streaming capabilities and protect playback during service failures | partial | #120, #181 | [receipt](../../docs/validation/issue-120-providers.md) |
| [ ] | [#222](https://github.com/michaelmonetized/omatainer/issues/222) | Follow external MIDI clock with transport and loss handling | planned | #112, #193, #41 |  |
| [ ] | [#223](https://github.com/michaelmonetized/omatainer/issues/223) | Support configurable MIDI encoder encodings and high-resolution controls | planned | #195, #42 |  |
| [ ] | [#224](https://github.com/michaelmonetized/omatainer/issues/224) | Drive controller LEDs and meters from actual application state | planned | #195, #112 |  |
| [ ] | [#225](https://github.com/michaelmonetized/omatainer/issues/225) | Add runtime controller discovery, enablement and reconnection | planned | #112, #195, #73 |  |
| [ ] | [#226](https://github.com/michaelmonetized/omatainer/issues/226) | Build an editable linear audio and MIDI Arrangement timeline | planned | #113, #82, #83, #179 |  |
| [ ] | [#227](https://github.com/michaelmonetized/omatainer/issues/227) | Give scenes names, tempo, meter and launch-state semantics | planned | #113, #196, #114 |  |
| [ ] | [#228](https://github.com/michaelmonetized/omatainer/issues/228) | Record and overdub audio or MIDI directly into Session slots | planned | #182, #196, #130, #112 |  |
| [ ] | [#229](https://github.com/michaelmonetized/omatainer/issues/229) | Export aligned stems, track groups and delivery versions in batches | planned | #179 |  |
| [ ] | [#230](https://github.com/michaelmonetized/omatainer/issues/230) | Expose a clean performance audio feed for OBS and broadcast tools | planned | #130, #176 |  |
| [ ] | [#231](https://github.com/michaelmonetized/omatainer/issues/231) | Validate loudness, true peaks and metadata before delivery | planned | #179 |  |
| [ ] | [#232](https://github.com/michaelmonetized/omatainer/issues/232) | Add selectable one-knob channel effects | planned | #187 |  |
| [ ] | [#233](https://github.com/michaelmonetized/omatainer/issues/233) | Add DVS signal calibration and diagnostics | planned | #201 |  |
| [ ] | [#234](https://github.com/michaelmonetized/omatainer/issues/234) | Save and share DJ effect presets and unit layouts | planned | #187 |  |
| [ ] | [#235](https://github.com/michaelmonetized/omatainer/issues/235) | Add instant doubles and deck transfer | planned | #189, #131 |  |
| [ ] | [#236](https://github.com/michaelmonetized/omatainer/issues/236) | Add momentary loop-roll pad performance | planned | #192, #203 |  |
| [ ] | [#237](https://github.com/michaelmonetized/omatainer/issues/237) | Add reverse playback and momentary censor controls | planned | #203 |  |
| [ ] | [#238](https://github.com/michaelmonetized/omatainer/issues/238) | Add beat-synced sampler playback and output assignments | planned | #101, #193, #130 |  |
| [ ] | [#239](https://github.com/michaelmonetized/omatainer/issues/239) | Add scratch banks with instant return to the previous deck track | planned | #192, #85, #98 |  |
| [ ] | [#240](https://github.com/michaelmonetized/omatainer/issues/240) | Add moving and fixed-loop slicer pad modes | planned | #192, #134 |  |
| [ ] | [#241](https://github.com/michaelmonetized/omatainer/issues/241) | Link set recordings to performed-track and cue markers | planned | #181, #106 |  |
| [ ] | [#242](https://github.com/michaelmonetized/omatainer/issues/242) | Convert recorded melodies, harmony and drums to editable MIDI | planned | #182, #110 |  |
| [ ] | [#243](https://github.com/michaelmonetized/omatainer/issues/243) | Add probabilistic clip and scene follow actions | planned | #196 |  |
| [ ] | [#244](https://github.com/michaelmonetized/omatainer/issues/244) | Support lawful legacy and platform-specific plug-in interoperability | planned | #183 |  |
| [ ] | [#245](https://github.com/michaelmonetized/omatainer/issues/245) | Support multichannel and immersive production as an advanced workflow | planned | #130, #179 |  |
| [ ] | [#246](https://github.com/michaelmonetized/omatainer/issues/246) | Expose performance timing and mixer state for lighting systems | planned | #193, #161 |  |
| [ ] | [#247](https://github.com/michaelmonetized/omatainer/issues/247) | Add chromatic cue-point performance pads | planned | #202, #192 |  |
| [ ] | [#248](https://github.com/michaelmonetized/omatainer/issues/248) | Support post-production session exchange and broadcast media metadata | planned | #179, #116 |  |
| [ ] | [#249](https://github.com/michaelmonetized/omatainer/issues/249) | Provide audio clip fades and editable arrangement crossfades | planned | #226, #182 |  |
| [ ] | [#250](https://github.com/michaelmonetized/omatainer/issues/250) | Offer distinct high-quality warp modes for drums, tones, textures and mixes | planned | #214, #178, #179 |  |
| [ ] | [#251](https://github.com/michaelmonetized/omatainer/issues/251) | Isolate plugin failures and restore processing without losing sessions | planned | #216, #97 |  |
| [ ] | [#252](https://github.com/michaelmonetized/omatainer/issues/252) | Publish hardware profiles backed by end-to-end compatibility evidence | planned | #224, #223, #225, #38, #40, #72 |  |
| [ ] | [#253](https://github.com/michaelmonetized/omatainer/issues/253) | Add editable multitrack automation lanes and breakpoint curves | planned | #226, #83 |  |
| [ ] | [#254](https://github.com/michaelmonetized/omatainer/issues/254) | Make device chains editable, inspectable and reusable with A/B states | planned | #82, #83, #130, #215 |  |
| [ ] | [#255](https://github.com/michaelmonetized/omatainer/issues/255) | Add named arrangement locators, musical scrubbing and loop-region navigation | planned | #226, #114 |  |
| [ ] | [#256](https://github.com/michaelmonetized/omatainer/issues/256) | Add count-in, punch recording and multi-pass recording workflows | planned | #130, #226, #82 |  |
| [ ] | [#257](https://github.com/michaelmonetized/omatainer/issues/257) | Send and receive Link Audio streams with track-level routing and latency controls | planned | #161, #130, #228 |  |
| [ ] | [#258](https://github.com/michaelmonetized/omatainer/issues/258) | Extract editable production stems from complete clips or selected time ranges | planned | #182, #226 |  |
| [ ] | [#259](https://github.com/michaelmonetized/omatainer/issues/259) | Add momentary and latched performance-pad FX | planned | #192, #234 |  |
| [ ] | [#260](https://github.com/michaelmonetized/omatainer/issues/260) | Add controller layers, modifiers and takeover policies | planned | #195, #219 |  |
| [ ] | [#261](https://github.com/michaelmonetized/omatainer/issues/261) | Implement a groove pool with audio/MIDI extraction and commitment | planned | #110, #214 |  |
| [ ] | [#262](https://github.com/michaelmonetized/omatainer/issues/262) | Create reviewable collaboration packages with mix notes and references | planned | #116, #229 |  |
| [ ] | [#263](https://github.com/michaelmonetized/omatainer/issues/263) | Trigger a saved loop from its associated cue marker | planned | #220, #98, #188 |  |
| [ ] | [#264](https://github.com/michaelmonetized/omatainer/issues/264) | Record and replay nondestructive cue and censor routines | planned | #98, #237, #188, #85 |  |
| [ ] | [#265](https://github.com/michaelmonetized/omatainer/issues/265) | Add optional synchronized DJ video playback and external output | planned | #81, #217 |  |
| [ ] | [#266](https://github.com/michaelmonetized/omatainer/issues/266) | Mix approved streaming providers concurrently as a preview-comparison feature | outside release | #120, #221 |  |
| [ ] | [#267](https://github.com/michaelmonetized/omatainer/issues/267) | Populate prearranged clip placeholders from a live recording | planned | #226, #228, #82 |  |
| [ ] | [#268](https://github.com/michaelmonetized/omatainer/issues/268) | Support safe DJ handover on qualified dual-computer hardware | planned | #130, #129, #252 |  |
| [ ] | [#269](https://github.com/michaelmonetized/omatainer/issues/269) | Implement arrangement split, consolidate and structural time editing | planned | #226, #253, #83 |  |
| [ ] | [#270](https://github.com/michaelmonetized/omatainer/issues/270) | Record performance automation with explicit override and re-enable behavior | planned | #253 |  |
| [ ] | [#271](https://github.com/michaelmonetized/omatainer/issues/271) | Implement take lanes and non-destructive audio/MIDI comping | planned | #226, #256, #249, #83 |  |
| [ ] | [#272](https://github.com/michaelmonetized/omatainer/issues/272) | Build nested instrument and effect racks with key, velocity and chain zones | planned | #130, #254, #82 |  |
| [ ] | [#273](https://github.com/michaelmonetized/omatainer/issues/273) | Deliver a musician-facing mixer with pan laws, stereo controls and solo semantics | planned | #130, #113, #253 |  |
| [ ] | [#274](https://github.com/michaelmonetized/omatainer/issues/274) | Provide a fully editable stereo algorithmic reverb | planned | #254 |  |
| [ ] | [#275](https://github.com/michaelmonetized/omatainer/issues/275) | Implement a playable record/overdub audio looper device | planned | #254, #130, #182 |  |
| [ ] | [#276](https://github.com/michaelmonetized/omatainer/issues/276) | Provide tempo-synced auto-pan, tremolo and rhythmic chopping | planned | #254 |  |
| [ ] | [#277](https://github.com/michaelmonetized/omatainer/issues/277) | Add tempo-synced beat repeat and controlled stutter processing | planned | #254 |  |
| [ ] | [#278](https://github.com/michaelmonetized/omatainer/issues/278) | Add an analog/digital character echo with modulation and noise shaping | planned | #254 |  |
| [ ] | [#279](https://github.com/michaelmonetized/omatainer/issues/279) | Expand chorus into chorus, ensemble and vibrato processing | planned | #254 |  |
| [ ] | [#280](https://github.com/michaelmonetized/omatainer/issues/280) | Implement convolution and hybrid reverb with licensed impulse responses | planned | #254, #178, #116 |  |
| [ ] | [#281](https://github.com/michaelmonetized/omatainer/issues/281) | Add an integrated drum-bus processor with transient shaping and tuned low-end enhancement | planned | #254, #178 |  |
| [ ] | [#282](https://github.com/michaelmonetized/omatainer/issues/282) | Implement parallel filtered delay lines with independent pan and feedback | planned | #254 |  |
| [ ] | [#283](https://github.com/michaelmonetized/omatainer/issues/283) | Add granular pitch-shifted delay for sound design | planned | #254 |  |
| [ ] | [#284](https://github.com/michaelmonetized/omatainer/issues/284) | Provide a metered mastering limiter with lookahead and stereo modes | planned | #254, #178, #179 |  |
| [ ] | [#285](https://github.com/michaelmonetized/omatainer/issues/285) | Provide bit reduction, erosion and vinyl-style texture processing | planned | #254 |  |
| [ ] | [#286](https://github.com/michaelmonetized/omatainer/issues/286) | Implement multiband compression, expansion and envelope shaping | planned | #254 |  |
| [ ] | [#287](https://github.com/michaelmonetized/omatainer/issues/287) | Build a routable multistage saturation device with modulation | planned | #254 |  |
| [ ] | [#288](https://github.com/michaelmonetized/omatainer/issues/288) | Add a graphical parametric equalizer with stereo and mid/side editing | planned | #254 |  |
| [ ] | [#289](https://github.com/michaelmonetized/omatainer/issues/289) | Add phaser and flanger effects with controllable feedback and modulation | planned | #254 |  |
| [ ] | [#290](https://github.com/michaelmonetized/omatainer/issues/290) | Add real-time monophonic pitch correction and MIDI-driven harmony | planned | #254, #168, #112, #178 |  |
| [ ] | [#291](https://github.com/michaelmonetized/omatainer/issues/291) | Add pitch shifting, frequency shifting and ring modulation | planned | #254, #178 |  |
| [ ] | [#292](https://github.com/michaelmonetized/omatainer/issues/292) | Provide tuned resonator banks and physically modeled resonant-body effects | planned | #254, #168, #112 |  |
| [ ] | [#293](https://github.com/michaelmonetized/omatainer/issues/293) | Provide waveshaping, tube and pedal-style saturation tools | planned | #254, #178 |  |
| [ ] | [#294](https://github.com/michaelmonetized/omatainer/issues/294) | Add spectral resonation, spectral delay/freeze and spectral blur devices | planned | #254, #178, #112 |  |
| [ ] | [#295](https://github.com/michaelmonetized/omatainer/issues/295) | Build a configurable stereo delay with tempo divisions and advanced LFO shapes | planned | #254 |  |
| [ ] | [#296](https://github.com/michaelmonetized/omatainer/issues/296) | Follow the tempo of live incoming audio with controlled fallback | planned | #130, #114, #250 |  |
| [ ] | [#297](https://github.com/michaelmonetized/omatainer/issues/297) | Provide gain/stereo/alignment utilities plus spectrum and tuning analysis | planned | #254, #210 |  |
| [ ] | [#298](https://github.com/michaelmonetized/omatainer/issues/298) | Enable live deck stem separation without interrupting playback | planned | #258, #81 |  |
| [ ] | [#299](https://github.com/michaelmonetized/omatainer/issues/299) | Prepare and manage cached stems before a performance | planned | #258, #85 |  |
| [ ] | [#300](https://github.com/michaelmonetized/omatainer/issues/300) | Support explicitly qualified HID DJ media players | planned | #252, #85 |  |
| [ ] | [#301](https://github.com/michaelmonetized/omatainer/issues/301) | Implement verified motorized platter control and calibration | planned | #252, #224 |  |
| [ ] | [#302](https://github.com/michaelmonetized/omatainer/issues/302) | Provide CC controls, MIDI monitoring and expression-shaping devices | planned | #112, #150, #253 |  |
| [ ] | [#303](https://github.com/michaelmonetized/omatainer/issues/303) | Build a programmable virtual-analog synth with mono, poly and expressive modes | planned | #254, #150 |  |
| [ ] | [#304](https://github.com/michaelmonetized/omatainer/issues/304) | Grab and release arrangement loops during live performance | planned | #226, #255, #196 |  |
| [ ] | [#305](https://github.com/michaelmonetized/omatainer/issues/305) | Provide linked and independent clip modulation envelopes | planned | #253, #152 |  |
| [ ] | [#306](https://github.com/michaelmonetized/omatainer/issues/306) | Create a per-hit drum sampler with transient shaping and creative playback | planned | #254, #182 |  |
| [ ] | [#307](https://github.com/michaelmonetized/omatainer/issues/307) | Provide a modeled electric-piano instrument | planned | #254, #112 |  |
| [ ] | [#308](https://github.com/michaelmonetized/omatainer/issues/308) | Host user-authored audio, MIDI and instrument devices through an open extension runtime | planned | #254, #116 |  |
| [ ] | [#309](https://github.com/michaelmonetized/omatainer/issues/309) | Add an FM and additive synthesis instrument | planned | #254, #150 |  |
| [ ] | [#310](https://github.com/michaelmonetized/omatainer/issues/310) | Freeze and unfreeze tracks without sacrificing editability | planned | #179, #182, #254, #82 |  |
| [ ] | [#311](https://github.com/michaelmonetized/omatainer/issues/311) | Provide a sample-based granular synthesis instrument | planned | #254, #150, #182, #130 |  |
| [ ] | [#312](https://github.com/michaelmonetized/omatainer/issues/312) | Create an expressive dual-layer macro-oscillator instrument | planned | #254, #150, #210 |  |
| [ ] | [#313](https://github.com/michaelmonetized/omatainer/issues/313) | Add physical-model mallet and resonant-percussion instruments | planned | #254, #210 |  |
| [ ] | [#314](https://github.com/michaelmonetized/omatainer/issues/314) | Build an editable chromatic single-sample instrument | planned | #254, #182, #250 |  |
| [ ] | [#315](https://github.com/michaelmonetized/omatainer/issues/315) | Add a physical-model string synthesis instrument | planned | #254, #150, #210 |  |
| [ ] | [#316](https://github.com/michaelmonetized/omatainer/issues/316) | Add a morphing wavetable instrument with user-table import | planned | #254, #150, #116 |  |
| [ ] | [#317](https://github.com/michaelmonetized/omatainer/issues/317) | Browse, tag and audition instruments, presets, clips and production samples | planned | #116, #254, #152 |  |
| [ ] | [#318](https://github.com/michaelmonetized/omatainer/issues/318) | Support linked multitrack editing for phase-coherent recordings | planned | #226, #271, #214 |  |
| [ ] | [#319](https://github.com/michaelmonetized/omatainer/issues/319) | Provide a user-editable drum rack with per-pad chains and choke groups | planned | #272, #306, #112 |  |
| [ ] | [#320](https://github.com/michaelmonetized/omatainer/issues/320) | Implement nested group tracks, subgroup mixing and track organization | planned | #113, #130, #273 |  |
| [ ] | [#321](https://github.com/michaelmonetized/omatainer/issues/321) | Add auxiliary sends, returns and musician-facing sidechain routing | planned | #130, #178, #273 |  |
| [ ] | [#322](https://github.com/michaelmonetized/omatainer/issues/322) | Record Session performances into Arrangement and restore arrangement control | planned | #226, #196, #270 |  |
| [ ] | [#323](https://github.com/michaelmonetized/omatainer/issues/323) | Add guitar/bass amplifier and cabinet simulation | planned | #254, #280, #130 |  |
| [ ] | [#324](https://github.com/michaelmonetized/omatainer/issues/324) | Apply performance FX and rolls to selected stems | planned | #298, #187 |  |
| [ ] | [#325](https://github.com/michaelmonetized/omatainer/issues/325) | Add per-stem mute, solo and level controls on decks | planned | #298, #224 |  |
| [ ] | [#326](https://github.com/michaelmonetized/omatainer/issues/326) | Publish library and deck information to supported controller screens | planned | #300, #85 |  |
| [ ] | [#327](https://github.com/michaelmonetized/omatainer/issues/327) | Provide composable pitch, scale, chord, random, velocity and note-length MIDI effects | planned | #272, #168, #112 |  |
| [ ] | [#328](https://github.com/michaelmonetized/omatainer/issues/328) | Add a multisample instrument with key/velocity zones and modulation | planned | #254, #314, #116 |  |
| [ ] | [#329](https://github.com/michaelmonetized/omatainer/issues/329) | Map rack macros and recall named macro variations | planned | #272, #253 |  |
| [ ] | [#330](https://github.com/michaelmonetized/omatainer/issues/330) | Add sequenced, pitch-reversing and dual-buffer creative delay devices | planned | #295, #291 |  |
| [ ] | [#331](https://github.com/michaelmonetized/omatainer/issues/331) | Provide visual/code device authoring, a musical object API and custom MIDI tools | planned | #308, #164, #208, #83 |  |
| [ ] | [#332](https://github.com/michaelmonetized/omatainer/issues/332) | Bounce clips, selected ranges and groups into editable audio tracks | planned | #179, #226, #320, #83 |  |
| [ ] | [#333](https://github.com/michaelmonetized/omatainer/issues/333) | Extend dynamics processing into configurable compressor and bus-compressor devices | planned | #254, #321 |  |
| [ ] | [#334](https://github.com/michaelmonetized/omatainer/issues/334) | Add a fully controllable gate and expander with sidechain audition | planned | #254, #321 |  |
| [ ] | [#335](https://github.com/michaelmonetized/omatainer/issues/335) | Add a modulation-capable filter device with envelope follower and sidechain | planned | #254, #321 |  |
| [ ] | [#336](https://github.com/michaelmonetized/omatainer/issues/336) | Add a playable vocoder with external carrier routing | planned | #254, #321, #112 |  |
| [ ] | [#337](https://github.com/michaelmonetized/omatainer/issues/337) | Split stems across linked deck mixer channels | planned | #189, #325, #235 |  |
| [ ] | [#338](https://github.com/michaelmonetized/omatainer/issues/338) | Provide evolving step, probability and polyrhythmic sequencer devices | planned | #327, #228 |  |
| [ ] | [#339](https://github.com/michaelmonetized/omatainer/issues/339) | Add performance arpeggiator and MIDI note-echo devices | planned | #327, #228 |  |
| [ ] | [#340](https://github.com/michaelmonetized/omatainer/issues/340) | Provide full production control for Push 1 and Push 2 | planned | #228, #319, #254, #329 |  |
| [ ] | [#341](https://github.com/michaelmonetized/omatainer/issues/341) | Ship an original tagged production library of kits, presets, loops and clips | planned | #317, #319, #303, #116 |  |
| [ ] | [#342](https://github.com/michaelmonetized/omatainer/issues/342) | Provide editable kick, snare, clap, hat, cymbal, tom and metallic drum synthesis | planned | #254, #319 |  |
| [ ] | [#343](https://github.com/michaelmonetized/omatainer/issues/343) | Add assignable LFO, envelope, shaper and performance-expression modulators | planned | #253, #329, #321 |  |
| [ ] | [#344](https://github.com/michaelmonetized/omatainer/issues/344) | Slice samples to playable pads and reconstruct their rhythm as MIDI | planned | #214, #319, #110 |  |
| [ ] | [#345](https://github.com/michaelmonetized/omatainer/issues/345) | Create customizable performance macro pages spanning the whole set | planned | #329, #270 |  |
| [ ] | [#346](https://github.com/michaelmonetized/omatainer/issues/346) | Add stem-specific slip performance gestures | planned | #203, #325 |  |
| [ ] | [#347](https://github.com/michaelmonetized/omatainer/issues/347) | Find and swap acoustically similar production samples | planned | #317, #319 |  |
| [ ] | [#348](https://github.com/michaelmonetized/omatainer/issues/348) | Insert external hardware effects with round-trip latency calibration | planned | #130, #178, #332 |  |
| [ ] | [#349](https://github.com/michaelmonetized/omatainer/issues/349) | Integrate external MIDI instruments as latency-compensated track devices | planned | #130, #112, #178, #332 |  |
| [ ] | [#350](https://github.com/michaelmonetized/omatainer/issues/350) | Support expressive Push 3 controller workflows without implying standalone licensing | planned | #340, #150 |  |
| [ ] | [#351](https://github.com/michaelmonetized/omatainer/issues/351) | Recall selective performance snapshots across tracks and devices | planned | #345, #196, #82 |  |
| [ ] | [#352](https://github.com/michaelmonetized/omatainer/issues/352) | Integrate pitch, gates, clocks and modulation with modular hardware | planned | #130, #343, #114 |  |
| [ ] | [#353](https://github.com/michaelmonetized/omatainer/issues/353) | Add optional licensed acoustic drums, pianos, electric keys, guitar and bass libraries | planned | #328, #341 |  |
| [ ] | [#354](https://github.com/michaelmonetized/omatainer/issues/354) | Add original vocal, foley, cinematic, electronic and generative production packs | planned | #341, #311, #343 |  |
| [ ] | [#355](https://github.com/michaelmonetized/omatainer/issues/355) | Add optional orchestral, chamber and global-percussion instrument collections | planned | #328, #341, #210 |  |
| [ ] | [#356](https://github.com/michaelmonetized/omatainer/issues/356) | Provide playable physics-inspired modulation and generative sound devices | planned | #338, #343, #311 |  |
| [ ] | [#357](https://github.com/michaelmonetized/omatainer/issues/357) | Integrate optional licensed sample-provider search and audition for production | outside release | #317, #347, #116 |  |
