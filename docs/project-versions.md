# Named project versions

Open Project → Named versions and choose a dedicated new or empty version storage folder.
An existing nonempty folder must already be an Omatainer version store.
Enter a version name and revision notes, then Save named snapshot. A snapshot
captures the complete native musical state and the project view. Saving a version
keeps the current document's explicit save path and unsaved status.

Each immutable recording is stored once by decoded audio content. Further versions
store document/settings metadata and references to shared audio. Names, source
aliases, BPM metadata and waveform information belong to each revision. The version
folder contains a checked native index, native revision metadata and native audio
containers; keep the whole folder when moving or backing up versions.

Refresh lists the saved names and notes. Select one version and Compare. The preview
shows added, removed and changed tracks/scenes, clip/controller lanes, routing/mixer,
instruments/effects, timing and project view. Track and scene identities distinguish
renames from unrelated material. Changed rows show bounded before/after fields;
large comparisons retain complete category counts and summarize the first twelve
changed rows with up to eight fields each. Unchanged media indices compare by audio
content, rather than by their position in the saved container.

Restore compared version as unsaved copy uses the normal unsaved-work dialog and
stopped project installation. Save as chooses a new destination. The original
explicit project and immutable named version are retained. A changed current project
invalidates the comparison; changed revision files or damaged audio refuse restore.
Branch compared version into new project writes a complete new native .omat document
at the chosen path, without overwriting an existing destination or replacing the
live session. Open that branch with Project → Open when ready.

Select versions to remove and Preview pruning and unused assets. The preview lists
the named versions, audio files and revision files that become unreferenced. Apply
reviewed pruning first publishes the new index, then reclaims only those reviewed
files. Shared audio referenced by retained versions stays. With no versions selected,
the same preview can reclaim files left by a cancelled snapshot or deferred cleanup.
A changed index, changed cleanup file, unexpected file or replaced directory refuses
publication. A committed write or cleanup with a sync failure reports its actual
committed result and a warning; retry cleanup through a fresh preview.

Operations run on one bounded cancellable worker, outside audio/UI threads, and
participate in performance protection. Close/hide of the panel does not cancel an
admitted operation; Cancel version operation does. Up to 128 named versions, 256
native media references, 1 GiB decoded PCM per revision and 64 MiB native metadata
are supported. Version folders must use actual directories; symbolic links and
unexpected filenames in their owned audio/revision directories refuse cleanup. Embedded
native audio is self-contained; external picture, hardware connections and other
external resources retain their existing native project requirements.

Application close cancels pending version/import work and waits for its outcome.
If publication already completed, its result remains visible; close again after
the worker settles.
