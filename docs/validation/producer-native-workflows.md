# Native producer import, plugin controls and saved state

This records another completed qualification step inside [#511](https://github.com/michaelmonetized/omatainer/issues/511) and [PR #510](https://github.com/michaelmonetized/omatainer/pull/510). The combined release and physical acceptance remain open.

## Native workflows

The optimized Linux application ran on its own authenticated Xvfb display with a private JACK dummy server, separate preferences/state and MIDI disabled. The installed powered-hub application, controller connections and saved session were preserved. The private server has no physical audio connections.

Actual native widgets reviewed an existing user-authored Live 10.0.1 Set from the owner's Music disk, displayed its differences, accepted acknowledgement, published a new native project and opened it stopped. The project retained six tracks, eight scenes, 170 source devices and 806 asset references. This Set contains no audio or MIDI clips: it qualifies structural import, not playback fidelity. Publishing again to the same destination was refused after a fresh review, with both source and destination hashes unchanged. Cancelling another real migration stopped its filesystem worker and preserved the current project.

Separate original private fixtures exercised producer discovery, installed Pack metadata review/selection and preset import. The native browser discovered the source preset and manifest, reviewed the selected installation, required acknowledgement, published and opened the native project. Its original Eq8 device XML and selected Pack identity remained retained. These fixtures use the observed file grammars; they are not claimed to have been authored by Live.

The plugin browser scanned the privately built licensed Nekobi and MVerb bundles in isolated processes. Cancelling an instrument routing draft preserved the project; applying an instrument and then an effect preserved their order. Both native editors opened, rendered and closed. Native Save retained the two processors' opaque states alongside the imported device and Pack records. Cancelling the unsaved-project prompt preserved the current project; Discard opened the selected project.

Both producer windows were exercised at the app's 860 × 640 minimum size. Their acknowledgement, destination and publication controls are now reachable by scrolling. The earlier fixed-height bodies could leave those controls below the window.

## A real sound-control failure and its correction

Nekobi declares 2,090 parameters. Its first 128 include hidden MIDI controls; the sound controls follow them. The initial generic editor and worker observation limit selected those hidden controls, making Cutoff unavailable. Filtering the UI alone exposed Cutoff but failed the native persistence test because the worker still observed the old selection.

The host now uses one bounded selection of up to 128 visible, writable parameters for the generic editor, worker observations and callback-side publication. It excludes the SDK's `kIsHidden` and read-only flags, preserves declared order and looks up callback observations by stable parameter ID. The [SDK parameter definition](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ParameterInfo.html) defines those flags. The native editor remains available for the plugin's complete controls.

The corrected native regression drove the actual egui Cutoff widget to 0.625, observed that value from a real processor, saved its opaque and parameter state, loaded the native archive and prepared a fresh processor with the same value. In the independent GUI, Cutoff was also set to 0.625, saved and reopened. Both processors restored with the original preset/Pack folder offline; the live Cutoff widget read 0.625 and reopening did not change the saved file.

## Failure paths and evidence boundaries

Changing selected Pack metadata after review refused publication without creating the requested destination or changing the open project's saved archive. After clearing that selection, a separate corrupt ALS file was refused as truncated XML. The first corrupt-file attempt stopped at the changed Pack check; only the subsequent attempt qualifies parser refusal.

Generation 27 passed 71 selected ordinary migration, discovery, plugin and routing tests. Its first ordinary migration run aborted on the test thread's default stack while constructing the existing renderer. The unchanged serial repeat used a 64 MiB test-thread stack and passed; both logs are retained. Generation 31, paired with worker release 5, passed 18 selected parameter, real-time, native migration/UI and native plugin graph regressions. These are selected checks, not a complete suite.

[The receipt](producer-native-workflows-receipt.json) records exact sources, frozen binaries, private evidence hashes and successful/failed logs. Generation 31/release 5 include the parameter correction on the recorded pre-merge source. The later retrospective-MIDI merge has not been qualified by these binaries. Private Sets, presets, plugin bundles, screenshots and executable archives are ignored project-local artifacts; none is a public factory-content fixture.

The non-real-time dummy server accumulated late blocks during builds and GUI work. These checks establish native workflows and saved state; they do not establish stage performance, physical listening, Live 10/11 source playback equivalence or hardware loopback. Those retained criteria still require their own evidence.
