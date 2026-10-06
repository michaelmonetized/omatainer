# Mic and auxiliary mixer qualification

Issue #180 adds native mic/aux roles through retained mono/stereo input aliases, independent master/booth/raw recording inclusion, gain/mute, optional three-band tone, input/output meters and deliberate talkover. No input device is opened by this panel. New selections begin muted. Source selection requires stopped playback/recording, Studio mode and a fresh review; pending activation and final encoding also keep the recording source fixed. Live scalar controls remain available in Performance Mode.

The optional configuration is retained in project state version 17. Coherent capture, real native container save/load, renderer replacement, sample-rate preparation, Undo/Redo and version comparison preserve it. Foreign selective imports keep the destination's external choices. Routing cannot remove used aliases or duplicate the selected physical feeds.

The renderer uses fixed two-role frames and filters. Gain/tone/missing-source transitions take 5 ms; a replacement source fades the previous role out before fading in. Input callback width validates availability. Talkover ducks music before voice addition with a finite threshold, 0–40 dB reduction, bounded attack/release and automatic/held/bypassed operation. Master monitoring includes the selected audience mix; PFL remains separate. Final output safety, limiting and conversion follow the new contributions.

## Verified software paths

- Eleven focused checks passed (1.68 s): actual input-pipe mono mic + ordered stereo aux, separate master/booth/raw record mixes, master/PFL headphones, real final-output versus raw recording delivery and decode, source replacement/loss, missing channels/nonfinite source, tone response and measured talkover envelopes at 44.1/48/96 kHz. Native review/application, protection, stale scopes, command coalescing, save/reopen/restore, selective import, comparisons and Undo are covered. Actual callback, role processing and relevant commands perform no allocation or free on the measured renderer thread.
- Complete serial unfiltered suite: **1838 passed, 0 failed, 44 ignored, 0 filtered**, 473.58 s, default test stack. The offline manual was regenerated through its real maintainer test.
- Private test executable SHA256: `7a864c2308dd3bd60df16fb0cb4122b52da39139e1d62e33f7d1fdce8cce8756`. Frozen compiled source inventory: 694 files, checked unchanged before and after qualification. [Receipt](mic-aux-receipt.json) binds source/log/binary hashes.

The initial extended gate passed ten checks and failed one expected-level assertion. That fixture omitted final limiting. The corrected reference and fresh build pass all eleven; the original log, executable and source inventory remain in ignored project artifacts. No DSP behavior was changed for that reference correction.

## Boundaries

Qualification is Linux ARM64 software with controlled source frames and unopened physical devices. A microphone, external line source, PA feedback behavior and sustained hardware listening remain physical acceptance work. These receipts do not claim those outcomes. The final-output recorder always captures the actual output including selected voices; use a raw recording mix for voice exclusion. Physical clock compensation remains tracked by #178.

## Installed release

Forced fresh Linux ARM64 release source `475c9d550026472bb338bb3cde9cdc558bd10a64`, version `0.1.0+475c9d550026`, executable SHA256 `96b7bc4a36197555f05c570fc54d85653ed57a195732f588b453d947fc84f320`, package manifest SHA256 `f3e6cde20a299b330d06d1c30bfcd317e5dcbe9149bb5490c17786ab8e98db5d`. The verified default executable is installed for the next normal launch. Existing independent controller GUI and follower processes retained their exact executable hashes. Pre/post state kept master 1, session stopped, recording off and both decks paused; no hardware was opened or playback restarted.
