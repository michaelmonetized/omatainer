#!/usr/bin/env python3
"""Lightweight driver contracts, not native/media qualification.

These controlled child/records substitutes exercise orchestration and bindings.
The actual ignored Rust App workload remains the long-file acceptance proof.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


driver = module('analysis_driver_contract', 'check-analysis.py')
guard = module('analysis_guard_contract', 'offline_guard.py')


class DriverContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='omatainer-analysis-driver-')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / 'evidence'
        self.root.mkdir(mode=0o700)
        (self.root / 'sources').mkdir(mode=0o700)
        self.checkout = self.base / 'checkout'
        (self.checkout / 'licenses').mkdir(parents=True)
        self.inventory = {'schema': 1, 'source_files': {'controlled': 'source-digest'}}
        self.manifest = self.checkout / 'licenses/manifest.json'
        self.manifest.write_text(json.dumps(self.inventory))
        self.binary = self.base / 'test-binary'
        self.binary.write_bytes(b'controlled executable identity')
        rows = []
        for name in driver.NAMES:
            source = self.root / 'sources' / name
            source.write_bytes(name.encode())
            rows.append(dict(name=name, bytes=source.stat().st_size,
                             file_sha256=driver.digest(source), sample_rate=48000,
                             channels=2, requested_seconds=180.0))
        driver.write(self.root / 'source-manifest.json', dict(schema=1, sources=rows))
        self.counts = dict(loads=0, validates=0, children=0)
        self.scenario = 'valid'
        self.offline = types.SimpleNamespace(
            gate=types.SimpleNamespace(licenses=self.licenses),
            execute_private=self.execute)
        self.root_patch = patch.object(driver, 'ROOT', self.checkout)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def licenses(self):
        self.counts['loads'] += 1
        return types.SimpleNamespace(validate=self.validate_records)

    def validate_records(self, checkout):
        self.assertEqual(checkout, self.checkout)
        self.counts['validates'] += 1

    def execute(self, command, **kwargs):
        self.counts['children'] += 1
        self.assertEqual(command, [str(self.binary), driver.TEST, '--exact',
                                  '--ignored', '--test-threads=1', '--nocapture'])
        self.assertEqual(kwargs['timeout'], 900)
        self.assertEqual(kwargs['limit'], 8 * 1024 * 1024)
        self.assertEqual(kwargs['cwd'], self.checkout)
        self.assertFalse(kwargs['log'].closed)
        self.assertEqual(kwargs['env']['OMATAINER_ANALYSIS_LONG_DIR'], str(self.root))
        for key in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR', 'TMPDIR'):
            private = Path(kwargs['env'][key])
            self.assertEqual(private.parent, self.root)
            guard.private_directory(private)
        (self.root / 'run').mkdir()
        report = dict(schema=1, status='pass',
                      embedded_manifest={} if self.scenario == 'stale' else self.inventory)
        driver.write(self.root / 'run/report.json', report)
        if self.scenario == 'source':
            (self.root / 'sources' / driver.NAMES[0]).write_bytes(b'changed source')
        if self.scenario == 'executable':
            self.binary.write_bytes(b'changed executable')
        if self.scenario == 'inventory':
            self.manifest.write_text('{"schema":1,"changed":true}')
        if self.scenario == 'corpus-manifest':
            (self.root / 'source-manifest.json').write_text('{"schema":1,"sources":[]}')
        code = 1 if self.scenario == 'failed' else 0
        stdout = b'test result: ok. 0 passed;' if self.scenario == 'zero-tests' else b'test result: ok. 1 passed;'
        return subprocess.CompletedProcess(command, code, stdout, b'')

    def run_driver(self):
        driver.run(self.root, self.binary, self.offline, guard)

    def test_local_license_loader_is_cached_and_all_success_bindings_are_retained(self):
        # The original driver mistakenly used nonexistent offline.records.
        self.assertFalse(hasattr(self.offline, 'records'))
        self.run_driver()
        self.assertEqual(self.counts, dict(loads=1, validates=2, children=1))
        proof = json.loads((self.root / 'qualification.json').read_text())
        self.assertEqual(proof['status'], 'pass')
        self.assertEqual(proof['bindings']['test_binary_sha256'], driver.digest(self.binary))
        self.assertEqual(proof['bindings']['source_inventory_sha256'], driver.digest(self.manifest))
        self.assertEqual(proof['bindings']['source_manifest_sha256'],
                         driver.digest(self.root / 'source-manifest.json'))
        self.assertEqual(proof['child_report_sha256'], driver.digest(self.root / 'run/report.json'))

    def test_failed_or_empty_child_never_creates_a_passing_qualification(self):
        for scenario in ('failed', 'zero-tests'):
            with self.subTest(scenario=scenario):
                self.scenario = scenario
                with self.assertRaisesRegex(ValueError, 'fixture failed'):
                    self.run_driver()
                self.assertFalse((self.root / 'qualification.json').exists())
                self.assertTrue((self.root / 'child.log').is_file())
                if scenario == 'failed':
                    # A failed run remains evidence and cannot be retried in place.
                    with self.assertRaisesRegex(ValueError, 'existing run evidence'):
                        self.run_driver()
                    self.tearDown_case_outputs()

    def tearDown_case_outputs(self):
        # Test-only reset of this fixture's own generated run artifacts.
        import shutil
        for name in ('run', 'config', 'data', 'state', 'cache', 'runtime', 'tmp'):
            shutil.rmtree(self.root / name)
        (self.root / 'child.log').unlink()

    def test_stale_embedded_manifest_is_rejected(self):
        self.scenario = 'stale'
        with self.assertRaisesRegex(ValueError, 'different source inventory'):
            self.run_driver()
        self.assertFalse((self.root / 'qualification.json').exists())

    def test_mutation_of_each_bound_input_is_rejected(self):
        for scenario in ('source', 'executable', 'inventory', 'corpus-manifest'):
            with self.subTest(scenario=scenario):
                old_binary = self.binary.read_bytes()
                old_inventory = self.manifest.read_bytes()
                old_manifest = (self.root / 'source-manifest.json').read_bytes()
                source = self.root / 'sources' / driver.NAMES[0]
                old_source = source.read_bytes()
                self.scenario = scenario
                with self.assertRaises(ValueError):
                    self.run_driver()
                self.assertFalse((self.root / 'qualification.json').exists())
                self.tearDown_case_outputs()
                self.binary.write_bytes(old_binary)
                self.manifest.write_bytes(old_inventory)
                (self.root / 'source-manifest.json').write_bytes(old_manifest)
                source.write_bytes(old_source)

    def test_extra_directory_file_or_symlink_is_not_an_unbound_scanned_input(self):
        extra = self.root / 'sources/unmanifested.wav'
        for kind in ('file', 'directory', 'symlink'):
            with self.subTest(kind=kind):
                if kind == 'file':
                    extra.write_bytes(b'extra source')
                elif kind == 'directory':
                    extra.mkdir()
                else:
                    extra.symlink_to(self.binary)
                with self.assertRaisesRegex(ValueError, 'unmanifested entry'):
                    self.run_driver()
                self.assertEqual(self.counts['children'], 0)
                if kind == 'directory':
                    extra.rmdir()
                else:
                    extra.unlink()

    def test_missing_known_file_or_known_name_symlink_is_rejected(self):
        source = self.root / 'sources' / driver.NAMES[0]
        source.unlink()
        with self.assertRaisesRegex(ValueError, 'missing a generated'):
            self.run_driver()
        source.symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, 'owned regular media'):
            self.run_driver()
        self.assertEqual(self.counts['children'], 0)

    def test_known_source_requires_ownership_and_total_size_bound_before_hashing(self):
        source = self.root / 'sources' / driver.NAMES[0]
        original = Path.lstat
        for field, value in (('st_uid', os.geteuid() + 1), ('st_size', driver.MAX_SOURCES + 1)):
            def changed(path, *args, **kwargs):
                metadata = original(path, *args, **kwargs)
                if path == source:
                    values = dict(st_mode=metadata.st_mode, st_uid=metadata.st_uid,
                                  st_size=metadata.st_size)
                    values[field] = value
                    return types.SimpleNamespace(**values)
                return metadata
            with self.subTest(field=field), patch.object(Path, 'lstat', changed):
                with self.assertRaisesRegex(ValueError, 'owned regular media'):
                    self.run_driver()
            self.assertEqual(self.counts['children'], 0)


if __name__ == '__main__':
    unittest.main()
