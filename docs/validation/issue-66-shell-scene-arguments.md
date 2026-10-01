# Issue #66 — preserve the requested shell scene

`Service.scene(n)` now validates a one-based scene number and queues it as a
separate process argument. Each queue entry/result owns its captured argument
array. The public `omatainer.scene` IpcHandler accepts text and routes through
the same validator, avoiding implicit fractional/boolean integer conversion.

Numbering at each boundary:

| Boundary | Valid scene numbers | Example |
| --- | --- | --- |
| Service / public shell IPC | 1 through 8 | `scene(4)` / `omatainer scene 4` |
| Process argv / native CLI | 1 through 8 | `omatainer ctl scene 4` |
| Native JSON protocol | 0 through 7 | `{"op":"scene","n":3}` |
| Engine `LaunchScene` | 0 through 7 | `scene: 3` |

The CLI's existing #2 validator performs the only one-based to zero-based
conversion. No production native protocol or audio handler changes are needed.
Invalid shell inputs return a correlated rejected result before any control
process starts. Existing queue saturation, acceptance-versus-application
semantics and command-error handling remain intact.

Validation on the local machine, with the private Cargo target:

- `cargo test --offline`: **308 passed, 1 opt-in test ignored**.
- `cargo build --offline`: passed; `git diff --check`: passed.
- `python3 scripts/check-shell-scenes.py`: passed using the real Service.qml
  in an offscreen Quickshell process and a private executable argv sink. Local
  scene1/4/8 and externally called scene2 generated exactly four separate
  `ctl scene <n>` argv arrays. Twenty-six invalid numeric/string/boolean/missing/
  non-finite/structured inputs returned rejection and generated no sink call.
  The test checks the shell's range against `engine::SCENES`.
- The opt-in `native_shell_scene_arguments_reach_distinct_renderer_scenes` test
  was explicitly run and passed with `OMATAINER_TEST_BINARY` pointing to the
  freshly built CLI. It repeats the real Quickshell fixture against a private
  native IpcServer, real protocol handler and CommandPort, then observes and
  applies each admitted command to the actual engine. Scenes1/4/8/2 produced
  exactly `[0,3,7,1]`, with every track's playing scene checked after each change.
  Invalid scene calls admitted nothing. The fixture's explicit per-command
  application preserves intermediate evidence; ordinary audio-block command
  draining remains covered by the existing #2/#5 tests.
- `scripts/check-shell-controls.py --cli <built-cli>` passed all existing real
  Quickshell serialization,64-command saturation, accepted/coalesced results,
  rejection/process/timeout recovery, and native private-protocol bridge checks.

To repeat the native check:

```sh
cargo build --offline
OMATAINER_TEST_BINARY="$PWD/target/debug/omatainer" \
  cargo test --offline native_shell_scene_arguments_reach_distinct_renderer_scenes \
  -- --ignored --nocapture
```

When using `CARGO_TARGET_DIR`, point `OMATAINER_TEST_BINARY` at that target's
fresh binary. All shell configurations/runtime sockets are temporary and private;
no running desktop service, installed binary, audio/MIDI device or user session
was changed. This is control-path evidence, not final hardware/mode QA.
