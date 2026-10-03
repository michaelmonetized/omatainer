# Licensed providers and Free To Use: issue 120

Source freeze: `9c17715c80fa6ca0a7ea52526c03d8e2d8ce82a9`. Base: PR #480,
`477894bbe0ed2d4ef35e76b5137c334fda17930b`.

Typed contracts cover stable remote identity, public/account authentication,
current capabilities, bounded search and licensed preview preparation. The real
Free To Use adapter uses its documented public API; no key or account is needed.
Network access starts disabled on every launch. Explicit non-commercial license
and attribution consent enables non-premium catalog search and one original-pitch
preview. No paid license, premium access or commercial approval is configured.

The native panel displays current offline/stems/recording/deck restrictions,
remote UUIDs and copyable credits. Latest item lookup rechecks premium status
before audio fetch. Transient PCM never enters local crates, sampler banks,
project capture, performance-history audio capture or portable exports. Closing,
revoking consent, native/scheduled Stop, project installation and safety changes
cancel obsolete preview audio. HTTPS and decoding run on one bounded job;
renderer ownership retires on the existing worker.

The [contract guide](../music-providers.md) records the official API/license
sources, enforced limits and exact validation boundary.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0, actual App, renderer and private HTTP
clients. Final-source evidence:

- **1,315 ordinary tests pass**: 1,314 serial, zero failures, 30 opt-in ignored,
  528.71 seconds. The separate valid-scene test passes in 0.06 seconds. The
  exhaustive invalid-scene boundary ran inside the serial group for this issue.
- **15 focused debug checks pass** and **140 optimized checks pass**, including
  provider/GUI/renderer contracts, decoding, loading, samplers, scheduling,
  project capture/installation, catalog, crate views and manual synchronization.
- Explicit real HTTPS search and single-track lookup pass: public authentication,
  query `lofi`, 266 matching tracks, item
  `5a4ac3dd-b6e0-42fb-9f2b-bca2f58225a2` (Sleepy, Johny Grimes), non-premium,
  observed `https://eu.data.freetouse.com` media origin. No provider music fetched.
  An additional public browse request returns 12 items from 1,604 catalog tracks.
- Private HTTP fixtures exercise actual search, lookup, latest premium status,
  fetch and own generated audio decoding. Premium changes refuse audio fetch.
  Expired accounts, HTTP 401, regional 403/451, redirects, malformed metadata,
  unapproved origins, byte bounds, cancellation and late publication rejection
  have structured failures.
- Native AccessKit actions exercise consent, search, preview, credits, stop and
  revocation through the actual App. Exact original-pitch samples, scheduled Stop
  on its due sample, cancellation, replacement and natural-end retirement perform
  zero callback allocations or frees. Actual project capture excludes preview PCM.
- Controlled gate: **eight workloads, three repeats, zero callback allocations
  and frees**, unchanged expected audio hashes and unchanged policy.
  CPU 6 run: `2026-10-03T04:50:18.835276+00:00` to `2026-10-03T05:00:09.438410+00:00`, after ordinary tests finish.
- Native AT-SPI preflight: **158 actions,
  257 visited nodes, 580 App frames**.
- Eight license/package fixtures pass; 488 retained source/build/gate files and
  346 license entries include the new HTTPS dependencies. No provider music asset
  or music license grant is bundled. `git diff --check` passes.
- Independent checking of the preserved production artifact passes.

## Artifact bindings

Preserved in `/home/michael/Projects/omatainer-work/issue-120-qualified-release`:

- Production: `e69e6524fdfafbe9611c64762e3e77a54fccae7aac9e2f0999ae8d2844c10fe3`
- Release tests: `ed7a146e8c996b8f8c2dbb5e742017b4a89917000023b71558c566afd8dfd174`
- License manifest: `edc2a02c62e5d513d1c6f2687ba387a86aa4de9e1eae10a2c4884d880931fcb4`
- Ordinary tests: `390aa6b75addb84b6ae66919ef23e5a470d1006526e56c39e916b2b4d4b107fb`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Local receipts share `/home/michael/Projects/omatainer-work/issue-120-`:
`qualified-ordinary-tests.json`, `qualified-suite.log`,
`qualified-scene-boundary.log`, `qualified-focused.log`, `qualified-live-api.log`,
`release-focused.json`, `release-focused.log`, `performance.json`,
`performance.raw.json`, `performance.log`, `license-tests-final.log` and
`preserved-check-before-qa.log`.

No paid license, premium music, commercial deployment, provider music playback,
OS window or physical audio/MIDI-device QA is claimed. The inherited issue-107
supplemental quiet-host wall limit remains separate. Public catalog success does
not grant paid or commercial music-use rights.
