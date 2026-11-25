# Production capability benchmark and scope

Research checked September 30, 2026 against official Ableton product pages, release notes, manuals and Pack pages. Source boundary for Omatainer is commit `cae546c7bc840ec1a643e4befe18f91e86d0e4c7`. This is a capability inventory and implementation specification, not a claim that Omatainer supports these features or that proposed acceptance criteria exactly reproduce Ableton behavior.

The latest stable release confirmed by opening the official release-notes page is **Live 12.4.6, September 15, 2026**. The comparison target is the released **Live 12 Suite** product, including capabilities represented by its included downloadable Packs. Search snippets sometimes returned 12.4.5 or 12.4.3; the directly opened stable page takes precedence. [Stable release notes](https://www.ableton.com/en/release-notes/live-12/).

The official beta page separately identified **12.4.15b5, September 30, 2026** when fetched directly over HTTPS. The web extraction tool repeatedly timed out on that page; direct retrieval of the official HTML succeeded. Its leading change concerned Extensions SDK globals. Beta-only Extensions SDK behavior is **not** treated as released parity. The open extension-runtime/authoring items instead benchmark the established Max for Live workflow. [Beta notes](https://www.ableton.com/en/release-notes/live-12-beta/), [Max for Live manual](https://www.ableton.com/en/manual/max-for-live/).

The current release boundary includes Link Audio and updated modulation/effect workflows in 12.4, together with stem separation, in-project bounce and sample-provider integration introduced in the 12.3 generation. The backlog explicitly includes production separation of audible or time-selected regions and merged selected stems; live DJ stem performance is owned separately by the DJ backlog. [12.4 overview](https://help.ableton.com/hc/en-us/articles/26963870210076-What-s-new-in-Live-12-4), [Stem Separation](https://www.ableton.com/en/live-manual/12/stem-separation/), [Bounce to Audio](https://www.ableton.com/en/live-manual/12/bounce-to-audio/).

## Inclusion and licensing rules

- The named commercial devices below identify capability classes. Implementations, controls, presets and artwork must be original or appropriately licensed; no item authorizes redistribution of proprietary Ableton, partner, Max or Pack assets.
- An included Pack can be an optional download and can depend on Max for Live. The inventory does not equate Suite inclusion with installation in every edition. Third-party marketplace products, historical legacy-device variants and unlimited community devices are not all separately promised.
- Sample libraries are grouped into independently deliverable content collections instead of filing one issue for each commercial Pack title. Artist identity, trademark, sample counts and gigabytes are not parity requirements.
- External services and hardware are separate dependencies. Splice supports free previews/free licensed material as well as paid access; a provider integration does not grant catalog rights. Hardware and subscriptions are not bundled application features. [Splice integration FAQ](https://help.ableton.com/hc/en-us/articles/22060471916316-Splice-integration-in-Ableton-Live-FAQ).
- Push issues distinguish controller integration from standalone hardware/software compatibility. They do not authorize firmware copying or claim permission to install Omatainer on proprietary standalone systems. [Push 3 manual](https://www.ableton.com/en/push/manual/).
- Counts on Ableton's shop and comparison pages differed when inspected. Coverage follows the named current capabilities, not a fragile aggregate device or Pack count. [Edition comparison](https://www.ableton.com/en/live/compare-editions/).

## Musical workflow sources

| Scope | Official source | Backlog coverage |
| --- | --- | --- |
| Session clips, tracks, scenes | [Session View](https://www.ableton.com/en/manual/session-view/) | Scalable set structure, editable clips, scene properties, Session-to-Arrangement capture |
| Clip launches and follow behavior | [Launching Clips](https://www.ableton.com/en/manual/launching-clips/) | Modes, quantization, legato, queued state, clip/scene follow actions |
| Linear song construction | [Arrangement View](https://www.ableton.com/en/manual/arrangement-view/) | Timeline, structural edits, locators, meter maps, linked editing, fades |
| Clip properties | [Clip View](https://www.ableton.com/en/manual/clip-view/) | Audio clip playback/editing, loop/offset and musical properties |
| Recording | [Recording New Clips](https://www.ableton.com/en/manual/recording-new-clips/) | Count-in, punch, Session recording, overdub, retrospective capture, step recording |
| Takes | [Comping](https://www.ableton.com/en/manual/comping/) | Take lanes, audition, range promotion, linked multitrack comping |
| MIDI editing | [Editing MIDI](https://www.ableton.com/en/manual/editing-midi/) | Piano roll, multi-clip editing, chance, velocity and scale context |
| Creative MIDI tools | [MIDI Tools](https://www.ableton.com/en/live-manual/12/midi-tools/) | Timing, articulation and recombination transforms; rhythm, melody and harmony generation |
| Per-note expression | [Editing MPE](https://www.ableton.com/en/manual/editing-mpe/) | Input ownership, recording, editing, playback and device expression |
| Microtonality | [Using Tuning Systems](https://www.ableton.com/en/live-manual/12/using-tuning-systems/) | Open tuning descriptions, keyboard mappings, reference pitch and bypass |
| Groove | [Using Grooves](https://www.ableton.com/en/live-manual/12/using-grooves/) | Extraction, shared templates, non-destructive application and commitment |
| Elastic audio | [Audio Clips, Tempo, and Warping](https://www.ableton.com/en/manual/audio-clips-tempo-and-warping/) | Transients, source/beat markers, material-specific stretch and quality controls |
| Transcription and slicing | [Converting Audio to MIDI](https://www.ableton.com/en/manual/converting-audio-to-midi/) | Melody/harmony/drum conversion and slice-to-rack reconstruction |
| Automation | [Automation](https://www.ableton.com/en/manual/automation/) | Lanes, curves, gesture recording, override and re-enable semantics |
| Clip modulation | [Clip Envelopes](https://www.ableton.com/en/manual/clip-envelopes/) | MIDI control lanes and clip-linked/unlinked modulation loops |
| Rack architecture | [Instrument, Drum and Effect Racks](https://www.ableton.com/en/manual/instrument-drum-and-effect-racks/) | Instrument/MIDI/audio chains, zones, drum pads, macros and variations |
| Device workflow | [Working with Instruments and Effects](https://www.ableton.com/en/manual/working-with-instruments-and-effects/) | Chain editing, named parameters, presets and A/B comparisons |
| Mixing | [Mixing](https://www.ableton.com/en/manual/mixing/), [Routing and I/O](https://www.ableton.com/en/manual/routing-and-i-o/) | Mixer, groups, sends/returns, sidechains and external hardware devices |
| Resource commitment | [Computer Audio Resources and Strategies](https://www.ableton.com/en/manual/computer-audio-resources-and-strategies/) | Recoverable freeze, alongside separate in-project bounce |
| Content discovery | [Working with the Browser](https://www.ableton.com/en/manual/working-with-the-browser/) | Production tags, presets, audition, swapping, similarity and optional providers |

## Built-in device capability crosswalk

The crosswalk includes alternative architectures where one Omatainer implementation can serve several related device roles. Distinct public device names are not an instruction to clone proprietary algorithms. Acceptance and validation in `production.json` are proposed Omatainer engineering requirements.

| Benchmark family | Capability issue(s) |
| --- | --- |
| Simpler | `prod-simple-sampler` |
| Sampler | `prod-multisampler` |
| Drum Sampler; Impulse-style per-hit sampling | `prod-drum-sampler`, `prod-drum-rack` |
| Drum Synths | `prod-drum-synths` |
| Analog, Drift, Bass, Poli | `prod-analog-synth` plus original presets in `prod-content-core` |
| Operator | `prod-fm-synth` |
| Wavetable | `prod-wavetable-synth` |
| Meld | `prod-macro-synth` |
| Granulator III | `prod-granular-synth` |
| Collision, Electric, Tension | `prod-mallet-model`, `prod-electric-piano`, `prod-string-model` |
| Instrument, Drum, Audio Effect and MIDI Effect Racks | `prod-instrument-racks`, `prod-drum-rack`, `prod-rack-macros` |
| Compressor, Glue Compressor | `prod-compressors` |
| Drum Buss | `prod-drum-bus` |
| Gate, Multiband Dynamics, Re-Enveloper | `prod-gate-expander`, `prod-multiband-dynamics` |
| Limiter, Color Limiter | `prod-limiter` |
| Channel EQ, EQ Eight | `prod-parametric-eq` plus streamlined saved presets |
| EQ Three | Existing implementation fixes plus DJ mixer requirements; production EQ expands the editing capability |
| Auto Filter, Auto Pan-Tremolo | `prod-modulated-filter`, `prod-autopan-tremolo` |
| Chorus-Ensemble, Phaser-Flanger | `prod-chorus-ensemble`, `prod-phaser-flanger` |
| Delay, Echo, Filter Delay, Grain Delay | Corresponding four production delay issues |
| Reverb, Convolution Reverb, Hybrid Reverb | `prod-algorithmic-reverb`, `prod-convolution-hybrid` |
| Beat Repeat, Looper | `prod-beat-repeat`, `prod-audio-looper` |
| Saturator, Dynamic Tube, Overdrive, Pedal | `prod-saturation-drive` |
| Roar | `prod-multistage-saturation` |
| Amp, Cabinet | `prod-amp-cabinet` |
| Redux, Erosion, Vinyl Distortion | `prod-lofi-texture` |
| Shifter, Auto Shift | `prod-pitch-frequency-shift`, `prod-pitch-correction` |
| Resonators, Corpus, Vocoder | `prod-resonator-effects`, `prod-vocoder` |
| Spectral Resonator, Spectral Time, Spectral Blur | `prod-spectral-processing` |
| Utility, Align Delay, Spectrum, Tuner | `prod-utility-analysis` |
| External Instrument, External Audio Effect | Corresponding production external-device issues |
| Pitch, Scale, Chord, Random, Velocity, Note Length | `prod-midi-note-effects` |
| Arpeggiator, Note Echo | `prod-midi-arpeggio-echo` |
| CC Control, MIDI Monitor, MPE Control | `prod-midi-control-tools` |
| LFO, Envelope Follower, Envelope MIDI, Shaper, Shaper MIDI, Expression Control | `prod-modulators` |
| Rotating Rhythm Generator, Melodic Steps and sequencer Packs | `prod-generative-sequencers` and editor-generator issues |

Device sources: [Instrument Reference](https://www.ableton.com/en/manual/live-instrument-reference/), [Audio Effect Reference](https://www.ableton.com/en/manual/live-audio-effect-reference/), [MIDI Effect Reference](https://www.ableton.com/en/manual/live-midi-effect-reference/), [Max for Live Devices](https://www.ableton.com/en/manual/max-for-live-devices/). These sources establish the available capability classes; the individual backlog requirements are original specifications.

## Included downloadable extensions and content

- [Performance Pack](https://www.ableton.com/en/packs/performance-pack/): global control pages, selective performance snapshots, arrangement loop grabs and prearranged recording are four separate workflow issues.
- [Creative Extensions](https://www.ableton.com/en/packs/creative-extensions/): creative delays, envelope/spectral processing, character limiting, analog synth roles and evolving sequencing are accounted for across dedicated families.
- [Sequencers](https://www.ableton.com/en/packs/sequencers/), [MIDI Tools](https://www.ableton.com/en/packs/midi-tools/): live generative devices and editable note-tool workflows are separate because playback devices and committed note transformations have different lifecycles.
- [Granulator III](https://www.ableton.com/en/packs/granulator-iii/), [PitchLoop89](https://www.ableton.com/en/packs/pitchloop89/), [Inspired by Nature](https://www.ableton.com/en/packs/inspired-nature/), [Convolution Reverb](https://www.ableton.com/en/packs/convolution-reverb/): granular, buffer-loop, physical-process and convolution categories each have explicit coverage.
- [CV Tools](https://www.ableton.com/en/packs/cv-tools/): hardware-calibrated pitch/gate/clock/modulation workflows are optional and require compatible interfaces.
- [Surround Panner](https://www.ableton.com/en/packs/surround-panner/): speaker-layout buses, multichannel positioning, downmix and export are covered by the shared `sys-surround-immersive` issue.
- Core, acoustic, orchestral/global and creative content issues cover the musical purposes of the current included sound collections. They require independently acquired rights and optional content management; they do not recreate a commercial Pack catalogue asset-for-asset.

## Hardware, synchronization and platform boundaries

[Link/Link Audio and Tempo Follower](https://www.ableton.com/en/manual/synchronizing-with-link-tempo-follower-and-midi/) are explicit production issues. Physical [Push 1/2 workflows](https://www.ableton.com/en/manual/using-push-2/) and [Push 3 controller workflows](https://www.ableton.com/en/push/manual/) require hardware validation before compatibility claims.

Generic project save/recovery/undo, audio device I/O, plugin formats and plugin sandboxing, delay compensation, file export/interchange, video scoring, platform accessibility, tutorials and quality infrastructure belong to the platform backlog. Full MIDI learn, MIDI clock and hotplug plus DJ media-library/live sampler workflows belong to the DJ backlog. Production references those shared foundations instead of filing duplicates. Max-like creative device authoring remains a production capability, distinct from plugin-host formats.

Omatainer source review covered `src/ui.rs`, `src/engine/mod.rs`, `src/engine/dsp.rs`, `src/engine/fx.rs` and controller maps. Each JSON issue includes immutable source anchors. Absence claims refer to the relevant current model/renderer/device boundaries, not a fabricated failing control. Existing partial implementation defects remain `[fix]/` issues; new complete workflows remain `[feat]/` issues.
