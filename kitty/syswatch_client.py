"""Read-only client for the syswatch daemon's cached status and suggestions.

Requires Python 3.11 or newer. Standard library only. Read-only: it sends
`status` and `suggestions` requests over the local Unix socket. It never opens
TCP, never runs shell commands, and never requests a process action.

The socket path resolution mirrors `crates/syswatch-core/src/ipc.rs`.

Only `status` and `suggestions` are read. The `processes` request is out of
scope: it would copy per-process data into the caller. Duplicate JSON keys keep
the last value; the Rust parser rejects them. This is a deliberate divergence.
"""

import json
import math
import os
import socket
import time
from dataclasses import dataclass

PROTOCOL_VERSION = 1
DEFAULT_TIMEOUT_MS = 1000
MAX_REQUEST_BYTES = 4096
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
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
            sock.settimeout(remaining)  # one deadline for the whole reply
            chunk = sock.recv(READ_CHUNK_BYTES)
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


@dataclass(frozen=True)
class Memory:
    total_bytes: int
    used_bytes: int


@dataclass(frozen=True)
class Status:
    timestamp_ms: int
    cpu_usage_percent: float
    cpu_updated_at_ms: int | None
    memory: Memory
    process_count: int | None
    processes_updated_at_ms: int | None


@dataclass(frozen=True)
class CpuEvidence:
    max_percent: float
    mean_percent: float
    valid_samples: int


@dataclass(frozen=True)
class MemoryEvidence:
    latest_bytes: int
    max_bytes: int


@dataclass(frozen=True)
class Evidence:
    sample_count: int
    span_ms: int
    cpu: CpuEvidence | None
    memory: MemoryEvidence | None


@dataclass(frozen=True)
class Finding:
    pid: int
    start_time_secs: int
    kind: str
    confidence: str
    observed_at_ms: int
    explanation: str
    evidence: Evidence


@dataclass(frozen=True)
class Suggestions:
    evaluated_at_ms: int
    processes_updated_at_ms: int
    findings: tuple


@dataclass(frozen=True)
class ClientResult:
    state: str
    value: object = None
    age_ms: int | None = None
    reason: str | None = None
    error_code: str | None = None
    message: str | None = None


CONFIDENCES = ("low", "medium", "high")
ERROR_STATES = {"unavailable": "warming_up", "unsupported_version": "protocol_mismatch"}


class _Malformed(Exception):
    pass


def _reject_constant(name):
    raise ValueError(f"non-finite JSON constant {name}")


def _uint(value):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise _Malformed
    return value


def _float(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise _Malformed
    return float(value)


def _text(value):
    if not isinstance(value, str):
        raise _Malformed
    return value


def _obj(value):
    if not isinstance(value, dict):
        raise _Malformed
    return value


def _opt(value, decode):
    return None if value is None else decode(value)


def _memory(value):
    value = _obj(value)
    return Memory(_uint(value["total_bytes"]), _uint(value["used_bytes"]))


def _status(obj):
    agg = _obj(obj["aggregate"])
    status = Status(
        timestamp_ms=_uint(agg["timestamp_ms"]),
        cpu_usage_percent=_float(agg["cpu_usage_percent"]),
        cpu_updated_at_ms=_opt(agg["cpu_updated_at_ms"], _uint),
        memory=_memory(agg["memory"]),
        process_count=_opt(obj["process_count"], _uint),
        processes_updated_at_ms=_opt(obj["processes_updated_at_ms"], _uint),
    )
    return status, status.timestamp_ms


def _cpu(value):
    value = _obj(value)
    return CpuEvidence(_float(value["max_percent"]), _float(value["mean_percent"]), _uint(value["valid_samples"]))


def _memory_evidence(value):
    value = _obj(value)
    return MemoryEvidence(_uint(value["latest_bytes"]), _uint(value["max_bytes"]))


def _finding(value):
    value = _obj(value)
    ident = _obj(value["id"])
    kind = _text(ident["kind"])
    confidence = _text(value["confidence"])
    if not kind or confidence not in CONFIDENCES:
        raise _Malformed
    ev = _obj(value["evidence"])
    return Finding(
        pid=_uint(ident["pid"]),
        start_time_secs=_uint(ident["start_time_secs"]),
        kind=kind,
        confidence=confidence,
        observed_at_ms=_uint(value["observed_at_ms"]),
        explanation=_text(value["explanation"]),
        evidence=Evidence(
            sample_count=_uint(ev["sample_count"]),
            span_ms=_uint(ev["span_ms"]),
            cpu=_opt(ev["cpu"], _cpu),
            memory=_opt(ev["memory"], _memory_evidence),
        ),
    )


def _suggestions(obj):
    if not isinstance(obj["findings"], list):
        raise _Malformed
    result = Suggestions(
        evaluated_at_ms=_uint(obj["evaluated_at_ms"]),
        processes_updated_at_ms=_uint(obj["processes_updated_at_ms"]),
        findings=tuple(_finding(item) for item in obj["findings"]),
    )
    return result, result.evaluated_at_ms


def timestamp_age_ms(now_ms, timestamp_ms):
    """Age in milliseconds, saturating at zero for a future timestamp."""
    return max(0, now_ms - timestamp_ms)


def _error_result(obj):
    code = _text(obj["code"])
    return ClientResult(
        ERROR_STATES.get(code, "daemon_error"), error_code=code, message=_text(obj["message"])
    )


def _decode_reply(body, command, decode, now_ms, max_age_ms):
    try:
        obj = json.loads(body, parse_constant=_reject_constant)
        if not isinstance(obj, dict):
            raise _Malformed
        kind = _text(obj["kind"])
        version = obj["version"]
        if isinstance(version, bool) or not isinstance(version, int):
            raise _Malformed
        if version != PROTOCOL_VERSION:
            return ClientResult("protocol_mismatch")
        if kind == "error":
            return _error_result(obj)
        if kind != command:
            raise _Malformed
        value, primary_ms = decode(obj)
    except (ValueError, KeyError, _Malformed, RecursionError, OverflowError):  # json errors are ValueError
        return ClientResult("malformed_response")
    age = timestamp_age_ms(now_ms, primary_ms)
    stale = max_age_ms is not None and age > max_age_ms
    return ClientResult("stale" if stale else "available", value=value, age_ms=age)


def _read(command, decode, socket_path, timeout_ms, now_ms, max_age_ms):
    if socket_path is not None and not isinstance(socket_path, str):
        raise TypeError("socket_path must be a string or None")
    if not isinstance(timeout_ms, int) or isinstance(timeout_ms, bool):
        raise TypeError("timeout_ms must be an integer")
    if max_age_ms is not None and max_age_ms < 0:
        raise ValueError("max_age_ms must not be negative")
    now = int(time.time() * 1000) if now_ms is None else now_ms
    if socket_path is None:
        try:
            socket_path = resolve_socket_path()
        except SocketPathError as exc:
            return ClientResult("socket_path_invalid", reason=exc.reason)
    try:
        body = request_line(socket_path, command, timeout_ms)
    except TransportError as exc:
        return ClientResult(exc.reason)
    return _decode_reply(body, command, decode, now, max_age_ms)


def read_status(socket_path=None, *, timeout_ms=DEFAULT_TIMEOUT_MS, now_ms=None, max_age_ms=None):
    """Read the cached status. `max_age_ms=None` reports age without ever marking stale."""
    return _read("status", _status, socket_path, timeout_ms, now_ms, max_age_ms)


def read_suggestions(socket_path=None, *, timeout_ms=DEFAULT_TIMEOUT_MS, now_ms=None, max_age_ms=None):
    """Read the cached suggestions. `max_age_ms=None` reports age without ever marking stale."""
    return _read("suggestions", _suggestions, socket_path, timeout_ms, now_ms, max_age_ms)
