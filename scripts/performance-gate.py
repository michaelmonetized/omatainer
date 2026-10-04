#!/usr/bin/env python3
"""Run and verify source-bound local workload evidence; never infer hardware QA."""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import selectors
import signal
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
POLICY = 'benchmarks/policy.json'
REPORT = 'target/performance.json'
INSTALLED = '.local/share/omatainer/validation/performance.json'
LIMIT = 64 * 1024 * 1024
MAX_SAMPLES = 1_000_000
WORKLOADS = ('callback_producer', 'callback_composer', 'callback_live_dj',
             'callback_hybrid', 'large_crate_ui', 'multi_controller_ipc',
             'project_roundtrip', 'long_note_recording')
DISTRIBUTIONS = {'callback_wall_ns','render_cpu_ns','frame_wall_ns','renderer_wall_ns','ipc_roundtrip_ns','midi_dispatch_ns'}
QUANTILES = {'min', 'p50', 'p95', 'p99', 'max', 'spread', 'p99_minus_p50'}

class GateError(ValueError):
    pass

def fail(message):
    raise GateError(message)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False)+'\n').encode()

def keys(value, expected, where):
    if type(value) is not dict or set(value) != set(expected):
        fail(f'{where}: missing or unexpected fields')

def text(value, where, limit=4096):
    if type(value) is not str or not value or len(value.encode()) > limit or '\0' in value:
        fail(f'{where}: expected bounded nonempty text')
    return value

def integer(value, where, low=0, high=2**63-1):
    if type(value) is not int or not low <= value <= high:
        fail(f'{where}: expected integer from {low} through {high}')
    return value

def number(value, where):
    if type(value) not in (int, float) or abs(value) > 2**63-1 or not math.isfinite(value):
        fail(f'{where}: expected finite number')
    return value

def pairs(items):
    result = {}
    for key, value in items:
        if key in result: fail(f'duplicate JSON field: {key}')
        result[key] = value
    return result

def parse(data):
    if len(data) > LIMIT: fail('performance record exceeds byte limit')
    try:
        return json.loads(data, object_pairs_hook=pairs,
                          parse_constant=lambda value: fail(f'nonfinite JSON value: {value}'))
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise GateError(f'invalid performance JSON: {error}') from error

def regular(path, limit=LIMIT):
    path = Path(path)
    if path.is_symlink() or not path.is_file(): fail(f'expected regular performance file: {path}')
    with path.open('rb') as stream:
        data = stream.read(limit+1)
    if len(data) > limit: fail(f'performance file exceeds byte limit: {path}')
    return data

def digest(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file(): fail(f'expected regular artifact: {path}')
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024*1024), b''): result.update(block)
    return result.hexdigest()

def timestamp(value):
    text(value, 'timestamp', 64)
    try: result = dt.datetime.fromisoformat(value)
    except ValueError as error: raise GateError('invalid report timestamp') from error
    if result.tzinfo != dt.timezone.utc: fail('report timestamp must be UTC')
    return result

def bounds(value, where):
    keys(value, ('unit', 'min', 'max'), where)
    text(value['unit'], where+' unit', 128)
    low, high = number(value['min'], where), number(value['max'], where)
    if low > high: fail(f'{where}: inverted bounds')

def policy(value):
    document=value
    keys(value, ('schema','suite','status','test','timeout_seconds','scope','unresolved_limits','targets','workloads'), 'policy')
    if type(value['schema']) is not int or value['schema'] != 1 or value['suite'] != 'supported-workloads-v1': fail('unsupported performance policy')
    if value['status'] != 'reviewed': fail('performance budgets have not been reviewed')
    text(value['test'], 'test', 256)
    if not all(c.isalnum() or c in '_:' for c in value['test']): fail('invalid benchmark test name')
    integer(value['timeout_seconds'], 'timeout', 1, 3600)
    text(value['scope'], 'scope')
    if type(value['unresolved_limits']) is not list or not 1 <= len(value['unresolved_limits']) <= 32: fail('unresolved limits must be retained')
    for limitation in value['unresolved_limits']: text(limitation, 'unresolved limit')
    if type(value['targets']) is not list or not value['targets'] or len(set(value['targets'])) != len(value['targets']) or not set(value['targets']) <= {'x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu'}: fail('invalid reviewed target set')
    if type(value['workloads']) is not list or tuple(w.get('id') for w in value['workloads'] if type(w) is dict) != WORKLOADS:
        fail('policy must contain every fixed workload exactly once in the reviewed order')
    for workload in value['workloads']:
        keys(workload, ('id','conditions','repetitions','samples','metrics','checks','expected_observations'), 'workload policy')
        if type(workload['conditions']) is not dict or len(encode(workload['conditions'])) > 16384: fail('invalid workload conditions')
        integer(workload['repetitions'], 'repetitions', 1, 32)
        if type(workload['samples']) is not dict or not workload['samples'] or not set(workload['samples']) <= DISTRIBUTIONS:
            fail('missing or unknown timing sample distributions')
        for name, spec in workload['samples'].items():
            keys(spec, ('count','max'), name)
            if type(spec['count']) is dict:
                keys(spec['count'],('min','max'),name+' count'); integer(spec['count']['min'],name+' minimum',1,MAX_SAMPLES); integer(spec['count']['max'],name+' maximum',spec['count']['min'],MAX_SAMPLES)
            else: integer(spec['count'], name+' count', 1, MAX_SAMPLES)
            if type(spec['max']) is not dict or not spec['max'] or not set(spec['max']) <= QUANTILES: fail('missing or invalid distribution budgets')
            for value in spec['max'].values(): integer(value, name+' bound')
        if type(workload['metrics']) is not dict or not workload['metrics']: fail('missing scalar metric bounds')
        for name, spec in workload['metrics'].items():
            text(name, 'metric name', 128); bounds(spec, name)
        if type(workload['checks']) is not list or not workload['checks'] or len(set(workload['checks'])) != len(workload['checks']): fail('missing or duplicate golden checks')
        for check in workload['checks']: text(check, 'golden check', 128)
        keys(workload['expected_observations'],document['targets'],'target observations')
        for target, observations in workload['expected_observations'].items():
            if type(observations) is not dict: fail('invalid golden observations')
            if workload['id'].startswith('callback_') and not observations: fail('missing callback audio golden')
            for name, value in observations.items():
                text(name,'observation name',128);text(value,'expected observation',4096)
    return document

def distribution(samples):
    ordered = sorted(samples)
    n = len(ordered)
    return {'min': ordered[0], **{f'p{p}': ordered[(n*p+99)//100-1] for p in (50,95,99)},
            'max': ordered[-1], 'spread': ordered[-1]-ordered[0],
            'p99_minus_p50': ordered[(n*99+99)//100-1]-ordered[(n*50+99)//100-1]}

def evaluate(raw, rules, target, manifest=None):
    policy(rules)
    if target not in rules['targets']: fail('no reviewed workload evidence for this target')
    keys(raw, ('schema','suite','embedded_manifest','workloads'), 'raw report')
    if type(raw['embedded_manifest']) is not dict: fail('missing benchmark executable embedded source manifest')
    if manifest is not None and raw['embedded_manifest'] != manifest: fail('benchmark executable embeds stale source, policy, lock or toolchain records')
    if type(raw['schema']) is not int or raw['schema'] != 1 or raw['suite'] != rules['suite']: fail('unsupported raw report schema/suite')
    if type(raw['workloads']) is not list or len(raw['workloads']) != len(WORKLOADS): fail('missing raw workloads')
    results, failures = [], []
    for observed, expected in zip(raw['workloads'], rules['workloads']):
        keys(observed, ('id','conditions','measurements'), 'raw workload')
        identity = observed['id']
        if identity != expected['id'] or observed['conditions'] != expected['conditions']: fail('raw workload identity or conditions differ from reviewed policy')
        if type(observed['measurements']) is not list or len(observed['measurements']) != expected['repetitions']: fail(f'{identity}: wrong repetition count')
        measurements = []
        for index, measurement in enumerate(observed['measurements']):
            keys(measurement, ('samples','metrics','checks','observations'), f'{identity} measurement')
            keys(measurement['samples'], expected['samples'], f'{identity} samples')
            keys(measurement['metrics'], expected['metrics'], f'{identity} metrics')
            keys(measurement['checks'], expected['checks'], f'{identity} checks')
            keys(measurement['observations'], expected['expected_observations'][target], f'{identity} observations')
            for name, value in measurement['observations'].items():
                text(value, name, 4096)
                if value != expected['expected_observations'][target][name]: failures.append(f'{identity}[{index}] golden observation differs: {name}')
            timings = {}
            for name, spec in expected['samples'].items():
                values = measurement['samples'][name]
                count=spec['count']; low=count['min'] if type(count) is dict else count; high=count['max'] if type(count) is dict else count
                if type(values) is not list or not low <= len(values) <= high: fail(f'{identity}/{name}: wrong sample count')
                for value in values: integer(value, name)
                timings[name] = distribution(values)
                for statistic, bound in spec['max'].items():
                    if timings[name][statistic] > bound: failures.append(f'{identity}[{index}] {name}.{statistic}={timings[name][statistic]} exceeds {bound}')
            for name, spec in expected['metrics'].items():
                value = number(measurement['metrics'][name], name)
                if not spec['min'] <= value <= spec['max']: failures.append(f'{identity}[{index}] {name}={value} outside [{spec["min"]},{spec["max"]}]')
            for name, value in measurement['checks'].items():
                if type(value) is not bool: fail(f'{identity}/{name}: golden check must be boolean')
                if not value: failures.append(f'{identity}[{index}] golden check failed: {name}')
            measurements.append({'distributions': timings, 'metrics': measurement['metrics'], 'checks': measurement['checks'], 'observations': measurement['observations']})
        results.append({'id': identity, 'measurements': measurements})
    return results, failures

def licenses():
    spec = importlib.util.spec_from_file_location('performance_licenses', Path(__file__).with_name('license-manifest.py'))
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module

def bindings(root, binary, manifest):
    return {'source_files': manifest['source_files'], 'binary_sha256': digest(binary),
            'license_manifest_sha256': digest(root/'licenses/manifest.json'),
            'license_notices_sha256': digest(root/'licenses/notices.json'),
            'policy_sha256': digest(root/POLICY), 'cargo_lock_sha256': digest(root/'Cargo.lock'),
            'toolchain': manifest['toolchain'], 'target': manifest['target']}

def native_evidence(value):
    keys(value, ('exit_code','report'), 'native accessibility')
    if type(value['exit_code']) is not int or value['exit_code'] != 0:
        fail('native accessibility preflight did not pass')
    report = value['report']
    if type(report) is not dict or report.get('platform') != 'Linux AT-SPI via private D-Bus':
        fail('missing private Linux AT-SPI evidence')
    integer(report.get('native_nodes_visited'), 'native node count', 1, 10_000)
    integer(report.get('frames'), 'native App frames', 5, 1_000_000)
    if report.get('pitch_role') != 'slider' or report.get('pitch_range') != [-8,8] or report.get('pitch_renderer_after_native_setvalue') != .25:
        fail('native numeric action did not reach the renderer')
    if report.get('persisted_notes') != 1 or report.get('reopened_notes') != 1 or report.get('preferences_saved_scale') != 1.25:
        fail('native persistence workflow evidence is incomplete')
    actions=report.get('actions')
    if type(actions) is not list or len(actions)>10_000 or not all(type(action) is dict for action in actions):
        fail('invalid native action evidence')
    if not {'Focus','SetValue','Click'} <= {action.get('action') for action in actions}:
        fail('required native focus/value/action paths did not run')
    text(report.get('scope'), 'native evidence limitations')

def verify(report, manifest, binary_sha256, manifest_sha256, notices_sha256):
    keys(report, ('schema','status','started_utc','finished_utc','host','build','bindings','policy_text','raw','summary','failures'), 'performance report')
    if type(report['schema']) is not int or report['schema'] != 1: fail('unsupported performance report schema')
    if timestamp(report['finished_utc']) < timestamp(report['started_utc']): fail('inverted performance report timestamps')
    keys(report['bindings'], ('source_files','binary_sha256','license_manifest_sha256','license_notices_sha256','policy_sha256','cargo_lock_sha256','toolchain','target'), 'bindings')
    expected = {'source_files': manifest['source_files'], 'binary_sha256': binary_sha256,
                'license_manifest_sha256': manifest_sha256, 'license_notices_sha256': notices_sha256,
                'policy_sha256': manifest['source_files'].get(POLICY),
                'cargo_lock_sha256': manifest['source_files'].get('Cargo.lock'),
                'toolchain': manifest['toolchain'], 'target': manifest['target']}
    if report['bindings'] != expected: fail('stale benchmark source, binary, policy, notices or toolchain identity')
    text(report['policy_text'], 'retained policy', 128*1024)
    if sha(report['policy_text'].encode()) != expected['policy_sha256']: fail('retained policy differs from reviewed source')
    rules = policy(parse(report['policy_text']))
    summary, failures = evaluate(report['raw'], rules, manifest['target'], manifest)
    if report.get('build',{}).get('test_exit_code') != 0: failures.append(f'benchmark process exited {report.get("build",{}).get("test_exit_code")}')
    if report['summary'] != summary or report['failures'] != failures or report['status'] != ('fail' if failures else 'pass'):
        fail('benchmark summary does not match independently recomputed raw evidence')
    keys(report['host'], ('execution','system','architecture','kernel','cpu_model','logical_cpus','affinity','memory_bytes','governors','load_average_before_workload','load_average_after_workload','process_nice','scheduler_policy'), 'host')
    if report['host']['execution'] != 'local' or report['host']['system'] != 'Linux': fail('benchmark must run locally on documented Linux hardware')
    for name in ('architecture','kernel','cpu_model'): text(report['host'][name], name)
    integer(report['host']['logical_cpus'], 'logical CPUs', 1, 4096)
    integer(report['host']['memory_bytes'], 'memory', 1)
    integer(report['host']['process_nice'], 'process nice', -20, 19)
    integer(report['host']['scheduler_policy'], 'scheduler policy', 0, 6)
    for name in ('load_average_before_workload','load_average_after_workload'):
        values=report['host'][name]
        if type(values) is not list or len(values)!=3: fail('missing workload load averages')
        for value in values:
            if number(value, name)<0: fail('negative host load average')
    if type(report['host']['affinity']) is not list or not report['host']['affinity'] or len(report['host']['affinity']) > 4096: fail('missing CPU affinity')
    for cpu in report['host']['affinity']: integer(cpu, 'CPU ID', 0, 4095)
    if type(report['host']['governors']) is not list or len(report['host']['governors']) > 64: fail('invalid governor inventory')
    for governor in report['host']['governors']: text(governor, 'governor', 128)
    keys(report['build'], ('profile','cargo','binary_info','environment','test_exit_code','test_binary_sha256','native_accessibility'), 'build')
    if report['build']['profile'] != 'release': fail('benchmark must use release profile')
    text(report['build']['cargo'], 'cargo version', 1024)
    keys(report['build']['binary_info'], ('schema','debug_assertions','pkg_version','arch','os'), 'binary build info')
    info = report['build']['binary_info']
    if type(info['schema']) is not int or info['schema'] != 1 or info['debug_assertions'] is not False or info['pkg_version'] != manifest['application'] or info['os'] != 'linux': fail('binary build does not match release evidence')
    if info['arch'] != report['host']['architecture']: fail('benchmark binary and host architectures differ')
    integer(report['build']['test_exit_code'], 'test exit code', -128, 255)
    text(report['build']['test_binary_sha256'], 'test binary digest', 64)
    if len(report['build']['test_binary_sha256']) != 64 or any(c not in '0123456789abcdef' for c in report['build']['test_binary_sha256']): fail('invalid test binary identity')
    native_evidence(report['build']['native_accessibility'])
    if type(report['build']['environment']) is not dict or report['build']['environment']: fail('unreviewed Rust build environment overrides')
    if failures: fail('performance budgets or golden checks failed: '+'; '.join(failures[:8]))
    return report

def check(root, binary, report_path=None, manifest=None):
    root, binary = Path(root), Path(binary)
    document = manifest or licenses().validate(root)
    report = parse(regular(report_path or root/REPORT))
    return verify(report, document, digest(binary), digest(root/'licenses/manifest.json'), digest(root/'licenses/notices.json'))

def atomic(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(prefix='.performance-', dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
            stream.close(); os.replace(temporary, path)
            descriptor = os.open(path.parent, os.O_RDONLY|os.O_DIRECTORY)
            try: os.fsync(descriptor)
            finally: os.close(descriptor)
        finally:
            temporary.unlink(missing_ok=True)

def execute(command, timeout=10, limit=65536, env=None, log=None, cwd=None):
    """Finite output/time, including descendants holding an inherited pipe open."""
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, env=env, cwd=cwd, start_new_session=True)
    output = {'stdout': bytearray(), 'stderr': bytearray()}
    end = time.monotonic()+timeout
    completed=False
    try:
        with selectors.DefaultSelector() as selector:
            for name, pipe in [('stdout',process.stdout),('stderr',process.stderr)]:
                os.set_blocking(pipe.fileno(),False); selector.register(pipe, selectors.EVENT_READ, name)
            while selector.get_map():
                remaining=end-time.monotonic()
                if remaining <= 0: fail('local process exceeded its finite deadline')
                for key, _ in selector.select(min(remaining,0.1)):
                    chunk=os.read(key.fd,65536)
                    if not chunk:
                        selector.unregister(key.fileobj); continue
                    if sum(map(len,output.values()))+len(chunk)>limit: fail('local process exceeded its output limit')
                    output[key.data].extend(chunk)
                    if log is not None: log.write(chunk); log.flush()
            remaining=end-time.monotonic()
            if remaining <= 0: fail('local process exceeded its finite deadline')
            code=process.wait(timeout=remaining)
            completed=True
            return subprocess.CompletedProcess(command,code,bytes(output['stdout']),bytes(output['stderr']))
    finally:
        # Cargo tests may spawn private services. Retire the whole private group,
        # even when the direct child exited before a descendant closed its pipe.
        if not completed:
            try: os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError: pass
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: pass
            # The direct child may exit on TERM while a descendant ignores it.
            # Always retire the remaining owned group, not only a live parent.
            try: os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError: pass
            process.wait(timeout=2)
        process.stdout.close();process.stderr.close()

def output(command, timeout=10):
    result=execute(command,timeout)
    if result.returncode: fail(f'identity command failed: {result.stderr[:1024].decode(errors="replace")}')
    return result.stdout.decode().strip()

def host():
    if platform.system() != 'Linux': fail('local Linux runner required')
    cpu = regular('/proc/cpuinfo', 2*1024*1024).decode()
    model = next((line.split(':',1)[1].strip() for line in cpu.splitlines() if line.startswith(('model name','Hardware'))), platform.machine())
    if model == platform.machine():
        try: model = regular('/sys/firmware/devicetree/base/model',4096).decode().rstrip('\0\n')
        except (OSError,GateError): model = 'CPU model unavailable ('+platform.machine()+')'
    memory = regular('/proc/meminfo', 65536).decode()
    memory = next(int(line.split()[1])*1024 for line in memory.splitlines() if line.startswith('MemTotal:'))
    governors = set()
    for path in sorted(Path('/sys/devices/system/cpu').glob('cpu[0-9]*/cpufreq/scaling_governor'))[:64]:
        try: governors.add(path.read_text()[:128].strip())
        except OSError: governors.add('unavailable')
    return {'execution':'local','system':'Linux','architecture':platform.machine(),'kernel':platform.release(),
            'cpu_model':model,'logical_cpus':os.cpu_count(),'affinity':sorted(os.sched_getaffinity(0)),
            'memory_bytes':memory,'governors':sorted(governors),
            'load_average_before_workload':list(os.getloadavg()),'load_average_after_workload':list(os.getloadavg()),
            'process_nice':os.getpriority(os.PRIO_PROCESS,0),'scheduler_policy':os.sched_getscheduler(0)}

def artifact(compiled, test):
    if compiled.returncode: fail('controlled release compilation failed; report remains incomplete')
    found=[]
    for line in compiled.stdout.splitlines():
        item=parse(line)
        if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='omatainer' and item.get('profile',{}).get('test') is test and item.get('executable'):
            if item['profile'].get('debug_assertions') is not False or item['profile'].get('opt_level') != '3': fail('executable is not the reviewed optimized release profile')
            found.append(Path(item['executable']).resolve())
    if len(found)!=1: fail('expected exactly one controlled release executable')
    return found[0]

def run(root, binary, destination):
    root, binary = Path(root).resolve(), Path(binary).resolve()
    destination = Path(destination)
    if os.environ.get('GITHUB_ACTIONS','').lower() == 'true' or os.environ.get('CI','').lower() in ('true','1'):
        fail('native benchmark/release workloads must run locally, not on CI')
    allowed_cargo = {'CARGO_HOME','CARGO_TARGET_DIR','CARGO_TERM_COLOR','CARGO_TERM_PROGRESS_WHEN'}
    overrides = {key:value for key,value in os.environ.items() if value and
                 ((key.startswith('CARGO_') and key not in allowed_cargo) or key in
                  ('RUSTFLAGS','RUSTDOCFLAGS','RUSTC','RUSTDOC','RUSTC_BOOTSTRAP','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER'))}
    cargo_home=Path(os.environ.get('CARGO_HOME',Path.home()/'.cargo'))
    configs=[cargo_home/'config',cargo_home/'config.toml']
    configs += [parent/'.cargo'/name for parent in (root,*root.parents) for name in ('config','config.toml')]
    if any(path.exists() or path.is_symlink() for path in configs): fail('unreviewed Cargo configuration can change build options; use a clean local Cargo configuration')
    if overrides: fail('remove unreviewed Rust build overrides before the release workload run')
    records=licenses(); manifest=records.validate(root)
    policy_text=regular(root/POLICY,128*1024).decode(); rules=policy(parse(policy_text))
    started=dt.datetime.now(dt.timezone.utc).isoformat()
    # A failed new attempt must never leave an older success eligible to ship.
    atomic(destination,encode({'schema':1,'status':'incomplete','started_utc':started}))
    machine=host()
    with tempfile.TemporaryDirectory(prefix='omatainer-workload-') as temporary:
        raw_path=Path(temporary)/'raw.json'
        log_path=destination.with_suffix('.log'); log_path.parent.mkdir(parents=True,exist_ok=True)
        environment=os.environ.copy(); environment['OMATAINER_BENCHMARK_RAW_REPORT']=str(raw_path)
        options=['--locked','--offline','--release','--manifest-path',str(root/'Cargo.toml')]
        with log_path.open('wb') as log:
            compiled=execute(['cargo','build',*options,'--message-format=json'],timeout=rules['timeout_seconds'],limit=16*1024*1024,env=environment,log=log,cwd=root)
            if artifact(compiled,False)!=binary: fail('binary path does not identify the artifact from this controlled release build')
            records.verify_binary(root,binary)
            before=bindings(root,binary,manifest)
            build={'profile':'release','cargo':output(['cargo','--version']),
                   'binary_info':parse(output([str(binary),'benchmark-build-info'])),'environment':{}}
            compiled=execute(['cargo','test',*options,'--no-run','--message-format=json'],timeout=rules['timeout_seconds'],limit=16*1024*1024,env=environment,log=log,cwd=root)
            test_binary=artifact(compiled,True);build['test_binary_sha256']=digest(test_binary)
            native=execute([sys.executable,str(root/'scripts/check-accessibility.py'),'--test-binary',str(test_binary)],timeout=120,limit=1024*1024,env=environment,log=log,cwd=root)
            if native.returncode: fail('required private native accessibility preflight failed; no waiver')
            build['native_accessibility']={'exit_code':0,'report':parse(native.stdout)}
            native_evidence(build['native_accessibility'])
            command=[str(test_binary),rules['test'],'--ignored','--exact','--test-threads=1']
            machine['load_average_before_workload']=list(os.getloadavg())
            result=execute(command,timeout=rules['timeout_seconds'],limit=16*1024*1024,env=environment,log=log,cwd=root)
            machine['load_average_after_workload']=list(os.getloadavg())
            build['test_exit_code']=result.returncode
            if digest(test_binary)!=build['test_binary_sha256']: fail('test executable changed during benchmark')
        raw_bytes=regular(raw_path)
        atomic(destination.with_suffix('.raw.json'),raw_bytes)
        raw=parse(raw_bytes)
        summary,failures=evaluate(raw,rules,manifest['target'],manifest)
        if result.returncode: failures.append(f'benchmark process exited {result.returncode}')
        records.validate(root)
        if bindings(root,binary,manifest)!=before: fail('source or binary changed during benchmark run')
        report={'schema':1,'status':'fail' if failures else 'pass','started_utc':started,
                'finished_utc':dt.datetime.now(dt.timezone.utc).isoformat(),'host':machine,'build':build,
                'bindings':before,'policy_text':policy_text,'raw':raw,'summary':summary,'failures':failures}
        atomic(destination,encode(report))
        check(root,binary,destination,manifest)
        return report

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=('run','check'))
    parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--report',type=Path)
    args=parser.parse_args()
    binary=args.binary or args.root/'target/release/omatainer'; report=args.report or args.root/REPORT
    try:
        (run if args.action=='run' else check)(args.root,binary,report)
        print('Verified local workload evidence:', report)
    except (OSError,ValueError,KeyError,TypeError,subprocess.SubprocessError) as error:
        parser.exit(1,f'Omatainer performance gate: {error}\n')

if __name__ == '__main__': main()
