"""Nonblocking one-request state machine for the syswatch Unix socket.

`NonblockingRequest.advance(mono_ms)` makes whatever progress the socket allows
and returns at once. It never waits for readiness, so it is safe to call from
Kitty's main thread. The caller supplies the monotonic clock. The request has
one absolute deadline, not a timeout restarted per read.
"""

import errno
import socket

from syswatch_client import (
    MAX_RESPONSE_BYTES,
    READ_CHUNK_BYTES,
    ClientResult,
    SocketPathError,
    decode_reply,
    request_bytes,
    resolve_socket_path,
)

CONNECTING = "connecting"
WRITING = "writing"
READING = "reading"
DONE = "done"

_IN_PROGRESS = (errno.EINPROGRESS, errno.EAGAIN, errno.EALREADY)
_CONNECTED = (errno.EISCONN,)
_REASONS = {
    errno.ENOENT: "daemon_unavailable",
    errno.ECONNREFUSED: "daemon_unavailable",
    errno.EACCES: "permission_denied",
    errno.EPERM: "permission_denied",
}


def _reason(exc):
    return _REASONS.get(exc.errno, "daemon_unavailable")


class NonblockingRequest:
    def __init__(self, command, socket_path, deadline_mono_ms, wall_ms, max_age_ms):
        self._command = command
        self._socket_path = socket_path
        self._deadline = deadline_mono_ms
        self._wall_ms = wall_ms
        self._max_age_ms = max_age_ms
        self._out = memoryview(request_bytes(command))
        self._in = bytearray()
        self._sock = None
        self._state = CONNECTING
        self._result = None

    def advance(self, mono_ms):
        """Return the ClientResult once done, otherwise None. Never blocks."""
        if self._result is not None:
            return self._result
        if mono_ms >= self._deadline:
            return self._fail("timeout")
        try:
            if self._state == CONNECTING:
                self._connect()
            if self._state == WRITING:
                self._write()
            if self._state == READING:
                self._read()
        except OSError as exc:
            return self._fail(_reason(exc))
        return self._result

    def cancel(self):
        """Close the socket and drop the request. Safe to call more than once."""
        if self._sock is not None:
            self._sock.close()
            self._sock = None
        self._state = DONE
        self._result = self._result or ClientResult("daemon_unavailable")

    def _connect(self):
        if self._sock is None:
            if self._socket_path is None:  # resolved per request, so a changed environment is picked up
                try:
                    self._socket_path = resolve_socket_path()
                except SocketPathError as exc:
                    self._state = DONE
                    self._result = ClientResult("socket_path_invalid", reason=exc.reason)
                    return
            self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self._sock.setblocking(False)
        pending = self._sock.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)
        if pending:
            raise OSError(pending, "connect failed")
        err = self._sock.connect_ex(self._socket_path)
        if err in _CONNECTED or err == 0:
            self._state = WRITING
        elif err not in _IN_PROGRESS:
            raise OSError(err, "connect failed")

    def _write(self):
        try:
            sent = self._sock.send(self._out)
        except BlockingIOError:
            return
        self._out = self._out[sent:]
        if not self._out:
            self._state = READING

    def _read(self):
        try:
            chunk = self._sock.recv(READ_CHUNK_BYTES)
        except BlockingIOError:
            return
        if not chunk:
            self._fail("malformed_response")
            return
        self._in += chunk
        if len(self._in) > MAX_RESPONSE_BYTES:
            self._fail("malformed_response")
            return
        end = self._in.find(b"\n")
        if end < 0:
            return
        body = bytes(self._in[:end])
        if body.endswith(b"\r"):
            body = body[:-1]
        self._close()
        self._result = decode_reply(self._command, body, self._wall_ms(), self._max_age_ms)
        self._state = DONE

    def _close(self):
        if self._sock is not None:
            self._sock.close()
            self._sock = None

    def _fail(self, reason):
        self._close()
        self._state = DONE
        self._result = ClientResult(reason)
        return self._result
