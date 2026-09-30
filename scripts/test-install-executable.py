#!/usr/bin/env python3
"""Temporary-file integration tests; never access the installed application."""

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

SPEC = importlib.util.spec_from_file_location(
    "install_executable", Path(__file__).with_name("install-executable.py")
)
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class AtomicExecutableTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="omatainer-install-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.old = self.build("old")
        self.new = self.build("new")
        self.destination = self.root / "bin" / "omatainer"
        installer.install_executable(self.old, self.destination)

    def build(self, version):
        source = self.root / f"{version}.c"
        source.write_text(
            '#include <stdio.h>\nint main(int argc, char **argv) {'
            f'puts("{version}"); fflush(stdout); '
            f'if (argc > 1) {{ getchar(); puts("{version}"); }} return 0; }}\n'
        )
        binary = self.root / version
        subprocess.run(["cc", str(source), "-o", str(binary)], check=True, capture_output=True)
        return binary

    def assert_clean(self):
        for suffix in ("stage", "backup", "current", "previous"):
            self.assertEqual(list(self.destination.parent.glob(f".*{suffix}-*")), [])

    def test_running_process_keeps_old_inode_and_backup_is_executable(self):
        self.destination.chmod(0o751)
        process = subprocess.Popen(
            [str(self.destination), "hold"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True,
        )
        self.addCleanup(lambda: process.poll() is None and process.kill())
        self.assertEqual(process.stdout.readline().strip(), "old")
        previous = installer.install_executable(self.new, self.destination)
        self.assertEqual(subprocess.check_output([self.destination], text=True).strip(), "new")
        self.assertEqual(subprocess.check_output([previous], text=True).strip(), "old")
        self.assertEqual(installer.digest(Path(f"/proc/{process.pid}/exe")), installer.digest(self.old))
        self.assertEqual(previous.stat().st_mode & 0o777, 0o751)
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o755)
        self.assertEqual(self.destination.stat().st_uid, os.geteuid())
        output, _ = process.communicate("\n", timeout=5)
        self.assertEqual(output.strip(), "old")
        self.assertEqual(process.returncode, 0)
        installer.install_executable(previous, self.destination)
        self.assertEqual(subprocess.check_output([self.destination], text=True).strip(), "old")
        self.assertEqual(subprocess.check_output([previous], text=True).strip(), "new")
        self.assert_clean()

    def test_readers_only_observe_complete_old_or_new_bytes(self):
        allowed = {self.old.read_bytes(), self.new.read_bytes()}
        stop = threading.Event()
        observations = []
        errors = []

        def reader():
            while not stop.is_set():
                try:
                    observations.append(self.destination.read_bytes() in allowed)
                except OSError as error:
                    errors.append(error)

        observer = threading.Thread(target=reader)
        observer.start()
        try:
            for index in range(24):
                installer.install_executable(self.new if index % 2 == 0 else self.old, self.destination)
        finally:
            stop.set()
            observer.join(timeout=5)
        self.assertFalse(observer.is_alive())
        self.assertFalse(errors)
        self.assertTrue(observations)
        self.assertTrue(all(observations))
        self.assert_clean()

    def test_partial_staging_failure_preserves_destination_and_existing_backup(self):
        previous = installer.install_executable(self.new, self.destination)
        old_bytes = self.destination.read_bytes()
        backup_bytes = previous.read_bytes()

        def fail_copy(source, output):
            output.write(source.read(64))
            raise OSError("injected staging failure")

        with patch.object(installer.shutil, "copyfileobj", fail_copy):
            with self.assertRaisesRegex(OSError, "injected staging"):
                installer.install_executable(self.old, self.destination)
        self.assertEqual(self.destination.read_bytes(), old_bytes)
        self.assertEqual(previous.read_bytes(), backup_bytes)
        self.assert_clean()

    def test_failed_publication_and_post_rename_sync_restore_previous_executable(self):
        original_replace = os.replace

        def fail_publish(source, target):
            if Path(target) == self.destination:
                raise OSError("injected publication failure")
            return original_replace(source, target)

        with patch.object(installer.os, "replace", fail_publish):
            with self.assertRaisesRegex(OSError, "publication failure"):
                installer.install_executable(self.new, self.destination)
        self.assertEqual(self.destination.read_bytes(), self.old.read_bytes())
        self.assertFalse(self.destination.with_name("omatainer.previous").exists())
        original_sync = os.fsync
        calls = 0

        def fail_after_rename(fd):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError("injected directory sync failure")
            return original_sync(fd)

        with patch.object(installer.os, "fsync", fail_after_rename):
            with self.assertRaisesRegex(OSError, "directory sync failure"):
                installer.install_executable(self.new, self.destination)
        self.assertEqual(self.destination.read_bytes(), self.old.read_bytes())
        self.assertFalse(self.destination.with_name("omatainer.previous").exists())
        self.assert_clean()

    def test_failed_rollback_preserves_current_and_existing_backup_at_each_step(self):
        self.destination.chmod(0o751)
        previous = installer.install_executable(self.new, self.destination)
        expected = {
            path: (path.read_bytes(), path.stat().st_mode, path.stat().st_ino)
            for path in (self.destination, previous)
        }
        original_replace = os.replace
        original_sync = os.fsync
        for failure in ("backup", "before", "publish", "after"):
            with self.subTest(failure=failure):
                sync_calls = 0

                def fail_replace(source, target):
                    if ((failure == "backup" and Path(target) == previous)
                            or (failure == "publish" and Path(target) == self.destination)):
                        raise OSError("injected failed rollback publication")
                    return original_replace(source, target)

                def fail_sync(fd):
                    nonlocal sync_calls
                    sync_calls += 1
                    if sync_calls == {"before": 2, "after": 3}.get(failure):
                        raise OSError("injected failed rollback sync")
                    return original_sync(fd)

                # .previous is deliberately the source: overwriting its inode
                # link before publication used to destroy the rollback version.
                with patch.object(installer.os, "replace", fail_replace), \
                        patch.object(installer.os, "fsync", fail_sync):
                    with self.assertRaisesRegex(OSError, "injected failed rollback"):
                        installer.install_executable(previous, self.destination)
                for path, (data, mode, inode) in expected.items():
                    self.assertEqual(path.read_bytes(), data)
                    self.assertEqual(path.stat().st_mode, mode)
                    self.assertEqual(path.stat().st_ino, inode)
                self.assert_clean()

    def test_same_inode_backup_rename_noop_leaves_no_temporary_links(self):
        previous = self.destination.with_name("omatainer.previous")
        os.link(self.destination, previous)
        installer.install_executable(self.new, self.destination)
        self.assertEqual(previous.read_bytes(), self.old.read_bytes())
        self.assertEqual(self.destination.read_bytes(), self.new.read_bytes())
        self.assert_clean()

    def test_failed_recovery_retains_original_executable_link_and_reports_it(self):
        previous = installer.install_executable(self.new, self.destination)
        original_replace = os.replace
        original_sync = os.fsync
        sync_calls = 0

        def fail_recovery(source, target):
            if Path(target) == self.destination and ".current-" in Path(source).name:
                raise OSError("injected recovery failure")
            return original_replace(source, target)

        def fail_after_publish(fd):
            nonlocal sync_calls
            sync_calls += 1
            if sync_calls == 3:
                raise OSError("injected post-publication sync failure")
            return original_sync(fd)

        with patch.object(installer.os, "replace", fail_recovery), \
                patch.object(installer.os, "fsync", fail_after_publish):
            with self.assertRaisesRegex(OSError, "original executables retained") as error:
                installer.install_executable(previous, self.destination)
        originals = list(self.destination.parent.glob(".*current-*"))
        self.assertEqual(len(originals), 1)
        self.assertEqual(originals[0].read_bytes(), self.new.read_bytes())
        self.assertIn(str(originals[0]), str(error.exception))
        self.assertEqual(previous.read_bytes(), self.old.read_bytes())

    def test_invalid_input_and_symlink_destination_fail_without_mutation(self):
        invalid = self.root / "invalid"
        invalid.write_text("partial executable")
        invalid.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "ELF"):
            installer.install_executable(invalid, self.destination)
        self.assertEqual(self.destination.read_bytes(), self.old.read_bytes())
        link = self.destination.with_name("linked")
        link.symlink_to(self.destination)
        with self.assertRaisesRegex(ValueError, "symlink"):
            installer.install_executable(self.new, link)
        self.assertTrue(link.is_symlink())
        self.assertEqual(self.destination.read_bytes(), self.old.read_bytes())
        self.assert_clean()


if __name__ == "__main__":
    unittest.main()
