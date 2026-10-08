# Controller data

The app ships original NS7, APC40 mkII, DDJ-SP1 and MPD232 LiveLite factory data. Existing compiled decoders, feedback and host-mode introductions remain in the app. No Ableton Python or downloaded executable is used.

Profile schema 1 requires portable MIDI preset version 6: at most 256 bindings and 64 KiB per profile. It describes exact USB model/release constraints, port roles, compiled capabilities, relative/control encoding, factory overlay, safe initialization, feedback and separate manufacturer/source/fixture/physical provenance. Unknown fields, unsupported capabilities and partial bindings are refused. Compiled surface addresses stay fixed; learned overrides and a saved MPD232 preset remain separate. New simple MIDI profiles may use the generic binding capability with explicit USB/port identities; that capability sends no surface feedback or host introduction.

`controller-profiles-export NEW_DIRECTORY` exports validated original factory data. Sign an immutable export with `node scripts/controller-profile-sign.mjs DIRECTORY PRIVATE_KEY_FILE GENERATION CATALOG_VERSION`. The key must be an owner-only Ed25519 PEM file matching `trust/ed25519-v1.pub`; keep it outside published source. The public trust root is compiled into the app. The signer never prints private key material.

The signed catalog authenticates the UTF-8 payload preceded by `Omatainer controller catalog schema 1\n`. Each entry pins a profile ID/version, exact filename, SHA-256 and length. Catalogs are bounded to 256 KiB and 64 profiles. Publish unchanged files as GitHub release assets under `controller-profiles-VERSION`, then update `profiles/catalog.json` to those same signed bytes. Older published versions remain immutable. This repository holds the data and app in the same PR; asset downloads work independently of native releases.

Acquisition uses the pinned HTTPS repository origin, HTTPS-only redirects, a ten-second request deadline, bounded reads, signature/hash/schema checks and a monotonic generation floor. The app validates the complete generation before publishing a private candidate directory and latest pointer. A failed download never changes working maps. The prior pointer and instance pins retain last-good data; an explicit stopped rollback does not lower the acquisition floor. History retains at most 64 generations and refuses a changed hash for an already published profile ID/version. Generation directories are never partially published.

## Runtime

Open **MIDI → Controller profiles and connection check**. USB attributes come from the ALSA port's actual sound card and sysfs parent. A surface output must pair uniquely with the same physical device and port. Names do not select native decoders. Duplicate serial identities, competing profiles and ambiguous outputs leave feedback disabled. Unknown models retain generic MIDI. Secondary ports of the four compiled surfaces require explicit input selection instead of automatic performance binding.

A private instance pin keeps the chosen profile and original MIDI assignment endpoint across ALSA renumbering. A serial-less device is tied to its USB topology; moving it to another hub socket requires review. Downloaded updates are cached until **Apply cached profiles** runs through the stopped connection transaction. **Restore previous profiles** retains earlier authenticated pins, or the bundled generation when no prior download exists. Learned assignments and local MPD presets are retained. During performance protection, a read-only presence poll retires missing inputs without opening replacements or downloading profiles. Reconnection and arrivals resume in Studio. Discovery previews and a single refresh are bounded to 256 endpoints; exact selected inputs are considered first. Connection history retains at most 320 endpoints, reusing pinned physical instances across ALSA renumbering.

Name one control, select its input and choose **Capture this control**. Captured messages are consumed before musical dispatch. **Finish input capture** resumes normal operation. Exercise the control normally, record the app response and lights/hardware you actually observed, then **Save controller evidence**. The check ends when its panel closes, its physical connection changes, or playback/protection starts. Queued captured packets remain consumed through finish and teardown. Only the selected input releases its held check state. It performs no automatic button, transport, motor or LED sweep. Successful output sends establish only output facts; absent input or physical observations remain pending.

Private check receipts include app binary hash, app/source/platform/compiled driver versions, profile data and acquired-file hashes, active binding hash/local preset flag, USB release, actual inquiry reply, bounded packets and separate user observations. `profile_data_sha256` hashes the validated structured profile; the authenticated catalog pins original file bytes. Native status exposes compact connection evidence. `controller-profiles-inspect` performs a read-only native USB/port inventory without opening callbacks or sending feedback.

The catalog's dated physical references do not qualify the current app, hub, firmware or every function. Recheck owned hardware after changes. Other models stay unqualified until their real input, application behavior and physical output are recorded.

## Source references

Live 11 Suite 11.0.11's read-only inventory contained 130 top-level script folders, including shared frameworks, and 1051 compiled `.pyc` files. That is not 130 proven models. No script code was copied, decompiled or executed. The earlier Sierra installation receipt identifies Live 10 Suite 10.1.43 on High Sierra; its current script inventory remains unverified. Live scripts are coverage references, not downloadable Omatainer drivers.

The catalogs under `tests/fixtures/controller-catalogs/` are original signed metadata fixtures. Generation 2 preserves existing file versions; generation 3 deliberately attempts to change one immutable version and must be refused. They are not published hardware releases and establish no physical qualification.

## Function evidence

| Model | Transport | Mixer | Pads / clip buttons | Jog / encoders | LED feedback | Audio |
| --- | --- | --- | --- | --- | --- | --- |
| Original NS7 | Compiled fixtures | Compiled fixtures | Compiled fixtures | Compiled fixtures | State/message fixtures | Native four-channel output previously observed; recovery qualification pending |
| APC40 mkII | Compiled fixtures | PAN/SENDS/USER physically observed in dated hub check | Scene 1, Stop All, Master and Device On/Off physically observed in dated hub check | Compiled fixtures; current sweep pending | Dated user light observations; current sweep pending | No audio interface |
| DDJ-SP1 | Compiled fixtures | Compiled fixtures | Compiled fixtures | Compiled fixtures | State/message fixtures | No audio interface |
| MPD232 LiveLite | Compiled fixtures | Compiled fixtures | Compiled fixtures | Compiled fixtures | Safe inquiry/send facts only; current response pending | No audio interface |

Fixture results establish the implemented message path, not physical compatibility. The reference for dated observations is [the powered-hub receipt](../docs/validation/powered-hub-controller-qualification.md). Current input, application, lights, motors and audio results remain separate pending checks. No NS7/SP1/MPD resweep occurred in that receipt.

The [acquisition checks](../docs/validation/controller-profile-acquisition.md) record the published release, installed HTTPS acquisition, stopped application of all four profiles, native inquiry responses, and network-isolated cache recovery. Their software results do not replace a physical control check.
