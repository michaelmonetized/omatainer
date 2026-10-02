#!/usr/bin/env python3
"""Real Service.qml scene calls, isolated argv sink, and optional native IPC peer.

No desktop shell service, audio device or existing application socket is used.
The optional native mode is driven by shell_scene_tests' private actual server.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def run_case(fixture, binary, runtime, name):
    config = fixture / (name + ".qml")
    config.write_text('''import QtQuick
import Quickshell
import Quickshell.Io
ShellRoot {
  id: test
  property var receipts: []
  property var results: []
  Service {
    id: svc
    bin: Quickshell.env("OMATAINER_SCENE_CLI")
    onCommandFinished: function(result) { test.results = test.results.concat([result]) }
  }
  IpcHandler {
    target: "scene-fixture"
    function evidence(): string {
      return JSON.stringify({receipts: test.receipts, results: test.results, pending: svc.pendingCommands})
    }
    function exit(): string { Qt.callLater(Qt.quit); return "bye" }
  }
  Component.onCompleted: {
    test.receipts = [JSON.parse(svc.scene(1)), JSON.parse(svc.scene(4)), JSON.parse(svc.scene(8))]
    var invalid = [0, 513, -1, 1.5, NaN, Infinity, true, false, null, undefined,
                   "", "1.0", "1e0", "01", "2; stop", [], {}, "99999999999999999999999999"]
    for (var i = 0; i < invalid.length; ++i)
      test.receipts = test.receipts.concat([JSON.parse(svc.scene(invalid[i]))])
  }
}
''')
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", XDG_RUNTIME_DIR=str(runtime),
               XDG_CACHE_HOME=str(fixture / "cache"), XDG_STATE_HOME=str(fixture / "state"),
               OMATAINER_SCENE_CLI=str(binary), OMATAINER_SCENE_LOG=str(fixture / "calls.jsonl"))
    logs = fixture / (name + ".log")
    with logs.open("w+") as log:
        shell = subprocess.Popen(["qs", "-p", str(config), "--no-color"], env=env,
                                 stdout=log, stderr=subprocess.STDOUT)

        def ipc(target, method, *args, ready=False):
            result = subprocess.run(["qs", "ipc", "-p", str(config), "call", target, method, *args],
                                    env=env, text=True, capture_output=True, timeout=3)
            if result.returncode != 0:
                if ready:
                    return None
                raise AssertionError((result.stdout, result.stderr, logs.read_text()))
            return json.loads(result.stdout) if method != "exit" else result.stdout

        def evidence():
            return ipc("scene-fixture", "evidence", ready=True)

        try:
            deadline = time.monotonic() + 8
            while True:
                data = evidence()
                if data and data["pending"] == 0 and len(data["results"]) == 21:
                    break
                assert shell.poll() is None and time.monotonic() < deadline, logs.read_text()
                time.sleep(.02)
            assert [x["status"] for x in data["receipts"]] == ["queued"] * 3 + ["rejected"] * 18, data
            invalid = [x for x in data["results"] if x["status"] == "rejected"]
            assert len(invalid) == 18 and all(x["accepted"] is False for x in invalid), data
            assert all("integer from 1 through 512" in x["error"] for x in invalid), data
            accepted = [x for x in data["results"] if x["status"] == "accepted"]
            assert [x["arguments"] for x in accepted] == [["1"], ["4"], ["8"]], data
            assert all(x["applied"] is None for x in accepted), data

            # Exercise the public IpcHandler rather than only a local QML call.
            public = ipc("omatainer", "scene", "2")
            assert public["status"] == "queued" and public["arguments"] == ["2"], public
            for value in ["0", "513", "-1", "1.5", "1e0", "true", "", "2; stop"]:
                rejected = ipc("omatainer", "scene", value)
                assert rejected["status"] == "rejected" and rejected["accepted"] is False, rejected
            deadline = time.monotonic() + 5
            while True:
                data = evidence()
                accepted = [x for x in data["results"] if x["status"] == "accepted"]
                if data["pending"] == 0 and len(accepted) == 4:
                    break
                assert time.monotonic() < deadline, (data, logs.read_text())
                time.sleep(.02)
            assert [x["arguments"] for x in accepted] == [["1"], ["4"], ["8"], ["2"]], data
            assert len({x["requestId"] for x in data["results"]}) == len(data["results"]), data
            assert all(x["operation"] == "scene" and x["engineRequestId"] for x in accepted), data
            for result in accepted:
                assert ipc("omatainer", "result", result["requestId"]) == result
            ipc("scene-fixture", "exit")
            assert shell.wait(timeout=3) == 0, logs.read_text()
        finally:
            if shell.poll() is None:
                shell.terminate()
                try:
                    shell.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    shell.kill()
                    shell.wait(timeout=3)
    print(f"PASS: {name}: real Service scene1/4/8/2, 26 explicit invalid rejections, public IPC and distinct captured arguments")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--runtime", type=Path)
    args = parser.parse_args()
    assert bool(args.cli) == bool(args.runtime), "--cli and --runtime must be used together"
    engine_count = re.search(r"pub const MAX_SCENES: usize = (\d+);", (ROOT / "src/engine/session.rs").read_text())
    shell_count = re.search(r"readonly property int sceneCount: (\d+)", (ROOT / "plugin/Service.qml").read_text())
    assert engine_count and shell_count and engine_count[1] == shell_count[1] == "512"
    with tempfile.TemporaryDirectory(prefix="omatainer-shell-scenes-") as directory:
        fixture = Path(directory)
        fixture.chmod(0o700)
        shutil.copy2(ROOT / "plugin/Service.qml", fixture / "Service.qml")
        mock = fixture / "argv-sink"
        mock.write_text('''#!/usr/bin/env python3
import json, os, sys, time
if sys.argv[1:] == ["ctl", "follow"]:
    print(json.dumps({"ok":True,"playing":False}), flush=True)
    time.sleep(60)
else:
    with open(os.environ["OMATAINER_SCENE_LOG"], "a") as log:
        log.write(json.dumps(sys.argv[1:]) + "\\n")
    assert len(sys.argv) == 4 and sys.argv[1:3] == ["ctl", "scene"]
    print(json.dumps({"ok":True,"accepted":True,"command_status":"accepted","id":"scene-"+sys.argv[3]}), flush=True)
''')
        mock.chmod(0o700)
        run_case(fixture, mock, fixture, "argv-sink")
        calls = [json.loads(line) for line in (fixture / "calls.jsonl").read_text().splitlines()]
        assert calls == [["ctl", "scene", n] for n in ["1", "4", "8", "2"]], calls
        print("PASS: process argv is distinct and exact; rejected scenes never start the command sink")
        if args.cli:
            run_case(fixture, args.cli.resolve(strict=True), args.runtime.resolve(strict=True), "native-protocol")


if __name__ == "__main__":
    main()
