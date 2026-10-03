#!/usr/bin/env python3
"""Capture connected USB audio/MIDI endpoints without opening streams or changing drivers."""
import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import platform
import subprocess


def read(path, limit=65536):
    """Read a bounded system attribute.
    Takes a path and byte limit; returns text or None for unavailable attributes.
    """
    try:
        with path.open('rb') as stream:
            data = stream.read(limit + 1)
        return data.decode('utf-8', errors='replace').strip() if len(data) <= limit else None
    except OSError:
        return None


def command(arguments):
    """Inspect one local backend inventory.
    Takes argv; returns bounded output and its actual status without interpreting failures as success.
    """
    try:
        result = subprocess.run(arguments, capture_output=True, timeout=5)
        return {'status': result.returncode, 'stdout': result.stdout[:65536].decode(errors='replace'),
                'stderr': result.stderr[:65536].decode(errors='replace'),
                'stdout_truncated': len(result.stdout) > 65536, 'stderr_truncated': len(result.stderr) > 65536}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'status': None, 'error': str(error)}


def snapshot():
    """Record actual attached hardware and backend visibility.
    Takes no arguments; returns a versioned read-only endpoint inventory, never a compatibility claim.
    """
    usb = []
    root = Path('/sys/bus/usb/devices')
    for path in sorted(root.iterdir()):
        vendor, product = read(path / 'idVendor'), read(path / 'idProduct')
        if vendor is None or product is None:
            continue
        serial = read(path / 'serial')
        interfaces = []
        for interface in sorted(root.glob(path.name + ':*')):
            driver = interface / 'driver'
            endpoints = [{name: read(endpoint / name) for name in (
                'bEndpointAddress', 'bmAttributes', 'wMaxPacketSize', 'bInterval', 'direction', 'type')}
                for endpoint in sorted(interface.glob('ep_*'))]
            interfaces.append({
                'number': read(interface / 'bInterfaceNumber'), 'class': read(interface / 'bInterfaceClass'),
                'subclass': read(interface / 'bInterfaceSubClass'), 'protocol': read(interface / 'bInterfaceProtocol'),
                'driver': driver.resolve().name if driver.exists() else None, 'endpoints': endpoints,
            })
        descriptors = path / 'descriptors'
        try:
            with descriptors.open('rb') as stream:
                raw = stream.read(65537)
            descriptor = {'error': 'USB descriptors exceed 65536 bytes'} if len(raw) > 65536 else {'sha256': hashlib.sha256(raw).hexdigest(), 'hex': raw.hex()}
        except OSError as error:
            descriptor = {'error': str(error)}
        usb.append({'sysfs': path.name, 'vendor': vendor, 'product': product,
                    'manufacturer': read(path / 'manufacturer'), 'name': read(path / 'product'),
                    'firmware': read(path / 'bcdDevice'), 'speed_mbps': read(path / 'speed'),
                    'serial_sha256': hashlib.sha256(serial.encode()).hexdigest() if serial else None,
                    'interfaces': interfaces, 'descriptors': descriptor})
    return {'schema': 1, 'captured': dt.datetime.now(dt.timezone.utc).isoformat(),
            'kernel': platform.release(), 'architecture': platform.machine(), 'usb': usb,
            'alsa_cards': read(Path('/proc/asound/cards')), 'alsa_pcm': read(Path('/proc/asound/pcm')),
            'alsa_streams': {str(path.relative_to('/proc/asound')): read(path)
                             for path in sorted(Path('/proc/asound').glob('card*/stream*'))},
            'alsa_midi': command(['amidi', '-l']), 'alsa_sequencer': command(['aconnect', '-l']),
            'pipewire': command(['pw-cli', 'ls', 'Node']),
            'scope': 'Read-only USB descriptors and actual backend inventories; no audio, MIDI interaction, firmware changes or device qualification'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    payload = json.dumps(snapshot(), indent=2, ensure_ascii=False) + '\n'
    with args.output.open('x') as destination:
        destination.write(payload)
    print(f'Hardware inventory recorded: {args.output}')
