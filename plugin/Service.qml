import QtQuick
import Quickshell
import Quickshell.Io

Item {
  id: root

  property var shell: null
  property var manifest: null

  property bool running: false
  property bool playing: false
  property bool recording: false
  property real bpm: 124
  property int bar: 1
  property real beat: 0
  property string deckA: ""
  property string deckB: ""
  property bool deckAPlaying: false
  property bool deckBPlaying: false
  property var midi: []
  property string error: ""

  readonly property string bin: {
    var home = Quickshell.env("HOME") || ""
    return home + "/.local/bin/omatainer"
  }

  function send(op) {
    ctl.command = [root.bin, "ctl", op]
    ctl.running = false
    ctl.running = true
    return "ok"
  }

  function togglePlay() { return send("togglePlay") }
  function play() { return send("play") }
  function stop() { return send("stop") }
  function record() { return send("record") }
  function tap() { return send("tap") }
  function deckAPlay() { return send("deckA") }
  function deckBPlay() { return send("deckB") }
  function cueA() { return send("cueA") }
  function cueB() { return send("cueB") }
  function scene(n) { return send("scene") }

  function applyState(payload) {
    running = payload.ok === true
    playing = payload.playing === true
    recording = payload.recording === true
    bpm = payload.bpm !== undefined ? Number(payload.bpm) : root.bpm
    bar = payload.bar !== undefined ? Number(payload.bar) : root.bar
    beat = payload.beat !== undefined ? Number(payload.beat) : root.beat
    deckA = payload.deckA ? String(payload.deckA) : ""
    deckB = payload.deckB ? String(payload.deckB) : ""
    deckAPlaying = payload.deckAPlaying === true
    deckBPlaying = payload.deckBPlaying === true
    midi = Array.isArray(payload.midi) ? payload.midi : []
    error = payload.error ? String(payload.error) : ""
  }

  function statusJson() {
    return JSON.stringify({
      running: running,
      playing: playing,
      recording: recording,
      bpm: bpm,
      bar: bar,
      beat: beat,
      deckA: deckA,
      deckB: deckB,
      error: error
    })
  }

  function launch() {
    Quickshell.execDetached(["omarchy-launch-or-focus", "org.omarchy.omatainer", "uwsm-app -- omatainer"])
  }

  Process {
    id: follow
    running: true
    command: [root.bin, "ctl", "follow"]
    stdout: SplitParser {
      onRead: function(line) {
        try {
          var payload = JSON.parse(line)
        } catch (e) {
          return
        }
        if (payload)
          root.applyState(payload)
      }
    }
    stderr: SplitParser {
      onRead: function(line) {
        console.warn("omatainer:", line)
      }
    }
    onExited: restartDelay.restart()
  }

  Process {
    id: ctl
    running: false
    command: [root.bin, "ctl", "status"]
  }

  Timer {
    id: restartDelay
    interval: 1500
    repeat: false
    onTriggered: {
      if (!follow.running)
        follow.running = true
    }
  }

  IpcHandler {
    target: "omatainer"

    function togglePlay(): string { return root.togglePlay() }
    function play(): string { return root.play() }
    function stop(): string { return root.stop() }
    function record(): string { return root.record() }
    function tap(): string { return root.tap() }
    function deckA(): string { return root.deckAPlay() }
    function deckB(): string { return root.deckBPlay() }
    function launch(): string { root.launch(); return "ok" }
    function status(): string { return root.statusJson() }
  }
}
