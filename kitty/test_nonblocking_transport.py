import ast
import json
import os
import pathlib
import shutil
import socket
import tempfile
import time
import unittest
from unittest import mock

import syswatch_transport
from syswatch_transport import NonblockingRequest
from test_responses import status_body

NOW = 2_000
FAR = 10**12


class Daemon:
    """A listening Unix socket driven from the test thread. Nothing blocks."""

    def __init__(self):
        self.dir = tempfile.mkdtemp()
        self.path = os.path.join(self.dir, "s.sock")
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.path)
        self.listener.listen(4)
        self.listener.setblocking(False)
        self.conn = None

    def accept(self):
        if self.conn is None:
            try:
                self.conn = self.listener.accept()[0]
                self.conn.setblocking(False)
            except BlockingIOError:
                pass
        return self.conn

    def received_line(self):
        buf = b""
        while not buf.endswith(b"\n"):
            try:
                chunk = self.conn.recv(1024)
            except BlockingIOError:
                break
            if not chunk:
                break
            buf += chunk
        return buf

    def close(self):
        if self.conn is not None:
            self.conn.close()
        self.listener.close()
        shutil.rmtree(self.dir)


def request(path, command="status", deadline=FAR):
    return NonblockingRequest(command, path, deadline, lambda: NOW, None)


def drive_until_connected(req, daemon, steps=50):
    for i in range(steps):
        req.advance(i)
        if daemon.accept() is not None:
            return daemon.conn
    return None


def drive(req, daemon, steps=200, start=0):
    """Advance until the result is ready, letting the fake daemon act between steps."""
    result = None
    for i in range(steps):
        daemon.accept()
        result = req.advance(start + i)
        if result is not None:
            return result
    return result


class NonblockingRequestTest(unittest.TestCase):
    def setUp(self):
        self.daemon = Daemon()
        self.addCleanup(self.daemon.close)

    def test_full_round_trip_returns_decoded_status(self):
        req = request(self.daemon.path)
        line = b""
        for i in range(50):
            req.advance(i)
            self.daemon.accept()
            if self.daemon.conn is not None:
                line = self.daemon.received_line()
                if line:
                    break
        self.assertEqual(line, b'{"version":1,"command":"status"}\n')
        reply = json.dumps(status_body()).encode() + b"\n"
        self.daemon.conn.setblocking(True)
        self.daemon.conn.sendall(reply)
        self.daemon.conn.setblocking(False)
        result = drive(req, self.daemon, start=200)
        self.assertEqual(result.state, "available")
        self.assertEqual(result.value.process_count, 3)

    def test_unset_socket_path_resolves_from_the_environment(self):
        with mock.patch.dict(os.environ, {"SYSWATCH_SOCKET": self.daemon.path}):
            req = NonblockingRequest("status", None, FAR, lambda: NOW, None)
            self.assertIsNotNone(drive_until_connected(req, self.daemon))
            self.assertEqual(req._socket_path, self.daemon.path)

    def test_relative_socket_path_from_the_environment_is_invalid(self):
        with mock.patch.dict(os.environ, {"SYSWATCH_SOCKET": "relative.sock"}):
            req = NonblockingRequest("status", None, FAR, lambda: NOW, None)
            result = req.advance(0)
            self.assertEqual((result.state, result.reason), ("socket_path_invalid", "relative_path"))

    def test_silent_daemon_never_blocks_and_hits_the_deadline(self):
        req = request(self.daemon.path, deadline=100)
        started = time.monotonic()
        self.assertIsNone(req.advance(0))
        for i in range(1, 50):
            self.daemon.accept()
            self.assertIsNone(req.advance(i))
        self.assertLess(time.monotonic() - started, 0.5)
        self.assertEqual(req.advance(100).state, "timeout")

    def test_missing_socket_is_daemon_unavailable(self):
        req = request(os.path.join(self.daemon.dir, "absent.sock"))
        self.assertEqual(drive(req, self.daemon).state, "daemon_unavailable")

    def test_daemon_closing_before_a_reply_is_malformed(self):
        req = request(self.daemon.path)
        drive(req, self.daemon, steps=5)
        self.daemon.accept()
        self.daemon.conn.close()
        self.daemon.conn = None
        self.assertEqual(drive(req, self.daemon, start=10).state, "malformed_response")

    def test_reply_over_the_size_limit_is_malformed(self):
        req = request(self.daemon.path)
        with mock.patch.object(syswatch_transport, "MAX_RESPONSE_BYTES", 16):
            drive(req, self.daemon, steps=5)
            self.daemon.accept()
            self.daemon.received_line()
            self.daemon.conn.sendall(b"x" * 64)
            self.assertEqual(drive(req, self.daemon, start=10).state, "malformed_response")

    def test_error_reply_is_decoded_not_raised(self):
        req = request(self.daemon.path)
        drive(req, self.daemon, steps=5)
        self.daemon.accept()
        self.daemon.received_line()
        error = {"kind": "error", "version": 1, "code": "unavailable", "message": "no snapshot yet"}
        self.daemon.conn.sendall(json.dumps(error).encode() + b"\n")
        result = drive(req, self.daemon, start=10)
        self.assertEqual((result.state, result.error_code), ("warming_up", "unavailable"))

    def test_cancel_closes_the_socket_and_returns_a_result(self):
        req = request(self.daemon.path)
        drive(req, self.daemon, steps=5)
        req.cancel()
        self.assertEqual(req.advance(10).state, "daemon_unavailable")
        self.assertIsNone(req._sock)


class SourceGuardTest(unittest.TestCase):
    def source(self, name):
        return pathlib.Path(__file__).with_name(name).read_text()

    def test_transport_has_no_blocking_socket_calls(self):
        source = self.source("syswatch_transport.py")
        for banned in ("sendall(", "settimeout(", "select.", "poll(", ".connect(", "sleep(", "threading"):
            self.assertNotIn(banned, source)


if __name__ == "__main__":
    unittest.main()
