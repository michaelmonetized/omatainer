#!/usr/bin/env python3
"""Real shipped CLI safe-start diagnostics; no native window or device QA claim."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import resource
import subprocess
import tempfile


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    options = parser.parse_args()
    binary = str(options.binary.resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix='omatainer-safe-startup-') as folder:
        root = Path(folder)
        for name in ('runtime', 'config', 'state', 'data', 'bin'):
            (root / name).mkdir(mode=0o700)
        environment = dict(os.environ, XDG_RUNTIME_DIR=str(root / 'runtime'),
                           XDG_CONFIG_HOME=str(root / 'config'),
                           XDG_STATE_HOME=str(root / 'state'),
                           XDG_DATA_HOME=str(root / 'data'))
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS'):
            environment.pop(key, None)
        focus = root / 'focus-was-called'
        helper = root / 'bin' / 'hyprctl'
        helper.write_text('#!/bin/sh\ntouch "' + str(focus) + '"\n')
        helper.chmod(0o700)
        environment['PATH'] = str(root / 'bin') + ':' + environment.get('PATH', '')

        def run(*args, ok=True):
            result = subprocess.run([binary, *args], env=environment,
                                    stdin=subprocess.DEVNULL, capture_output=True,
                                    text=True, timeout=10, preexec_fn=no_core,
                                    close_fds=True)
            assert (result.returncode == 0) == ok, (result.returncode, result.stderr[:1000])
            return json.loads(result.stdout) if ok else result

        result = run('--safe-mode', '--startup-check')
        assert result['scope'] == 'headless_safe_startup_not_native_gui_or_device_qa'
        assert result['safe_mode'] and result['project_capture_available'] and result['transport_stopped']
        assert result['backend_callbacks'] == 0
        assert not result['audio_output_open'] and not result['midi_manager_available']
        assert not result['audio_plugin_host_available'] and not result['upload_client_available']
        assert result['support_marker_clean']
        assert not list((root / 'config').rglob('preferences.json'))
        assert not list((root / 'data').rglob('*'))
        preferences = root / 'config' / 'omatainer' / 'preferences.json'
        preferences.parent.mkdir(mode=0o700, exist_ok=True)
        original = b'{"unknown_newer_version":"private fixture"'
        preferences.write_bytes(original)
        result = run('--safe-mode', '--startup-check')
        assert not result['preferences_readable'] and result['support_marker_clean']
        assert preferences.read_bytes() == original
        for args in (('--startup-check',), ('--safe-mode', '--defaults-once'),
                     ('--safe-mode', '--safe-mode'), ('--unknown',)):
            run(*args, ok=False)
        lock_path = root / 'runtime' / 'omatainer.lock'
        with lock_path.open('r+b') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for args in (('--safe-mode', '--startup-check'), ('--safe-mode',)):
                result = run(*args, ok=False)
                assert 'already owns this runtime' in result.stderr
            assert not focus.exists(), 'safe startup must not focus an existing normal GUI'
        support = root / 'state' / 'omatainer' / 'support'
        support.chmod(0o500)
        try:
            result = run('--safe-mode', '--startup-check')
            assert not result['support_marker_clean'], 'read-only support storage must not claim durability'
        finally:
            support.chmod(0o700)
        assert preferences.read_bytes() == original
        assert not focus.exists()
    print('PASS: real headless safe startup, real offline project service, malformed preferences preservation, strict arguments, existing-instance refusal, read-only diagnostics; no native GUI/device QA')


if __name__ == '__main__':
    main()
