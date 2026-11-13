"""Read-only client for the syswatch daemon's cached status and suggestions.

Requires Python 3.11 or newer. Standard library only. Read-only: it sends
`status` and `suggestions` requests over the local Unix socket. It never opens
TCP, never runs shell commands, and never requests a process action.

The socket path resolution mirrors `crates/syswatch-core/src/ipc.rs`.
"""

import json
import os
import socket
import time

PROTOCOL_VERSION = 1
DEFAULT_TIMEOUT_MS = 1000
MAX_REQUEST_BYTES = 4096
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
READ_POLL_SECONDS = 0.01
READ_CHUNK_BYTES = 8192
READ_COMMANDS = ("status", "suggestions")


class SocketPathError(Exception):
    """Raised when no usable socket path can be resolved."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason  # "relative_path" or "not_configured"


def resolve_socket_path(environ=None):
    """Return the daemon socket path, using the same precedence as the Rust core."""
    env = os.environ if environ is None else environ

    explicit = env.get("SYSWATCH_SOCKET")
    if explicit:
        if os.path.isabs(explicit):
            return explicit
        raise SocketPathError("relative_path")

    runtime_dir = env.get("XDG_RUNTIME_DIR")
    if runtime_dir and os.path.isabs(runtime_dir):
        return os.path.join(runtime_dir, "syswatch.sock")

    home = env.get("HOME")
    if home and os.path.isabs(home):
        return os.path.join(home, ".local", "run", "syswatch.sock")

    raise SocketPathError("not_configured")


class TransportError(Exception):
    """A failed request. `reason` is daemon_unavailable, permission_denied, timeout, or malformed_response."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def request_line(socket_path, command, timeout_ms=DEFAULT_TIMEOUT_MS):
    """Send one read-only request and return the reply body, without its terminator."""
    if not isinstance(command, str) or command not in READ_COMMANDS:
        raise ValueError(f"unsupported command: {command!r}")
    if timeout_ms <= 0:
        raise ValueError("timeout_ms must be positive")
    line = json.dumps({"version": PROTOCOL_VERSION, "command": command}, separators=(",", ":")).encode() + b"\n"
    assert len(line) <= MAX_REQUEST_BYTES

    timeout = timeout_ms / 1000
    deadline = time.monotonic() + timeout  # one deadline for the whole reply
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.settimeout(timeout)
        sock.connect(socket_path)
        sock.sendall(line)
        buf = bytearray()
        while b"\n" not in buf:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TransportError("timeout")
            sock.settimeout(min(remaining, READ_POLL_SECONDS))  # slow-drip guard
            try:
                chunk = sock.recv(READ_CHUNK_BYTES)
            except socket.timeout:
                continue
            if not chunk:
                raise TransportError("malformed_response")
            buf += chunk
            if len(buf) > MAX_RESPONSE_BYTES:
                raise TransportError("malformed_response")
        body = bytes(buf[: buf.index(b"\n")])
        if body.endswith(b"\r"):
            body = body[:-1]
        if not body:
            raise TransportError("malformed_response")
        return body
    except socket.timeout:
        raise TransportError("timeout") from None
    except (FileNotFoundError, ConnectionRefusedError):
        raise TransportError("daemon_unavailable") from None
    except PermissionError:
        raise TransportError("permission_denied") from None
    except OSError:
        raise TransportError("daemon_unavailable") from None
    finally:
        sock.close()
