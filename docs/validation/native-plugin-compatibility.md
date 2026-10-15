# Native plugin compatibility

AUDIO-02 remains open until each format's acceptance and native editor qualification are complete. Loading a real native processor in an isolated worker is software proof. It does not prove installed-app playback, audibility, hardware latency, desktop editor behavior or every plugin in that format.

| Format | Implemented route | Qualification boundary |
| --- | --- | --- |
| VST3 | Existing Linux isolated scanner/worker, graph and editor | Prior receipts remain in `vst3-core.md` and `vst3-graph.md`. This stack must also pass focused regression checks. |
| CLAP | Native Linux ELF modules, factories, state, bus and parameter metadata, MIDI and exact-offset addressed events, latency and X11/floating editor ownership | DPF v1.7 native aarch64 release modules plus an original compiled C ABI conformance plugin. Editor windows and installed-app playback are untested in this stream. |
| LV2 | System `liblilv-0.so.0`, isolated RDF discovery/instantiation, required-feature validation, grouped audio ports, controls, Atom/MIDI, URI-keyed state and embedded path assets, native floating idle/show UIs | DPF v1.7 aarch64 bundles. Required unsupported extensions fail explicitly before native instantiation. Embedded-only toolkit UIs need a toolkit host; generic parameter controls remain available. Worker scheduling, unsupported port types and optional extensions are not claimed. |
| VST2 | Unavailable. No SDK, binary translation or compatibility bridge is included. | The scanner reports legacy libraries/presets explicitly. Install a compatible native edition or preserve the original device and import an aligned render. No VST2 hosting claim. |
| Audio Units | Unavailable in this Linux build. Conditional on a future qualified macOS port. | No AU SDK shim or Linux substitute. The native process boundary currently uses Linux `prctl` and Unix sockets; no macOS compile or runtime claim. |
| Windows plugins | Unavailable in the native Linux worker. No Windows bridge is included. | ELF architecture and format mismatches are refused before loading. No Windows compile or runtime claim. |

## State and migration

Projects retain exact binary hashes, descriptor/class URI, plugin version, codec and opaque state. Existing VST3 state remains `vst3-host-0.9-state`; CLAP and LV2 use distinct codecs. New cached metadata and modulation fields have serde defaults. An unsupported format is not silently replaced by a similarly named processor.

For imported Ableton projects, **Import → Ableton Live project → review dependencies** retains the original device chain and source state. A compatible VST3 replacement still requires its original class identity. Use the existing aligned-render import to supply the source-rendered audio, choose timing/tail/printed mixer scope, and review before applying. The original editable tracks, devices, media provenance and mute/routing restoration data remain in the project. See the focused `ableton::renders::tests` save/reopen and restore checks. CLAP/LV2 native plugin state is not represented as converted AU/VST2 state.

## Source and license review

- Comparison reference: SoundCraft `3a28a657692c1b98b28c276d8f61df9471074f0f`, `crates/mix/src/lib.rs`, `crates/clap-host` and `crates/au-host`; MIT OR Apache-2.0, reviewed at the shared pinned checkout. No SoundCraft source or assets were copied into this implementation.
- CLAP bindings: `clap-sys` 0.5.0, MIT/Apache-2.0, checksum recorded in Cargo.lock. `libloading` 0.8.9 and `x11-dl` 2.21.0 were already locked transitively and are now direct dependencies. The app remains 0.1.0.
- CLAP public SDK 1.2.2, MIT, is selected explicitly for the original C conformance fixture. The fixture is built only inside the serialized focused test; no SDK is vendored in the product.
- Native DPF v1.7 binaries come from [the upstream release](https://github.com/DISTRHO/DPF-Plugins/releases/tag/v1.7). Plugin licenses vary; these private test installations are not committed or redistributed. The release license inventory was reviewed. No proprietary SDK or plugin binary is included in these PRs.

## Required remaining proof

Native editor open/resize/close/reopen, shipped desktop UI use, installed-app playback and other OS/CPU runs remain unproven. No desktop, hardware, audio-device or benchmark automation was run. See the stream's machine-readable handoff for exact commits, test receipts and failures; implementation is not an acceptance sign-off.
