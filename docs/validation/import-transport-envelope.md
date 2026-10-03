# Persisted transport allowance for project import

Follow-up to PR #488, stacked above PR #489. Import metadata admission retains
space for native capture fields that can grow without an edit revision: every
track launch, transport/deck scalars, and up to 256 held-note durations. The
worker reduces its 64 MiB metadata admission limit by a bounded allowance;
ordinary native saving keeps its existing limit. Launch/stop and continuing
playback remain usable during review. Existing revision/rate/view guards still
reject ordinary changes. No renderer-side serialization or allocation is added.

Each track reserves 96 bytes (more than the entire largest launch object), each
floating-point transport field reserves 32 bytes (more than a complete finite
JSON number), and held durations reserve at most the recording engine's 256
capture slots. The maximum allowance is below 24 KiB. The regression demonstrates
that an exact metadata envelope fits its original limit but fails the reserved
limit, then changes launches and transport position between review and Apply
without a revision increment, confirms allocation-free application, and proves
that the resulting serialized growth fits the retained allowance.

Eleven focused local Linux aarch64 tests pass (six import model and five native
import UI fixtures), along with eight license/package fixtures. Debug test SHA:
`9096d08464dfd25cea04101b78c740444b3763d92751081954956ed4191d8935`. Logs are retained at
`/home/michael/Projects/omatainer-work/import-transport-{model,ui,package}.log`.
The full preceding release qualification remains documented in
`issue-124-project-versions.md`; this layer does not claim a separate full gate.
