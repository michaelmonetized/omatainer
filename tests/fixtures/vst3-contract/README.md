# Original VST3 graph contracts

`contract.cpp` is original Omatainer test code under the repository's MIT license. It contains two native processors and one controller. It is not copied from a vendor demonstration, song, script, plugin or sound library.

The effect has stereo input, an initially disabled mono sidechain, stereo output and an initially disabled mono output that reports the actual VST3 transport context. Its output is `(main + 2 * sidechain) * gain`, with a switchable 64-sample delay. The instrument receives channel-specific note events and generates a constant signal while a note is held. Both retain gain, latency and bypass in their own component state. Their auxiliary buses deliberately require explicit host activation. Neither has an editor or a CC123 MIDI mapping.

Build against Steinberg VST3 SDK 3.8.1, revision `3cdf9ca5d1f5b1b21e0a86832aa4abe55607bd96`, after building its Release SDK libraries:

```sh
tests/fixtures/vst3-contract/build.zsh /path/to/vst3sdk /path/to/sdk-build "$PWD/target/vst3-fixtures"
```

The builder accepts Linux aarch64 and x86_64, checks the SDK revision, and writes only into the selected output directory. It does not fetch or execute remote code. Keep SDK builds, fixture binaries and private sessions outside tracked source. Retain the SDK's MIT notices with any distributed linked binary; the SDK is a separate dependency, not bundled here.

Set `OMATAINER_TEST_BIN` to a freshly built Omatainer executable and `OMATAINER_VST3_FIXTURES` to the fixture output directory. The ignored `engine::audio::routing::plugin_tests::native_` tests exercise real isolated plugin processes and the same graph used by playback and export. The UI attachment test is in `ui::audio_routing::tests`. Headless widget tests do not prove native editor rendering or physical audibility.
