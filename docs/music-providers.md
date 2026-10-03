# Music provider contract

Open **Music providers** in Setup. Every launch starts with network access and
provider playback disabled. Opening the panel creates no outbound request.
Enable only after accepting the current non-commercial license and attribution
requirement, then choose Search or Preview. Commercial use is unavailable because
no paid license for Omatainer is configured. No provider account, subscription or
commercial approval is claimed.

Free To Use documents a public HTTPS API with no account or key. Its developer
guide permits non-commercial use of non-premium music with title, artist and
Free To Use link attribution wherever music plays. Commercial apps, including
commercial music discovery, require a developer license for their digital
product. Users need suitable rights for their own published content. Check the
[developer guide](https://freetouse.com/blog/royalty-free-music-api-for-the-apps-you-build)
and [current license](https://freetouse.com/license) before enabling music use.
The app does not turn public API access into a paid music grant.

## Interfaces and enforced capabilities

`MusicProvider` defines identity, public/account authentication, current
capabilities, bounded search and licensed preview preparation. Access requires
documented authorization, music-use consent, supported platform and unexpired
authentication. Regional denial is a structured failure. The contract provider
exercises account expiry; Free To Use itself uses public authentication.

`TrackId` contains provider identity and the provider's UUID. Catalog metadata
and transient preview PCM never become `LibSource` file paths, crate members,
sampler banks or project media. Search results show premium status, duration,
remote identity and copyable credits. Preview credits remain visible while audio
plays. Closing the panel or revoking consent cancels playback.

Current **adapter** limits are one original-pitch preview voice, up to five minutes,
and zero DJ decks, offline storage, stems or recording/export. These are enforced
adapter capabilities, not claims that the provider forbids every other possible
integration. Paid/premium playback and other named providers remain unavailable.

## Worker and renderer boundaries

One native panel job runs at a time. HTTPS uses Rustls, normal certificate
verification, no proxy inherited from the environment, no redirects and a
12-second per-request timeout. The operation has a 20-second cancellation lease.
Search pages contain at most 12 items; query text is limited to 1,024 bytes,
metadata to 1 MiB, encoded audio to 32 MiB and decoded PCM to 128 MiB. Catalog
fields, UUIDs, pagination and the observed `https://eu.data.freetouse.com` media
origin are validated. Unrecognized media origins fail closed.

Preview resolves the remote item again before fetching its music. A premium
status change refuses the fetch. Network, decoding and validation run off the UI
and audio callback. Cancellation is checked while reading and decoding and again
before publishing. Closing a running request cancels it without blocking a GUI
frame; its worker owns only the bounded operation and finishes under its timeout.
Safe mode and performance protection exclude optional network/decode work.

The renderer applies only current transport and safety epochs. Native and
musically scheduled Stop, project installation, safety changes, consent revocation
and cancellation invalidate old audio. One voice resamples only to the output
rate, keeps original pitch and uses 25% preview level followed by the existing
master level, limiter and safety output. Preview audio bypasses deck/master
effects and enters after performance-history audio capture. PCM ownership moves
to the existing retirement worker; no network I/O, allocation or final heap
release occurs on the callback.

## Validation boundary

Private real HTTP clients exercise catalog parsing, lookup, latest premium status,
fetch, own generated WAV decoding, oversized responses, malformed metadata,
unauthorized origins, HTTP 401, regional 403/451 and redirect refusal. Contract
tests exercise expired accounts, cancellation and capability restrictions. Native
AccessKit actions exercise consent, search, preview, copy credits, stop and
revocation through the actual App and renderer. Actual project capture excludes
preview PCM; original-pitch and scheduled-stop samples have zero callback heap
work.

The opt-in `live_public_freetouse_catalog_search_and_current_lookup` test performs
real HTTPS search and single-item lookup, reports the remote ID and explicitly
does not fetch provider music. No paid license, premium audio, commercial release,
provider music playback, physical audio device or desktop window QA is claimed.
