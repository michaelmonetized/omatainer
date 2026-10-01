#!/usr/bin/env python3
"""Actual child/exec tests: no desktop service, Internet connection or Rust build."""
import errno
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
GUARD = Path(__file__).with_name('offline_guard.py').resolve()


class OfflineGuardTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='omatainer-offline-guard-')
        self.root = Path(self.directory.name)

    def tearDown(self):
        self.directory.cleanup()

    def launch(self, code, receipt='guard.json', **options):
        return subprocess.run([sys.executable, str(GUARD), '--receipt', str(self.root / receipt),
                               '--', sys.executable, '-c', code],
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              text=True, timeout=5, **options)

    def test_real_exec_retains_denial_uid_unix_ipc_and_sanitized_environment(self):
        code = '''import errno,json,os,socket
result={}
for family in [socket.AF_INET,socket.AF_INET6]:
 for kind in [socket.SOCK_STREAM,socket.SOCK_DGRAM]:
  try: socket.socket(family,kind)
  except OSError as error: assert error.errno==errno.EPERM;result[str((family,kind))]=error.errno
  else: raise AssertionError('network socket admitted after exec')
a,b=socket.socketpair();a.sendall(b'local');assert b.recv(5)==b'local';a.close();b.close()
for key in ['HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','DBUS_SESSION_BUS_ADDRESS','AT_SPI_BUS_ADDRESS','LD_PRELOAD','SECRET_TEST_TOKEN']:
 assert key not in os.environ,key
print(json.dumps({'uid':os.geteuid(),'denied':len(result),'home':os.environ.get('HOME')}))'''
        env = dict(os.environ, HTTP_PROXY='http://sentinel.invalid',
                   DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent-private-test',
                   SECRET_TEST_TOKEN='must-not-pass')
        completed = self.launch(code, env=env)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        observed = json.loads(completed.stdout)
        self.assertEqual(observed, {'uid': os.geteuid(), 'denied': 4, 'home': os.environ.get('HOME')})
        record = json.loads((self.root / 'guard.json').read_text())
        self.assertEqual(set(record['direct_network'].values()), {'EPERM'})
        self.assertTrue(record['unix_listener'] and record['unix_socketpair'])
        self.assertTrue(record['no_new_privileges'] and record['seccomp_filter'])
        self.assertEqual(record['io_uring_setup'], 'EPERM')
        self.assertEqual((self.root / 'guard.json').stat().st_mode & 0o777, 0o600)
        self.assertFalse((self.root / 'network-guard.sock').exists())

    def test_inherited_network_descriptor_is_closed_before_exec(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as network:
            os.dup2(network.fileno(), 198, inheritable=True)
            try:
                completed = self.launch('''import errno,os
try: os.fstat(198)
except OSError as error: assert error.errno==errno.EBADF
else: raise AssertionError('inherited network descriptor survived')''', pass_fds=(198,))
            finally:
                os.close(198)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_occupied_receipt_and_symlink_directory_preserve_originals(self):
        original = self.root / 'guard.json'
        original.write_text('preserve')
        self.assertEqual(self.launch("raise AssertionError('must not execute')").returncode, 2)
        self.assertEqual(original.read_text(), 'preserve')
        alias = self.root / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        completed = subprocess.run([sys.executable, str(GUARD), '--receipt', str(alias / 'new.json'),
                                    '--', sys.executable, '-c', 'raise AssertionError()'],
                                   capture_output=True, timeout=5)
        self.assertEqual(completed.returncode, 2)
        self.assertFalse((self.root / 'new.json').exists())

    def test_filter_install_failure_never_executes_requested_program(self):
        code = '''import importlib.util,sys,pathlib
spec=importlib.util.spec_from_file_location('guard',sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
m.ctypes.util.find_library=lambda name:None
try: m.run(pathlib.Path(sys.argv[2]),[sys.executable,'-c','raise SystemExit(99)'])
except m.GuardError: raise SystemExit(0)
raise AssertionError('did not fail closed')'''
        completed = subprocess.run([sys.executable, '-c', code, str(GUARD), str(self.root / 'guard.json')],
                                   capture_output=True, timeout=5)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertFalse((self.root / 'guard.json').exists())

    def test_child_exit_failure_is_preserved_and_does_not_unrestrict_parent(self):
        completed = self.launch('raise SystemExit(23)')
        self.assertEqual(completed.returncode, 23)
        # Merely creating an unconnected socket makes no network request. The
        # parent has not inherited the child's filter or changed host settings.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM):
            pass

    def test_socket_stdio_is_rejected(self):
        first, second = socket.socketpair()
        with first, second:
            completed = subprocess.run([sys.executable, str(GUARD), '--receipt', str(self.root / 'guard.json'),
                                        '--', sys.executable, '-c', 'raise AssertionError()'],
                                       stdin=first, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertEqual(completed.returncode, 2)
        self.assertIn(b'socket stdio', completed.stderr)
        self.assertFalse((self.root / 'guard.json').exists())


if __name__ == '__main__':
    unittest.main()
