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
  property string stateError: ""
  property string commandError: ""
  readonly property string error: commandError || stateError
  property var commandQueue: []
  property var activeCommand: null
  property var commandResults: []
  property var lastCommandResult: null
  property int nextRequestId: 1
  readonly property int queueLimit: 64
  // Keep in sync with engine::SCENES; checked by check-shell-scenes.py.
  readonly property int sceneCount: 8
  readonly property int pendingCommands: commandQueue.length + (activeCommand ? 1 : 0)
  property string commandOutput: ""
  property string commandStderr: ""
  property bool outputOverflow: false
  property bool commandTimedOut: false
  signal commandFinished(var result)

  property string bin: {
    var home = Quickshell.env("HOME") || ""
    return home + "/.local/bin/omatainer"
  }

  // Queued here is distinct from accepted by the engine. Neither proves that
  // an audio callback has applied the command; follow is the observed state.
  function newCommand(op, args) {
    return { requestId: String(nextRequestId++), operation: op,
      arguments: args ? args.slice() : [], status: "queued", applied: null }
  }

  function rejectCommand(request, message) {
    request.status = "rejected"
    request.accepted = false
    request.error = message
    commandError = request.error
    rememberResult(request)
    return JSON.stringify(request)
  }

  function send(op, args) {
    var request = newCommand(op, args)
    if (pendingCommands >= queueLimit)
      return rejectCommand(request, "Shell command queue is full")
    commandQueue = commandQueue.concat([request])
    Qt.callLater(startNextCommand)
    return JSON.stringify(request)
  }

  function startNextCommand() {
    if (activeCommand || ctl.running || commandQueue.length === 0)
      return
    activeCommand = commandQueue[0]
    commandQueue = commandQueue.slice(1)
    commandOutput = ""
    commandStderr = ""
    outputOverflow = false
    commandTimedOut = false
    ctl.command = [root.bin, "ctl", activeCommand.operation].concat(activeCommand.arguments)
    ctl.running = true
    commandDeadline.restart()
  }

  function rememberResult(result) {
    lastCommandResult = result
    commandResults = commandResults.concat([result]).slice(-64)
    commandFinished(result)
  }

  function commandResult(requestId) {
    if (activeCommand && activeCommand.requestId === requestId)
      return JSON.stringify({requestId: requestId, status: "in_flight", applied: null})
    for (var i = 0; i < commandQueue.length; ++i)
      if (commandQueue[i].requestId === requestId)
        return JSON.stringify(commandQueue[i])
    for (var j = commandResults.length - 1; j >= 0; --j)
      if (commandResults[j].requestId === requestId)
        return JSON.stringify(commandResults[j])
    return JSON.stringify({requestId: requestId, status: "unknown"})
  }

  function finishCommand(exitCode, exitStatus, startError) {
    if (!activeCommand)
      return
    commandDeadline.stop()
    var result = {
      requestId: activeCommand.requestId, operation: activeCommand.operation,
      arguments: activeCommand.arguments,
      status: "failed", exitCode: exitCode, exitStatus: exitStatus,
      accepted: null, applied: null, stderr: commandStderr
    }
    var payload = null
    try { payload = JSON.parse(commandOutput) } catch (e) {}
    if (payload && payload.accepted === false)
      result.accepted = false
    if (commandTimedOut) {
      result.error = "Control command timed out; application state is unknown"
    } else if (startError) {
      result.error = startError
    } else if (exitCode !== 0 || exitStatus !== 0) {
      result.error = commandStderr.trim() || (payload && payload.error) || "Control process failed"
    } else if (outputOverflow || !payload || payload.ok !== true) {
      result.error = (payload && payload.error) || "Invalid control response"
    } else if (payload.accepted !== true || (payload.command_status !== "accepted" && payload.command_status !== "coalesced")) {
      result.error = "Control response did not acknowledge command acceptance"
    } else {
      result.status = "accepted"
      result.accepted = true
      result.engineStatus = payload.command_status
      result.engineRequestId = payload.id
    }
    commandError = result.error || ""
    activeCommand = null
    rememberResult(result)
    // Wait until Process has finished emitting its previous lifecycle signals.
    Qt.callLater(startNextCommand)
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
  // Shell and CLI scene numbers are one-based. The CLI converts to zero-based
  // protocol n exactly once. Strings here are also used by the public IPC call.
  function scene(n) {
    var value = typeof n === "number" ? n :
      (typeof n === "string" && /^[1-9][0-9]*$/.test(n) ? Number(n) : NaN)
    if (!isFinite(value) || Math.floor(value) !== value || value < 1 || value > sceneCount)
      return rejectCommand(newCommand("scene", []), "Scene must be an integer from 1 through " + sceneCount)
    return send("scene", [String(value)])
  }

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
    stateError = payload.error ? String(payload.error) : ""
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
      error: error,
      pendingCommands: pendingCommands,
      lastCommandResult: lastCommandResult
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
    stdout: SplitParser {
      onRead: function(line) {
        if (root.commandOutput.length + line.length + 1 <= 65536)
          root.commandOutput += line + "\n"
        else
          root.outputOverflow = true
      }
    }
    stderr: SplitParser {
      onRead: function(line) {
        root.commandStderr = (root.commandStderr + line + "\n").slice(-4096)
      }
    }
    onExited: function(exitCode, exitStatus) { root.finishCommand(exitCode, exitStatus, "") }
    // FailedToStart has no exited signal in Quickshell's Process API.
    onRunningChanged: {
      if (!running && root.activeCommand) {
        var requestId = root.activeCommand.requestId
        Qt.callLater(function() {
          if (!ctl.running && root.activeCommand && root.activeCommand.requestId === requestId)
            root.finishCommand(null, null, "Could not start control process: " + root.bin)
        })
      }
    }
  }

  Timer {
    id: commandDeadline
    interval: 10000
    onTriggered: {
      root.commandTimedOut = true
      ctl.signal(9)
    }
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
    function scene(n: string): string { return root.scene(n) }
    function deckA(): string { return root.deckAPlay() }
    function deckB(): string { return root.deckBPlay() }
    function launch(): string { root.launch(); return "ok" }
    function status(): string { return root.statusJson() }
    function result(requestId: string): string { return root.commandResult(requestId) }
  }
}
