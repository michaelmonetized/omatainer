#!/usr/bin/env python3
"""Generate local synthetic long media and qualify the real analysis App path.

No audio device, physical controller, network resource or user's media is used.
This is a bounded functional/concurrency fixture, not a realtime timing gate.
"""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
TEST = 'ui::library_analysis::tests::long::long_files_under_callback_analysis'
MAX_SOURCES = 128 * 1024 * 1024
NAMES = ('long.wav', 'long.flac', 'long.ogg')


def module(name):
    spec = importlib.util.spec_from_file_location('analysis_' + name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def digest(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def write(path, value):
    encoded = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as file:
        file.write(encoded)
        file.flush()
        os.fsync(file.fileno())


def validate(root, guard):
    guard.private_directory(root)
    guard.private_directory(root / 'sources')
    path = root / 'source-manifest.json'
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > 64 * 1024:
        raise ValueError('invalid bounded source manifest')
    manifest = json.loads(path.read_text())
    rows = manifest.get('sources')
    if manifest.get('schema') != 1 or not isinstance(rows, list) or tuple(row.get('name') for row in rows) != NAMES:
        raise ValueError('expected the exact three generated long-source records')
    total = 0
    for row in rows:
        source = root / 'sources' / row['name']
        info = source.lstat()
        total += info.st_size
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or total > MAX_SOURCES:
            raise ValueError('source is not owned regular media within the 128 MiB bound')
        if row.get('bytes') != info.st_size or row.get('file_sha256') != digest(source):
            raise ValueError('generated source changed')
        if (row.get('sample_rate'), row.get('channels'), row.get('requested_seconds')) != (48000, 2, 180.0):
            raise ValueError('long-source geometry does not match the fixed fixture')
    return manifest


def prepare(root, offline, guard):
    if not root.is_absolute() or '..' in root.parts or root.exists() or root.is_symlink():
        raise ValueError('preparation requires a fresh absolute destination')
    root.mkdir(mode=0o700)
    guard.private_directory(root)
    sources = root / 'sources'
    sources.mkdir(mode=0o700)
    info = offline.execute_private(['ffmpeg', '-version'], env=dict(os.environ), cwd=ROOT, timeout=15, limit=64 * 1024)
    if info.returncode:
        raise ValueError('ffmpeg is required for synthetic local fixtures')
    rows = []
    # Independent stereo tones with a continuous 2 Hz amplitude envelope.
    expression = 'aevalsrc=0.2*sin(2*PI*220*t)*(0.6+0.4*sin(2*PI*2*t))|0.15*sin(2*PI*330*t)*(0.6+0.4*sin(2*PI*2*t)):s=48000:d=180'
    for name, codec, extra in [('long.wav', 'pcm_s16le', []), ('long.flac', 'flac', ['-sample_fmt', 's16']), ('long.ogg', 'libvorbis', ['-q:a', '5'])]:
        source = sources / name
        command = ['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-n', '-f', 'lavfi', '-i', expression,
                   '-map_metadata', '-1', '-c:a', codec, '-threads', '2', *extra, str(source)]
        result = offline.execute_private(command, env=dict(os.environ), cwd=ROOT, timeout=180, limit=1024 * 1024)
        if result.returncode:
            raise ValueError('synthetic media generation failed: ' + result.stderr.decode(errors='replace')[:2048])
        source.chmod(0o600)
        rows.append(dict(name=name, codec=codec, sample_rate=48000, channels=2, requested_seconds=180.0,
                         bytes=source.stat().st_size, file_sha256=digest(source)))
        if sum(row['bytes'] for row in rows) > MAX_SOURCES:
            raise ValueError('generated sources exceeded the fixed 128 MiB bound')
    write(root / 'source-manifest.json', dict(schema=1, generator='Omatainer synthetic stereo 2 Hz envelope',
        ffmpeg_version=info.stdout.decode().splitlines()[0], sources=rows))
    return validate(root, guard)


def run(root, binary, offline, guard):
    source_manifest = validate(root, guard)
    if (root / 'run').exists() or (root / 'run').is_symlink():
        raise ValueError('existing run evidence is preserved; choose fresh prepared sources')
    binary = binary.resolve(strict=True)
    if not binary.is_file():
        raise ValueError('test binary must be a regular executable')
    offline.records.validate(ROOT)
    inventory = json.loads((ROOT / 'licenses/manifest.json').read_text())
    before = dict(test_binary_sha256=digest(binary), source_inventory_sha256=digest(ROOT / 'licenses/manifest.json'),
                  source_manifest_sha256=digest(root / 'source-manifest.json'))
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    environment = dict(os.environ, OMATAINER_ANALYSIS_LONG_DIR=str(root))
    for name, variable in [('config', 'XDG_CONFIG_HOME'), ('data', 'XDG_DATA_HOME'), ('state', 'XDG_STATE_HOME'),
                           ('cache', 'XDG_CACHE_HOME'), ('runtime', 'XDG_RUNTIME_DIR'), ('tmp', 'TMPDIR')]:
        path = root / name
        path.mkdir(mode=0o700)
        environment[variable] = str(path)
    with (root / 'child.log').open('xb') as log:
        os.fchmod(log.fileno(), 0o600)
        result = offline.execute_private([str(binary), TEST, '--exact', '--ignored', '--test-threads=1', '--nocapture'],
            env=environment, cwd=ROOT, timeout=900, limit=8 * 1024 * 1024, log=log)
    if result.returncode or not re.search(rb'test result: ok\. 1 passed;', result.stdout):
        raise ValueError('actual long-file analysis fixture failed; retained child.log is authoritative')
    path = root / 'run/report.json'
    meta = path.lstat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_size > 4 * 1024 * 1024:
        raise ValueError('invalid bounded child report')
    report = json.loads(path.read_text())
    if report.get('schema') != 1 or report.get('status') != 'pass' or report.get('embedded_manifest') != inventory:
        raise ValueError('child report failed or belongs to a different source inventory')
    offline.records.validate(ROOT)
    after = dict(test_binary_sha256=digest(binary), source_inventory_sha256=digest(ROOT / 'licenses/manifest.json'),
                 source_manifest_sha256=digest(root / 'source-manifest.json'))
    if before != after or validate(root, guard) != source_manifest:
        raise ValueError('source, inventory or executable changed during qualification')
    write(root / 'qualification.json', dict(schema=1, status='pass', started_utc=started,
        finished_utc=dt.datetime.now(dt.timezone.utc).isoformat(), bindings=before, child_report_sha256=digest(path),
        scope='Actual App, shared decoder and four-channel callback functional evidence; no hardware, human listening or deadline qualification'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'run'])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--test-binary', type=Path)
    args = parser.parse_args()
    if args.action == 'run' and args.test_binary is None:
        parser.error('run requires --test-binary')
    root = args.out.absolute()
    offline, guard = module('check-offline'), module('offline_guard')
    try:
        if args.action == 'prepare': prepare(root, offline, guard)
        else: run(root, args.test_binary, offline, guard)
        print(json.dumps(dict(status='pass', action=args.action, evidence=str(root))))
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        print('analysis qualification failed: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
