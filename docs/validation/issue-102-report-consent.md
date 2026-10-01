# Exact report consent and linked recovery action identity

The initial support UI cleared its consent boolean when a newly inspected report
arrived, but reused the native checkbox/export identities. A delayed action from
the previously exposed checkbox could therefore consent to a different report.
The linked-recovery preview action had the same identity reuse across two exact
verified candidates.

Successful Preview/Reopen admission now immediately retires preview, consent
and the linked candidate. A checked generation and the SHA-256 of the exact
worker-encoded JSON scope preview controls. Export additionally requires a
consent marker matching that generation/digest, and captures the same immutable
report. New lookup admission retires its old candidate; lookup generation,
session, sequence and verified record digest scope the linked preview action.
Category changes retire the entire preview/consent/candidate relationship.

The report digest is computed on the existing bounded support worker. It adds
no callback work, fields to the exported schema, raw logging or uploader.
Committed export result handling is unchanged: cancellation after publication
cannot turn it into an unsaved failure.

Two actual App regressions fail on the previous source in 0.92 seconds:

- `old native consent applied to a different report`
- `old linked action opened a different exact recovery`

The tests retain real egui native action IDs, then collect a changed report or
verify a second durable recovery record. Old actions cannot consent/export or
open the new candidate. Fresh review exports the exact preview bytes; fresh
linked preview restores the second record's 132 BPM state. Reopening identical
report bytes still requires a fresh generation of consent. A separate real
export test waits for file publication without GUI polling, cancels, and then
checks a truthful committed success and exact bytes.

Validation on integrated102 base28d0c04:

- Ten actual support App groups pass in 3.04 seconds.
- All22 support/model/storage/App groups pass, one opt-in child entry point
  ignored, in 5.27 seconds with two test threads.
- Fresh private Linux AT-SPI support workflow passes: 245 visited native nodes,
  nine actions, 194 App frames, no audio callbacks or admitted/rejected engine
  commands. Scope remains actual App/offline owner and native accessibility API;
  no native window, Orca, physical devices or actual process restart claim.
- Production build and `git diff --check` pass.

Logs are `/tmp/issue102-consent-red.log`, `issue102-consent-green.log`,
`issue102-consent-support.log`, `issue102-consent-native.log`, and
`issue102-consent-build.log`. Final full suite and performance qualification are
performed on the parent's ordered assembled stack.

Duplicate-reference follow-up: the actual collector can retain A/B/A recovery
references. The new actual-App regression confirms both A controls have distinct
native action identities and neither substitutes another record or mutates the
project. This regression also passed before the explicit reference-index salt,
because egui already distinguished the controls by their parent insertion
positions. The index change makes that identity intentional; it is hardening
and existing-behavior coverage, not a second reproduced defect. All eleven
support App groups pass in 3.06 seconds with two test threads. Baseline-pass
evidence is `/tmp/issue102-duplicate-red.log` (despite its provisional filename);
the final result is `/tmp/issue102-duplicate-final.log`.
