#!/usr/bin/env python3
"""Exercise production runtime discovery in child CLIs with private fixtures."""

import argparse
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import threading


def invoke(binary, environment):
    # Deliberately permissive child umask must not widen the created directory.
    return subprocess.run(
        [binary, "ctl", "status"], env=environment, umask=0,
        text=True, capture_output=True, timeout=5,
    )


def rejected(binary, environment):
    result = invoke(binary, environment)
    assert result.returncode != 0, result
    assert not result.stdout.strip(), result
    assert result.stderr.strip(), result
    return result


def round_trip(binary, environment, endpoint):
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(endpoint))
    listener.listen(1)
    listener.settimeout(5)
    errors = []

    def serve():
        try:
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(3)
                request = json.loads(connection.makefile("rb").readline())
                assert request["op"] == "status", request
                connection.sendall(json.dumps({
                    "id": request["id"], "ok": True, "marker": "isolated",
                }).encode() + b"\n")
        except Exception as error:
            errors.append(error)

    worker = threading.Thread(target=serve)
    worker.start()
    identity = endpoint.stat().st_ino
    try:
        result = invoke(binary, environment)
    finally:
        worker.join(timeout=6)
        listener.close()
    assert not worker.is_alive()
    assert not errors, errors
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["marker"] == "isolated", result
    assert endpoint.stat().st_ino == identity


def check(binary):
    with tempfile.TemporaryDirectory(prefix="omatainer-runtime-cli-") as temporary:
        root = Path(temporary)
        fake_uid = str(os.geteuid() + 1)
        environment = dict(os.environ, TMPDIR=temporary, UID=fake_uid, USER="other-user")
        environment.pop("XDG_RUNTIME_DIR", None)
        shared = root / "omatainer.sock"
        shared.write_text("unrelated shared occupant")
        fallback = root / f"omatainer-{os.geteuid()}"
        endpoint = fallback / "omatainer.sock"
        result = rejected(binary, environment)
        assert "XDG_RUNTIME_DIR is unset" in result.stderr, result.stderr
        assert fallback.stat().st_uid == os.geteuid()
        assert stat.S_IMODE(fallback.stat().st_mode) == 0o700
        assert not (root / f"omatainer-{fake_uid}").exists()
        assert shared.read_text() == "unrelated shared occupant"
        round_trip(binary, environment, endpoint)
        assert shared.read_text() == "unrelated shared occupant"
        print("unset XDG: private EUID directory, 0700 despite umask 000, real CLI round trip")

        runtime = root / "valid"
        runtime.mkdir(mode=0o700)
        xdg_env = dict(environment, XDG_RUNTIME_DIR=str(runtime))
        round_trip(binary, xdg_env, runtime / "omatainer.sock")
        print("valid XDG: real CLI round trip through the configured endpoint")

        endpoint.unlink()  # Remove only this fixture's own closed listener.
        endpoint.write_text("regular collision")
        rejected(binary, environment)
        assert endpoint.read_text() == "regular collision"
        endpoint.unlink()
        endpoint.symlink_to(shared)
        rejected(binary, environment)
        assert endpoint.is_symlink()
        assert shared.read_text() == "unrelated shared occupant"
        print("endpoint collisions: regular file and symlink rejected without mutation")

        bad = root / "bad"
        bad.mkdir(mode=0o700)
        bad.chmod(0o755)
        result = rejected(binary, dict(environment, XDG_RUNTIME_DIR=str(bad)))
        assert str(bad) in result.stderr, result.stderr
        assert stat.S_IMODE(bad.stat().st_mode) == 0o755
        assert not (bad / "omatainer.sock").exists()
        alias = root / "alias"
        alias.symlink_to(runtime, target_is_directory=True)
        # Keep '/.' in the actual environment string; pathlib normalizes it.
        result = rejected(binary, dict(environment, XDG_RUNTIME_DIR=f"{alias}/."))
        assert str(alias) in result.stderr, result.stderr
        assert alias.is_symlink()
        rejected(binary, dict(environment, XDG_RUNTIME_DIR="relative"))
        print("invalid XDG: unsafe permissions, symlink-dot alias and relative path fail closed")

        hostile = root / "hostile"
        hostile.mkdir(mode=0o700)
        hostile.chmod(0o777)
        rejected(binary, dict(environment, TMPDIR=str(hostile)))
        assert not list(hostile.iterdir())
        print("unsafe temp parent: no runtime directory or endpoint created")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    check(parser.parse_args().binary.resolve(strict=True))
    print("All runtime checks passed; no application socket or audio device was used.")


if __name__ == "__main__":
    main()
