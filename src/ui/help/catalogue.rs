//! The offline manual and widget descriptions share these reviewed definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Topic {
    Setup,
    Recording,
    Editing,
    Mixing,
    Dj,
    Controllers,
    Emergency,
    Projects,
}
impl Topic {
    pub const ALL: [Self; 8] = [
        Self::Setup,
        Self::Recording,
        Self::Editing,
        Self::Mixing,
        Self::Dj,
        Self::Controllers,
        Self::Emergency,
        Self::Projects,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Setup => "Audio setup",
            Self::Recording => "Record a held note",
            Self::Editing => "Edit and undo",
            Self::Mixing => "Mix a session",
            Self::Dj => "Prepare a DJ deck",
            Self::Controllers => "Connect a controller",
            Self::Emergency => "Stop and recover",
            Self::Projects => "Save and reopen",
        }
    }
    pub fn text(self) -> &'static str {
        match self {
        Self::Setup => "Start with the output quiet. Audio devices and latency reports the backend-accepted logical output, observed callback sizes and fixed main route; physical negotiated rate and converter latency are unavailable through CPAL. Main uses outputs 1/2, mono sums them, and additional outputs are silent. Settings profiles separate saved preferences from running audio: Apply and save persists audio choices without changing the stream. Preview saved audio, Use saved audio now, then Stop and change output explicitly stops performance and changes the stream, or restart to use the saved setup. A failed change restores the prior output when possible; if rollback also fails, the retained session can still Save, New/Open and Close. Playback never resumes automatically. Optional Measure loopback requires a suitable line-level cable/interface route and explicit confirmation; it reports qualified host callback-to-callback return timing only after three reliable probes, not physical converter roundtrip. Cancellation, missing loopback and corrupt evidence produce no current measurement. This build has one stereo main bus; the cue blend does not provide a separate headphone output. Load a built-in stem, play it, then verify sound at your physical output. Connection and meter evidence cannot prove that speakers or headphones are audible.",
        Self::Recording => "Choose an empty clip cell and use its Arm compose action (Shift-click or Shift+F10), or Arm selected cell. The armed destination is displayed above the sampler and stays fixed while browsing. Choose Keys, Analog or Pad; hold a pad and release it to record its actual duration. Stopped composition starts at local beat zero. Playing clips capture launch-relative positions. Disarm finalizes captures; Stop also disarms. Launch the resulting clip to hear it later. This is MIDI/pad note capture, not microphone or external audio recording. There is no piano-roll editor in this build.",
        Self::Editing => "Open a clip's alternate actions and choose Edit clip gain. Gain changes future note onsets; already held notes retain their onset gain. Use Edit → Undo / Redo, or the active shortcut bindings, to compare. A continuous slider gesture is one named history entry. History is bounded; admission failures or truncation are shown explicitly. Transport and physical held gates are not creative edits. This build does not offer a timeline, note-drawing editor, warp editor, automation lanes or plugins.",
        Self::Mixing => "Launch a scene, then adjust a track's gain. Track headers toggle mute; alternate actions provide solo and its effect rack. Muted tracks keep advancing, so unmuting returns to the current musical position. Scene effects process track audio assigned to that scene; monitoring and tails follow the track’s scene bus. Master effect slots process the stereo bus. Observe meters and callback diagnostics while adding load. Render CPU, callback elapsed time and UI frame time are different measurements; deadline overruns are not a backend XRUN count.",
        Self::Dj => "Select a crate row and explicitly load A or B. Built-in Drums (session) and Harmony (session) work without media files. Wait for Loaded: Queued only confirms admission. The filename BPM/key hints and heuristic analyzed BPM have distinct provenance and are not a verified beat grid. Set a main cue or hot cues, set loop in/out, then audition. Pitch lock separates pitch from tempo. Match follows the crossfader-favored deck's effective tempo. Remaining time is an estimate and repeating loops suppress runout alerts. Preparation persists in the library for the exact file identity.",
        Self::Controllers => "Open the MIDI window and Retry / rescan MIDI. Settings can request all inputs, selected exact port names or disabled inputs. Check requested versus applied policy and per-port errors before playing. The native callback only queues bounded input; overflow releases that source's gates and reports counters. Factory profiles are limited, documented mappings; a matching device name is not physical compatibility proof. NS7 motorized platter and NS7II support remain unverified. MIDI clock input is an observable tick hook, not tempo synchronization or clock output. There is no editable MIDI-learn mapping UI.",
            Self::Emergency => "Enable performance mode to protect playing/touched deck replacement, destructive edits and project/device changes across GUI, MIDI and IPC. Continuous mixing, live composition/recording, stopped-deck loads and Save remain available; optional scans/imports/analysis/theme/export work is refused or deferred. Safe stop deliberately stops the session and both decks, finalizes held captures and releases input notes; finite sample and effect tails continue naturally. Emergency silence additionally fades output to latched mute over 2 ms. Wait for renderer acknowledgment: request acceptance is not silence. Release physical keys, pads and touches, then explicitly acknowledge input recovery. This is your report, not a hardware health check. Emergency mute remains until a deliberate stopped DSP reset through the audio owner; a two-second −80 dBFS tail observation cannot prove bypassed/delayed history is empty. Failed reset keeps mute; you can acknowledge inputs and remain muted to Save and deliberately leave protection or close. No action automatically resumes playback. Ordinary Session Stop retains its narrower session-only behavior. Read Diagnostics queue/reset counters and explicit failures before recovery; physical hardware QA remains separate.",
        Self::Projects => "Project → New, Open and window close protect unsaved work with Save changes / Discard changes / Cancel. Save project as chooses a .omat path; replacing an existing file requires the checkbox. Save copy leaves the current path and unsaved baseline unchanged. Native projects embed playable media and creative state. Reopen restores playback stopped and excludes physical held keys, connections and DSP tails. Later edits during saving remain dirty. Cancel before commit preserves the old document; a completed commit is reported honestly. Use a private test destination for the guided example.",
    }
    }
}

pub(crate) struct Definition {
    pub title: &'static str,
    pub units: &'static str,
    pub purpose: &'static str,
    pub topic: Topic,
}
macro_rules! controls { ($( $id:ident, $title:literal, $units:literal, $purpose:literal, $topic:ident; )*) => {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub(crate) enum Control { $( $id, )* }
    impl Control {
        pub const ALL: &'static [Self] = &[ $(Self::$id,)* ];
        pub fn definition(self) -> Definition { match self { $( Self::$id => Definition { title: $title, units: $units, purpose: $purpose, topic: Topic::$topic }, )* } }
    }
}}
controls! {
    Help, "Help and lessons", "Offline reference", "Open task guidance, focused-control context, active shortcuts and observed-state lessons. Opening help never changes audio.", Setup;
    HelpTopic, "Choose help workflow", "Reference only", "Read a task chapter without changing your active lesson or musical state. Start this lesson deliberately captures a fresh baseline.", Setup;
    HelpSearch, "Search control reference", "Text filter", "Find implemented control descriptions by name, units or purpose. Filtering help does not filter the crate.", Setup;
    HelpReference, "Control reference entry", "Expandable offline description", "Read this control's purpose and units. The entry does not perform the named musical action.", Setup;
    HelpShortcuts, "Active shortcut reference", "Current profile bindings", "Expand the effective shortcut table and keyboard/assistive input instructions. Customize bindings in Preferences; this reference alone does not change them.", Setup;
    LessonStart, "Start guided lesson", "Observes current document", "Start or restart this task guide. Lessons never reset your document, start audio, bypass unsaved-work prompts or submit musical edits for you.", Setup;
    LessonNext, "Next lesson step", "Observed completion required", "Advance only after the current step has published evidence. Queue acceptance is insufficient. For physical checks the separate checkbox records only your self-reported observation.", Setup;
    LessonCancel, "Cancel lesson", "Guide only", "Stop this guide without undoing your work, stopping audio or releasing a held gate. Use the actual session/deck controls to stop playback.", Setup;
    LessonPhysical, "Physical observation", "Self-reported; not software proof", "Confirm only what you personally heard or tested on hardware. This does not certify compatibility, routing or lack of dropouts.", Setup;
    Preferences, "Preferences and profiles", "Saved versus running configuration", "Edit a profile, preview resolved changes, then Apply and save. Draft changes do not affect playback. Saving audio choices does not switch the running stream. Use the separate Audio devices confirmation or restart; live MIDI policy has its own requested/applied result.", Setup;
    PreferenceProfile, "Profile to edit", "Studio / performance / named copies", "Select which saved profile to edit. Selecting the editor profile alone does not activate it.", Setup;
    PreferenceActivate, "Activate profile", "Selected preference profile", "Choose the profile intended for activation on Apply and save. Audio differences remain saved intent until explicit device confirmation or restart; MIDI and appearance report actual application separately.", Setup;
    PreferenceClone, "Clone profile", "Independent named copy", "Create an editable copy in the draft. Nothing is persisted until Apply and save.", Setup;
    PreferenceRename, "Profile name", "Nonempty unique text", "Rename the draft profile. Validation reports conflicts before saving.", Setup;
    PreferenceAudioDevice, "Output device", "System default or exact device name", "Save the exact output device name, or follow the system default. Preview resolves availability and rejects ambiguous names. Apply saves the choice; a separate Audio devices confirmation or restart changes the stream.", Setup;
    PreferenceAudioRate, "Requested sample rate", "8000–384000 Hz; advertised options or device default", "Save an advertised output rate. The common 44.1/48/96/192 kHz choices appear only when advertised. Preview validates the whole configuration; saved intent and backend-accepted logical settings are distinct from physical negotiation.", Setup;
    PreferenceAudioBuffer, "Requested buffer frames", "16–32768 frames; optional device default", "Save an advertised output buffer request, or let the backend choose. Apply saves it without switching audio. Requested buffer duration, observed callback size and backend scheduling estimates differ; smaller requests do not prove fewer dropouts.", Setup;
    AudioDevices, "Audio devices and latency", "Saved intent / active stream / observed timing", "Inspect the active output and preview saved choices. Opening this window neither changes devices nor emits a probe. Save edits in Preferences before previewing them here.", Setup;
    AudioBackend, "Audio backend", "System backend or advertised backend name", "Save the backend used to resolve input and output device names. An unavailable saved backend fails visibly rather than silently choosing another route.", Setup;
    AudioOutputFormat, "Output sample format", "Advertised integer or floating-point PCM", "Save a format supported by the selected output layout and rate. Device default lets the backend select; unsupported combinations fail preview instead of silently falling back.", Setup;
    AudioInputDevice, "Calibration input device", "System default or exact input name", "Save the input used only for explicit loopback calibration. This does not enable input monitoring or general external-audio recording. Calibration uses the currently active output's logical rate.", Setup;
    AudioInputChannels, "Calibration input channels", "1–64 advertised interleaved channels", "Save the temporary input stream's channel count. Select the actual capture channel separately; all channel numbers are interleaved positions, not verified connector labels.", Setup;
    AudioInputFormat, "Calibration input sample format", "Advertised integer or floating-point PCM", "Save the temporary input format. Preview checks it against the active output rate and chosen input layout before any probe can run.", Setup;
    AudioInputBuffer, "Calibration input buffer", "16–32768 advertised frames or device default", "Save the temporary input buffer request. Together with an explicit output buffer this permits a buffer-only estimate; driver and converter time are excluded.", Setup;
    AudioInputChannel, "Calibration capture channel", "One-based channel 1–64", "Choose which input channel captures the loopback probe. Preview rejects a channel outside the selected input stream. No captured input is monitored to output.", Setup;
    AudioOutputChannel, "Loopback probe output channel", "One-based channel 1–64", "Choose which active output channel emits the probe. Other probe-stream channels are silent. Preview rejects channels outside the active output stream; verify the physical route yourself.", Setup;
    AudioProbeLevel, "Loopback probe level", "−60 to −24 dBFS", "Set the digital level of three short coded probes. This does not control external amplifier volume. Use a suitable line-level route, disable monitoring and turn down speakers before explicit confirmation.", Setup;
    AudioPreview, "Preview saved audio", "Read-only device discovery", "Resolve the saved profile against current input/output capabilities and the active calibration route. Preview changes no stream and plays no probe. Save draft preference edits first.", Setup;
    AudioUse, "Use saved audio now", "Opens disruptive-change confirmation", "Review the proposed output before choosing Stop and change output. This first button alone does not stop playback or activate the device.", Setup;
    AudioConfirm, "Stop and change output", "Explicit disruptive operation", "Stop decks, clips, recording and held notes, then activate the previewed output. Failure attempts the previous output; double failure retains the session for Save, New/Open and Close. Playback remains stopped even after success or rollback.", Setup;
    AudioKeep, "Keep current audio", "Dismiss confirmation", "Dismiss the device-change or probe confirmation without submitting it. Saved preferences remain saved; the current output is unchanged.", Setup;
    AudioMeasure, "Measure loopback", "Opens physical-route confirmation", "Review the chosen input, active output, channel numbers and probe level. This button alone emits no sound. A suitable physical cable or interface loopback is required.", Setup;
    AudioProbeConfirm, "Cable ready: stop and measure", "Explicit line-level loopback probe", "Confirm your chosen route, disable input monitoring and turn down external speakers. Stop performance, emit three low-level probes, capture up to three seconds, then restore output without resuming. Missing, noisy, clipped, ambiguous or inconsistent evidence produces no measurement.", Setup;
    AudioCancel, "Cancel audio operation", "Cooperative cancellation", "Request cancellation at safe worker boundaries; operating-system calls may still finish. Cancellation before output activation restores the prior route; a change already committed is reported honestly. Cancelled calibration produces no current measurement.", Setup;
    AudioNotice, "Audio operation status", "Pending / applied / rollback / offline", "Read the operation's actual result. Dismiss hides only the notice. An offline retained session still supports Save, New/Open and Close; queue acceptance is not proof that an output is running.", Setup;
    AudioCapabilities, "Advertised device capabilities", "Backend ranges; not physical qualification", "Expand input/output device ranges for formats, channels, rates and buffers. Exact names are not portable serial identities; duplicates are rejected. Discovery errors and truncation are explicit, and unknown buffer limits are not invented.", Setup;
    AudioActive, "Backend-accepted output", "Logical device configuration", "Read the configuration accepted by the backend. CPAL does not report the physical negotiated sample rate or converter latency; these logical settings are not a physical hardware measurement.", Setup;
    AudioTiming, "Observed callback timing", "Frames / milliseconds", "Read completed callbacks for the current stream. Callback duration uses its logical sample rate; backend output scheduling estimates are separate from driver/converter roundtrip and measured dropouts.", Setup;
    AudioBufferEstimate, "Roundtrip buffer estimate", "Requested input plus output frames / logical rate", "Estimate only the sum of explicitly requested buffers. Driver and converter time are excluded. If either buffer is backend-selected this estimate is unavailable.", Setup;
    AudioMeasurement, "Measured loopback return", "Host callback-to-callback timing / resolution / repeat spread", "Read a result matched to the exact current profile and preview. Three reliable recorded probes establish host callback-to-callback return timing, not converter-only physical roundtrip. Old, cancelled or unrelated measurements are not shown as current.", Setup;
    PreferenceNotice, "Dismiss preferences notice", "Message only", "Hide this notice without retrying, saving or changing the running profile.", Setup;
    PreferenceDelete, "Delete profile", "Inactive draft profile", "Remove this inactive profile from the draft. The active profile cannot be deleted; Apply and save commits the draft.", Setup;
    PreferenceReset, "Reset profile to defaults", "Selected draft profile", "Replace this profile's draft values with defaults. Cancel discards the draft; Apply and save commits it.", Setup;
    PreferenceReload, "Reload saved preferences", "Local saved file", "Read the saved preferences back into the editor. Unsaved draft changes may be replaced; validate and apply deliberately.", Setup;
    PreferenceRecover, "Preserve old file and reset preferences", "Explicit recovery", "Keep the unreadable/changed file as a backup, then create default preferences through the recovery worker. Failure remains visible rather than destroying the only copy.", Setup;
    PreferenceAudioChannels, "Requested output channels", "1–64 advertised channels or device default", "Save an advertised output stream channel count. Main uses channels 1/2, mono sums them and additional channels are silent; this does not create extra mixer or headphone buses.", Setup;
    PreferenceMidiRetry, "Retry saved MIDI policy", "Saved policy to live connection worker", "Request the saved profile’s policy again. Inspect applied generation and missing/failed ports; retry admission is not connection success.", Controllers;
    PreferenceMidiPolicy, "MIDI input policy", "All / selected exact names / disabled", "After Apply and save, request the live connection policy. Check requested/applied generation, missing ports and errors; saving alone is not connection success.", Controllers;
    PreferenceMidiNames, "Selected MIDI input names", "One exact port name per line", "Enter explicitly allowed inputs for Selected policy. Missing names remain missing; they never fall back to enabling all ports.", Controllers;
    PreferenceLibraryRoots, "Library locations", "One local path per line", "Choose locations for background scan. A changed active profile cancels/restarts discovery while keeping the visible crate until complete results arrive.", Dj;
    PreferenceScale, "UI scale", "50–300%", "Scale the interface after a successful Apply. Draft values alone do not change the live window.", Setup;
    PreferenceFont, "Font size", "8–48 points; optional desktop/default size", "Override the text size after Apply or keep the selected theme's size. Font selection remains the desktop font policy.", Setup;
    PreferenceTheme, "Follow desktop theme and font", "On / off", "Use validated live desktop theme/font updates when enabled. Disabling selects the app's default appearance; invalid updates retain last good settings.", Setup;
    PreferenceStartup, "Startup options", "Scan / help / MIDI windows", "Choose which background scan and panels start next time. Saving this option does not close the current project.", Setup;
    PreferenceShortcut, "Shortcut override", "Exact key and modifiers / disabled", "Override a stable named action in the active profile. Preview/Apply validate conflicts and reserved editing chords. In-app help shows effective bindings.", Setup;
    PreferencePreview, "Preview changes", "Read-only background resolution", "Resolve devices/routes and validate the draft without saving or changing the running engine. Missing hardware and unsupported routes remain explicit.", Setup;
    PreferenceApply, "Apply and save preferences", "Validated atomic local file", "Persist the validated draft before applying appearance/library/MIDI changes. Audio choices remain saved intent until the separate Audio devices confirmation or restart. Failure keeps running preferences unchanged.", Setup;
    PreferenceCancel, "Cancel preference changes", "Draft / pending operation", "Cancel pending work before commit or discard the draft and close the editor. A save already committed is reported as committed.", Setup;
    PreferenceImport, "Import preferences", "Validated Omatainer preference JSON", "Read a preference file into the draft on a worker. It does not activate or overwrite running preferences until Apply and save.", Setup;
    PreferenceExport, "Export preferences", "Local preference JSON", "Export validated draft preferences to the chosen local file. Existing-destination policy is explicit; exporting does not activate a profile.", Setup;
    PreferenceFilePath, "Preference import/export path", "Local filesystem path", "Choose a source/destination for portable preferences. No network transfer occurs.", Setup;
    ClosePanel, "Close panel", "View only", "Hide this panel without changing the audio or deleting its data.", Setup;
    Actions, "Control actions", "Focused control", "Open the focused control's alternate actions. Shift+F10 provides the same choices; controls without alternatives disable the menu.", Setup;
    NumericEditor, "Direct numeric entry", "Control's displayed units and bounds", "F2 opens a focused value editor. Apply accepts a finite in-range value; Cancel preserves the previous value. Arrows adjust and Shift makes fine adjustments.", Editing;
    Scroll, "Scrollable surface", "Pixels", "Scroll to controls outside the viewport. Focused controls reveal themselves; arrows and Page/Home/End work on the focused scroll view.", Setup;
    PerformanceMode, "Performance protection", "Session safety state", "Protect playing/touched deck loads, destructive edits and project/device changes centrally. Mixing, composing, recording, stopped-deck loads and Save remain available. Leaving requires a deliberate decision; opening a project never disables protection.", Emergency;
    PerformanceStop, "Safe stop", "Session and both decks", "Confirm stopping transports, finalizing held captures and releasing input notes. Natural sample/effect tails continue. Input recovery is explicitly acknowledged; transport never resumes automatically.", Emergency;
    PerformanceSilence, "Emergency silence", "All output", "Confirm all-notes-off and a 2 ms output fade to latched mute. Quiet observation alone never unmutes delayed or bypassed history. Explicit stopped DSP reset is required to unmute.", Emergency;
    PerformanceRecovery, "Controlled recovery", "User-confirmed physical release", "Release physical controls and explicitly acknowledge. The renderer drains old queued onsets before reopening controls. This is a user report, not hardware qualification. Emergency output mute stays latched until deliberate DSP reset.", Emergency;
    PerformanceReset, "Reset stopped DSP and unmute", "Audio-owner worker", "Deliberately reclaim the stopped graph, erase voice/effect/filter histories off the callback and reopen the same output. Playback remains stopped. Failed recovery retains emergency mute and reports its status.", Emergency;
    PerformanceCancel, "Keep safety state", "Cancel pending decision", "Close this confirmation without changing protection, transport, input recovery or output mute.", Emergency;
    AdmissionDismiss, "Dismiss admission error", "Message only", "Hide this rejection message. The rejected action was not applied; dismissal does not retry it.", Emergency;
    PitchLock, "Pitch lock", "On / off", "On preserves the file's pitch while the pitch fader changes tempo; off changes tempo and pitch together. This is independent of deck tempo sync.", Dj;
    Pitch, "Deck pitch", "Percent; selected ±8, ±16 or ±50 range", "Adjust playback rate around zero. Pitch lock determines whether the file's pitch follows the rate. Center resets to the original rate.", Dj;
    PitchRange, "Pitch range", "±8%, ±16%, ±50%", "Cycle the pitch fader's range. Check the current value after changing range.", Dj;
    HotCue, "Hot cue", "Eight positions per deck", "An empty slot stores the current position; a filled slot jumps there. Shift-click or Delete cue in alternate actions removes it. Loading and preparation follow the exact source identity.", Dj;
    DeckEq, "Deck EQ", "Bass / mid / treble: 0–340% linear gain", "Drag to change the band's gain; click cuts the band, alternate action solos it. Center is unity; left/right channel histories are independent.", Mixing;
    DeckGain, "Deck gain", "0–120% linear gain", "Set the deck's level before mixing. Click or alternate actions can cut/solo the gain path. Zero is silent.", Mixing;
    Platter, "Deck platter", "Playback / cue / jog", "Click toggles play/pause; right-click or Cue action returns to the main cue; Shift-click or Unload removes media. Drag while touching to scratch; release ends that touch. Keyboard/assistive actions expose the same operations.", Dj;
    Seek, "Deck waveform position", "0–source duration in seconds", "Seek within loaded media. Seeking resets grain history with a bounded transition; it does not change the stored file. A stopped deck stays stopped.", Dj;
    DeckFilter, "Deck filter (mapped MIDI)", "−100% low-pass to +100% high-pass", "Center is exact bypass. Moving toward either edge attenuates more of the opposite spectrum. Smooth transitions preserve independent stereo histories.", Mixing;
    Crossfader, "Crossfader", "0% A to 100% B", "Blend the two decks. At or left of center A is favored for Match; right of center B is favored. This is independent of session track gains.", Mixing;
    DeckTime, "Deck time display", "Elapsed source seconds / remaining wall estimate", "Choose elapsed or estimated remaining time. Rate changes affect the remaining estimate; this is not a measured hardware output latency.", Dj;
    RunoutWarning, "Runout warning", "0–300 seconds; 0 disables", "Set a per-deck warning before file end. Repeating loops suppress the warning. The alert does not stop or switch the deck.", Dj;
    Quantize, "Quantize", "On / off", "Toggle session launch and supported deck cue/loop quantization. Pending clip launches display queued until the selected beat boundary.", Dj;
    LoopBounds, "Loop in / out", "Source positions", "Primary action sets loop in; right-click or Loop out action sets loop out. The loop needs a valid ordered region.", Dj;
    LoopDouble, "Double loop", "×2 duration", "Double the current loop region within source bounds.", Dj;
    LoopHalf, "Halve loop", "½ duration", "Halve the current loop region within source bounds.", Dj;
    Reloop, "Reloop", "Four bars when creating a loop", "Toggle the current loop; if none exists, create the supported four-bar region.", Dj;
    Match, "Match decks", "Effective beats per minute", "Match the unfavored deck to the favored deck's file BPM times pitch rate. If both play, align the unfavored phase. The favored position does not jump.", Dj;
    ComposeArm, "Arm compose target", "One track and scene cell", "Explicitly capture the selected cell as the pad-writing destination; an empty target becomes MIDI. Browsing does not retarget it. Arming itself does not play a note.", Recording;
    ComposeDisarm, "Disarm compose", "Armed / disarmed", "Finalize current pad captures and stop automatic pad writes. Existing monitor voices stay held until their own releases. Session Stop also disarms.", Recording;
    SamplerBank, "Sampler bank", "Kit / Perc / Hits", "Choose one of three banks of sixteen distinct samples. New pad presses use the new bank; held voices retain their captured source.", Recording;
    SamplerInstrument, "Sampler instrument", "Samples / Analog / Keys / Pad", "Choose the source for new pad presses. Synth choices have distinct sound/envelopes; held voices keep their original instrument. Drums remain sample banks, not a separate synth choice.", Recording;
    SamplerOctave, "Sampler octave", "C1–C7; steps of 12 semitones", "Transpose held instrument notes by an octave; sample rate changes by powers of two around octave 3. Matching releases retain the original input identity.", Recording;
    SamplerPad, "Sampler pad", "Sixteen sample identities / thirteen piano keys", "Press holds a gate; release stops it, including outside the cell. Space/Enter hold the focused pad; assistive click toggles hold. Sample pads are 9–16 above 1–8; blank piano cells are inert. Armed compose writes one captured note.", Recording;
    CrateSearch, "Search crate", "Text filter", "Filter the crate without changing loaded media. Text editing owns its keys; clearing the filter restores the available rows.", Dj;
    CrateScan, "Scan library", "Background filesystem discovery", "Scan the configured music location. Progress and errors are visible; only a complete result replaces the crate. Existing selection and valid metadata survive.", Dj;
    CrateCancel, "Cancel scan", "Pending operation", "Request cancellation and retain the existing crate. An in-progress filesystem call finishes before cancellation is acknowledged.", Dj;
    CrateRow, "Crate row / selection", "Filtered row number", "Select with click or focused Up/Down/Page/Home/End. Double-click or Enter loads the selected deck; alternate actions choose A or B. BPM provenance, duration and history describe the exact source.", Dj;
    DeckSelect, "Selected deck", "A / B", "Choose the destination for F, Enter and controller Load selected. Explicit →A/→B always use their named deck. Browsing later does not redirect an already accepted load.", Dj;
    DeckLoad, "Load selected crate source", "Deck A / B", "Request the captured source on the named deck. Queued/decoding are not Loaded; wait for renderer confirmation. Failures retain existing media. Built-in stems bypass file decoding.", Dj;
    Track, "Track header", "Eight tracks", "Click toggles mute; right-click or alternate actions toggle solo. Alternate actions also select the track or open its effect rack. Muted playback keeps advancing.", Mixing;
    TrackGain, "Track gain", "0–120% linear gain", "Adjust track output level. Click toggles mute; right-click toggles solo; Control-click or alternate action opens effects. Held source ownership does not move with selection.", Mixing;
    Scene, "Scene launch", "Eight scene rows", "Primary action launches or stops the scene; Shift adds its clips alongside other scenes; right-click restarts. Alternate actions expose the same choices and the scene effect rack.", Mixing;
    Clip, "Clip cell", "Track × scene", "Click launches once; right-click launches looping. Shift-click explicitly arms composition; Alt-click opens gain. Shift+F10 exposes named actions. Queued launch is distinct from playing.", Recording;
    ClipGain, "Clip gain", "0–150% linear gain", "Change gain for future note onsets. Already held synth/sample notes retain their onset gain; track gain still affects the whole output.", Editing;
    FxAdd, "Add effect", "Track or scene rack", "Add the named implemented processor to the current rack. Arp is available only on a MIDI track's upstream events, not a scene audio bus. Capacity rejection is explicit.", Mixing;
    FxToggle, "Effect enabled", "On / off", "Enable or bypass this slot with a bounded 5 ms transition. Arp changes MIDI event scheduling rather than blending audio.", Mixing;
    FxParameter, "Effect parameter", "Per-effect displayed units and range", "Each slider names its implemented DSP control. The detailed label, current value and tooltip come from the same DSP metadata. Delay time is fixed at 250 ms; Arp has only enable/bypass.", Mixing;
    FxClose, "Close effect rack", "View only", "Return to the clip grid without removing or bypassing the rack's effects.", Mixing;
    MasterFxSelect, "Master effect type", "Echo → Reverb → Filter", "Cycle this stereo master slot's processor. Filter is a fixed 1 kHz low-pass. Each slot has independent left/right histories.", Mixing;
    MasterFxWet, "Master effect wet", "0–100%", "Blend dry and processed output for this master slot. Zero preserves dry audio; 100% selects processed output.", Mixing;
    Midi, "MIDI connections", "Port status and input counters", "Open the MIDI window to inspect actual connection state, queue/drop/reset counters and per-port errors. Device names alone do not prove mapping or hardware behavior.", Controllers;
    MidiRetry, "Retry / rescan MIDI", "Background connection request", "Discover current ports and retry the requested input policy. Busy means work is pending; missing/failed ports remain explicit. Keyboard and mouse remain available.", Controllers;
    ProjectMenu, "Project menu", "Native .omat documents", "Create, save or open a document. New/Open/Close protect unsaved work and restore playback stopped.", Projects;
    ProjectNew, "New project", "Empty session", "Replace the document with an empty session and factory sampler resources only after the unsaved-work decision. Use Cancel to keep working.", Projects;
    ProjectOpen, "Open project", "Native .omat file", "Read and validate a project off the UI/audio thread before replacing the current document. Failed or unsupported files preserve the current session.", Projects;
    ProjectRecent, "Recent projects", "Local paths", "Open a previously successful project path with the same validation and unsaved-work protection as Open. A listed path may no longer exist.", Projects;
    ProjectSave, "Save project", "Current .omat path", "Save a coherent renderer capture. Edits made after capture remain unsaved. An error does not mark the document clean.", Projects;
    ProjectSaveAs, "Save project as", "New current .omat path", "Choose a destination and make it current after successful save. Explicit replacement permission is required for an existing destination.", Projects;
    ProjectSaveCopy, "Save copy", "Independent .omat destination", "Write a copy without changing the current project path or clean/dirty baseline.", Projects;
    ProjectPath, "Project path", "Absolute or working-directory-relative path", "Enter the local native project destination or source. No network upload occurs. Cancel preserves the current document.", Projects;
    ProjectReplace, "Replace existing project file", "Explicit overwrite permission", "Permit replacement of this chosen destination for Save as/Copy. The file is staged and committed atomically; rejected saves preserve the previous file.", Projects;
    ProjectDiscard, "Discard unsaved changes", "Destructive document decision", "Authorize this pending New/Open/Close without saving the current unsaved work. This is not an undo-history operation.", Projects;
    ProjectCancel, "Cancel project operation", "Pending dialog or worker", "Keep the current document by cancelling the pending decision/operation before commit. A commit already completed is reported as completed, not undone.", Projects;
    Undo, "Undo", "Previous named creative transaction", "Restore the previous recorded creative edit while preserving unrelated transport and input gates. Availability is renderer-confirmed, bounded and separate from queue admission.", Editing;
    Redo, "Redo", "Next named creative transaction", "Reapply the next recorded edit. A new edit clears the redo branch. Rejected history operations leave a visible reason.", Editing;
    History, "Edit history", "Bounded named transactions", "Inspect applied/redo entries, target context and memory accounting. Continuous gestures group; unsupported future features are not represented.", Editing;
    HistoryDismiss, "Dismiss history message", "Message only", "Clear the renderer's history notice. It does not retry a rejected edit or recover evicted history.", Editing;
    Diagnostics, "Audio diagnostics", "Measured CPU, wall time, queues and sampled load", "Inspect callback elapsed time/deadlines separately from render-thread CPU, UI update time and reported output latency. Backend XRUN count remains unavailable when the backend supplies none.", Emergency;
    DiagnosticCapture, "Start diagnostic capture", "Bounded local sample report", "Capture measured counters and sampled render cost with bounded storage. No project audio or personal media paths are exported.", Emergency;
    DiagnosticStop, "Stop diagnostic capture", "Capture state", "Finish collecting diagnostic samples and keep the captured report for review/export. Audio playback is unchanged.", Emergency;
    DiagnosticCancel, "Cancel diagnostic operation", "Capture or file worker", "Cancel the pending capture/file operation. The audio engine is unaffected; an already committed file is reported honestly.", Emergency;
    DiagnosticFileCancel, "Cancel diagnostic file operation", "Pending worker", "Cancel the pending report export or reopen before commit. A file already committed is reported as completed; live audio is unchanged.", Emergency;
    DiagnosticPath, "Diagnostic file path", "Local JSON path", "Choose a local destination/source for a redacted diagnostic report. Export refuses an existing destination instead of overwriting it.", Emergency;
    DiagnosticExport, "Export redacted diagnostics", "Local JSON file", "Write the bounded captured report on a worker. Wait for exported confirmation; no upload or automatic sharing occurs.", Emergency;
    DiagnosticReopen, "Reopen diagnostic capture", "Validated local JSON file", "Read a prior report for inspection without changing the live engine. Invalid/oversized reports are rejected.", Emergency;
    Library, "Library catalog", "Persistent preparation and media identity", "Inspect catalog save status or import an Omatainer catalog. The library is separate from native projects and never substitutes another file's preparation by path alone.", Dj;
    LibraryImportPath, "Catalog import path", "Local Omatainer catalog JSON", "Enter the catalog to validate and merge. This does not import arbitrary third-party library formats.", Dj;
    LibraryImport, "Import catalog", "Validated background merge", "Merge compatible identities and preparation off the UI thread. Malformed/newer/conflicting data is rejected; unrelated pending edits remain preserved.", Dj;
    LibraryRetry, "Retry library save", "Persistent preparation", "Retry a failed catalog publication. Keep the visible failure until durable saving is confirmed.", Emergency;
    LibraryCloseWithoutSaving, "Close without saving library", "Explicit data-loss decision", "Exit despite unsaved library preparation after a save failure. Keep working or Retry preserves a chance to save it.", Emergency;
    License, "Licenses and notices", "Installed build records", "Read bundled notices for this build offline. The native manifest identifies the packaged components and exact source records.", Setup;
    LicenseSearch, "Search license notices", "Text filter", "Filter local component/notice entries. No network lookup occurs.", Setup;
    LibraryKeepWorking, "Keep working", "Cancel pending exit", "Cancel the pending close while library saving is pending or failed and return to the current document. Unsaved library edits remain available for retry.", Emergency;
    LicenseSource, "Notice source link", "External upstream URL", "Open the recorded upstream source in your browser. This leaves the offline viewer; the bundled notice remains available without network access.", Setup;
    LicenseNotice, "Full license notice", "Bundled text", "Expand the complete license or attribution text recorded for this build. Reading does not change audio or project state.", Setup;
    LicenseEntry, "License entry", "Component record / notice text", "Select or expand a bundled notice to read its attribution and terms.", Setup;
    LoadRetry, "Retry media load", "Captured source and destination", "Retry the failed source on its original deck. Later crate selection does not redirect Retry. Existing playing media stays until accepted replacement is applied.", Dj;
    LoadDismiss, "Dismiss media status", "Message only", "Hide this load status. It does not unload the deck, cancel playback or discard an already rendered play-history event.", Dj;
}

pub fn manual() -> String {
    let mut text = String::from("# Omatainer offline manual\n\nGenerated from the in-app help catalogue. Hardware observations remain separate from software confirmation.\n\n");
    for topic in Topic::ALL {
        text.push_str(&format!("## {}\n\n{}\n\nGuided example (use Start this lesson in Help; Next requires observed evidence):\n\n", topic.title(), topic.text()));
        for (index, step) in super::lessons::steps(topic).iter().enumerate() {
            text.push_str(&format!("{}. {}\n", index + 1, step));
        }
        text.push_str("\n");
    }
    text.push_str("## Control reference\n\n");
    for &control in Control::ALL {
        let d = control.definition();
        text.push_str(&format!(
            "### {}\n\n{}\n\n{}\n\nWorkflow: {}.\n\n",
            d.title,
            d.units,
            d.purpose,
            d.topic.title()
        ));
    }
    text.push_str("## Shortcut reference\n\nThe in-app Help window shows the active bindings. Defaults follow; text fields and dialogs own their keys before performance shortcuts.\n\n");
    for b in super::super::shortcuts::BINDINGS {
        text.push_str(&format!("- {}: {}\n", b.label, b.description));
    }
    text
}
