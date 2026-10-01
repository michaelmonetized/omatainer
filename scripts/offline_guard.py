#!/usr/bin/env python3
"""Restrict only a new qualification child, preserving local Unix IPC.

This is a direct-network capability boundary, not a hostile-code sandbox:
AF_UNIX remains available and a deliberately contacted host proxy is outside
the claim. The orchestrator must use private buses and must not open browsers.
No host network settings, identities, or desktop configuration are changed.
"""
import argparse
import ctypes
import ctypes.util
import errno
import json
import os
from pathlib import Path
import platform
import resource
import socket
import stat
import sys

sys.dont_write_bytecode = True
SCHEMA = 1
LIMIT = 16 * 1024


class GuardError(RuntimeError):
    pass


def private_directory(path):
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise GuardError('guard directory must be an absolute private path')
    uid = os.geteuid()
    for current in [path, *path.parents]:
        info = current.lstat()
        if not stat.S_ISDIR(info.st_mode):
            raise GuardError('guard directory must not contain symlinks')
        mode = stat.S_IMODE(info.st_mode)
        if current == path:
            if info.st_uid != uid or mode != 0o700:
                raise GuardError('guard directory must be owned by the effective UID and mode 0700')
        elif info.st_uid not in (0, uid) or (mode & 0o022 and not mode & stat.S_ISVTX):
            raise GuardError('guard directory has an untrusted ancestor')
    return path


def close_inherited():
    # The controlled parent uses pipes/devnull. A socket inherited as stdio could
    # already be an Internet connection and would evade socket-creation denial.
    for descriptor in range(3):
        try:
            if stat.S_ISSOCK(os.fstat(descriptor).st_mode):
                raise GuardError('socket stdio is not permitted in offline qualification')
        except OSError as error:
            if error.errno != errno.EBADF:
                raise
    # Enumeration is Linux-only and closes every inherited descriptor, including
    # sockets without CLOEXEC. The enumeration directory FD is already closed.
    inherited = [int(name) for name in os.listdir('/proc/self/fd') if int(name) > 2]
    for descriptor in inherited:
        try:
            os.close(descriptor)
        except OSError as error:
            if error.errno != errno.EBADF:
                raise


def restrict():
    if sys.platform != 'linux' or platform.machine() not in ('aarch64', 'x86_64'):
        raise GuardError('offline qualification currently supports native Linux aarch64/x86_64')
    name = ctypes.util.find_library('seccomp')
    if not name:
        raise GuardError('libseccomp is required; qualification was not run unrestricted')
    library = ctypes.CDLL(name, use_errno=True)

    class Comparison(ctypes.Structure):
        _fields_ = [('arg', ctypes.c_uint), ('op', ctypes.c_int),
                    ('datum_a', ctypes.c_uint64), ('datum_b', ctypes.c_uint64)]

    library.seccomp_init.argtypes = [ctypes.c_uint32]
    library.seccomp_init.restype = ctypes.c_void_p
    library.seccomp_release.argtypes = [ctypes.c_void_p]
    library.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
    library.seccomp_syscall_resolve_name.restype = ctypes.c_int
    library.seccomp_rule_add_array.argtypes = [ctypes.c_void_p, ctypes.c_uint32,
                                             ctypes.c_int, ctypes.c_uint, ctypes.POINTER(Comparison)]
    library.seccomp_rule_add_array.restype = ctypes.c_int
    library.seccomp_load.argtypes = [ctypes.c_void_p]
    library.seccomp_load.restype = ctypes.c_int
    context = library.seccomp_init(0x7fff0000)  # SCMP_ACT_ALLOW
    if not context:
        raise GuardError('could not allocate the child network filter')
    try:
        # Native libseccomp rejects foreign syscall architectures by default.
        # NE is 1: deny every domain except AF_UNIX, including netlink/packet.
        comparison = Comparison(0, 1, socket.AF_UNIX, 0)
        for syscall in (b'socket', b'socketpair'):
            number = library.seccomp_syscall_resolve_name(syscall)
            if number < 0 or library.seccomp_rule_add_array(
                    context, 0x50000 | errno.EPERM, number, 1, ctypes.byref(comparison)) != 0:
                raise GuardError('could not install the complete socket-family filter')
        # io_uring can create sockets without the socket syscall. Current app
        # code does not use it; refusing setup also excludes that alternate path.
        number = library.seccomp_syscall_resolve_name(b'io_uring_setup')
        if number < 0 or library.seccomp_rule_add_array(
                context, 0x50000 | errno.EPERM, number, 0, None) != 0:
            raise GuardError('could not deny io_uring network setup')
        libc = ctypes.CDLL(None, use_errno=True)
        libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong,
                               ctypes.c_ulong, ctypes.c_ulong]
        libc.prctl.restype = ctypes.c_int
        if libc.prctl(38, 1, 0, 0, 0) != 0:  # PR_SET_NO_NEW_PRIVS
            raise GuardError('could not set no-new-privileges')
        if library.seccomp_load(context) != 0:
            raise GuardError('could not load the child network filter')
        return number
    finally:
        library.seccomp_release(context)


def controls(directory, io_uring_setup):
    denied = {}
    for family, label in [(socket.AF_INET, 'ipv4'), (socket.AF_INET6, 'ipv6')]:
        for kind, transport in [(socket.SOCK_STREAM, 'tcp'), (socket.SOCK_DGRAM, 'udp')]:
            try:
                connection = socket.socket(family, kind)
            except OSError as error:
                if error.errno != errno.EPERM:
                    raise GuardError('network denial returned an unexpected error') from error
                denied[label + '_' + transport] = 'EPERM'
            else:
                connection.close()
                raise GuardError('network socket creation succeeded; refusing offline evidence')
    first, second = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    with first, second:
        first.sendall(b'local')
        if second.recv(5) != b'local':
            raise GuardError('local Unix socketpair did not work')
    path = directory / 'network-guard.sock'
    if path.exists() or path.is_symlink():
        raise GuardError('refusing an occupied guard endpoint')
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        listener.bind(str(path))
        listener.listen(1)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(1)
            client.connect(str(path))
            connection, _ = listener.accept()
            with connection:
                client.sendall(b'ipc')
                if connection.recv(3) != b'ipc':
                    raise GuardError('local Unix listener did not work')
    finally:
        listener.close()
        path.unlink(missing_ok=True)
    values = dict(line.split(':', 1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
    if values.get('NoNewPrivs', '').strip() != '1' or values.get('Seccomp', '').strip() != '2':
        raise GuardError('kernel did not report an active unprivileged filter')
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    result = libc.syscall(ctypes.c_long(io_uring_setup), ctypes.c_uint(1), ctypes.c_void_p())
    if result != -1 or ctypes.get_errno() != errno.EPERM:
        if result >= 0:
            os.close(result)
        raise GuardError('io_uring setup was not refused by the network filter')
    return {'schema': SCHEMA, 'mechanism': 'child-only libseccomp socket-family denial',
            'direct_network': denied, 'unix_socketpair': True, 'unix_listener': True,
            'io_uring_setup': 'EPERM', 'effective_uid': os.geteuid(),
            'kernel': platform.release(), 'architecture': platform.machine(),
            'no_new_privileges': True, 'seccomp_filter': True,
            'scope': 'No non-AF_UNIX socket creation or inherited network FDs. Private buses required; host proxies and external browser handoffs are not qualification evidence.'}


def run(receipt, command):
    if not command or not Path(command[0]).is_absolute():
        raise GuardError('guarded executable must be an explicit absolute path')
    receipt = Path(receipt)
    directory = private_directory(receipt.parent)
    if receipt.name in ('', '.', '..') or receipt.exists() or receipt.is_symlink():
        raise GuardError('guard receipt must be a new file')
    close_inherited()
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))  # private crash fixtures must not dump user-sized cores
    original_uid = os.geteuid()
    io_uring_setup = restrict()
    evidence = controls(directory, io_uring_setup)
    if os.geteuid() != original_uid:
        raise GuardError('effective identity changed during qualification')
    data = (json.dumps(evidence, sort_keys=True, indent=2) + '\n').encode()
    if len(data) > LIMIT:
        raise GuardError('guard receipt exceeds its limit')
    descriptor = os.open(receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as file:
        file.write(data)
        file.flush()
    os.execve(command[0], command, child_environment())


def child_environment():
    # HOME keeps its real meaning; fixture-specific app paths are explicit.
    # No inherited browser, portal/session bus, proxy, loader, or credential env.
    allowed = {'HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME',
               'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR', 'TMPDIR'}
    result = {key: value for key, value in os.environ.items()
              if key in allowed or key.startswith('OMATAINER_')}
    result.update(PATH='/usr/bin:/bin', LANG='C.UTF-8', LC_ALL='C.UTF-8',
                  GSETTINGS_BACKEND='memory', PYTHONDONTWRITEBYTECODE='1',
                  RUST_BACKTRACE='0')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipt', required=True, type=Path)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    try:
        run(args.receipt, command)
    except (OSError, GuardError) as error:
        print('offline guard failed: ' + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
