#!/usr/bin/env python3
"""Source-bound two-keylocked-deck callback diagnostics with fixed #95 ceilings.

This is an offline, local timing gate, not an audio-driver or XRUN measurement.
The matched 630/126 quality matrix and original #95 policy remain unchanged.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import struct
import sys

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
POLICY = REPO / 'benchmarks/keylock-show-policy.json'


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, REPO / 'scripts' / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


quality = module('keylock_quality', 'check-keylock-quality.py')
gate = module('keylock_gate', 'performance-gate.py')


def validate_policy(policy):
    gate.keys(policy, ('schema', 'suite', 'test', 'timeout_seconds', 'rates', 'frames', 'ratios',
        'blocks', 'warmup_blocks', 'repetitions', 'conditions', 'source', 'events', 'deadline_rule',
        'checks', 'scope'), 'show policy')
    assert policy['schema'] == 1 and policy['suite'] == 'keylock-showload-v1'
    assert policy['test'] == 'engine::keylock_showload_tests::export_keylock_showload'
    assert policy['rates'] == [44100, 48000, 96000] and policy['frames'] == [128, 256]
    assert policy['ratios'] == [.5, .84, 1.5]
    assert (policy['blocks'], policy['warmup_blocks'], policy['repetitions']) == (2048, 128, 3)
    assert policy['timeout_seconds'] == 900
    assert policy['conditions'] == dict(tracks=8, notes_per_track=1024, fx_per_track=3,
        recorded_notes=128, locked_decks=2, parameter_interval_blocks=8, source_sr=48000)
    assert policy['checks'] == ['finite_output', 'nonzero_output', 'original_notes_intact',
        'exact_recorded_notes', 'all_commands_applied', 'both_locked_at_fixed_ratio', 'coincident_search_hops']


def evaluate(raw, policy, manifest):
    validate_policy(policy)
    gate.keys(raw, ('schema', 'suite', 'policy', 'embedded_manifest', 'workloads'), 'show report')
    if raw['schema'] != 1 or raw['suite'] != policy['suite'] or raw['policy'] != policy:
        raise ValueError('raw report differs from reviewed show policy')
    if raw['embedded_manifest'] != manifest:
        raise ValueError('test executable embeds stale source/dependency/toolchain records')
    expected = [(rate, frames, ratio) for rate in policy['rates'] for frames in policy['frames'] for ratio in policy['ratios']]
    if len(raw['workloads']) != len(expected):
        raise ValueError('incomplete fixed show matrix')
    results, failures = [], []
    for row, (rate, frames, ratio) in zip(raw['workloads'], expected):
        gate.keys(row, ('id', 'conditions', 'measurements'), 'show workload')
        identity = f'show_{rate}_{frames}_{ratio:.2f}'
        conditions = dict(policy['conditions'], sample_rate=rate, frames=frames,
            ratio=struct.unpack('<f', struct.pack('<f', ratio))[0], blocks=2048, warmup_blocks=128)
        if row['id'] != identity or row['conditions'] != conditions or len(row['measurements']) != 3:
            raise ValueError('unexpected show workload identity, conditions or repeat count')
        deadline = frames * 1_000_000_000 // rate
        samples = dict(callback_wall_ns={'p99': deadline, 'max': 2*deadline, 'p99_minus_p50': deadline//2},
            render_cpu_ns={'p99': 3*deadline//4}, full_callback_thread_cpu_ns={'p99': 3*deadline//4})
        repeats = []
        hashes = []
        for repetition, measured in enumerate(row['measurements']):
            prefix = f'{identity}[{repetition}]'
            gate.keys(measured, ('samples', 'metrics', 'checks', 'observations'), prefix)
            gate.keys(measured['samples'], samples, prefix+' samples')
            stats = {}
            exceedances = {}
            for name, limits in samples.items():
                values = measured['samples'][name]
                if type(values) is not list or len(values) != policy['blocks']:
                    raise ValueError('wrong timing sample count')
                for value in values:
                    gate.integer(value, name)
                stats[name] = gate.distribution(values)
                exceedances[name] = sum(value > deadline for value in values)
                for statistic, limit in limits.items():
                    if stats[name][statistic] > limit:
                        failures.append(f'{prefix}: {name}.{statistic}={stats[name][statistic]} exceeds {limit}')
            gate.keys(measured['metrics'], ('allocations', 'frees', 'rejected_commands', 'rms', 'peak',
                'wall_deadline_exceedances', 'render_cpu_deadline_exceedances', 'full_cpu_deadline_exceedances'), prefix+' metrics')
            for name in ['allocations', 'frees', 'rejected_commands']:
                gate.integer(measured['metrics'][name], name)
                if measured['metrics'][name] != 0:
                    failures.append(f'{prefix}: {name}={measured["metrics"][name]}')
            for name in ['rms', 'peak']:
                value = gate.number(measured['metrics'][name], name)
                if not .001 <= value <= 1:
                    failures.append(f'{prefix}: {name}={value} outside [.001,1]')
            for name, sample in [('wall_deadline_exceedances', 'callback_wall_ns'),
                                 ('render_cpu_deadline_exceedances', 'render_cpu_ns'),
                                 ('full_cpu_deadline_exceedances', 'full_callback_thread_cpu_ns')]:
                gate.integer(measured['metrics'][name], name)
                if measured['metrics'][name] != exceedances[sample]:
                    raise ValueError('incorrect actual-deadline exceedance count')
            gate.keys(measured['checks'], policy['checks'], prefix+' checks')
            for name, passed in measured['checks'].items():
                if type(passed) is not bool:
                    raise ValueError('checks must be booleans')
                if not passed:
                    failures.append(f'{prefix}: check failed: {name}')
            gate.keys(measured['observations'], ('searches_per_deck', 'analysis_hops_per_deck',
                'coincident_search_callbacks', 'quantized_audio_hash'), prefix+' observations')
            searches = measured['observations']['searches_per_deck']
            hops = measured['observations']['analysis_hops_per_deck']
            for name, values in [('search', searches), ('analysis hop', hops)]:
                if type(values) is not list or len(values) != 2:
                    raise ValueError('invalid per-deck '+name+' counts')
                for value in values:
                    gate.integer(value, name+' count', 1)
            if searches[0] != searches[1] or searches != hops:
                failures.append(f'{prefix}: full searches differ from coincident analysis hops')
            gate.integer(measured['observations']['coincident_search_callbacks'],
                'coincident search callbacks', 1, min(policy['blocks'], *searches))
            audio_hash = measured['observations']['quantized_audio_hash']
            if type(audio_hash) is not str or len(audio_hash) != 16 or any(c not in '0123456789abcdef' for c in audio_hash):
                raise ValueError('invalid audio observation')
            hashes.append(audio_hash)
            repeats.append(dict(distributions=stats, metrics=measured['metrics'], checks=measured['checks'], observations=measured['observations']))
        if len(set(hashes)) != 1:
            failures.append(f'{identity}: audio differs across identical repeated schedules')
        results.append(dict(id=identity, deadline_ns=deadline, limits=samples, measurements=repeats))
    return results, failures


def run(args):
    quality.ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    destination = quality.fresh(args.out)
    policy = gate.parse(gate.regular(POLICY, 128*1024)); validate_policy(policy)
    records = quality.licenses(); manifest = records.validate(REPO)
    binary = args.test_binary.resolve(); before = quality.bindings(binary, manifest)
    machine = gate.host()
    environment = dict(os.environ, OMATAINER_KEYLOCK_EVIDENCE_ROOT=str(quality.ROOT),
                       OMATAINER_KEYLOCK_SHOW_OUT=str(destination))
    completed = gate.execute([str(binary), '--ignored', '--exact', policy['test'], '--nocapture', '--test-threads=1'],
        timeout=policy['timeout_seconds'], limit=2*1024*1024, env=environment, cwd=REPO)
    if destination.is_dir():
        quality.write(destination/'execution.log', completed.stdout.decode(errors='replace')+completed.stderr.decode(errors='replace'))
    if completed.returncode:
        raise ValueError('native show workload failed; no verified timing result')
    machine['load_average_after_workload'] = list(os.getloadavg())
    raw = gate.parse(gate.regular(destination/'raw.json', 32*1024*1024))
    after = records.validate(REPO)
    if after != manifest or before != quality.bindings(binary, after):
        raise ValueError('source or executable changed during show measurement')
    results, failures = evaluate(raw, policy, manifest)
    report = dict(schema=1, status='fail' if failures else 'pass', host=machine, bindings=before,
        raw_sha256=quality.file_digest(destination/'raw.json'), policy=policy, results=results, failures=failures,
        scope=policy['scope'], backend_xruns=None, device_latency=None, human_scores=None)
    quality.write(destination/'verified-showload.json', json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps(dict(status=report['status'], groups=len(results), repetitions=3, failures=failures)))
    if failures:
        raise ValueError('show-load policy failed; see retained report')


def self_test():
    import copy
    policy = gate.parse(gate.regular(POLICY, 128*1024)); validate_policy(policy)
    manifest = {'synthetic': True}
    workloads = []
    for rate in policy['rates']:
        for frames in policy['frames']:
            for ratio in policy['ratios']:
                measured = dict(samples={name:[10]*2048 for name in ['callback_wall_ns','render_cpu_ns','full_callback_thread_cpu_ns']},
                    metrics=dict(allocations=0,frees=0,rejected_commands=0,rms=.1,peak=.2,wall_deadline_exceedances=0,
                        render_cpu_deadline_exceedances=0,full_cpu_deadline_exceedances=0),
                    checks={name:True for name in policy['checks']}, observations=dict(searches_per_deck=[100,100],
                        analysis_hops_per_deck=[100,100],coincident_search_callbacks=100,quantized_audio_hash='0123456789abcdef'))
                workloads.append(dict(id=f'show_{rate}_{frames}_{ratio:.2f}',
                    conditions=dict(policy['conditions'],sample_rate=rate,frames=frames,ratio=struct.unpack('<f',struct.pack('<f',ratio))[0],blocks=2048,warmup_blocks=128),
                    measurements=[copy.deepcopy(measured) for _ in range(3)]))
    raw=dict(schema=1,suite=policy['suite'],policy=policy,embedded_manifest=manifest,workloads=workloads)
    assert not evaluate(raw,policy,manifest)[1]
    changed=copy.deepcopy(raw); measured=changed['workloads'][0]['measurements'][0]
    measured['samples']['callback_wall_ns'][-1]=10**9;measured['metrics']['wall_deadline_exceedances']=1
    measured['checks']['both_locked_at_fixed_ratio']=False;measured['metrics']['frees']=1
    assert len(evaluate(changed,policy,manifest)[1])>=3
    changed['workloads'][0]['measurements'][0]['metrics']['wall_deadline_exceedances']=0
    try:evaluate(changed,policy,manifest)
    except ValueError as error:assert 'exceedance' in str(error)
    else:raise AssertionError('incorrect deadline count accepted')
    try:evaluate(raw,policy,{'different':True})
    except ValueError as error:assert 'stale source' in str(error)
    else:raise AssertionError('stale binary manifest accepted')
    changed=copy.deepcopy(raw);changed['workloads'][0]['measurements'][0]['metrics']['wall_deadline_exceedances']=False
    try:evaluate(changed,policy,manifest)
    except ValueError:pass
    else:raise AssertionError('boolean deadline count accepted')
    changed=copy.deepcopy(raw);changed['workloads'][0]['measurements'][0]['observations']['searches_per_deck']=[99,99]
    changed['workloads'][0]['measurements'][0]['observations']['coincident_search_callbacks']=99
    assert any('full searches' in failure for failure in evaluate(changed,policy,manifest)[1])
    print('Show verifier fixtures pass: fixed matrix, unchanged deadline ceilings, state/heap/timing refusal and source binding.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='action',required=True)
    sub.add_parser('self-test')
    execute=sub.add_parser('run');execute.add_argument('--test-binary',type=Path,required=True);execute.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    if args.action=='self-test':self_test()
    else:run(args)


if __name__=='__main__':
    try:main()
    except (OSError,ValueError,KeyError,TypeError,AssertionError) as error:sys.exit('Keylock show-load: '+str(error))
