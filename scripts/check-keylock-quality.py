#!/usr/bin/env python3
"""Reproduce objective key-lock diagnostics and prepare an unscored blind pack.

This uses a freshly built native Rust test executable and private evidence files.
It does not open an audio device or establish perceptual/physical qualification.
Outputs must be fresh descendants of OMATAINER_KEYLOCK_EVIDENCE_ROOT
(default: repository target/keylock-quality).
"""
import argparse
import csv
import hashlib
import importlib.util
import datetime as dt
import platform
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace

REPO = Path(__file__).resolve().parent.parent
ROOT = Path(os.environ.get('OMATAINER_KEYLOCK_EVIDENCE_ROOT',
            Path(__file__).resolve().parent.parent / 'target/keylock-quality'))
if not ROOT.is_absolute():
    raise ValueError('OMATAINER_KEYLOCK_EVIDENCE_ROOT must be absolute')
ROOT = ROOT.resolve()
TEST = 'engine::keylock_quality_tests::export_keylock_quality'
LIMIT = 2 * 1024 ** 3


def fresh(path):
    path = path.absolute()
    if path.exists() or path.is_symlink() or not path.parent.resolve().is_relative_to(ROOT):
        raise ValueError('use a new evidence directory beneath ' + str(ROOT))
    return path


def write(path, value):
    with path.open('x', encoding='utf-8', newline='') as file:
        file.write(value)


def file_digest(path):
    result = hashlib.sha256()
    with path.open('rb') as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def licenses():
    spec = importlib.util.spec_from_file_location('quality_licenses', REPO / 'scripts/license-manifest.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def bindings(binary, manifest):
    return {'source_files': manifest['source_files'], 'binary_sha256': file_digest(binary),
            'manifest_sha256': file_digest(REPO / 'licenses/manifest.json'),
            'notices_sha256': file_digest(REPO / 'licenses/notices.json'),
            'toolchain': manifest['toolchain'], 'target': manifest['target']}


def verify_native(result, manifest, before, after):
    if result.get('embedded_manifest') != manifest:
        raise ValueError('test executable embeds stale source/dependency/toolchain records')
    if before != after or result['build']['executable_sha256'] != before['binary_sha256']:
        raise ValueError('source or executable changed during the measurement run')


def document(root, require_binding=True):
    if (root / 'INCOMPLETE').exists():
        raise ValueError('incomplete export: ' + str(root))
    path = root / 'report.json'
    if path.stat().st_size > 32 * 1024 ** 2:
        raise ValueError('report exceeds 32 MiB')
    result = json.loads(path.read_text())
    if result.get('schema') != 1 or result.get('human_listening_scores') is not None:
        raise ValueError('unsupported report or fabricated listening scores')
    if len(result['records']) != 630 or len(result['callbacks']) != 126:
        raise ValueError('incomplete fixed workload matrix')
    if require_binding:
        receipt_path = root / 'verified-run.json'
        if receipt_path.stat().st_size > 2 * 1024 ** 2:
            raise ValueError('run binding exceeds 2 MiB')
        receipt = json.loads(receipt_path.read_text())
        manifest = result['embedded_manifest']
        if (receipt.get('schema') != 1 or receipt['report_sha256'] != file_digest(path)
                or receipt['bindings']['source_files'] != manifest['source_files']
                or receipt['bindings']['toolchain'] != manifest['toolchain']
                or receipt['bindings']['target'] != manifest['target']
                or receipt['bindings']['binary_sha256'] != result['build']['executable_sha256']):
            raise ValueError('report does not match the retained source/executable binding')
    return result


def artifact(root, record):
    path = (root / record['wav']).resolve()
    if not path.is_relative_to(root.resolve()) or not path.is_file():
        raise ValueError('invalid audio artifact path')
    if path.stat().st_size > 128 * 1024 ** 2:
        raise ValueError('individual WAV exceeds 128 MiB')
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != record['wav_sha256']:
        raise ValueError('audio artifact hash mismatch: ' + str(path))
    return path


def compare(args):
    baseline, candidate = document(args.baseline), document(args.candidate)
    if baseline['workload_sha256'] != candidate['workload_sha256'] or baseline['workload'] != candidate['workload']:
        raise ValueError('baseline and candidate workloads/corpus differ; no matched comparison is valid')
    before = {r['id']: r for r in baseline['records']}
    after = {r['id']: r for r in candidate['records']}
    if len(before) != 630 or before.keys() != after.keys():
        raise ValueError('duplicate or mismatched render identities')
    selected = [r for r in after.values() if r['locked'] and r['block_frames'] == 128 and r['output_sr'] == 48000]
    assert len(selected) == 35
    order = lambda name: hashlib.sha256((str(args.seed) + '\0' + name).encode()).digest()
    selected.sort(key=lambda r: order(r['id']))
    files = []
    for number, row in enumerate(selected, 1):
        old, new = artifact(args.baseline, before[row['id']]), artifact(args.candidate, row)
        unlocked = after[row['id'].removesuffix('_locked') + '_unlocked']
        reference = artifact(args.candidate, unlocked)
        files.append((number, row, old, new, reference))
    total = sum(path.stat().st_size for _, _, *paths in files for path in paths)
    if total > LIMIT:
        raise ValueError('blind pack exceeds 2 GiB cap')
    out = fresh(args.out)
    out.mkdir(mode=0o700)
    listen, operator = out / 'listen', out / 'operator'
    listen.mkdir(mode=0o700)
    operator.mkdir(mode=0o700)
    write(out / 'INCOMPLETE', 'Do not use until completion.json exists.\n')
    key = []
    with (listen / 'scores.csv').open('x', newline='', encoding='utf-8') as sheet:
        writer = csv.writer(sheet)
        writer.writerow(['pair', 'listener', 'date', 'A_pitch_stability', 'B_pitch_stability',
                         'A_artifacts', 'B_artifacts', 'A_transients', 'B_transients', 'preference', 'notes'])
        for number, row, old, new, reference in files:
            pair = f'pair-{number:02d}'
            reverse = bool(order(row['id'] + '\0side')[0] & 1)
            a, b = (new, old) if reverse else (old, new)
            for suffix, source in [('A', a), ('B', b), ('rate-shifted-reference', reference)]:
                with source.open('rb') as reader, (listen / f'{pair}-{suffix}.wav').open('xb') as writer_file:
                    shutil.copyfileobj(reader, writer_file)
            writer.writerow([pair] + [''] * 10)  # Deliberately no human ratings.
            key.append({'pair': pair, 'case': row['id'], 'source': row['source'], 'ratio': row['ratio'],
                        'A': 'candidate' if reverse else 'baseline', 'B': 'baseline' if reverse else 'candidate',
                        'baseline_wav_sha256': before[row['id']]['wav_sha256'], 'candidate_wav_sha256': row['wav_sha256']})
    for file in ['README.md', 'sources.json', 'VocalSet-CC-BY-4.0.txt']:
        with (args.candidate / file).open('rb') as reader, (listen / ('corpus-' + file)).open('xb') as target:
            shutil.copyfileobj(reader, target)
    write(listen / 'LISTENING.md', '''# Unscored blind key-lock comparison

These 35 pairs cover two recorded VocalSet singing vowels (CCBY4, see corpus
attribution), original analytical bass/transients and a complete procedural
Omatainer mix. Audio is stereo Float32 WAV at 48 kHz. A/B order is deterministic
and blinded; filenames reveal neither implementation. The recordings are
derivatives processed at the playback ratios recorded in the concealed key. No level normalization
or alignment correction was applied: note differences when judging. The third
file is an explicitly unlocked, rate-shifted reference; its pitch intentionally
changes with playback speed. It is not a transparent time-stretch ideal.

Choose a comfortable listening level before playback. Record your own scale,
equipment, listening conditions and observations in scores.csv. Every rating
cell starts empty. Objective metrics and the implementation key are kept in the
separate operator directory; avoid them until listening is finished. A small
corpus cannot qualify all music or the physical live setup.
''')
    def measurements(report):
        return [{'id': row['id'], 'bass': row.get('bass'), 'transients': row.get('transients'),
                 'levels': row['levels'], 'cpu': row['measurement'],
                 'unity_content_alignment': row.get('unity_content_alignment')} for row in report['records'] if row['block_frames'] == 128]
    write(operator / 'key.json', json.dumps({'schema': 1, 'seed': args.seed, 'workload_sha256': candidate['workload_sha256'],
          'baseline_runtime_checkout_commit': baseline['runtime_checkout_commit'],
          'candidate_runtime_checkout_commit': candidate['runtime_checkout_commit'],
          'baseline_executable_sha256': baseline['build']['executable_sha256'],
          'candidate_executable_sha256': candidate['build']['executable_sha256'],
          'source_qualification': 'Both verified-run receipts bind the exact report and executable to the reviewed embedded source inventory', 'pairs': key}, indent=2) + '\n')
    write(operator / 'objective-diagnostics.json', json.dumps({'schema': 1, 'human_scores': None,
          'baseline': measurements(baseline), 'candidate': measurements(candidate),
          'scope': 'Objective diagnostics only; no automated quality winner or listening score'}, indent=2) + '\n')
    write(out / 'completion.json', json.dumps({'schema': 1, 'pairs': len(key), 'audio_bytes': total,
          'workload_sha256': candidate['workload_sha256'], 'human_scores': None}, indent=2) + '\n')
    (out / 'INCOMPLETE').unlink()
    print('Created 35 deterministic blind pairs and an EMPTY human score sheet:', out)


def self_test():
    # Tiny synthetic containers test orchestration only, never DSP/quality evidence.
    with tempfile.TemporaryDirectory(prefix='keylock-script-test-', dir=ROOT) as temporary:
        root = Path(temporary)
        reports = []
        for implementation in ['before', 'after']:
            folder = root / implementation
            folder.mkdir()
            payload = b'private synthetic artifact for script tests\n'
            (folder / 'audio.wav').write_bytes(payload)
            rows = []
            for source in ['vocal_f1', 'vocal_m1', 'bass', 'transients', 'full_mix']:
                for sr in [44100, 48000, 96000]:
                    for block in [64, 128, 512]:
                        for ratio in [.5, .84, .92, 1., 1.08, 1.16, 1.5]:
                            for locked in [False, True]:
                                rows.append(dict(id=f"{source}_{sr}_{block}_{ratio:.2f}_{'locked' if locked else 'unlocked'}",
                                    source=source, output_sr=sr, block_frames=block, ratio=ratio,
                                    locked=locked, wav='audio.wav', wav_sha256=hashlib.sha256(payload).hexdigest(),
                                    levels=[], measurement={}))
            report = dict(schema=1, workload={'test': True}, workload_sha256='synthetic-script-fixture',
                          records=rows, callbacks=[{}] * 126, human_listening_scores=None, runtime_checkout_commit=implementation,
                          build={'executable_sha256': 'synthetic-' + implementation},
                          embedded_manifest={'source_files': {}, 'toolchain': {}, 'target': 'synthetic'})
            (folder / 'report.json').write_text(json.dumps(report))
            receipt = dict(schema=1, report_sha256=file_digest(folder/'report.json'),
                           bindings=dict(source_files={}, toolchain={}, target='synthetic',
                                         binary_sha256='synthetic-'+implementation))
            (folder / 'verified-run.json').write_text(json.dumps(receipt))
            for name in ['README.md', 'sources.json', 'VocalSet-CC-BY-4.0.txt']:
                (folder / name).write_text('Synthetic orchestration fixture; not corpus evidence.\n')
            reports.append(folder)
        first = SimpleNamespace(baseline=reports[0], candidate=reports[1], out=root/'pack1', seed=100)
        second = SimpleNamespace(**vars(first)); second.out=root/'pack2'
        compare(first); compare(second)
        assert (first.out/'operator/key.json').read_bytes() == (second.out/'operator/key.json').read_bytes()
        with (first.out/'listen/scores.csv').open() as file:
            rows = list(csv.reader(file))
        assert len(rows) == 36 and all(all(not cell for cell in row[1:]) for row in rows[1:])
        def refuses(args, text):
            try:
                compare(args)
            except ValueError as error:
                assert text in str(error), str(error)
            else:
                raise AssertionError('unsafe comparison unexpectedly succeeded')
        marker = first.out/'keep'; marker.write_text('preserve')
        refuses(first, 'new evidence directory'); assert marker.read_text() == 'preserve'
        rejected = SimpleNamespace(**vars(first)); rejected.out=root/'rejected'
        report_path = reports[1]/'report.json'
        report = json.loads(report_path.read_text()); report['workload_sha256']='changed'
        report_path.write_text(json.dumps(report))
        receipt_path = reports[1]/'verified-run.json'
        receipt = json.loads(receipt_path.read_text()); receipt['report_sha256']=file_digest(report_path)
        receipt_path.write_text(json.dumps(receipt))
        refuses(rejected, 'workloads/corpus differ')
        assert not rejected.out.exists()
        report['workload_sha256']='synthetic-script-fixture'; report_path.write_text(json.dumps(report))
        receipt['report_sha256']=file_digest(report_path); receipt_path.write_text(json.dumps(receipt))
        native = dict(embedded_manifest={'source_files': {'new.rs': 'new'}}, build={'executable_sha256': 'new'})
        try:
            verify_native(native, {'source_files': {'old.rs': 'old'}}, {}, {})
        except ValueError as error:
            assert 'stale source' in str(error)
        else:
            raise AssertionError('stale executable was accepted')
        try:
            verify_native(native, native['embedded_manifest'], {'binary_sha256': 'old'}, {'binary_sha256': 'new'})
        except ValueError as error:
            assert 'changed during' in str(error)
        else:
            raise AssertionError('changed executable was accepted')
        (reports[1]/'audio.wav').write_bytes(b'corrupt')
        refuses(rejected, 'hash mismatch'); assert not rejected.out.exists()
    print('Script fixtures pass: matched corpus, deterministic blind order, empty scores, no overwrite, mismatched/corrupt refusal.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    sub.add_parser('self-test')
    run = sub.add_parser('run')
    run.add_argument('--test-binary', type=Path, required=True)
    run.add_argument('--out', type=Path, required=True)
    comparison = sub.add_parser('compare')
    comparison.add_argument('--baseline', type=Path, required=True)
    comparison.add_argument('--candidate', type=Path, required=True)
    comparison.add_argument('--out', type=Path, required=True)
    comparison.add_argument('--seed', type=int, default=100)
    args = parser.parse_args()
    ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    if args.action == 'run':
        destination = fresh(args.out)
        records = licenses()
        manifest = records.validate(REPO)
        binary = args.test_binary.resolve()
        before = bindings(binary, manifest)
        started = dt.datetime.now(dt.timezone.utc).isoformat()
        host = {'system': platform.system(), 'machine': platform.machine(), 'kernel': platform.release(),
                'logical_cpus': os.cpu_count(), 'load_average_before': list(os.getloadavg())}
        env = dict(os.environ, OMATAINER_KEYLOCK_QUALITY_OUT=str(destination),
                   OMATAINER_KEYLOCK_EVIDENCE_ROOT=str(ROOT))
        subprocess.run([str(args.test_binary.resolve()), '--ignored', '--exact', TEST, '--nocapture', '--test-threads=1'], env=env, check=True)
        result = document(destination, require_binding=False)
        after_manifest = records.validate(REPO)
        if after_manifest != manifest:
            raise ValueError('reviewed source manifest changed during measurement')
        verify_native(result, manifest, before, bindings(binary, after_manifest))
        host['load_average_after'] = list(os.getloadavg())
        write(destination/'verified-run.json', json.dumps(dict(schema=1, started_utc=started,
              finished_utc=dt.datetime.now(dt.timezone.utc).isoformat(), host=host, bindings=before,
              report_sha256=file_digest(destination/'report.json')), indent=2)+'\n')
        document(destination)
        print(json.dumps({'workload_sha256': result['workload_sha256'], 'renders': len(result['records']),
                          'callback_cases': len(result['callbacks']), 'human_scores': None}))
    elif args.action == 'compare':
        compare(args)
    else:
        self_test()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, AssertionError, subprocess.SubprocessError) as error:
        sys.exit('Key-lock qualification: ' + str(error))
