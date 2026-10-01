# Issue 43: explicit composition arming

The sampler displays renderer-confirmed Compose disarmed or Compose armed with
track and scene. Arm selected cell and Disarm compose are explicit controls;
shift-clicking a sequencer cell explicitly arms that cell. Plain Select only
browses. Empty cells become MIDI clips at arm time, not selection time.

The renderer stores an independent optional ComposeTarget. Playback, scene
launches and browsing cannot retarget it. Both sample and instrument pad
monitoring use its track, and capture uses its scene. Existing held input
ownership preserves the original route and release after retargeting.
Disarm/retarget finalize held pad captures without cutting monitor voices or
changing live MIDI captures. Stop and transport toggle-off disarm; restarting
cannot implicitly arm. Explicit Record retains its existing independent path.

Validation on assembled issue 37 prerequisite:

- 219 local Rust tests and production build pass.
- Five new engine groups cover exact arm/Stop/TogglePlay/pad regression,
  sample/synth routes across selection and playback, explicit retarget/disarm
  with held capture duration, recording without composition, invalid targets
  and actual snapshot worker publication.
- Two new real 1440x1000 egui App interaction tests click sequencer shift-arm,
  sampler Arm/Disarm and pad pointer-down/up. They assert visible armed target
  and disarmed text, stable writes while playing, and no new notes after the
  Stop/restart regression. Stop is consumed before restart, matching command
  admission's safety-stop reservation.
- Existing recording, arpeggiator and scene validation fixtures now explicitly
  arm when composition is intended; release/ownership tests still only browse.
- A pre-existing concurrent command-budget test assumed final playhead position
  measured progress through finite demo media. Producer descheduling can let
  the renderer reach EOF. The fixture now loops and sums wrapped distance,
  preserving its dequeue, frame-count, liveness and final-release assertions.
- The isolated scene CLI/IPC/engine probe also passes with panic=abort. Its
  shared scene fixture now imports BufRead itself and calls the production
  bounded IPC handler directly; the old cfg(test)-only wrapper was unavailable
  to the standalone probe after issue30.
- Peer review found no blocker. No physical MIDI/audio hardware was exercised.
