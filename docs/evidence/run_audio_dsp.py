#!/usr/bin/env python3
"""Compile an audit harness against unchanged repository source and cached Cargo deps.

Usage: python3 run_audio_dsp.py /path/to/omatainer [--out-dir /path/to/evidence]
Run `cargo test --locked` in that repository first if target/debug/deps is absent.
This runner does not invoke Cargo, open audio devices, or edit repository source.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('repo', type=Path)
parser.add_argument('--out-dir', type=Path)
args = parser.parse_args()
repo = args.repo.resolve()
harness = Path(__file__).resolve().with_name('audio_dsp.rs')
deps = repo / 'target/debug/deps'
output = args.out_dir or Path(tempfile.mkdtemp(prefix='omatainer-audio-audit-'))
output.mkdir(parents=True, exist_ok=True)
output = output.resolve()
command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}', str(harness), '-o', str(output / 'audio_dsp')]
for dep in ['anyhow', 'cpal', 'parking_lot', 'crossbeam_channel', 'serde', 'symphonia', 'midir']:
    candidates = sorted(deps.glob(f'lib{dep}-*.rlib'), key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        parser.error(f'Missing cached dependency {dep}; run cargo test --locked first')
    command += ['--extern', f'{dep}={candidates[0]}']
environment = dict(os.environ, OMATAINER_AUDIT_REPO=str(repo))
subprocess.run(command, env=environment, check=True)
result = subprocess.run([str(output / 'audio_dsp')], check=True, text=True, capture_output=True)
(output / 'audio_dsp-results.txt').write_text(result.stdout)
print(result.stdout, end='')
print(f'Raw results: {output / "audio_dsp-results.txt"}')
