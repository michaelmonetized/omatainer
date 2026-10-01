# Issue 64: persistent bounded status follow

The CLI subscribes once using `{"op":"follow","id":...}`. The native server
keeps that client worker and emits the latest status every 250 ms after the
previous bounded write. There is no update queue and no catch-up burst. This is
an explicit read-only mode: further request bytes cannot issue musical commands.
Ordinary connections still close after 32 requests.

The existing maximum of eight tracked client workers includes followers. The
initial line remains limited to 4096 bytes, 500 ms idle and 2 seconds total read;
frames remain at most 8192 bytes with a 200 ms write budget. Slow nonreading peers
are retired on write failure. Server shutdown closes the tracked sockets before
joining workers; subscription pacing adds at most 250 ms to that shutdown path.
Snapshot acquisition remains bounded to 50 ms. Temporary snapshot contention
reports a state error on the same subscription, not a fabricated command receipt.

Frames preserve the ordinary status fields and add `follow:true`. Subscription
IDs are validated, and `accepted`/`command_status` remain null. The server copies
only scalar telemetry and capped names (eight MIDI labels of up to 64 bytes and
two deck titles of up to 256 bytes). Unchanged bounded state reuses its encoded
frame without allocation. Changing real callback metrics naturally invalidates
that cache; this does not claim zero serialization during active audio. Tracks,
clip notes and peak arrays are never copied or serialized by the follower.

The CLI makes nonblocking Unix connection attempts, uses a 2-second response
budget, and retries at 250/500/1000/2000/4000 ms (capped at 4 seconds). Receiving
a valid frame resets the delay. It has no helper threads, pending command queue
or retryable musical actions. Closing stdout yields a successful exit and drops
the connection; SIGINT/SIGTERM process teardown also closes its descriptors.

Validation includes the production handler with 36 updates over one connection
and one worker, the unchanged ordinary 32-request cap, eight simultaneous
followers with a busy ninth client, snapshot contention/recovery, a saturated
socket write, and joined worker cleanup. CLI tests reject malformed, oversized,
wrong-ID and receipt-shaped stream frames. The cache test checks exact ordinary
status parity, then 1000 unchanged checks with eight 1 MB track names and one
million peak entries: zero allocations and one initial serialization.

The separately selected 130-second native test uses the freshly built CLI as a
child process and the production server on a private 0700 temporary runtime
directory. It counts accepted connections and started handlers on both sides of
a server restart, checks exponential retry timing, then closes the output reader
and verifies successful child exit and released worker. It never contacts the
running application's socket or opens audio/MIDI hardware.

Local results: all 297 regular tests passed (the soak is separately opt-in), and
the production build passed. The native soak passed in 130.79 seconds with 489
updates, exactly two accepted connections and two created handlers across the
restart, peak active handlers of one, five outage attempts with increasing
spacing, and successful output-close exit. The old source-derived rate would
have required roughly 520 connections over the same interval. This measures
connection/worker churn, not a claim of prior catastrophic CPU usage or physical
hardware performance. New modules pass rustfmt and `git diff --check` passes.

Reproduce the native run after `cargo build` with the private Cargo target:

```sh
OMATAINER_TEST_BINARY=/home/michael/Projects/omatainer-work/issue-36/target/debug/omatainer \
CARGO_TARGET_DIR=/home/michael/Projects/omatainer-work/issue-36/target \
cargo test native_cli_multiminute_connection_counts_restart_backoff_and_clean_exit -- --ignored --nocapture
```

Assembled stack validation: 350 ordinary tests and production build passed. The
separate native soak completed in 130.88 seconds with 490 updates, two accepted
connections, two handlers, one peak active handler, five outage attempts, and
clean output-close exit. Seven CLI protocol checks, five runtime isolation
checks, three real shell/native CLI fixtures and the scene panic-abort probe
passed. The scene probe now imports its own Write trait after the follower
removed that production-main import; the shell fixture recognizes subscriptions.
