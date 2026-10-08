# Ableton migration matrix 1

Project → Setup → **Import Ableton Live Set…** reviews an owned Live 10/11/12 Set, `.alc` clip Set, or `.adg`/`.adv` user device preset without changing it. The saved XML format must have `MajorVersion=5` and a `MinorVersion` beginning with 10, 11 or 12. A filename or installed Live version does not establish compatibility. Raw UTF-8 XML and one CRC-checked gzip member are supported.

Review track, scene, device and asset counts and every playback difference. Supply an explicit source-prefix/replacement-folder pair for cross-machine media. Acknowledge the report and publish to a new `.omatainer` path. Publication is atomic; an existing destination is refused. The running session remains intact until **Open imported project**, which uses the ordinary unsaved-work decision. Migration metadata uses project state version 36 and retained-source schema 1 or 2; schema 2 retains explicitly selected installed Pack manifests.

| Source feature | Native behavior | Retained or unresolved behavior |
| --- | --- | --- |
| Audio, MIDI, group and return tracks; scenes | Stable native IDs, Unicode names, ordinary mixer gain/pan/mute, groups and returns | Source IDs, parent roles and palette indices remain in the archive; palette colors need source comparison |
| Session and Arrangement clips | Audio/MIDI source placements, note timing/length/velocity/release, loop ranges and source offsets | Positive Session start offsets, clip envelopes, expression/probability and unsupported launch quantization are individually reported |
| Tempo and meter | Target-linked tempo automation, linear BPM ramps, packed meter changes and pre-roll values | Integer MIDI tempo precision and Bezier approximation are reported; original handles remain retained |
| Locators and song loop | Native named sections and musical loop braces | Source transport is reopened stopped |
| Media | Project-relative paths, explicit prefix maps, stable mounted-volume identity and embedded verified PCM | Missing, offline, inaccessible and unsupported references remain distinct |
| Audio playback | Unwarped source trim, pitch resampling and reverse at the initial tempo | Future tempo changes resample unwarped clips; warped clips remain disabled until reviewed render/relink, with markers/mode retained |
| Routing | Internal group/main outputs and active pre/post sends with cycle checks | Unknown outputs remain disconnected and named; external destinations, rack branches, multi-output and sidechain inference require explicit review |
| Devices | Ordered source inventory, enabled/bypass state, original XML/state and automation targets | Ableton engines, Max devices, nested racks, MIDI effects, AU and VST2 remain unresolved |
| VST3 relink | Exact retained class ID plus installed native binary/version, original component/controller state, normalized parameters and supported linear automation | Version changes require a compatibility decision; display names alone never match. Nested racks and ambiguous effect-before-instrument chains are refused |
| Frozen/flattened or exported audio | One reviewed ordinary-track print or complete mix, aligned source start/body/tail, source tempo clock and printed-processing routing | Original MIDI/devices stay editable and muted. Group/return prints use complete-mix scope; source clock follows its original tempo map |
| Save, reopen and merge | Retained source XML, dependencies, plugin state, clocks, report and render receipt persist; selected-track merge uses fresh graph IDs and one Undo | Printed-master projects require whole-project opening. Imported render backups become nonrestorable when a partial merge changes their routing namespace |

These are implemented behaviors and explicit limits, not identical-sound certification. The current qualification uses independently authored format contracts, a private saved Live 11 default Set and native SDK/third-party plugin fixtures. Original user-authored Live 10/11 reference Sets, source-app comparisons and listening remain required by #511.

## Stock device conversion

| Device category | Conversion | Resolution |
| --- | --- | --- |
| Track/master mixer volume, pan and mute | Explicit native values | Source mixer automation is retained and reported when not converted |
| Ableton instruments and stock effects | No automatic engine substitution | Retain original state; choose an authorized aligned render or future explicit replacement |
| Instrument/audio/MIDI racks | No automatic branch flattening | Retain branches, macros, key/velocity ranges and source routing in the original XML |
| Max for Live and licensed Pack engines | No automatic native-plugin conversion | Retain identity and dependencies; use authorized source playback/export |
| Native VST3 instrument/effect | Compatible installed-class relink | Preserve original processor/controller state and declare mixer/chain-order differences |
| Foreign OS/CPU plugin, AU or VST2 | Unresolved platform/format dependency | Install a compatible native VST3 version with reviewed state compatibility or supply audio |

## Render and restore

Use a source export with the declared start and musical body, including its complete tail. Track prints include track effects and mixer gain, excluding returns/master; pre-fader or additional outputs must be reviewed separately. Complete mixes may include returns and, when explicitly declared, master processing/gain. Omatainer disconnects the printed source routes and uses a neutral render track so baked processing is not applied twice. A printed master bypasses native master gain; the physical output limiter still applies. [Ableton's stem instructions](https://help.ableton.com/hc/en-us/articles/360000843404-Importing-and-exporting-stems) explain source export alignment and processing choices.

**Restore pre-render draft** reverses an unpublished attachment. To restore after reopening, review the `.omatainer` with **Read a saved Omatainer migration archive**, then **Restore saved editable source**. The original Live Set, installation and render file can be offline. Restoration checks the printed route hash, source mute state, references and print placements; later conflicting edits are refused. The saved archive must remain available and unchanged during review/publication. Relink devices after restoring the editable draft.

## Producer libraries

Choose **Discover producer libraries** for paged searches across home, configured/custom roots and current accessible mounts. Search and continue are explicit; sources, limits, exclusions and failures remain visible. Samples are candidates until the ordinary decoder verifies them. **Use sample folder for relink** supplies a replacement folder; provide the original prefix and review the source again. No filename-only matching is inferred.

**Review Pack manifest** supports installed `Ableton Folder Info/Properties.cfg` metadata with the observed `Ableton#04I` grammar. It retains the declared Pack identity/vendor/version, original metadata, SHA-256, fingerprint and stable volume-relative identity. After reviewing applicable use, **Use reviewed Pack for next import** resolves only matching Pack IDs and bounded relative paths. The importer embeds verified referenced audio while keeping the device engine unresolved. Changed, unavailable or ambiguous installations are refused before publication. Saved native archives retain their metadata and embedded audio when original installations are offline.

`.alp` archives require installation with the authorized source application; discovery does not unpack or execute them. Unsupported manifest versions remain explicit review failures. Ordinary `.adg`/`.adv` device presets produce one editable source track while retaining original XML, chain order, nested branches, macros, key/velocity ranges and state. Top-level compatible VST3 effects can relink with their exact class and original state. Instrument role must be declared or recognized; unknown plugin roles, nested rack branches and Ableton-native engines require explicit resolution. Clip Sets use the same structural checks as Sets. These paths do not certify identical playback or convert factory engines into native plugins.

## Bounds and failure behavior

Source and expanded XML are capped at 32 MiB, with strict UTF-8, depth/node/event limits and no DTD, external entities or network references. Decoded migration audio is capped at 512 MiB. Source descriptors, fingerprints and content hashes are checked before publication. Review, relink, render attachment, restoration and source verification run in disposable app processes with bounded pipes, descriptors, memory and deadlines. Private native containers use sealed [Linux memory descriptors](https://man7.org/linux/man-pages/man2/memfd_create.2.html), not source-volume temporary files. Cancellation kills the process group; kernel-stalled children retain one of four worker slots until reaped. No vendor integration script is downloaded or executed.

Public fixtures are authored here or separately licensed. Private sessions, plugin binaries, factory sounds, Pack content and compiled vendor scripts remain outside releases. Installed content is not automatically a loadable or redistributable library; applicable rights remain source-specific. [Ableton's terms](https://www.ableton.com/en/eula/) are the primary installation/content reference.
