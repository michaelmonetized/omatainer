#!/usr/bin/env python3
"""Small strict-report/preflight negative controls; real workflows run separately."""
import copy
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('offline_contract',Path(__file__).with_name('check-offline.py'))
contract=importlib.util.module_from_spec(spec);spec.loader.exec_module(contract)

class Contract(unittest.TestCase):
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
