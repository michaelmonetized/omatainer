#!/usr/bin/env python3
"""Exercise the actual Service.qml in an isolated, real Quickshell process."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import socket
import threading
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--cli", type=Path, help="also exercise the production CLI through a private IPC fixture")
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="omatainer-shell-check-") as folder:
    fixture = Path(folder)
    fixture.chmod(0o700)
    shutil.copy2(root / "plugin/Service.qml", fixture / "Service.qml")
    cli = fixture / "fake-cli"
    cli.write_text('''#!/usr/bin/env python3
import json, os, signal, sys, time
op = sys.argv[2]
if op == "follow":
    print(json.dumps({"ok": True, "playing": False}), flush=True)
    time.sleep(60)
    sys.exit(0)
with open(os.environ["OMATAINER_TEST_LOG"], "a") as f:
    f.write(json.dumps({"op": op, "event": "start", "time": time.monotonic()}) + "\\n")
if op == "play": time.sleep(.25)
if op == "hang": time.sleep(30)
response = {"ok": True, "accepted": True, "command_status": "coalesced" if op == "stop" else "accepted", "id": "engine-" + op}
if op == "tap":
    print("deliberate rejection", file=sys.stderr)
    sys.exit(7)
elif op == "record": response = {"ok": False, "error": "engine rejected record"}
elif op == "deckA": print("not json", flush=True)
elif op == "deckB": pass
elif op == "cueA":
    print(json.dumps(response), flush=True)
    sys.exit(9)
elif op == "cueB": os.kill(os.getpid(), signal.SIGKILL)
if op not in ("deckA", "deckB"): print(json.dumps(response), flush=True)
with open(os.environ["OMATAINER_TEST_LOG"], "a") as f:
    f.write(json.dumps({"op": op, "event": "end", "time": time.monotonic()}) + "\\n")
''')
    cli.chmod(0o700)
    (fixture / "shell.qml").write_text('''import QtQuick
import Quickshell
import Quickshell.Io
ShellRoot {
  id: test
  property var receipts: []
  property var results: []
  property var lookupResults: []
  property bool observedInFlight: false
  property bool followKeptError: false
  property string realBin: Quickshell.env("OMATAINER_TEST_BIN")
  Service {
    id: svc
    bin: test.realBin
    onCommandFinished: function(result) {
      test.results = test.results.concat([result])
      test.lookupResults = test.lookupResults.concat([JSON.parse(svc.commandResult(result.requestId))])
      if (result.operation === "tap") {
        svc.applyState({ok: true})
        test.followKeptError = svc.error.indexOf("deliberate rejection") >= 0
      }
      if (test.results.length === 8) {
        svc.bin = test.realBin + "-missing"
        test.receipts = test.receipts.concat([JSON.parse(svc.play())])
      } else if (test.results.length === 9) {
        svc.bin = test.realBin
        test.receipts = test.receipts.concat([JSON.parse(svc.stop())])
      } else if (test.results.length === 10) {
        test.receipts = test.receipts.concat([JSON.parse(svc.send("hang")), JSON.parse(svc.stop())])
      } else if (test.results.length === 12) {
        console.log("SHELL_TEST_RESULT " + JSON.stringify({
          receipts: test.receipts, results: test.results,
          lookups: test.lookupResults, inFlight: test.observedInFlight,
          followKeptError: test.followKeptError, status: JSON.parse(svc.statusJson())
        }))
        Qt.quit()
      }
    }
  }
  Timer {
    interval: 30; running: true
    onTriggered: {
      test.observedInFlight = test.results.length === 0 &&
        JSON.parse(svc.commandResult(test.receipts[0].requestId)).status === "in_flight"
    }
  }
  Component.onCompleted: {
    var operations = ["play", "tap", "record", "deckA", "deckB", "cueA", "cueB", "stop"]
    for (var i = 0; i < operations.length; ++i)
      test.receipts = test.receipts.concat([JSON.parse(svc.send(operations[i]))])
  }
}
''')
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", XDG_RUNTIME_DIR=str(fixture),
               XDG_CACHE_HOME=str(fixture / "cache"), XDG_STATE_HOME=str(fixture / "state"),
               OMATAINER_TEST_BIN=str(cli), OMATAINER_TEST_LOG=str(fixture / "calls.jsonl"))
    try:
        run = subprocess.run(["qs", "-p", str(fixture / "shell.qml"), "--no-color"],
                         env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
    except subprocess.TimeoutExpired as e:
        raise AssertionError(e.stdout.decode() if isinstance(e.stdout, bytes) else e.stdout) from e
    marker = "SHELL_TEST_RESULT "
    lines = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    assert run.returncode == 0 and len(lines) == 1, run.stdout
    data = json.loads(lines[0])
    receipts, results = data["receipts"], data["results"]
    assert len(receipts) == len(results) == 12, data
    assert all(item["status"] == "queued" and item["applied"] is None for item in receipts), data
    assert [x["requestId"] for x in receipts] == [x["requestId"] for x in results], data
    assert data["lookups"] == results, data
    assert data["inFlight"] and data["followKeptError"], data
    assert [x["status"] for x in results] == ["accepted"] + ["failed"] * 6 + ["accepted", "failed", "accepted", "failed", "accepted"], data
    assert all(x["applied"] is None for x in results), data
    assert results[0]["engineStatus"] == "accepted" and results[7]["engineStatus"] == "coalesced", data
    assert results[1]["exitCode"] == 7 and "deliberate rejection" in results[1]["error"], data
    assert results[2]["error"] == "engine rejected record", data
    assert results[5]["exitCode"] == 9 and results[6]["exitStatus"] != 0, data
    assert results[8]["exitCode"] is None and "Could not start" in results[8]["error"], data
    assert "timed out" in results[10]["error"] and results[10]["accepted"] is None, data
    assert data["status"]["pendingCommands"] == 0 and data["status"]["error"] == "", data
    calls = [json.loads(line) for line in (fixture / "calls.jsonl").read_text().splitlines()]
    assert [x["op"] for x in calls if x["event"] == "start"] == ["play", "tap", "record", "deckA", "deckB", "cueA", "cueB", "stop", "stop", "hang", "stop"], calls
    assert calls[1]["event"] == "end" and calls[1]["op"] == "play", calls
    print("PASS: real Quickshell serialized commands, correlated results, queued vs accepted, failure/exit reporting, persistent errors, failed-start recovery, timeout recovery, and every stop command")

    # Fill the real service queue before the event loop can launch a command.
    # Every admitted command runs, and the extra request is explicitly rejected.
    (fixture / "queue.qml").write_text('''import QtQuick
import Quickshell
ShellRoot {
  id: test
  property var receipts: []
  property var results: []
  Service {
    id: svc
    bin: Quickshell.env("OMATAINER_TEST_BIN")
    onCommandFinished: function(result) {
      test.results = test.results.concat([result])
      if (test.results.length === 65) {
        console.log("SHELL_QUEUE_RESULT " + JSON.stringify({receipts: test.receipts, results: test.results, pending: svc.pendingCommands}))
        Qt.quit()
      }
    }
  }
  Component.onCompleted: {
    for (var i = 0; i < 65; ++i)
      test.receipts = test.receipts.concat([JSON.parse(svc.stop())])
  }
}
''')
    try:
        run = subprocess.run(["qs", "-p", str(fixture / "queue.qml"), "--no-color"],
                             env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
    except subprocess.TimeoutExpired as e:
        raise AssertionError(e.stdout) from e
    marker = "SHELL_QUEUE_RESULT "
    lines = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    assert run.returncode == 0 and len(lines) == 1, run.stdout
    data = json.loads(lines[0])
    assert [x["status"] for x in data["receipts"]] == ["queued"] * 64 + ["rejected"], data
    assert [x["status"] for x in data["results"]] == ["rejected"] + ["accepted"] * 64, data
    assert [x["requestId"] for x in data["results"][1:]] == [str(i) for i in range(1, 65)], data
    assert data["results"][0]["requestId"] == "65" and data["pending"] == 0, data
    print("PASS: real Quickshell queue saturation explicitly rejects overflow and completes all 64 admitted stop commands in order")

    if args.cli:
        native_cli = args.cli.resolve(strict=True)
        endpoint = fixture / "omatainer.sock"
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(endpoint))
        listener.listen(8)
        listener.settimeout(.1)
        stop = threading.Event()
        requests = []
        server_errors = []

        def serve():
            while not stop.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    break
                try:
                    with connection:
                        connection.settimeout(1)
                        with connection.makefile("rb") as stream:
                            request = json.loads(stream.readline(4096))
                        requests.append(request)
                        op = request["op"]
                        status = None if op in ("status", "follow") else ("coalesced" if op == "stop" else "accepted")
                        response = {"ok": True, "id": request["id"], "accepted": True if status else None,
                                    "command_status": status, "playing": False, "event": "state"}
                        if op == "follow":
                            response["follow"] = True
                        connection.sendall((json.dumps(response) + "\n").encode())
                except Exception as error:
                    server_errors.append(str(error))

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        (fixture / "native.qml").write_text('''import QtQuick
import Quickshell
ShellRoot {
  id: test
  property var receipts: []
  property var results: []
  Service {
    id: svc
    bin: Quickshell.env("OMATAINER_NATIVE_CLI")
    onCommandFinished: function(result) {
      test.results = test.results.concat([result])
      if (test.results.length === 2) {
        console.log("SHELL_NATIVE_RESULT " + JSON.stringify({receipts: test.receipts, results: test.results}))
        Qt.quit()
      }
    }
  }
  Component.onCompleted: test.receipts = [JSON.parse(svc.play()), JSON.parse(svc.stop())]
}
''')
        try:
            run = subprocess.run(["qs", "-p", str(fixture / "native.qml"), "--no-color"],
                                 env=dict(env, OMATAINER_NATIVE_CLI=str(native_cli)), text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
        finally:
            stop.set()
            listener.close()
            thread.join(timeout=2)
        assert not thread.is_alive() and not server_errors, server_errors
        marker = "SHELL_NATIVE_RESULT "
        lines = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
        assert run.returncode == 0 and len(lines) == 1, run.stdout
        data = json.loads(lines[0])
        assert [x["status"] for x in data["receipts"]] == ["queued", "queued"], data
        assert [x["status"] for x in data["results"]] == ["accepted", "accepted"], data
        assert [x["engineStatus"] for x in data["results"]] == ["accepted", "coalesced"], data
        assert [x["op"] for x in requests if x["op"] not in ("status", "follow")] == ["play", "stop"], requests
        assert all(x["engineRequestId"] and x["applied"] is None for x in data["results"]), data
        print("PASS: real Service -> native CLI -> private IPC bridge, with actual accepted/coalesced labels and request IDs")
