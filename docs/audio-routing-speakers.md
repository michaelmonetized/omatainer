# Declared speakers

Routing owns `speakers::Layout`, `Speaker`, `Binding` and `OutputContract`. A saved `Model::speaker_layouts` binding attaches a layout to a retained graph identity. Missing bindings preserve the old mono/stereo defaults. A wide port does not establish speaker identity.

The first wide layout is `Layout::StereoLfe`: FL, FR, LFE, in that exact order, with WAVE mask `0xB`. WAVE interleaving follows ascending mask-bit order; physical device indices may be nonconsecutive. These are [WAVEFORMATEXTENSIBLE file rules](https://learn.microsoft.com/en-us/windows/win32/api/mmreg/ns-mmreg-waveformatextensible), not panning or speaker calibration rules.

`Model::channel_contract(Group::Output(id) | Group::Record(id) | Group::Input(id), encoded_count: u16) -> Result<OutputContract, String>` returns the actual encoded `channel_count`, ordered physical source `channels`, ordered encoded `speakers`, and `wave_mask`. Native wide delivery must match its explicit declared layout. Mono/stereo delivery is an explicit fold: LFE is excluded; mono averages the resulting stereo pair. Old mono duplication and stereo fold behavior remain unchanged.

The public delivery adapter remains `audio_delivery::output_contract(&State, &Export) -> Result<OutputContract, String>`. The contribution adapter remains `audio_delivery::render_selected(Captured, &Export, Option<&Selection>, &Path, bounds, &Arc<AtomicBool>, &Reporter)`. The renderer writes generic interleaved frames. Delivery owns encoded writers, variable headers, recovery and publication. A writer's mask/count check cannot establish the meaning of supplied samples; the graph's contract supplies that order.

Built-in stereo inserts process FL/FR. The dedicated LFE channel passes those stereo inserts without processing or an automatic 10 dB boost. Track, return and master faders, mute/solo gates, track crossfader assignment, causal routing and output safety apply to the complete bed. A shared native wide return must preserve the declared primary bed; auxiliary ports retain their independent identities. Use explicit reviewed maps for a format conversion.

These are chosen DSP policies. Physical speakers, crossover calibration, listening levels and device clock behavior are separate proof. This source layer is pending software qualification; positional controls, explicit monitor bass management, recording integration, save/reopen and encoded graph fixtures remain required before 2.1 is complete.
