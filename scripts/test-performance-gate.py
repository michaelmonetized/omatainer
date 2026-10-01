#!/usr/bin/env python3
"""Private synthetic validator cases and real bounded subprocess lifecycle tests."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import time
import unittest

sys.dont_write_bytecode=True
SPEC=importlib.util.spec_from_file_location('fixture',Path(__file__).with_name('performance-test-support.py'))
fixture=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(fixture)
gate=fixture.gate

class PerformanceTests(unittest.TestCase):
    def test_quantiles_recompute_from_every_raw_sample_in_each_fresh_repetition(self):
        self.assertEqual(gate.distribution([30,10,20]),{'min':10,'p50':20,'p95':30,'p99':30,'max':30,'spread':20,'p99_minus_p50':10})
        self.assertEqual(gate.distribution(list(range(1,101)))['p99'],99)
        rules=fixture.rules();raw=fixture.raw(rules)
        summary,failures=gate.evaluate(raw,rules,'x86_64-unknown-linux-gnu')
        self.assertEqual(failures,[]);self.assertEqual(len(summary),8)
        raw['workloads'][3]['measurements'][2]['samples']['frame_wall_ns'][0]=31
        _,failures=gate.evaluate(raw,rules,'x86_64-unknown-linux-gnu')
        self.assertTrue(any('callback_hybrid[2]' in failure for failure in failures))
    def test_missing_extra_duplicate_nonfinite_and_wrong_raw_counts_fail_closed(self):
        for mutate in [lambda v:v['workloads'].pop(),lambda v:v['workloads'].reverse(),
                       lambda v:v['workloads'][0]['measurements'].pop(),
                       lambda v:v['workloads'][0]['measurements'][0]['samples']['frame_wall_ns'].append(1),
                       lambda v:v['workloads'][0]['measurements'][0]['samples']['frame_wall_ns'].__setitem__(0,True),
                       lambda v:v['workloads'][0]['measurements'][0]['metrics'].__setitem__('allocations',float('nan')),
                       lambda v:v['workloads'][0]['conditions'].__setitem__('extra',1),
                       lambda v:v['workloads'][0]['measurements'][0].__setitem__('summary',{'p99':0})]:
            raw=fixture.raw();mutate(raw)
            with self.assertRaises(gate.GateError):gate.evaluate(raw,fixture.rules(),'x86_64-unknown-linux-gnu')
        for value in [b'{"schema":1,"schema":1}',b'{"x":NaN}',b'{"x":Infinity}']:
            with self.assertRaises(gate.GateError):gate.parse(value)
    def test_allocations_golden_hashes_and_checks_are_independently_required(self):
        for field,value in [('metrics',{'allocations':1}),('checks',{'finite_output':False}),('observations',{'state_hash':'wrong'})]:
            raw=fixture.raw();raw['workloads'][0]['measurements'][0][field]=value
            self.assertTrue(gate.evaluate(raw,fixture.rules(),'x86_64-unknown-linux-gnu')[1])
        rules=fixture.rules();rules['status']='draft'
        with self.assertRaisesRegex(gate.GateError,'reviewed'):gate.evaluate(fixture.raw(),rules,'x86_64-unknown-linux-gnu')
    def test_report_identity_tampering_and_forged_summary_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'licenses').mkdir();(root/'Cargo.lock').write_text('private')
            binary=root/'binary';binary.write_bytes(b'private test artifact')
            manifest={'source_files':{'Cargo.lock':gate.digest(root/'Cargo.lock')},'toolchain':{'release':'fixture'},'target':'x86_64-unknown-linux-gnu','application':'0.1'}
            (root/'licenses/manifest.json').write_bytes(gate.encode(manifest));(root/'licenses/notices.json').write_text('{}')
            good=fixture.report(root,binary);manifest=json.loads((root/'licenses/manifest.json').read_text())
            def verify(value):return gate.verify(value,manifest,gate.digest(binary),gate.digest(root/'licenses/manifest.json'),gate.digest(root/'licenses/notices.json'))
            verify(good)
            for mutate in [lambda v:v['bindings'].__setitem__('binary_sha256','0'*64),
                           lambda v:v['bindings']['source_files'].__setitem__('Cargo.lock','wrong'),
                           lambda v:v['raw']['embedded_manifest']['source_files'].__setitem__('Cargo.lock','stale test source'),
                           lambda v:v['raw'].pop('embedded_manifest'),
                           lambda v:v['summary'][0]['measurements'][0]['distributions']['frame_wall_ns'].__setitem__('p99',0),
                           lambda v:v['build']['binary_info'].__setitem__('debug_assertions',True),
                           lambda v:v['build'].__setitem__('profile','dev'),
                           lambda v:v['build'].__setitem__('native_accessibility',{'exit_code':1,'report':{}}),
                           lambda v:v['build']['native_accessibility']['report'].__setitem__('actions',[{'action':'Click'}]),
                           lambda v:v['build'].__setitem__('test_binary_sha256','z'*64),
                           lambda v:v['host'].__setitem__('execution','github-actions')]:
                altered=copy.deepcopy(good);mutate(altered)
                with self.assertRaises(gate.GateError):verify(altered)
    def test_subprocess_output_deadlines_descendants_and_cancellation_are_bounded(self):
        start=time.monotonic()
        with self.assertRaisesRegex(gate.GateError,'output limit'):
            gate.execute([sys.executable,'-c','import os;os.write(1,b"x"*65536)'],limit=1024)
        with tempfile.TemporaryDirectory() as directory:
            pid=Path(directory)/'pid'
            program='import subprocess,time,pathlib,sys; p=subprocess.Popen([sys.executable,"-c","import time;time.sleep(20)"]); pathlib.Path(sys.argv[1]).write_text(str(p.pid)); time.sleep(20)'
            with self.assertRaisesRegex(gate.GateError,'deadline'):
                gate.execute([sys.executable,'-c',program,str(pid)],timeout=.15)
            child=int(pid.read_text())
            # A killed grandchild may briefly be an adopted zombie, never running.
            state=Path(f'/proc/{child}/stat')
            if state.exists():self.assertEqual(state.read_text().split()[2],'Z')
        self.assertLess(time.monotonic()-start,3)
    def test_atomic_failed_attempt_replaces_prior_success_without_partial_json(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'report.json'
            gate.atomic(path,gate.encode({'status':'pass'}));gate.atomic(path,gate.encode({'status':'incomplete'}))
            self.assertEqual(gate.parse(path.read_bytes()),{'status':'incomplete'})
            self.assertEqual(list(path.parent.glob('.performance-*')),[])

if __name__=='__main__':unittest.main()
