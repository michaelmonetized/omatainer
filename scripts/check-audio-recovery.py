#!/usr/bin/env python3
"""Exercise real CPAL/ALSA recovery using an isolated PipeWire null sink."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

TEST = 'engine::audio::owner::recovery_tests::private_native_pipewire_restart_retains_recorded_document_and_requires_explicit_fallback'
SERVER = '''context.properties = { core.daemon = true core.name = "omatainer-test" support.dbus = false default.clock.rate = 48000 default.clock.quantum = 128 }
context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }
context.modules = [
 { name = libpipewire-module-protocol-native }
 { name = libpipewire-module-metadata }
 { name = libpipewire-module-spa-node-factory }
 { name = libpipewire-module-client-node }
 { name = libpipewire-module-adapter }
 { name = libpipewire-module-link-factory }
 { name = libpipewire-module-access args = { access.legacy = true } }
]
context.objects = [
 { factory = spa-node-factory args = { factory.name = support.node.driver node.name = Test-Driver node.group = pipewire.dummy priority.driver = 200000 } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test-sink media.class = Audio/Sink audio.position = [ FL FR ] node.always-process = true adapter.auto-port-config = { mode = dsp monitor = false position = preserve } } }
]
'''
ALSA = '''ctl.hw { @args [ CARD ] @args.CARD { type string default "0" } type hw card $CARD }
defaults.namehint.showall off
defaults.namehint.basic on
defaults.namehint.extended off
pcm_type.pipewire { lib "/usr/lib/alsa-lib/libasound_module_pcm_pipewire.so" }
pcm.!default { type pipewire server "omatainer-test" playback_node "test-sink" rate 48000 channels 2 hint { show on description "Private recovery fixture" ioid "Output" } }
'''
PORTS = '{ direction = Output mode = dsp monitor = false format = { mediaType = audio mediaSubtype = raw format = F32P rate = 48000 channels = 2 position = [ FL FR ] } }'


def stop(process):
    """Retire an owned child.
    Takes its process; returns after termination, escalating only that PID if needed.
    """
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=3)


def command(args, env):
    """Run a private-server command.
    Takes argv and isolated environment; returns bounded output or raises on failure.
    """
    return subprocess.run(args, env=env, capture_output=True, timeout=3, check=True)


def connect(env, configured, links):
    """Connect only fixture playback ports.
    Takes the private environment and observed IDs; returns after configuring new ports.
    """
    nodes = json.loads(command(['pw-dump', '-r', 'omatainer-test'], env).stdout)
    names = {item['id']: item.get('info', {}).get('props', {}).get('node.name')
             for item in nodes if item['type'].endswith(':Node')}
    for node, name in names.items():
        if name and name.startswith('alsa_playback.') and node not in configured:
            command(['pw-cli', '-r', 'omatainer-test', 'set-param', str(node), 'PortConfig', PORTS], env)
            configured.add(node)
    ports = [(item['id'], item.get('info', {}).get('props', {}))
             for item in nodes if item['type'].endswith(':Port')]
    for output, props in ports:
        if props.get('port.direction') != 'out' or not names.get(props.get('node.id'), '').startswith('alsa_playback.'):
            continue
        for destination, target in ports:
            if target.get('port.direction') == 'in' and names.get(target.get('node.id')) == 'test-sink' and target.get('audio.channel') == props.get('audio.channel'):
                pair = (output, destination)
                if pair not in links:
                    command(['pw-link', '-r', 'omatainer-test', str(output), str(destination)], env)
                    links.add(pair)


def run(binary, destination):
    """Qualify a native test executable.
    Takes its path and a new evidence directory; returns a verified restart receipt.
    """
    destination.mkdir()
    root = destination.resolve()
    runtime = root / 'runtime'
    runtime.mkdir(mode=0o700)
    (root / 'pipewire.conf').write_text(SERVER)
    (root / 'alsa.conf').write_text(ALSA)
    env = os.environ.copy()
    env.update(XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
               PIPEWIRE_REMOTE='omatainer-test', PIPEWIRE_DEBUG='1',
               ALSA_CONFIG_PATH=str(root / 'alsa.conf'), OMATAINER_NATIVE_RECOVERY_DIR=str(root),
               PIPEWIRE_CONFIG_DIR='/usr/share/pipewire', XDG_CONFIG_HOME=str(root / 'config'))
    server_env = {**env, 'PIPEWIRE_CONFIG_DIR': str(root)}
    servers, child = [], None
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    try:
        def start_server():
            log = (root / f'server-{len(servers)}.log').open('wb')
            process = subprocess.Popen(['pipewire', '-c', str(root / 'pipewire.conf')],
                                       env=server_env, stdout=log, stderr=log)
            log.close()
            servers.append(process)
            deadline = time.monotonic() + 5
            while not (runtime / 'omatainer-test').exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('Private PipeWire server did not start')
                time.sleep(.02)
            return process
        server = start_server()
        log = (root / 'native.log').open('wb')
        child = subprocess.Popen([str(binary.resolve()), TEST, '--ignored', '--exact', '--test-threads=1'],
                                 env=env, stdout=log, stderr=log)
        log.close()
        configured, links = set(), set()
        deadline = time.monotonic() + 40
        killed = restarted = False
        while child.poll() is None:
            if time.monotonic() > deadline:
                raise RuntimeError('Native recovery test exceeded 40 seconds')
            if (root / 'started.json').exists() and not killed:
                stop(server)
                killed = True
            if (root / 'offline.json').exists() and not restarted:
                server = start_server()
                configured.clear()
                links.clear()
                restarted = True
                (root / 'restart.ready').write_text('Explicit fallback is authorized by this fixture.\n')
            if not killed or restarted:
                connect(env, configured, links)
            time.sleep(.03)
        if child.returncode or not restarted or not (root / 'done.json').exists():
            raise RuntimeError('Native recovery test failed; inspect native.log')
        receipt = {
            'status': 'pass', 'started': started, 'finished': dt.datetime.now(dt.timezone.utc).isoformat(),
            'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'pipewire': command(['pipewire', '--version'], env).stdout.decode().strip(),
            'server_pids': [process.pid for process in servers], 'test_pid': child.pid,
            'started_stream': json.loads((root / 'started.json').read_text()),
            'offline': json.loads((root / 'offline.json').read_text()),
            'restored': json.loads((root / 'done.json').read_text()),
            'scope': 'Real CPAL/ALSA stream, private PipeWire restart and null sink; no physical interface, USB removal or system suspend',
        }
    finally:
        stop(child)
        for server in servers:
            stop(server)
    receipt['owned_children_exited'] = True
    (root / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(run(args.test_binary, args.destination), indent=2))
