# Shared fixture adapter

`scripts/suite-qualification.mjs` translates existing production test results into
the shared SUITE-02 receipt contract in Omadesign's `qualification/suite/index.json`.
It adds no decoder, render engine, schema or device route. The shared runner owns
source/binary/tool/input/reference integrity, process groups, the global lock,
receipt storage and per-fixture candidate decisions.

The tone cohort reuses the original authored tone files: exact WAV/FLAC content,
400/800 Hz channel identity in compressed files, bounded length evidence, malformed
FLAC/truncated WAV and allocation refusal. Lossy priming/padding is an explicit
expected loss; this is not a gapless claim. The native session cohort reuses
attachment/rack/note/control capture, actual history commands, native archive and
fresh-process reopen. The Groove case uses the existing CC BY 4.0 WAV/MIDI members
and source attribution; it does not credit automatic grids as human ground truth.

Run `node scripts/suite-qualification.mjs --plan` to inspect the exact assertions.
Run `node --test scripts/suite-qualification.test.mjs` for the Node-only rejection
check: an exit-zero command with no actual test and a missing binary both fail.
No product/audio device is started by these two commands.

Runtime requests are private project-local JSON with `{fixture:{id},input,output,
binary,driver:"libtest"}`. Use only through the shared globally serialized runner,
with a build receipt from the exact clean source/tree and actual test executable.
Each named test must report exactly one pass and zero ignored tests. Assertions
and process logs are retained as check artifacts; zero-test filters cannot pass.
Software checks are separate from installed app/native windows, audio devices,
listening, physical controllers and independent users. Missing evidence stays
unavailable; historical plugin or device receipts do not qualify a newer tree.

AUDIO-03/AUDIO-04 producer matrices and generator outputs remain owned by their
feature streams. Their stricter required delivery/routing gates still apply.
This adapter does not copy or redistribute private DPF plugins or SDK archives.
