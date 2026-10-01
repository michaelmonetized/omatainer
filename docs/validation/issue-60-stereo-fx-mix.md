# Issue 60: Spread and Balance mix in serial stereo chains

Spread and Balance now return their full-effect result into the common slot
blend instead of returning before it. The common blend applies the slot mix
exactly once, preserving the exact zero/full/neutral guards from issue 53 and
the single-interpolation policy from issue 54. Their existing full-effect laws
are unchanged. Every instance remains at its actual position in the serial
chain and retains independent controls/history.

Four new regression groups cover:

- Bit-exact zero mix for 129 amount positions on both effects at 32, 44.1, 48
  and 96 kHz, including signed zero and the original hard-left Balance fixture:
  input `(0.8, 0.8)` now remains `(0.8, 0.8)` at mix zero.
- 33 mix values from zero to full, seven amount positions including both sides
  of noon, and three sample rates. Over 2.8 million stereo frames match an
  independent reference bit-for-bit. Spread's reference owns an explicit ring
  buffer and does not reuse the DSP Delay or FxSlot/FxChain implementation.
- Four-slot chains with duplicate Spread/Balance instances match independent
  references in both orders. Balancing before narrowing differs from balancing
  afterward, proving placement affects the result.
- Spread history continues at zero mix and matches an always-wet processor when
  mix rises. A quarter-mix Balance retains its mixed endpoint through bypass
  reversal and the existing bounded 5 ms fade. Intermediate bypass arithmetic
  permits ordinary float roundoff (1e-7 on the unaffected 0.8 channel); zero mix
  and completed bypass remain exact.

No new processor state or callback allocation is introduced. Full-mix behavior,
neutral Spread's untouched delay history and per-slot bypass policy are retained.
These are local numerical render checks, not physical hardware QA.

Validation: all 278 local tests passed, the production build passed, the new
reference module passes rustfmt, and `git diff --check` passed. Existing tests
that require fully narrowed or hard-panned output now explicitly select full
slot mix; their original output assertions remain intact.
