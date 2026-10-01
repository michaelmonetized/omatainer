#!/usr/bin/env python3
"""Small strict-report/preflight negative controls; real workflows run separately."""
import copy
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('offline_contract',Path(__file__).with_name('check-offline.py'))
contract=importlib.util.module_from_spec(spec);spec.loader.exec_module(contract)

class Contract(unittest.TestCase):
    def test_guarded_benchmark_socket_uses_short_owned_temp_and_cleanup(self):
        # Match private_directory("ipc") in the unchanged Rust performance
        # fixture, including PID and nanosecond suffix, then actually bind it.
        code = """
import json, os, socket, tempfile, time
from pathlib import Path
root = Path(tempfile.gettempdir())
assert root.parent == Path('/tmp') and root.name.startswith('o103-')
assert root.stat().st_mode & 0o777 == 0o700
name = f'omatainer-performance-{os.getpid()}-ipc-{time.time_ns()}'
directory = root/name
directory.mkdir(mode=0o700)
path = directory/'control.sock'
assert len(os.fsencode(path)) < 108
old = Path(os.environ['OMATAINER_OFFLINE_DIR']).parent/'t'/name/'control.sock'
assert len(os.fsencode(old)) >= 108
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
    listener.bind(str(path)); listener.listen(1)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.connect(str(path)); connection, _ = listener.accept()
        with connection:
            client.sendall(b'benchmark-ipc'); assert connection.recv(13) == b'benchmark-ipc'
(directory/'retained-temp').write_bytes(b'1234567')
print(json.dumps({'temporary':str(root),'bound':True}))
"""
        with tempfile.TemporaryDirectory(prefix='o103test-', dir='/tmp') as root:
            destination=Path(root)/('e'*36)
            run=contract.Run(destination,Path(sys.executable),Path(sys.executable))
            result=run.execute('ipc',[sys.executable,'-c',code],env={'TMPDIR':str(destination/'wrong')})
            report=contract.gate.parse(result.stdout)
            self.assertTrue(report['bound'])
            self.assertFalse(Path(report['temporary']).exists())
            self.assertEqual(run.stages[0]['temporary_bytes'],7)
            self.assertTrue((destination/'00-ipc/guard.json').is_file())
            # Nonzero exit also retires the group before owned TMPDIR cleanup.
            failure="import os; from pathlib import Path; p=Path(os.environ['TMPDIR']); Path(os.environ['OMATAINER_OFFLINE_DIR'],'failed-temp').write_text(str(p)); (p/'file').write_bytes(b'x'); raise SystemExit(7)"
            with self.assertRaisesRegex(ValueError,'failed; see retained'):
                run.execute('failure',[sys.executable,'-c',failure])
            failed=Path((destination/'document/failed-temp').read_text())
            self.assertFalse(failed.exists())

    def test_child_temporary_files_still_count_toward_aggregate_cap(self):
        with tempfile.TemporaryDirectory(prefix='o103test-',dir='/tmp') as root:
            run=contract.Run(Path(root)/'evidence',Path(sys.executable),Path(sys.executable))
            code="import os; from pathlib import Path; p=Path(os.environ['TMPDIR']); Path(os.environ['OMATAINER_OFFLINE_DIR'],'temp-root').write_text(str(p)); (p/'large').write_bytes(b'x'*65536)"
            with mock.patch.object(contract,'CAP',32768), self.assertRaisesRegex(ValueError,'fixed 128 MiB limit'):
                run.execute('cap',[sys.executable,'-c',code])
            self.assertFalse(Path((run.path/'document/temp-root').read_text()).exists())

    def test_receipt_requires_each_measured_denial_and_local_control(self):
        proof=dict(schema=1,direct_network=dict(ipv4_tcp='EPERM',ipv4_udp='EPERM',ipv6_tcp='EPERM',ipv6_udp='EPERM'),
            io_uring_setup='EPERM',effective_uid=os.geteuid(),unix_socketpair=True,unix_listener=True,no_new_privileges=True,seccomp_filter=True)
        contract.guard_receipt(proof)
        for key in proof:
            invalid=copy.deepcopy(proof);invalid.pop(key)
            with self.assertRaises(ValueError):contract.guard_receipt(invalid)
        for key in proof['direct_network']:
            invalid=copy.deepcopy(proof);invalid['direct_network'][key]='not tested'
            with self.assertRaises(ValueError):contract.guard_receipt(invalid)
        invalid=copy.deepcopy(proof);invalid['effective_uid']+=1
        with self.assertRaises(ValueError):contract.guard_receipt(invalid)

    def test_existing_and_symlinked_evidence_destinations_are_preserved(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root); existing=root/'previous';existing.mkdir();(existing/'evidence').write_bytes(b'preserve')
            for destination in [existing, root/'link'/'new']:
                if destination!=existing:(root/'link').symlink_to(existing,target_is_directory=True)
                with self.assertRaises(ValueError):contract.Run(destination,Path(sys.executable),Path(sys.executable))
                self.assertEqual((existing/'evidence').read_bytes(),b'preserve')
                self.assertFalse((existing/'new').exists())

    def test_child_time_and_output_are_bounded(self):
        for code,timeout,limit in [("import time; time.sleep(30)",.05,1024),("print('x'*4096)",2,128)]:
            with self.assertRaises(ValueError):
                contract.execute_private([sys.executable,'-c',code],timeout=timeout,limit=limit,env={},cwd=Path.cwd())

    def test_failed_parent_does_not_leave_its_private_descendant_running(self):
        import time
        with tempfile.TemporaryDirectory() as root:
            marker=Path(root)/'pid'
            child="import os,signal,time; from pathlib import Path; signal.signal(signal.SIGTERM,signal.SIG_IGN); Path("+repr(str(marker))+").write_text(str(os.getpid())); time.sleep(30)"
            parent="import subprocess,time,sys; from pathlib import Path; subprocess.Popen([sys.executable,'-c',"+repr(child)+"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); p=Path("+repr(str(marker))+"); end=time.monotonic()+2;\nwhile not p.exists() and time.monotonic()<end: time.sleep(.005)\nsys.exit(7)"
            result=contract.execute_private([sys.executable,'-c',parent],timeout=3,limit=1024,env={},cwd=Path.cwd())
            self.assertEqual(result.returncode,7)
            pid=int(marker.read_text());end=time.monotonic()+2
            while time.monotonic()<end:
                path=Path(f'/proc/{pid}/stat')
                if not path.exists() or path.read_text().split()[2]=='Z':break
                time.sleep(.005)
            else:self.fail('failed child left an active descendant')

    def test_unchanged_policy_evaluators_remain_the_source_of_thresholds(self):
        policy=contract.gate.policy(contract.gate.parse(contract.gate.regular(contract.REPO/contract.gate.POLICY)))
        self.assertEqual(len(policy['workloads']),8)
        self.assertTrue(all(item['repetitions']==3 for item in policy['workloads']))
        show=contract.gate.parse(contract.gate.regular(contract.show.POLICY))
        contract.show.validate_policy(show)
        self.assertEqual(len(show['rates'])*len(show['frames'])*len(show['ratios']),18)

if __name__=='__main__':unittest.main()
