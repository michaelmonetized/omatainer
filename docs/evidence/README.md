**Evidence for the Omatainer adversarial post-mortem**

Audited September 30, 2026, against clean product commit `cae546c7bc840ec1a643e4befe18f91e86d0e4c7`. The audit adds documentation and isolated reproduction tools; it makes no product fixes. Three subagents covered audio/DSP, GUI/theme/workflows, and MIDI/CLI/integration. The coordinating review checked cross-cutting contracts and assembled [the article](../adversarial-post-mortem.md).

The baseline `cargo test --locked` passed all 13 existing tests, with 19 compiler warnings. Its warm local test run finished in 0.30 seconds. That baseline is separate from the defect probes below. A passing defect probe confirms the audited bug remains present; it is not a regression-test pass for a repaired application.

Toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)`. Local CPU inventory reported 10 M1 Pro cores, with Icestorm and Firestorm clusters. Scheduling and load affect timing; the recorded numbers are observations, not portable thresholds.

Run these commands from the repository root after building the baseline dependencies:

```bash
cargo test --locked
python3 docs/evidence/run_audio_dsp.py "$PWD"
python3 docs/evidence/run_core.py "$PWD"
python3 docs/evidence/ui_probe.py "$PWD"
python3 docs/evidence/integration_probe.py "$PWD" --abort-check
```

Run performance probes sequentially. The standalone runners expect `target/debug/deps`; a custom Cargo target directory must be adjusted accordingly. They select cached dependency artifacts, so cleanly rebuild that cache if stale feature combinations cause linker errors. The integration runner uses an isolated source copy and reuses the repository's Cargo target cache. It appends probe code only to the copy. The other runners compile imported source or generated harnesses in temporary output directories. Those directories contain binaries and can be removed after inspection.

The optional abort check intentionally terminates only its isolated, engine-only harness. It disables core dumps and opens no audio device or application window. Socket tests use Unix socket pairs and a private temporary runtime directory. None of these commands runs the installer or edits desktop configuration.

| Probe | Method and limits | Recorded output |
| --- | --- | --- |
| [Audio/DSP](audio_dsp.rs) | Imports actual engine; `rustc opt-level=3`; allocator instrumentation surrounds rendering, excluding construction/output-buffer allocation. Counts are exact for the fixture; elapsed times include instrumentation. Tests frozen-deck output, channel-state reuse, velocity, FX controls, and deliberate snapshot contention. No CPAL stream. | [audio-results.txt](audio-results.txt) |
| [Core contracts](core_probe.rs) | Actual engine; optimized standalone harness. Quantized one-shot, selection-changing note-off, scene stop, and 512-frame publication-lock contention. The lock is intentionally held for 80 ms; this is not a measured live UI stall. | [core-results.txt](core-results.txt) |
| [GUI](ui_probe.py) | Original UI methods and engine core, with only the hardware-facing `Engine` wrapper replaced by an in-memory wrapper. Real egui input/focus; crate elapsed-time benchmark uses synthetic metadata, 1440×108 viewport, two warmups, ten timed frames, `opt-level=1`. No native window, GPU presentation, audio device, or full-app FPS or process CPU-time measurement. Outputs report observations rather than asserting every result. | [ui-results.txt](ui-results.txt) |
| [MIDI/IPC](integration_probe.py) | Eight tests appended to isolated source copy; synthetic MIDI and actual parser/map/engine handlers; temporary sockets. Separate `panic=abort` engine harness confirms SIGABRT. No physical controllers, actual shell, or application startup. | [integration-results.txt](integration-results.txt) |

The count of 192,000 demo note-vector allocations per second is `4096 / 1024 × 48000`, extrapolated from the measured block. Allocated bytes are traffic, not retained memory. The 14,400 follower connections/threads per hour is source-derived arithmetic at a nominal 250 ms polling interval; request time reduces that rate. Neither calculation measures an underrun or desktop latency.

Raw probe fields named `cpu_wall_ms` and `median_cpu_ms` use `Instant::elapsed`: they measure elapsed wall time around headless work, not consumed process CPU time. The application's own `reported_cpu_fraction` is a different metric with the omissions described in the article. Captured outputs preserve one run; subsequent runs can differ in timing while reproducing the same behavior and allocation counts.

Entry-point coverage was allocated as follows. “Reviewed” means the path was traced in source; it does not imply every control received an empirical or hardware test.

| Area | Reviewed entry points |
| --- | --- |
| Native lifecycle | `main`, engine/audio startup, single-instance detection, focus, IPC bind/thread lifecycle, normal shutdown |
| CLI | `status`, `follow`, `play`, `stop`, toggle aliases, `record`, `tap`, `scene`, deck A/B play, cue A/B, `reload-theme`, unknown arguments |
| Socket | Every opcode; malformed/unknown input; scene indexing; state replies; connection/read lifetime |
| MIDI | Initial input/output discovery; all seven factory-map builders; note fallback; shift; mapped dispatch; relative jog; pitch; learn; input/output clock; LEDs |
| GUI startup/update | Theme/fonts, scan, decoder jobs/completions, snapshot, redraw scheduling, help/MIDI windows |
| Keyboard and drop | Every implemented binding and advertised missing binding; focused search; dropped-file selection and deck routing |
| Deck controls | Play/cue/unload; platter touch/jog/release; waveform seek; pitch/range/keylock; hot cues/delete; EQ/gain/cut/solo; quantize; loop controls; match; crossfader |
| Sampler | Bank/instrument selection; octave; pad down/up; sample/instrument mapping; compose insertion |
| Crate | Scan/search/filter/sort/metadata; selection/double-click; load A/B; built-ins and external-file decode |
| Session/FX | Track mute/solo/gain; scene toggle/restart/add; clip one-shot/loop; compose selection; track/scene FX add/toggle/mix/parameters/back |
| Render/DSP | Callback conversion; command drain; sample-rate setup; track/deck/pad rendering; voice/envelope/filter/EQ/delay/reverb; pitch lock; snapshot; resampling/drum synthesis/peaks/BPM/decode |
| Shell/install | Both QML components, manifest, desktop launcher, contributed window rule, installer/config edits/theme hook; release manifest |
| Advertised project surface | Arrange/piano-roll/persistence/export implementation search; pad composition distinguished from absent piano-roll editing |

Additional findings were kept out of the article's main narrative to keep it readable. Unless marked reproduced, these are source-traced observations or risks.

| Finding | Evidence and qualification |
| --- | --- |
| Compose remains armed and can mutate another clip after transport changes | `compose_armed` is set by `Select` and never cleared. `LaunchClip` changes the selected target. GUI probe reproduces Stop → TogglePlay → pad adding a note to the existing Pad clip. [Selection](../../src/engine/mod.rs#L1286), [pad recording](../../src/engine/mod.rs#L1499). |
| Synth menu labels do not match synth kinds | UI labels drums/analog/keys/pad as 0/1/2/3, handler clamps to 0/1/2, DSP implements analog/keys/pad. Keys and pad therefore construct the same kind. [Menu](../../src/ui.rs#L503), [handler](../../src/engine/mod.rs#L1522), [voice](../../src/engine/dsp.rs#L217). |
| Sampler duplicates instrument note-on into global and selected-track synths | The global sampler is mixed independently of track gain/mute/FX; source establishes the routing, not a listening result. [Note-on](../../src/engine/mod.rs#L1495), [mix](../../src/engine/mod.rs#L702). |
| Recording does not capture held duration | Both recording paths append fixed 0.25-beat notes; pad composition uses beat modulo four. [Live notes](../../src/engine/mod.rs#L1310), [pads](../../src/engine/mod.rs#L1499). |
| 21 of 39 exposed FX parameter controls lack consumers | Counted by effect type, not 39 empirical sweeps. Reverb/chorus/arp consume none of the three exposed parameters; delay consumes only p1. Arp itself works upstream and is not a universal no-op. [Sliders](../../src/ui.rs#L816), [consumers](../../src/engine/fx.rs#L139), [arp](../../src/engine/mod.rs#L765). |
| Drum velocity discarded | Reproduced identical output for velocities 1 and 127; `let _ = vel` and voice storage explain it. [Trigger](../../src/engine/mod.rs#L847). |
| Empty FX path introduces one-sample channel offset | Reproduced impulse `(1,0)` then `(0,1)` at neutral width. Any nonempty scene FX list also folds the session sum to mono, including disabled slots. [Haas](../../src/engine/fx.rs#L189), [scene mix](../../src/engine/mod.rs#L691). |
| Legacy hardware FX selection/third wet control are inert | `FxSelect` updates `fx_kind`, never read by renderer; `fx_wet[2]` also lacks a consumer. [Commands](../../src/engine/mod.rs#L1344), [render setup](../../src/engine/mod.rs#L666). |
| FX slots have no removal/reordering workflow | UI and command enum only expose add/toggle/mix/parameters. Added slots continue to be traversed/cloned even when disabled. [UI](../../src/ui.rs#L799), [clone](../../src/engine/fx.rs#L134). |
| Per-sample avoidable work extends beyond allocation | Drum rendering clones six sample Arcs each call; voices recalculate pitch exponentials; crossfader gains are recomputed per frame. No separate deadline claim. [Drums](../../src/engine/mod.rs#L865), [pitch](../../src/engine/dsp.rs#L246), [crossfader](../../src/engine/mod.rs#L699). |
| Snapshot rebuild copies waveform vectors | Both 2,048-bucket peak arrays become new Arcs approximately eight times per second, in the render publication path. [Peaks](../../src/engine/mod.rs#L1644). |
| Sample-rate reconfiguration leaves some rate-dependent state unchanged | Track poly/EQ/drums reset, but sampler envelopes and track/scene FX retain construction-rate state. No device-rate-switch test. [Setup](../../src/engine/mod.rs#L533). |
| File loads queue without cancellation or request identity | One serial unbounded decode queue, completions applied unconditionally; late completion can reload a deck after unload. No timed slow-file reproduction. Decoder buffers whole tracks and ignores some decode errors. [Worker](../../src/ui.rs#L45), [completion](../../src/ui.rs#L200), [decoder](../../src/engine/dsp.rs#L637). |
| New deck audio retains previous loop state | `DeckAudio` resets position/cues but not loop bounds/on state, which rendering continues to enforce. [Load](../../src/engine/mod.rs#L1215), [loop](../../src/engine/mod.rs#L905). |
| Crate metadata and deck selection are incomplete | File duration stays zero in library; BPM/key derive from filenames; last-play changes before decode succeeds. Double-click defaults to A because no UI path dispatches `SelectDeck`. [Scan](../../src/ui.rs#L110), [load](../../src/ui.rs#L135), [double-click](../../src/ui.rs#L625). |
| Theme font discovery is not wired to font installation | External font command output is stored but installation loads fixed JetBrainsMono paths. Only colors mtime triggers reload. UTF-8 color parsing slices byte offsets and can panic on malformed text. [Reload](../../src/theme.rs#L68), [font](../../src/theme.rs#L118), [installation](../../src/theme.rs#L167), [parser](../../src/theme.rs#L243). |
| MIDI discovery has no hotplug refresh | Ports enumerated once; input labels are added before connection success. No hardware connection test. [Discovery](../../src/engine/midi.rs#L98). |
| MIDI/event queues have asymmetric overload behavior | MIDI sends block on the 256-item channel; GUI `try_send` failures are discarded; audio drains commands without a fixed per-block limit. Saturation was not load-tested. [Channel](../../src/engine/mod.rs#L1943), [GUI](../../src/engine/mod.rs#L1957), [drain](../../src/engine/mod.rs#L660). |
| Socket connections have no request-size cap or read timeout | Each thread can wait indefinitely or grow its input string. Local resource-exhaustion risk only; no attack executed and no remote exposure claimed. [Reader](../../src/main.rs#L166). |
| Shell scene helper discards its argument; acknowledgments precede execution | `scene(n)` sends only `scene`; `send()` returns ok immediately and has no control-result handler. Underlying IPC MIDI status stays empty while the GUI enriches only its returned clone. Source review only. [Service](../../plugin/Service.qml#L29), [snapshot](../../src/engine/mod.rs#L1966). |
| Installation lacks transactional rollback | Files/config are overwritten before final checks, with no backup/rollback; final Hyprland errors are suppressed. Bash and embedded Python syntax checked, installer not executed. [Installer](../../scripts/install-omarchy.sh#L18). |

The article deliberately excludes dead helpers as active defects, does not infer an absent arpeggiator from its pass-through FX branch, and does not treat a passing internal contract test as an end-to-end guarantee. No live audio audibility, hardware compatibility, desktop smoothness, or shell deployment success is claimed.
