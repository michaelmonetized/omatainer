#!/usr/bin/env python3
"""Refuse release claims that lose scope, routes or acceptance evidence."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location('matrix', Path(__file__).with_name('capability-matrix.py'))
matrix = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(matrix)
ROOT = Path(__file__).resolve().parent.parent


class CapabilityMatrixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='omat-capability-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copytree(ROOT / 'docs/backlog', self.root / 'docs/backlog')
        registry = self.read('capability-status.json')
        for record in registry['issues'].values():
            for route in record['paths']:
                target = self.root / route['file']
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / route['file'], target)

    def read(self, name):
        return json.loads((self.root / 'docs/backlog' / name).read_text())

    def write(self, name, data):
        (self.root / 'docs/backlog' / name).write_text(json.dumps(data))

    def test_current_scope_keeps_all_229_issues_and_never_closes_exclusions(self):
        text = matrix.render(self.root)
        for number in range(129, 358):
            self.assertIn(f'| [#{number}](https://github.com/michaelmonetized/omatainer/issues/{number})', text)
        for number in self.read('release-scope.json')['excluded_issues']:
            row = next(line for line in text.splitlines() if line.startswith(f'| [#{number}]'))
            self.assertTrue(row.endswith('| excluded from release |'))
        self.assertNotIn('| accepted |', text)

    def test_dropped_duplicate_or_unordered_issue_cannot_silently_disappear(self):
        rows = self.read('remaining-issues.json')
        for changed in [rows[:-1], [rows[1], rows[0], *rows[2:]], [rows[0], *rows[:-1]]]:
            self.write('remaining-issues.json', changed)
            with self.assertRaises(ValueError):
                matrix.render(self.root)

    def test_excluded_provider_cannot_be_promoted_by_capability_record(self):
        registry = self.read('capability-status.json')
        registry['issues']['158'] = registry['issues']['131']
        self.write('capability-status.json', registry)
        with self.assertRaisesRegex(ValueError, 'excluded'):
            matrix.render(self.root)

    def test_empty_or_misspelled_rust_fixture_cannot_be_qualified(self):
        registry = self.read('capability-status.json')
        registry['issues'] = {'131': registry['issues']['131']}
        names = [fixture + '::actual_case' for fixture in registry['issues']['131']['fixtures']]
        matrix.verify_fixture_names(registry, names)
        for changed in [[], names[:-1], [name.replace('deck_load_lock', 'misspelled') for name in names]]:
            with self.assertRaisesRegex(ValueError, 'no compiled tests'):
                matrix.verify_fixture_names(registry, changed)

    def test_missing_or_escaping_route_and_missing_acceptance_evidence_fail(self):
        registry = self.read('capability-status.json')
        original = registry['issues']['131']['paths'][0]['file']
        for path in ['src/nonexistent.rs', '../../outside.rs']:
            registry['issues']['131']['paths'][0]['file'] = path
            self.write('capability-status.json', registry)
            with self.assertRaisesRegex(ValueError, 'source route'):
                matrix.render(self.root)
        registry['issues']['131']['paths'][0]['file'] = original
        registry['issues']['131']['status'] = 'accepted'
        self.write('capability-status.json', registry)
        with self.assertRaisesRegex(ValueError, 'complete evidence'):
            matrix.render(self.root)
        registry['issues']['131']['acceptance'] = 'all criteria passed'
        self.write('capability-status.json', registry)
        with self.assertRaisesRegex(ValueError, 'acceptance evidence'):
            matrix.render(self.root)


if __name__ == '__main__':
    unittest.main()
