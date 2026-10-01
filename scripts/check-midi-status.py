#!/usr/bin/env python3
"""Compare synthetic native GUI/IPC MIDI state with real Service.qml state."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="omatainer-midi-status-") as directory:
    work = Path(directory)
    work.chmod(0o700)
    evidence = work / "native-status.json"
    env = dict(os.environ, OMATAINER_MIDI_STATUS_EVIDENCE=str(evidence))
    subprocess.run([
        "cargo", "test", "--offline",
        "shared_midi_status_matches_gui_protocol_and_survives_audio_publication",
    ], cwd=root, env=env, check=True)
    native = json.loads(evidence.read_text())
    assert len(native) == 4
    shutil.copy2(root / "plugin/Service.qml", work / "Service.qml")
    # This fixture exercises the real state's consumer, without launching the
    # installed application's follower or touching its live socket.
    fake_cli = work / "private-cli"
    fake_cli.write_text("#!/bin/sh\nexit 0\n")
    fake_cli.chmod(0o700)
    (work / "shell.qml").write_text('''import QtQuick
import Quickshell
ShellRoot {
  id: test
  Service { id: svc; bin: Quickshell.env("OMATAINER_TEST_BIN") }
  Timer { interval: 20; running: true; repeat: false; onTriggered: {
    var input = JSON.parse(Quickshell.env("OMATAINER_NATIVE_STATUS"))
    var result = []
    for (var i = 0; i < input.length; ++i) {
      svc.applyState(input[i])
      result.push(svc.midi.slice())
    }
    console.log("MIDI_STATE_RESULT " + JSON.stringify(result))
    Qt.quit()
  } }
}
''')
    run = subprocess.run([
        "qs", "-p", str(work / "shell.qml"), "--no-color",
    ], env=dict(os.environ, QT_QPA_PLATFORM="offscreen", XDG_RUNTIME_DIR=str(work),
                OMATAINER_TEST_BIN=str(fake_cli), OMATAINER_NATIVE_STATUS=json.dumps(native)),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
    marker = "MIDI_STATE_RESULT "
    lines = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    assert run.returncode == 0 and len(lines) == 1, run.stdout
    assert json.loads(lines[0]) == [frame["midi"] for frame in native]
    print("PASS: native GUI/shared snapshot/private IPC and real Service.qml agree on all four MIDI states")
