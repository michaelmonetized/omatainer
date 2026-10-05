#!/usr/bin/env python3
"""Capture every native output through an owned 32-channel PipeWire loopback."""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time

spec = importlib.util.spec_from_file_location('recovery_fixture', Path(__file__).with_name('check-audio-recovery.py'))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
TEST = 'engine::audio::routing::native_tests::private_native_32_channel_loopback_captures_each_physical_output'
POSITIONS = '[ ' + ' '.join(f'AUX{channel}' for channel in range(32)) + ' ]'


def connect(env, configured, links):
    """Connect owned stream ports by their exact channel identities.
    Takes the private environment and retained link sets; returns the current linked channel count.
    """
    nodes = json.loads(fixture.command(['pw-dump', '-r', 'omatainer-test'], env).stdout)
    names = {item['id']: item.get('info', {}).get('props', {}).get('node.name', '')
             for item in nodes if item['type'].endswith(':Node')}
    for node, name in names.items():
        if node not in configured and name.startswith(('alsa_playback.', 'alsa_capture.')):
            direction = 'Output' if name.startswith('alsa_playback.') else 'Input'
            ports = f'{{ direction = {direction} mode = dsp monitor = false format = {{ mediaType = audio mediaSubtype = raw format = F32P rate = 48000 channels = 32 position = {POSITIONS} }} }}'
            fixture.command(['pw-cli', '-r', 'omatainer-test', 'set-param', str(node), 'PortConfig', ports], env)
            configured.add(node)
    ports = [(item['id'], item.get('info', {}).get('props', {})) for item in nodes if item['type'].endswith(':Port')]
    for output, source in ports:
        if source.get('port.direction') != 'out':
            continue
        for destination, target in ports:
            if target.get('port.direction') != 'in' or source.get('audio.channel') != target.get('audio.channel'):
                continue
            from_name, to_name = names.get(source.get('node.id'), ''), names.get(target.get('node.id'), '')
            route = from_name.startswith('alsa_playback.') and to_name == 'test-sink'
            route |= from_name == 'test-sink' and to_name.startswith('alsa_capture.')
            if route and (output, destination) not in links:
                fixture.command(['pw-link', '-r', 'omatainer-test', str(output), str(destination)], env)
                links.add((output, destination))
    return len(links)


def run(binary, destination, buffer_frames=128):
    """Qualify a native routing test executable.
    Takes a test binary, new private directory and requested buffer; returns captured channel receipts and owned-child cleanup proof.
    """
    destination.mkdir()
    root = destination.resolve()
    runtime = root / 'runtime'
    if len(os.fsencode(runtime / 'omatainer-test-manager')) >= 108:
        raise ValueError('Use a shorter fixture directory for native Unix socket paths')
    runtime.mkdir(mode=0o700)
    if buffer_frames not in (128, 256, 512, 1024, 2048):
        raise ValueError('Choose a supported private fixture buffer')
    server_config = fixture.SERVER.replace('[ FL FR ]', POSITIONS).replace('monitor = false', 'monitor = true').replace('default.clock.quantum = 128', f'default.clock.quantum = {buffer_frames}')
    (root / 'pipewire.conf').write_text(server_config)
    alsa_config = fixture.ALSA.replace('channels 2', 'channels 32').replace('playback_node "test-sink"', 'playback_node "test-sink" capture_node "test-sink"')
    (root / 'alsa.conf').write_text(alsa_config)
    env = {**os.environ, 'XDG_RUNTIME_DIR': str(runtime), 'PIPEWIRE_RUNTIME_DIR': str(runtime),
           'PIPEWIRE_REMOTE': 'omatainer-test', 'PIPEWIRE_DEBUG': '1', 'ALSA_CONFIG_PATH': str(root / 'alsa.conf'),
           'OMATAINER_NATIVE_ROUTING_DIR': str(root), 'OMATAINER_NATIVE_ROUTING_FRAMES': str(buffer_frames), 'PIPEWIRE_CONFIG_DIR': '/usr/share/pipewire', 'XDG_CONFIG_HOME': str(root / 'config')}
    server, child = None, None
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    try:
        with (root / 'server.log').open('wb') as log:
            server = subprocess.Popen(['pipewire', '-c', str(root / 'pipewire.conf')], env={**env, 'PIPEWIRE_CONFIG_DIR': str(root)}, stdout=log, stderr=log)
        deadline = time.monotonic() + 5
        while not (runtime / 'omatainer-test').exists():
            if server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Private PipeWire server did not start')
            time.sleep(.02)
        with (root / 'native.log').open('wb') as log:
            child = subprocess.Popen([str(binary.resolve()), TEST, '--ignored', '--exact', '--test-threads=1', '--nocapture'], env=env, stdout=log, stderr=log)
        configured, links = set(), set()
        deadline = time.monotonic() + 90
        while child.poll() is None:
            if time.monotonic() > deadline:
                raise RuntimeError('Native routing test exceeded 90 seconds')
            count = connect(env, configured, links)
            if count == 64 and (root / 'streams.ready').exists():
                (root / 'links.ready').write_text('32 playback and 32 capture links established in the private server\n')
            time.sleep(.05)
        if child.returncode or not (root / 'done.json').exists():
            raise RuntimeError('Native routing test failed; inspect native.log')
        receipt = {'status': 'pass', 'started': started, 'finished': dt.datetime.now(dt.timezone.utc).isoformat(),
                   'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                   'pipewire': fixture.command(['pipewire', '--version'], env).stdout.decode().strip(),
                   'server_pid': server.pid, 'test_pid': child.pid, 'links': len(links),
                   'requested_buffer_frames': buffer_frames,
                   'capture': json.loads((root / 'done.json').read_text()),
                   'scope': 'Actual CPAL/ALSA output and input, private 32-channel PipeWire null-sink loopback, renderer record aliases and decoded WAVs. No physical converter or external interface.'}
    finally:
        fixture.stop(child)
        fixture.stop(server)
    receipt['owned_children_exited'] = True
    (root / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--buffer-frames', type=int, choices=[128, 256, 512, 1024, 2048], default=128)
    args = parser.parse_args()
    print(json.dumps(run(args.test_binary, args.destination, args.buffer_frames), indent=2))
