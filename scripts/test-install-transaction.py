#!/usr/bin/env python3
"""Exercise installation in temporary user roots; desktop commands are isolated stubs."""

import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "install_transaction", Path(__file__).with_name("install-transaction.py")
)
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)
FIXTURE_SPEC=importlib.util.spec_from_file_location('performance_fixture',Path(__file__).with_name('performance-test-support.py'))
performance_fixture=importlib.util.module_from_spec(FIXTURE_SPEC);FIXTURE_SPEC.loader.exec_module(performance_fixture)
REPOSITORY = Path(__file__).resolve().parent.parent
CARGO_METADATA = subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', json.loads((REPOSITORY/'licenses/manifest.json').read_text())['target'], '--manifest-path', str(REPOSITORY/'Cargo.toml')], check=True, capture_output=True).stdout


class InstallerTransactionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="omatainer-transaction-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "user"
        self.root.mkdir()
        self.state = self.base / "journals"
        self.source = self.base / "source"
        self.source.mkdir()
        for name in ("contrib", "plugin"):
            shutil.copytree(REPOSITORY / name, self.source / name)
        for name in ('Cargo.toml', 'Cargo.lock', 'LICENSE'):
            shutil.copy2(REPOSITORY/name, self.source/name)
        shutil.copytree(REPOSITORY/'licenses', self.source/'licenses')
        manifest = json.loads((self.source/'licenses/manifest.json').read_text())
        for name in manifest['source_files']:
            destination = self.source/name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPOSITORY/name, destination)
        (self.source/'metadata.json').write_bytes(CARGO_METADATA)
        self.new = self.binary("new", 2)
        destination = self.source / "target/release/omatainer"
        destination.parent.mkdir(parents=True)
        shutil.copy2(self.new, destination)
        performance_fixture.report(self.source,destination)
        self.write(installer.HYPR, '-- unrelated window configuration\nrequire("hypr.apps.synchro")\n', 0o640)
        self.write(installer.BINDINGS, '-- custom bindings\no.bind("SUPER + X", "custom", "true")\n')
        self.write(installer.SHELL, json.dumps({
            "plugins": [{"id": "custom", "enabled": False}, "future-format"],
            "bar": {"layout": {"right": [{"id": "omarchy.audio"}, "keep-me"]}},
            "unrelated": {"value": [1, 2, 3]},
        }) + '\n', 0o600)
        self.write(installer.MENU, '// user menu comment\n{\n  "custom": {"url":"https://example.test/a//b",},\n}\n')
        self.write("unrelated/nested/data", "unchanged bytes\n", 0o600)
        self.commands = self.base / "commands"
        self.commands.mkdir()
        self.log = self.base / "commands.jsonl"
        stub = '''#!{python}
import json, os, pathlib, sys, time
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name == 'cargo' and args[:1] == ['metadata']:
    source = pathlib.Path(args[args.index('--manifest-path')+1]).parent
    print((source/'metadata.json').read_text()); sys.exit(0)
log = pathlib.Path(os.environ["OMATAINER_TEST_LOG"])
old = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
with log.open("a") as stream: stream.write(json.dumps([name, *args]) + "\\n")
if name == "hyprctl":
    responses = json.loads(os.environ.get("OMATAINER_TEST_HYPR_RESPONSES", "{{}}"))
    sequence = responses.get(args[0], [])
    index = old.count([name, *args])
    if index < len(sequence):
        response = sequence[index]
        if "ready" in response:
            pathlib.Path(response["ready"]).touch()
            deadline = time.monotonic() + 10
            while not pathlib.Path(response["wait_for"]).exists():
                if time.monotonic() >= deadline: sys.exit(98)
                time.sleep(0.01)
        print(response.get("stdout", ""), end="")
        print(response.get("stderr", ""), end="", file=sys.stderr)
        sys.exit(response.get("status", 0))
failure = os.environ.get("OMATAINER_TEST_FAILURE", "")
if (failure == "build" and name == "cargo") or (failure == "plugin" and name == "omarchy"):
    print("injected external validation failure", file=sys.stderr); sys.exit(23)
if failure == "reload" and [name, *args] == ["hyprctl", "reload"] and ["hyprctl", "reload"] not in old:
    print("injected reload failure", file=sys.stderr); sys.exit(24)
if failure == "errors" and [name, *args] == ["hyprctl", "configerrors"] and old.count(["hyprctl", "reload"]) == 1:
    print("injected installed configuration error")
if failure == "cache" and name in ("update-desktop-database", "gtk-update-icon-cache"):
    print("injected optional cache failure", file=sys.stderr); sys.exit(25)
'''.format(python=sys.executable)
        for name in ("cargo", "omarchy", "hyprctl", "update-desktop-database", "gtk-update-icon-cache"):
            path = self.commands / name
            path.write_text(stub)
            path.chmod(0o755)
        self.environment = patch.dict(os.environ, {
            "PATH": str(self.commands) + os.pathsep + os.environ["PATH"],
            "OMATAINER_TEST_LOG": str(self.log),
            "OMATAINER_TEST_FAILURE": "",
            "OMATAINER_TEST_HYPR_RESPONSES": "{}",
            "PYTHONUNBUFFERED": "1",
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def binary(self, name, result):
        source = self.base / f"{name}.c"
        source.write_text('#include <stdio.h>\n#include <string.h>\n' +
            'int main(int argc, char **argv) { if(argc>1 && !strcmp(argv[1],"omatainer:offline-license-records:v1"))return 7; if(argc==3 && !strcmp(argv[1],"licenses")) { const char *p = NULL;' +
            'if(!strcmp(argv[2],"--manifest")) p = '+json.dumps(str(self.source/'licenses/manifest.json'))+';' +
            'if(!strcmp(argv[2],"--notices")) p = '+json.dumps(str(self.source/'licenses/notices.json'))+';' +
            'if(!p)return 8;FILE *f=fopen(p,"rb");if(!f)return 9;int c;while((c=fgetc(f))!=EOF)putchar(c);fclose(f);return 0;}' +
            f'return {result};' + '}\n')
        destination = self.base / name
        subprocess.run(["cc", str(source), "-o", str(destination)], check=True, capture_output=True)
        return destination

    def write(self, relative, contents, mode=0o644):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        path.chmod(mode)

    def prior_install(self):
        path = self.root / installer.BINARY
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(self.binary("old", 1), path)
        path.chmod(0o751)
        shutil.copy2(self.binary("older", 0), self.root / installer.PREVIOUS)
        self.write(next(iter(installer.STATIC)), "old desktop artifact\n", 0o640)
        self.write(installer.PLUGIN + "/manifest.json", '{"id":"prior-plugin"}\n')
        self.write(installer.PLUGIN + "/custom/data.txt", "unrelated plugin data\n", 0o600)

    def tree(self):
        result = {}
        for path in sorted(self.root.rglob("*")):
            relative = str(path.relative_to(self.root))
            mode = stat.S_IMODE(path.lstat().st_mode)
            if path.is_symlink():
                result[relative] = ("link", os.readlink(path), mode)
            elif path.is_dir():
                result[relative] = ("directory", mode)
            else:
                result[relative] = ("file", path.read_bytes(), mode)
        return result

    def install(self, after_mutation=None):
        return installer.install(self.source, self.root, self.state, after_mutation)

    def test_missing_stale_or_failing_performance_evidence_never_changes_the_desktop(self):
        self.prior_install()
        self.write(installer.licenses.gate.INSTALLED,'previous retained evidence')
        original=self.tree();report=self.source/installer.licenses.gate.REPORT;prior=report.read_bytes()
        for failure in ('missing','stale','failed'):
            with self.subTest(failure=failure):
                if failure=='missing':report.unlink()
                else:
                    value=json.loads(prior)
                    if failure=='stale':value['bindings']['binary_sha256']='0'*64
                    else:value['raw']['workloads'][0]['measurements'][0]['checks']['finite_output']=False
                    report.write_text(json.dumps(value))
                with self.assertRaises(installer.InstallError):self.install()
                self.assertEqual(self.tree(),original)
                report.write_bytes(prior)

    def test_success_is_idempotent_preserves_unrelated_data_and_retains_recoverable_backups(self):
        self.prior_install()
        original = self.tree()
        journal, warnings = self.install()
        self.assertEqual(warnings, [])
        first = self.tree()
        self.assertEqual((self.root / installer.BINARY).read_bytes(), self.new.read_bytes())
        for installed, source in installer.licenses.INSTALLED.items():
            self.assertEqual((self.root / installed).read_bytes(), (self.source / source).read_bytes())
        self.assertTrue((self.root / installer.licenses.RECEIPT).is_file())
        installer.licenses.verify_package(journal.parent / "release")

        self.assertEqual((self.root / installer.PREVIOUS).read_bytes(), original[installer.BINARY][1])
        self.assertEqual((self.root / installer.HYPR).stat().st_mode & 0o777, 0o640)
        self.assertEqual((self.root / installer.SHELL).stat().st_mode & 0o777, 0o600)
        self.assertEqual(first["unrelated/nested/data"], original["unrelated/nested/data"])
        extra = installer.PLUGIN + "/custom/data.txt"
        self.assertEqual(first[extra], original[extra])
        self.assertEqual((journal.parent / "new" / extra).read_text(), "unrelated plugin data\n")
        shell = json.loads((self.root / installer.SHELL).read_text())
        self.assertEqual(shell["unrelated"], {"value": [1, 2, 3]})
        self.assertIn("keep-me", shell["bar"]["layout"]["right"])
        menu = (self.root / installer.MENU).read_text()
        self.assertIn("// user menu comment", menu)
        self.assertIn('"url":"https://example.test/a//b"', menu)
        backup = json.loads(journal.read_text())
        self.assertEqual(backup["state"], "committed")
        self.assertEqual((journal.parent / "old" / installer.BINARY).read_bytes(), original[installer.BINARY][1])
        self.install()
        second = self.tree()
        # Each successful install advances the executable rollback slot.
        first.pop(installer.PREVIOUS)
        second.pop(installer.PREVIOUS)
        self.assertEqual(first, second)

    def test_failure_at_every_target_mutation_restores_files_modes_and_new_directories(self):
        # Learn actual publication/mkdir boundaries from a successful run.
        original = self.tree()
        events = []
        journal, _ = self.install(events.append)
        installer.recover(journal, self.root)
        self.assertEqual(self.tree(), original)
        self.assertGreater(len(events), len(installer.STATIC))
        for boundary in range(len(events)):
            with self.subTest(boundary=events[boundary]):
                visited = []

                def fail_at(event):
                    visited.append(event)
                    if len(visited) - 1 == boundary:
                        raise OSError(f"injected boundary {boundary}: {event}")

                with self.assertRaisesRegex(installer.InstallError, "injected boundary"):
                    self.install(fail_at)
                self.assertEqual(self.tree(), original)
                self.assertEqual(visited[-1], events[boundary])

    def test_failure_after_binary_publication_restores_both_existing_versions(self):
        self.prior_install()
        original = self.tree()

        def fail(event):
            if event == "publish:binary-and-previous":
                raise OSError("injected after executable publication")

        with self.assertRaisesRegex(installer.InstallError, "after executable"):
            self.install(fail)
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_build_stage_validation_reload_and_config_errors_restore_exact_prior_tree(self):
        self.prior_install()
        original = self.tree()
        for failure in ("build", "plugin", "reload", "errors"):
            with self.subTest(failure=failure), patch.dict(os.environ, {"OMATAINER_TEST_FAILURE": failure}):
                self.log.unlink(missing_ok=True)
                with self.assertRaisesRegex(installer.InstallError, "injected"):
                    self.install()
                self.assertEqual(self.tree(), original)
                self.assertEqual(list(self.state.glob("install-*")), [])

    def test_invalid_generated_config_and_symlink_targets_fail_before_publication(self):
        for invalid in ("json", "lua", "symlink"):
            with self.subTest(invalid=invalid):
                if invalid == "json":
                    self.write(installer.SHELL, '{"plugins": [')
                elif invalid == "lua":
                    self.write(installer.SHELL, '{}\n')
                    self.write(installer.HYPR, 'this is invalid Lua !\n')
                else:
                    self.write(installer.HYPR, '-- valid\n')
                    hook = self.root / installer.HOOK
                    hook.parent.mkdir(parents=True)
                    hook.symlink_to(self.root / "unrelated/nested/data")
                original = self.tree()
                with self.assertRaises(installer.InstallError):
                    self.install()
                self.assertEqual(self.tree(), original)
                self.assertEqual(list(self.state.glob("install-*")), [])

    def test_recovery_journal_survives_rollback_failure_and_can_restore_later(self):
        self.prior_install()
        original = self.tree()
        desktop = next(iter(installer.STATIC))
        target = self.root / desktop
        replace = os.replace

        def fail_restore(source, destination):
            if Path(destination) == target and Path(source).name.startswith(".omatainer-restore-"):
                raise OSError("injected recovery failure")
            return replace(source, destination)

        def fail_publish(event):
            if event == f"publish:{desktop}":
                raise OSError("injected commit failure")

        with patch.object(installer.os, "replace", fail_restore):
            with self.assertRaisesRegex(installer.InstallError, "rollback incomplete"):
                self.install(fail_publish)
        journals = list(self.state.glob("*/journal.json"))
        self.assertEqual(len(journals), 1)
        self.assertEqual(json.loads(journals[0].read_text())["state"], "rollback_failed")
        with self.assertRaisesRegex(installer.InstallError, "unfinished installation"):
            self.install()
        installer.recover(journals[0], self.root)
        self.assertEqual(self.tree(), original)
        self.assertEqual(json.loads(journals[0].read_text())["state"], "rolled_back")

    def test_optional_cache_failure_does_not_undo_valid_install(self):
        with patch.dict(os.environ, {"OMATAINER_TEST_FAILURE": "cache"}):
            journal, warnings = self.install()
        self.assertEqual(len(warnings), 2)
        self.assertEqual(json.loads(journal.read_text())["state"], "committed")
        self.assertEqual((self.root / installer.BINARY).read_bytes(), self.new.read_bytes())

    def test_failed_atomic_config_rename_restores_earlier_executable_commit(self):
        self.prior_install()
        original = self.tree()
        desktop = next(iter(installer.STATIC))
        target = self.root / desktop
        replace = os.replace

        def fail_replace(source, destination):
            if Path(destination) == target and "/new/" in str(source):
                raise OSError("injected atomic config rename failure")
            return replace(source, destination)

        with patch.object(installer.os, "replace", fail_replace):
            with self.assertRaisesRegex(installer.InstallError, "atomic config rename"):
                self.install()
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_lock_refuses_concurrent_install_and_wrong_root_recovery(self):
        original = self.tree()
        with installer.installation_lock(self.state):
            with self.assertRaisesRegex(installer.InstallError, "another installer"):
                self.install()
        self.assertEqual(self.tree(), original)
        journal, _ = self.install()
        other = self.base / "other-user"
        other.mkdir()
        with self.assertRaisesRegex(installer.InstallError, "user root does not match"):
            installer.recover(journal, other)
        self.assertEqual(list(other.iterdir()), [])

    def test_cancellation_after_license_publication_restores_old_records(self):
        self.prior_install()
        original = self.tree()
        def cancel(event):
            if event == "publish:" + next(iter(installer.licenses.INSTALLED)):
                raise KeyboardInterrupt("cancel during license publication")
        with self.assertRaisesRegex(installer.InstallError, "cancel during license"):
            self.install(cancel)
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_recovery_does_not_require_a_working_rust_toolchain(self):
        self.prior_install()
        original = self.tree()
        journal, _ = self.install()
        rustc = self.commands / 'rustc'
        rustc.write_text('#!/bin/sh\necho rustc-must-not-run >&2\nexit 91\n')
        rustc.chmod(0o755)
        result = subprocess.run([sys.executable, str(REPOSITORY/'scripts/install-transaction.py'),
                                 '--user-root', str(self.root), '--recover', str(journal)],
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('rustc-must-not-run', result.stderr)
        self.assertEqual(self.tree(), original)

    def test_commented_require_does_not_prevent_active_integration(self):
        self.write(installer.HYPR, '-- require("hypr.apps.omatainer")\n--[[\nrequire("hypr.apps.omatainer")\n]]\n')
        self.install()
        hypr = (self.root / installer.HYPR).read_text()
        self.assertTrue(hypr.endswith('require("hypr.apps.omatainer")\n'))
        first = hypr
        self.install()
        self.assertEqual((self.root / installer.HYPR).read_text(), first)

    def test_modified_staged_artifact_or_backup_is_rejected_before_publication(self):
        original = self.tree()
        validate = installer.validate
        for area in ("new", "old"):
            with self.subTest(area=area):
                def corrupt_after_validation(transaction):
                    validate(transaction)
                    (transaction.directory / area / installer.SHELL).write_text('{}\n')

                with patch.object(installer, "validate", corrupt_after_validation):
                    with self.assertRaisesRegex(installer.InstallError, "verification|changed after validation"):
                        self.install()
                self.assertEqual(self.tree(), original)
                self.assertEqual(list(self.state.glob("install-*")), [])

    def shell_install(self, *extra):
        return subprocess.run([
            "bash", str(REPOSITORY / "scripts/install-omarchy.sh"),
            "--user-root", str(self.root), "--source-root", str(self.source),
            "--state-root", str(self.state), *map(str, extra),
        ], text=True, capture_output=True)

    def hypr_calls(self):
        return [call[1] for line in self.log.read_text().splitlines()
                if (call := json.loads(line))[0] == "hyprctl"]

    def test_hyprctl_error_output_on_either_stream_rejects_both_exit_statuses(self):
        self.prior_install()
        original = self.tree()
        for status in (0, 17):
            for stream in ("stdout", "stderr", "both"):
                response = {"status": status}
                if stream in ("stdout", "both"):
                    response["stdout"] = "fixture invalid binding on stdout\n"
                if stream in ("stderr", "both"):
                    response["stderr"] = "fixture invalid rule on stderr\n"
                with self.subTest(status=status, stream=stream), patch.dict(os.environ, {
                    "OMATAINER_TEST_HYPR_RESPONSES": json.dumps({"configerrors": [{}, response, {}]}),
                }):
                    self.log.unlink(missing_ok=True)
                    result = self.shell_install()
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertIn("hyprctl configerrors", result.stderr)
                    self.assertIn("installed desktop", result.stderr)
                    for name in ("stdout", "stderr"):
                        if name in response:
                            self.assertIn(response[name].strip(), result.stderr)
                    self.assertIn("prior files restored", result.stderr)
                    self.assertNotIn("Installed Omatainer", result.stdout)
                    self.assertNotIn("Restored prior files; recovery journal", result.stdout)
                    self.assertEqual(self.hypr_calls(), ["configerrors", "reload", "configerrors", "reload", "configerrors"])
                    self.assertEqual(self.tree(), original)
                    self.assertEqual(list(self.state.glob("install-*")), [])

    def test_hyprctl_reload_exit_failure_preserves_both_diagnostic_streams(self):
        self.prior_install()
        original = self.tree()
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "reload": [{"status": 19, "stdout": "fixture reload rejected\n", "stderr": "fixture IPC unavailable\n"}, {}],
        })}):
            result = self.shell_install()
        self.assertNotEqual(result.returncode, 0)
        for message in ("hyprctl reload", "exited 19", "fixture reload rejected", "fixture IPC unavailable", "prior files restored"):
            self.assertIn(message, result.stderr)
        self.assertNotIn("Installed Omatainer", result.stdout)
        self.assertEqual(self.hypr_calls(), ["configerrors", "reload", "reload", "configerrors"])
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_hyprctl_preexisting_stderr_error_fails_before_any_publication(self):
        original = self.tree()
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "configerrors": [{"stderr": "fixture existing desktop error\n"}],
        })}):
            result = self.shell_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("current desktop", result.stderr)
        self.assertIn("fixture existing desktop error", result.stderr)
        self.assertNotIn("Installed Omatainer", result.stdout)
        self.assertEqual(self.hypr_calls(), ["configerrors"])
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_hyprctl_nonzero_empty_output_still_rejects_and_restores(self):
        original = self.tree()
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "configerrors": [{}, {"status": 23}, {}],
        })}):
            result = self.shell_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("hyprctl configerrors exited 23", result.stderr)
        self.assertIn("then retry", result.stderr)
        self.assertNotIn("Installed Omatainer", result.stdout)
        self.assertEqual(self.tree(), original)
        self.assertEqual(list(self.state.glob("install-*")), [])

    def test_hyprctl_failed_restored_validation_retains_journal_until_recovery_passes(self):
        # Exercise existing release records as well as executable/config backups.
        spaced = self.base / "user root with spaces"
        self.root.rename(spaced)
        self.root = spaced
        self.prior_install()
        self.install()
        original = self.tree()
        self.log.unlink()
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "configerrors": [{}, {"stdout": "fixture installed error\n"},
                             {"stderr": "fixture restored error\n"}],
        })}):
            result = self.shell_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Installed Omatainer", result.stdout)
        self.assertIn("fixture installed error", result.stderr)
        self.assertIn("fixture restored error", result.stderr)
        pending = [path for path in self.state.glob("*/journal.json")
                   if json.loads(path.read_text())["state"] == "reload_failed"]
        self.assertEqual(len(pending), 1)
        journal = pending[0]
        self.assertIn(installer.recovery_command(self.root, journal), result.stderr)
        self.assertEqual(self.tree(), original)
        self.assertTrue((journal.parent / "old" / installer.licenses.RECEIPT).is_file())
        self.assertEqual(self.hypr_calls(), ["configerrors", "reload", "configerrors", "reload", "configerrors"])

        self.log.unlink()
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "configerrors": [{"status": 7, "stderr": "fixture retry still invalid\n"}],
        })}):
            failed = self.shell_install("--recover", journal)
        self.assertNotEqual(failed.returncode, 0)
        self.assertNotIn("Restored prior files; recovery journal", failed.stdout)
        self.assertIn("desktop recovery is incomplete", failed.stderr)
        self.assertIn(installer.recovery_command(self.root, journal), failed.stderr)
        self.assertEqual(json.loads(journal.read_text())["state"], "reload_failed")
        self.assertEqual(self.tree(), original)

        self.log.unlink()
        # Execute the exact command the failure tells the user to copy; paths
        # containing spaces must still address this private fixture root.
        recovered = subprocess.run(shlex.split(installer.recovery_command(self.root, journal)),
                                   text=True, capture_output=True)
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertIn("Restored prior files; recovery journal", recovered.stdout)
        self.assertEqual(self.hypr_calls(), ["reload", "configerrors"])
        self.assertEqual(json.loads(journal.read_text())["state"], "rolled_back")
        self.assertEqual(self.tree(), original)

    def test_hyprctl_clean_validation_must_finish_before_success_is_printed(self):
        import select
        import time
        ready = self.base / "validation-running"
        release = self.base / "finish-validation"
        with patch.dict(os.environ, {"OMATAINER_TEST_HYPR_RESPONSES": json.dumps({
            "reload": [{"stdout": "ok\n", "stderr": "reload informational output\n"}],
            "configerrors": [{}, {"stdout": " \n\t", "stderr": "\n", "ready": str(ready), "wait_for": str(release)}],
        })}):
            process = subprocess.Popen([
                "bash", str(REPOSITORY / "scripts/install-omarchy.sh"),
                "--user-root", str(self.root), "--source-root", str(self.source),
                "--state-root", str(self.state),
            ], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 8
                while not ready.exists() and process.poll() is None:
                    self.assertLess(time.monotonic(), deadline, "stub never reached final validation")
                    time.sleep(0.01)
                self.assertIsNone(process.poll(), "installer exited before final validator completed")
                self.assertTrue(ready.exists())
                # Unbuffered Python makes this an output-order assertion, rather
                # than an observation of a buffered success line.
                self.assertEqual(select.select([process.stdout], [], [], 0.05)[0], [])
                journals = list(self.state.glob("*/journal.json"))
                self.assertEqual(len(journals), 1)
                self.assertNotEqual(json.loads(journals[0].read_text())["state"], "committed")
                release.touch()
                stdout, stderr = process.communicate(timeout=8)
                self.assertEqual(process.returncode, 0, stderr)
                self.assertIn("Installed Omatainer", stdout)
                self.assertEqual(json.loads(journals[0].read_text())["state"], "committed")
            finally:
                release.touch()
                if process.poll() is None:
                    process.kill()
                process.communicate()
        self.assertEqual(self.hypr_calls(), ["configerrors", "reload", "configerrors"])

    def test_shell_entrypoint_accepts_temporary_user_root_without_changing_home(self):
        home = os.environ.get("HOME")
        result = subprocess.run([
            "bash", str(REPOSITORY / "scripts/install-omarchy.sh"),
            "--user-root", str(self.root), "--source-root", str(self.source),
            "--state-root", str(self.state),
        ], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Installed Omatainer", result.stdout)
        self.assertEqual(os.environ.get("HOME"), home)
        self.assertEqual((self.root / installer.BINARY).read_bytes(), self.new.read_bytes())


if __name__ == "__main__":
    unittest.main()
