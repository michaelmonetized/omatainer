# Recovery lock lifetime during sampler qualification

The parallel sampler integration suite exposed intermittent refusal of the next
recovery append after the previous one had returned. A deterministic raw-fork
fixture held inherited descriptors across that boundary: both the direct root
transaction test and actual append→next append test failed on the old code with
`another recovery storage transaction is active` (0 passed, 2 failed, 0.01 s).
The private child only pauses and is killed/reaped by its parent; it performs no
Rust destruction, allocation, device access or storage mutation.

Recovery now owns root and session locks with a non-clonable guard that explicitly
unlocks before closing its file. Linux flock ownership follows the open file
description: an inherited descriptor can retain the lock after the parent closes
its copy, including during the interval before a CLOEXEC descriptor reaches exec.
An explicit unlock ends the owner's critical section without waiting for that
unrelated child. See [Linux flock semantics](https://man7.org/linux/man-pages/man2/flock.2.html)
and [Rust File::unlock](https://doc.rust-lang.org/std/fs/struct.File.html#method.unlock).

The regressions keep the child alive while proving immediate next-transaction
admission, exact second durable append and completed-session recovery. A live
owner remains excluded; closing the old inherited description cannot unlock a
new owner. Existing real killed-writer tests still cover abrupt termination at
append/asset/checkpoint boundaries. No lock retry, timeout, or concurrent-writer
exclusion was relaxed. This fix is included with the sampler integration that
exposed it; it does not change the recovery file format.

After the change, all 19 recovery storage tests passed with four test threads
(one explicitly child-only fixture ignored), including both deterministic
regressions. The complete stable-parent suite passed 809 ordinary tests with
15 opt-in fixtures ignored in 47.51 seconds. Final assembled sampler validation
is still required after integration. Red/green/full logs are retained as
`issue-101-recovery-lock-{red,green,full}.log` in the local work directory.
