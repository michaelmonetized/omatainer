#!/usr/bin/env python3
"""Run the real status/follow CLI against strict private Unix socket servers."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading


def frame(request, **values):
    return dict(ok=True, id=request['id'], follow=True, event='state',
                accepted=None, command_status=None, state_available=True,
                playing=False) | values


def request(connection, expected):
    connection.settimeout(3)
    line = connection.makefile('rb').readline(4097)
    assert line.endswith(b'\n') and len(line) <= 4097, line[:100]
    value = json.loads(line)
    assert set(value) == {'op', 'id'}, value
    assert value['op'] == expected, value
    assert isinstance(value['id'], str) and 0 < len(value['id']) <= 128, value
    return value


def send(connection, value):
    connection.sendall(json.dumps(value).encode() + b'\n')


@contextmanager
def follower(binary, environment):
    process = subprocess.Popen([binary, 'ctl', 'follow'], env=environment,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    output = queue.Queue()
    reader = threading.Thread(target=lambda: [output.put(line) for line in process.stdout])
    reader.start()
    try:
        yield lambda: json.loads(output.get(timeout=5))
    finally:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(timeout=3)
        reader.join(timeout=3)
        assert not reader.is_alive(), 'CLI stdout did not close after termination'
        process.stdout.close(); process.stderr.close()


@contextmanager
def server(handler):
    with tempfile.TemporaryDirectory(prefix='omatainer-strict-follow-') as directory:
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(Path(directory) / 'omatainer.sock'))
        listener.listen(2); listener.settimeout(5)
        errors = []
        def serve():
            try:
                handler(listener)
            except BaseException as error:
                errors.append(error)
        worker = threading.Thread(target=serve)
        worker.start()
        try:
            yield dict(os.environ, XDG_RUNTIME_DIR=directory)
        finally:
            worker.join(timeout=6); listener.close()
            assert not worker.is_alive(), 'strict server did not complete'
            assert not errors, errors


def check_status_and_follow(binary):
    requests = []
    def handle(listener):
        connection, _ = listener.accept()
        with connection:
            ordinary = request(connection, 'status'); requests.append(ordinary)
            send(connection, {'id': ordinary['id'], 'ok': True, 'playing': False})
        connection, _ = listener.accept()
        with connection:
            stream = request(connection, 'follow'); requests.append(stream)
            send(connection, frame(stream, ok=False, state_available=False,
                                   error_code='snapshot_unavailable', error='snapshot temporarily unavailable'))
            send(connection, frame(stream, playing=True))
            send(connection, frame(stream, playing=False))
    with server(handle) as environment:
        ordinary = subprocess.run([binary, 'ctl', 'status'], env=environment,
                                  capture_output=True, text=True, timeout=5)
        assert ordinary.returncode == 0, ordinary.stderr
        assert json.loads(ordinary.stdout)['playing'] is False
        with follower(binary, environment) as next_frame:
            assert next_frame()['error_code'] == 'snapshot_unavailable'
            assert next_frame()['playing'] is True
            assert next_frame()['playing'] is False
    assert len(requests) == 2 and requests[0]['id'] != requests[1]['id']
    print('strict status + persistent follow: valid correlated JSON and unchanged state frames')


def check_reconnect(binary, case):
    requests = []
    def handle(listener):
        connection, _ = listener.accept()
        with connection:
            first = request(connection, 'follow'); requests.append(first)
            if case == 'malformed': connection.sendall(b'not-json\n')
            elif case == 'wrong_id': send(connection, frame(first, id='unrelated'))
            elif case == 'oversized': connection.sendall(b'x' * 8193 + b'\n')
            elif case != 'eof': raise AssertionError(case)
        connection, _ = listener.accept()
        with connection:
            second = request(connection, 'follow'); requests.append(second)
            send(connection, frame(second, playing=True))
    with server(handle) as environment, follower(binary, environment) as next_frame:
        failure = next_frame()
        assert failure['ok'] is False and failure['state_available'] is False, failure
        assert failure['error_code'] == ('transport_error' if case == 'eof' else 'protocol_error'), failure
        assert failure['error'] and 'not running' not in failure['error'], failure
        assert next_frame()['playing'] is True
    assert len(requests) == 2 and requests[0]['id'] != requests[1]['id']
    print(case + ': classified error, bounded reconnect and fresh correlated request')


def check_stopped(binary):
    with tempfile.TemporaryDirectory(prefix='omatainer-stopped-follow-') as directory:
        with follower(binary, dict(os.environ, XDG_RUNTIME_DIR=directory)) as next_frame:
            result = next_frame()
            assert result['ok'] is False and result['error_code'] == 'not_running', result
    print('absent application: not_running is distinct from protocol and transport errors')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    binary = str(parser.parse_args().binary.resolve(strict=True))
    check_status_and_follow(binary)
    for case in ['malformed', 'wrong_id', 'oversized', 'eof']:
        check_reconnect(binary, case)
    check_stopped(binary)
    print('All six native CLI groups passed; no live application socket or audio device was used.')


if __name__ == '__main__':
    main()
