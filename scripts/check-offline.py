#!/usr/bin/env python3
"""Qualify implemented local workflows under child-only direct-network denial.

Build before running. No host networking, user desktop, audio device, browser or
cloud service is changed. AF_UNIX remains available for private IPC/AT-SPI;
host proxies/remote mounts/audio/display and explicit browser handoffs are not
qualified. Performance uses the unchanged #95/#100 workloads and evaluators.
"""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
import re
import stat
import selectors
import signal
import subprocess
import time
from pathlib import Path
import sys

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
CHILD = 'ui::sampler_editor::tests::offline::document_child'
CAP = 128 * 1024 * 1024


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, REPO/'scripts'/filename)
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value)
    return value


gate = module('offline_performance', 'performance-gate.py')
show = module('offline_show', 'check-keylock-show-load.py')
guard = module('offline_network_guard', 'offline_guard.py')


def write(path, value):
    data = gate.encode(value)
    with Path(path).open('xb') as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(data); stream.flush(); os.fsync(stream.fileno())


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024*1024), b''): result.update(chunk)
    return result.hexdigest()


def guard_receipt(value):
    expected = dict(ipv4_tcp='EPERM', ipv4_udp='EPERM', ipv6_tcp='EPERM', ipv6_udp='EPERM')
    if value.get('schema') != 1 or value.get('direct_network') != expected or value.get('io_uring_setup') != 'EPERM':
        raise ValueError('missing direct-network negative controls')
    if value.get('effective_uid') != os.geteuid() or not all(value.get(key) is True for key in
            ('unix_socketpair', 'unix_listener', 'no_new_privileges', 'seccomp_filter')):
        raise ValueError('missing same-identity/Unix IPC/kernel restriction evidence')


def bindings(binary, test_binary, manifest):
    return dict(production_sha256=digest(binary), test_sha256=digest(test_binary),
        manifest_sha256=digest(REPO/'licenses/manifest.json'), source_files=manifest['source_files'],
        policy_sha256=digest(REPO/gate.POLICY), show_policy_sha256=digest(show.POLICY))


def execute_private(command, *, timeout, limit, env, cwd, log=None):
    """Bound streams/time and retire the whole owned group even on child error."""
    process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,env=env,cwd=cwd,start_new_session=True)
    output={'stdout':bytearray(),'stderr':bytearray()};deadline=time.monotonic()+timeout
    try:
        with selectors.DefaultSelector() as selector:
            for name,pipe in [('stdout',process.stdout),('stderr',process.stderr)]:
                os.set_blocking(pipe.fileno(),False);selector.register(pipe,selectors.EVENT_READ,name)
            while selector.get_map():
                remaining=deadline-time.monotonic()
                if remaining<=0:raise ValueError('offline child exceeded its time limit')
                for key,_ in selector.select(min(.1,remaining)):
                    chunk=os.read(key.fd,65536)
                    if not chunk:selector.unregister(key.fileobj);continue
                    if sum(map(len,output.values()))+len(chunk)>limit:raise ValueError('offline child exceeded its output limit')
                    output[key.data].extend(chunk)
                    if log is not None:log.write(chunk);log.flush()
        remaining=deadline-time.monotonic()
        if remaining<=0:raise ValueError('offline child exceeded its time limit')
        code=process.wait(timeout=remaining)
        return subprocess.CompletedProcess(command,code,bytes(output['stdout']),bytes(output['stderr']))
    finally:
        try:os.killpg(process.pid,signal.SIGTERM)
        except ProcessLookupError:pass
        try:process.wait(timeout=2)
        except subprocess.TimeoutExpired:pass
        try:os.killpg(process.pid,signal.SIGKILL)
        except ProcessLookupError:pass
        process.wait(timeout=2)
        process.stdout.close();process.stderr.close()


class Run:
    def __init__(self, path, binary, test_binary):
        self.path = Path(path).absolute()
        if self.path.exists() or self.path.is_symlink() or '..' in self.path.parts:
            raise ValueError('evidence destination must be a fresh absolute directory')
        for parent in self.path.parents:
            info=parent.lstat(); mode=stat.S_IMODE(info.st_mode)
            if not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0,os.geteuid()) or (mode & 0o022 and not mode & stat.S_ISVTX):
                raise ValueError('evidence ancestors must be trusted directories without symlinks')
        self.path.mkdir(mode=0o700)
        guard.private_directory(self.path)
        self.binary = binary.resolve(strict=True); self.tests = test_binary.resolve(strict=True)
        self.stages = []
        for name in ('t', 'runtime', 'config', 'data', 'state', 'cache', 'document'):
            (self.path/name).mkdir(mode=0o700)
        self.environment = dict(os.environ, TMPDIR=str(self.path/'t'),
            XDG_RUNTIME_DIR=str(self.path/'runtime'), XDG_CONFIG_HOME=str(self.path/'config'),
            XDG_DATA_HOME=str(self.path/'data'), XDG_STATE_HOME=str(self.path/'state'),
            XDG_CACHE_HOME=str(self.path/'cache'), OMATAINER_OFFLINE_CHILD='1',
            OMATAINER_OFFLINE_SENTINEL='OMATAINER_OFFLINE_CREDENTIAL_SENTINEL_7e31')
        self.environment['OMATAINER_OFFLINE_DIR'] = str(self.path/'document')

    def bounded(self):
        size = sum(file.stat().st_size for file in self.path.rglob('*') if file.is_file())
        if size > CAP: raise ValueError('offline evidence exceeded its fixed 128 MiB limit')
        return size

    def execute(self, name, command, *, env=None, timeout=120, limit=8*1024*1024):
        self.bounded()
        folder = self.path/f'{len(self.stages):02}-{name}'; folder.mkdir(mode=0o700)
        receipt = folder/'guard.json'
        write(folder/'attempt.json',dict(name=name,timeout_seconds=timeout,output_limit=limit))
        with (folder/'stream.log').open('xb') as log:
            os.fchmod(log.fileno(),0o600)
            completed = execute_private([sys.executable, str(REPO/'scripts/offline_guard.py'),
                '--receipt', str(receipt), '--', *map(str, command)], timeout=timeout,
                limit=limit, env=dict(self.environment, **(env or {})), cwd=REPO,log=log)
        write(folder/'output.json', dict(exit_code=completed.returncode,
            stdout=completed.stdout.decode(errors='replace'), stderr=completed.stderr.decode(errors='replace')))
        proof = gate.parse(gate.regular(receipt, 16*1024)); guard_receipt(proof)
        self.stages.append(dict(name=name, guard=str(receipt.relative_to(self.path)),
            guard_sha256=digest(receipt), exit_code=completed.returncode,
            output=str((folder/'output.json').relative_to(self.path))))
        self.bounded()
        if completed.returncode: raise ValueError(f'{name} failed; see retained output.json')
        return completed

    def test(self, name, filter, *, ignored=False, env=None):
        command = [self.tests, filter, '--test-threads=1']
        if ignored: command += ['--exact', '--ignored', '--nocapture']
        result = self.execute(name, command, env=env)
        text = result.stdout.decode()
        matched=re.search(r'test result: ok\. (\d+) passed;',text)
        if matched is None or int(matched.group(1)) == 0:
            raise ValueError(f'{name} did not run its required tests')
        return result

    def functional(self, manifest):
        self.execute('cold-safe-start', [sys.executable, REPO/'scripts/check-safe-startup.py', '--binary', self.binary])
        self.test('local-document-write', CHILD, ignored=True, env={'OMATAINER_OFFLINE_STAGE':'prepare'})
        # A second test process must use embedded audio. Deletion is limited to
        # the WAV created by the first private process; never arbitrary sources.
        source = self.path/'document/sample.wav'
        source.unlink()
        self.test('local-document-reopen', CHILD, ignored=True, env={'OMATAINER_OFFLINE_STAGE':'reopen'})
        documents = [gate.parse(gate.regular(self.path/f'document/{stage}.json', 4*1024*1024))
                     for stage in ('prepare', 'reopen')]
        for value, stage in zip(documents, ('prepare','reopen')):
            if value['stage'] != stage or value['embedded_manifest'] != manifest or value['direct_ipv4_socket'] != 'EPERM':
                raise ValueError('stale binary or invalid fresh-process document receipt')
        if documents[0]['state'] != documents[1]['state']:
            raise ValueError('fresh process changed persisted embedded state')
        self.test('safe-document-reopen','ui::support::tests::offline_safe_document_child',ignored=True)
        safe=gate.parse(gate.regular(self.path/'document/safe_reopen.json',4*1024*1024))
        if safe.get('embedded_manifest')!=manifest or safe.get('safe_mode') is not True or safe.get('callbacks')!=0 or safe.get('saved_copy') is not True or safe.get('source_absent') is not True or safe.get('embedded_bank_pcm')!=documents[0]['state']['bank_pcm']:
            raise ValueError('safe owner did not preserve embedded local media without backend callbacks')

        # Real existing UI/worker suites include failures, cancellation, local
        # reference resolution, and save/overwrite guards. No alternate loader.
        for name, filter in [('ui-workflows','ui::'), ('support','support::'),
                             ('native-codec','project_file::tests::')]:
            self.test(name, filter)
        normal = self.execute('native-ui', [sys.executable, REPO/'scripts/check-accessibility.py', '--test-binary', self.tests])
        gate.native_evidence(dict(exit_code=0, report=gate.parse(normal.stdout)))
        support = self.execute('native-support', [sys.executable, REPO/'scripts/check-accessibility.py', '--test-binary', self.tests, '--support'])
        return dict(fresh_process_embedded_state=True, original_media_removed=True, safe_owner_reopen_save=True,
            native_ui=gate.parse(normal.stdout), native_support=gate.parse(support.stdout))

    def performance(self, manifest):
        info = self.execute('release-profile', [self.binary, 'benchmark-build-info'])
        info = gate.parse(info.stdout)
        if info.get('debug_assertions') is not False: raise ValueError('timing requires an optimized release build')
        rules = gate.policy(gate.parse(gate.regular(REPO/gate.POLICY,128*1024)))
        native = self.execute('timing-native-preflight', [sys.executable, REPO/'scripts/check-accessibility.py', '--test-binary', self.tests])
        gate.native_evidence(dict(exit_code=0, report=gate.parse(native.stdout)))
        raw_path = self.path/'performance.raw.json'
        machine = gate.host()
        self.execute('full-performance', [self.tests,rules['test'],'--ignored','--exact','--test-threads=1'],
            env={'OMATAINER_BENCHMARK_RAW_REPORT':str(raw_path)}, timeout=rules['timeout_seconds'],limit=16*1024*1024)
        machine['load_average_after_workload'] = list(os.getloadavg())
        raw = gate.parse(gate.regular(raw_path))
        summary, failures = gate.evaluate(raw,rules,manifest['target'],manifest)
        observed = []
        for row in raw['workloads']:
            rate, frames = row['conditions'].get('sample_rate'), row['conditions'].get('frames')
            if rate and frames:
                deadline = frames*1_000_000_000//rate
                observed.append(dict(id=row['id'],deadline_ns=deadline,repeats=[{
                    name:sum(value>deadline for value in sample) for name,sample in measured['samples'].items()
                    if name in ('callback_wall_ns','render_cpu_ns')}
                    for measured in row['measurements']]))
        write(self.path/'performance.verified.json',dict(summary=summary,failures=failures,
            host=machine,policy_sha256=digest(REPO/gate.POLICY),raw_sha256=digest(raw_path),actual_deadline_exceedances=observed))
        if failures: raise ValueError('unchanged #95 performance policy failed')
        policy = gate.parse(gate.regular(show.POLICY,128*1024))
        show_dir = self.path/'keylock'
        machine = gate.host()
        self.execute('keylock-show', [self.tests,policy['test'],'--ignored','--exact','--test-threads=1'],
            env={'OMATAINER_KEYLOCK_EVIDENCE_ROOT':str(self.path), 'OMATAINER_KEYLOCK_SHOW_OUT':str(show_dir)},
            timeout=policy['timeout_seconds'],limit=2*1024*1024)
        machine['load_average_after_workload'] = list(os.getloadavg())
        raw = gate.parse(gate.regular(show_dir/'raw.json',32*1024*1024))
        summary, failures = show.evaluate(raw,policy,manifest)
        write(self.path/'keylock.verified.json',dict(summary=summary,failures=failures,host=machine,
            policy_sha256=digest(show.POLICY),raw_sha256=digest(show_dir/'raw.json')))
        if failures: raise ValueError('unchanged #100 show-load policy failed')
        return dict(workload_groups=8,show_groups=18,repetitions=3,backend_xruns=None,
            physical_device_qualification=False,unmodified_policies=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=('functional','performance','all'))
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--test-binary',required=True,type=Path)
    parser.add_argument('--out',required=True,type=Path,help='Fresh private directory; choose a short path for Unix IPC')
    args = parser.parse_args()
    run = None
    try:
        records = gate.licenses(); manifest = records.validate(REPO)
        records.verify_binary(REPO,args.binary.resolve())
        before = bindings(args.binary,args.test_binary,manifest)
        run = Run(args.out,args.binary,args.test_binary)
        result = dict(schema=1,started_utc=dt.datetime.now(dt.timezone.utc).isoformat(),
            mode=args.action,bindings=before,scope=__doc__,backend_xruns=None)
        if args.action in ('functional','all'): result['functional'] = run.functional(manifest)
        if args.action in ('performance','all'): result['performance'] = run.performance(manifest)
        records.validate(REPO)
        if bindings(args.binary,args.test_binary,manifest) != before:
            raise ValueError('source/executable changed during qualification')
        result.update(status='pass',stages=run.stages,artifact_bytes=run.bounded(),
            finished_utc=dt.datetime.now(dt.timezone.utc).isoformat())
        write(run.path/'report.json',result)
        print(json.dumps(dict(status='pass',mode=args.action,stages=len(run.stages),report=str(run.path/'report.json'))))
    except (OSError,ValueError,AssertionError,KeyError,TypeError,subprocess.SubprocessError) as error:
        if run is not None:
            write(run.path/'failure.json',dict(schema=1,status='fail',message=str(error)[:2048],stages=run.stages))
        print('offline qualification failed: '+str(error),file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
