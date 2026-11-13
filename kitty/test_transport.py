import json
import os
import shutil
import socket
import tempfile
import threading
import time
import unittest
from contextlib import contextmanager
from unittest.mock import patch

from syswatch_client import MAX_RESPONSE_BYTES, TransportError, read_status, request_line


def read_line(conn):
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = conn.recv(1024)
        if not chunk:
            break
        buf += chunk
    return buf


@contextmanager
def fake_daemon(reply=b"", delay=0.0, drip=0.0):
    """Accept one connection, record its request, then answer as configured."""
    tmp = tempfile.mkdtemp()
    path = os.path.join(tmp, "s")
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(path)
    listener.listen(1)
    received = []

    def run():
        try:
            conn, _ = listener.accept()
            with conn:
                received.append(read_line(conn))
                time.sleep(delay)
                if drip:
                    for byte in reply:
                        conn.sendall(bytes([byte]))
                        time.sleep(drip)
                else:
                    conn.sendall(reply)
        except OSError:
            pass  # client gave up first; that is the case under test

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    try:
        yield path, received
    finally:
        thread.join(5)
        listener.close()
        shutil.rmtree(tmp, ignore_errors=True)


class ReadStatusOverSocketTests(unittest.TestCase):
    def test_stale_status_is_decoded_end_to_end_over_socket(self):
        reply = {
            "kind": "status",
            "version": 1,
            "aggregate": {
                "timestamp_ms": 1_000,
                "cpu_usage_percent": 1.5,
                "cpu_updated_at_ms": 1_000,
                "memory": {"total_bytes": 8, "used_bytes": 4},
            },
            "process_count": 2,
            "processes_updated_at_ms": 1_000,
        }
        with fake_daemon(json.dumps(reply).encode() + b"\n") as (path, received):
            result = read_status(path, timeout_ms=200, now_ms=2_000, max_age_ms=500)
        self.assertEqual(received, [b'{"version":1,"command":"status"}\n'])
        self.assertEqual(result.state, "stale")
        self.assertEqual(result.age_ms, 1_000)
        self.assertEqual(result.value.cpu_usage_percent, 1.5)


class RequestLineTests(unittest.TestCase):
    def test_status_request_is_exact(self):
        with fake_daemon(b'{"ok":1}\n') as (path, received):
            request_line(path, "status")
        self.assertEqual(received, [b'{"version":1,"command":"status"}\n'])

    def test_suggestions_request_is_exact(self):
        with fake_daemon(b'{"ok":1}\n') as (path, received):
            request_line(path, "suggestions")
        self.assertEqual(received, [b'{"version":1,"command":"suggestions"}\n'])

    def test_reply_is_returned_without_terminator(self):
        with fake_daemon(b'{"a":1}\n') as (path, _):
            self.assertEqual(request_line(path, "status"), b'{"a":1}')

    def test_crlf_reply_is_accepted(self):
        with fake_daemon(b'{"a":1}\r\n') as (path, _):
            self.assertEqual(request_line(path, "status"), b'{"a":1}')

    def test_bytes_after_first_newline_are_ignored(self):
        with fake_daemon(b'{"a":1}\njunk\n') as (path, _):
            self.assertEqual(request_line(path, "status"), b'{"a":1}')

    def test_immediate_close_is_malformed(self):
        with fake_daemon(b"") as (path, _):
            with self.assertRaises(TransportError) as ctx:
                request_line(path, "status")
        self.assertEqual(ctx.exception.reason, "malformed_response")

    def test_partial_reply_without_newline_is_malformed(self):
        with fake_daemon(b'{"a":1') as (path, _):
            with self.assertRaises(TransportError) as ctx:
                request_line(path, "status")
        self.assertEqual(ctx.exception.reason, "malformed_response")

    def test_oversized_reply_is_malformed(self):
        with fake_daemon(b"x" * (MAX_RESPONSE_BYTES + 1) + b"\n") as (path, _):
            with self.assertRaises(TransportError) as ctx:
                request_line(path, "status")
        self.assertEqual(ctx.exception.reason, "malformed_response")

    def test_delayed_reply_times_out(self):
        with fake_daemon(b'{"a":1}\n', delay=0.2) as (path, _):
            with self.assertRaises(TransportError) as ctx:
                request_line(path, "status", timeout_ms=30)
        self.assertEqual(ctx.exception.reason, "timeout")

    def test_slow_drip_shares_one_deadline(self):
        with fake_daemon(b"x" * 20 + b"\n", drip=0.01) as (path, _):
            with self.assertRaises(TransportError) as ctx:
                request_line(path, "status", timeout_ms=60)
        self.assertEqual(ctx.exception.reason, "timeout")

    def test_missing_socket_is_daemon_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(TransportError) as ctx:
                request_line(os.path.join(tmp, "missing.sock"), "status")
        self.assertEqual(ctx.exception.reason, "daemon_unavailable")

    def test_permission_error_on_connect(self):
        with patch("socket.socket.connect", side_effect=PermissionError("denied")):
            with self.assertRaises(TransportError) as ctx:
                request_line("/unused.sock", "status")
        self.assertEqual(ctx.exception.reason, "permission_denied")

    def test_unsupported_commands_raise_before_connecting(self):
        for command in ("close_confirm", "processes", "", None, 5):
            with self.subTest(command=command), patch("syswatch_client.socket.socket") as sock:
                with self.assertRaises(ValueError):
                    request_line("/unused.sock", command)
                sock.assert_not_called()

    def test_zero_timeout_raises_before_connecting(self):
        with patch("syswatch_client.socket.socket") as sock:
            with self.assertRaises(ValueError):
                request_line("/unused.sock", "status", timeout_ms=0)
            sock.assert_not_called()


if __name__ == "__main__":
    unittest.main()
