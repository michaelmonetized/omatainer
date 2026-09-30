#!/usr/bin/env python3
"""Check the built CLI against private Unix-socket protocol fixtures."""

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading


def check(binary: Path, case: str) -> None:
    with tempfile.TemporaryDirectory(prefix="omatainer-ipc-cli-") as directory:
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(Path(directory) / "omatainer.sock"))
        listener.listen(1)
        listener.settimeout(5)
        failures = []

        def serve():
            try:
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(3)
                    request = json.loads(connection.makefile("rb").readline())
                    assert request["op"] == "play"
                    assert isinstance(request["id"], str)
                    if case == "eof":
                        return
                    replies = {
                        "accepted": {"id": request["id"], "ok": True, "accepted": True,
                                     "command_status": "accepted"},
                        "rejected": {"id": request["id"], "ok": False, "error": "queue full"},
                        "missing_ok": {"id": request["id"]},
                        "wrong_type": {"id": request["id"], "ok": "true"},
                        "wrong_id": {"id": "another-request", "ok": True},
                    }
                    payload = json.dumps(replies[case]) if case in replies else "{partial"
                    connection.sendall(payload.encode() + b"\n")
            except Exception as error:
                failures.append(error)

        worker = threading.Thread(target=serve)
        worker.start()
        environment = dict(os.environ, XDG_RUNTIME_DIR=directory)
        try:
            result = subprocess.run(
                [binary, "ctl", "play"], env=environment,
                text=True, capture_output=True, timeout=5,
            )
        finally:
            worker.join(timeout=6)
            listener.close()
        assert not worker.is_alive(), case
        assert not failures, (case, failures)
        if case == "accepted":
            assert result.returncode == 0, result.stderr
            assert json.loads(result.stdout)["accepted"] is True
        else:
            assert result.returncode != 0, (case, result)
            assert not result.stdout.strip(), (case, result.stdout)
            assert result.stderr.strip(), case
        print(f"{case}: exit {result.returncode}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    binary = parser.parse_args().binary.resolve(strict=True)
    for case in ["accepted", "rejected", "missing_ok", "wrong_type", "wrong_id", "malformed", "eof"]:
        check(binary, case)
    print("All CLI protocol checks passed; no application socket or audio device was used.")


if __name__ == "__main__":
    main()
