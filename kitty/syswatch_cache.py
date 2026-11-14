"""Client-side cache for the `status` and `suggestions` daemon replies.

Drawing code reads `TelemetryCache.view()`, which only reads one immutable
snapshot and does no I/O. `_ingest` replaces the whole snapshot with a single
attribute assignment. Only the caller that drives `advance()` writes, so
no lock is needed.

`advance(mono_ms)` is driven from Kitty's main-loop timer (see
`syswatch_refresh`). It starts at most one nonblocking request, advances it by
one step, and returns the delay until the next useful call. The cache creates
no threads and never blocks.

Polling limits: `status` at most every 1000 ms, `suggestions` at most every
5000 ms. A shorter interval raises `ValueError`. A channel is `stale` when its
data is older than three intervals (3000 ms, 15000 ms). A failed request backs
off from 1000 ms up to 10000 ms, and the next request starts only after the
previous one completes, so a slow daemon cannot cause a catch-up burst.
"""

import time
from dataclasses import dataclass, replace

from syswatch_client import ClientResult
from syswatch_transport import NonblockingRequest

DEFAULT_STATUS_INTERVAL_MS = 1000
DEFAULT_SUGGESTIONS_INTERVAL_MS = 5000
MIN_STATUS_INTERVAL_MS = 1000
MIN_SUGGESTIONS_INTERVAL_MS = 5000
STALE_FACTOR = 3
REQUEST_DEADLINE_MS = 250
PROGRESS_MS = 10
BACKOFF_MIN_MS = 1000
BACKOFF_MAX_MS = 10000

LIVE_STATES = ("available", "stale")
FLOORS = {"status": MIN_STATUS_INTERVAL_MS, "suggestions": MIN_SUGGESTIONS_INTERVAL_MS}
COMMANDS = {"status": "status", "suggestions": "suggestions"}


@dataclass(frozen=True)
class ChannelView:
    state: str
    value: object
    age_ms: int | None
    refreshed_age_ms: int | None
    last_value: object
    last_age_ms: int | None
    error_code: str | None
    message: str | None


@dataclass(frozen=True)
class CacheView:
    status: ChannelView
    suggestions: ChannelView
    revision: int


@dataclass(frozen=True)
class _Record:
    state: str = "not_refreshed"
    error_code: str | None = None
    message: str | None = None
    last: object = None  # last successful value
    last_age_ms: int | None = None  # data age of `last` when it was fetched
    last_mono: int | None = None  # monotonic ms of the last successful fetch


@dataclass(frozen=True)
class _Snapshot:
    status: _Record = _Record()
    suggestions: _Record = _Record()
    revision: int = 0


def _wall_ms():
    return int(time.time() * 1000)


def _mono_ms():
    return int(time.monotonic() * 1000)


def _channel_view(record, now, stale_bound_ms):
    grown = None if record.last_mono is None else record.last_age_ms + now - record.last_mono
    live = record.state in LIVE_STATES
    state = record.state
    if live and grown > stale_bound_ms:
        state = "stale"
    return ChannelView(
        state=state,
        value=record.last if live else None,
        age_ms=grown if live else None,
        refreshed_age_ms=None if record.last_mono is None else now - record.last_mono,
        last_value=record.last,
        last_age_ms=grown,
        error_code=record.error_code,
        message=record.message,
    )


class TelemetryCache:
    def __init__(
        self,
        *,
        status_interval_ms=DEFAULT_STATUS_INTERVAL_MS,
        suggestions_interval_ms=DEFAULT_SUGGESTIONS_INTERVAL_MS,
        deadline_ms=REQUEST_DEADLINE_MS,
        socket_path=None,
        request_factory=NonblockingRequest,
        now_ms=None,
        monotonic_ms=None,
    ):
        intervals = {"status": status_interval_ms, "suggestions": suggestions_interval_ms}
        for channel, interval in intervals.items():
            if isinstance(interval, bool) or not isinstance(interval, int):
                raise TypeError(f"{channel} interval must be an integer")
            if interval < FLOORS[channel]:
                raise ValueError(f"{channel} interval must be at least {FLOORS[channel]} ms")
        if not callable(request_factory):
            raise TypeError("request_factory must be callable")
        self._intervals = intervals
        self._deadline_ms = deadline_ms
        self._socket_path = socket_path
        self._request_factory = request_factory
        self._now_ms = now_ms or _wall_ms
        self._mono_ms = monotonic_ms or _mono_ms
        self._snapshot = _Snapshot()
        self._due = {"status": 0, "suggestions": 0}
        self._failures = {"status": 0, "suggestions": 0}
        self._active = None  # (channel, request) while one request is in flight

    def view(self):
        snapshot = self._snapshot
        now = self._mono_ms()
        return CacheView(
            status=_channel_view(snapshot.status, now, STALE_FACTOR * self._intervals["status"]),
            suggestions=_channel_view(
                snapshot.suggestions, now, STALE_FACTOR * self._intervals["suggestions"]
            ),
            revision=snapshot.revision,
        )

    def advance(self, mono_ms):
        """Take one nonblocking step. Return ms until the next useful call."""
        if self._active is None:
            channel = self._next_due_channel(mono_ms)
            if channel is None:
                return max(0, min(self._due.values()) - mono_ms)
            request = self._request_factory(
                COMMANDS[channel],
                self._socket_path,
                mono_ms + self._deadline_ms,
                self._now_ms,
                STALE_FACTOR * self._intervals[channel],
            )
            self._active = (channel, request)
        channel, request = self._active
        result = request.advance(mono_ms)
        if result is None:
            return PROGRESS_MS
        self._active = None
        self._ingest(channel, result, mono_ms)
        self._schedule_next(channel, result, mono_ms)
        return max(0, min(self._due.values()) - mono_ms)

    def close(self):
        """Cancel the request in flight, if any. The last view stays readable."""
        if self._active is not None:
            self._active[1].cancel()
            self._active = None

    def _next_due_channel(self, mono_ms):
        ready = [c for c in ("status", "suggestions") if self._due[c] <= mono_ms]
        return min(ready, key=lambda c: self._due[c]) if ready else None

    def _schedule_next(self, channel, result, mono_ms):
        interval = self._intervals[channel]
        if result.state in LIVE_STATES or result.state == "warming_up":
            self._failures[channel] = 0
            self._due[channel] = mono_ms + interval
            return
        self._failures[channel] += 1
        backoff = min(BACKOFF_MIN_MS * 2 ** (self._failures[channel] - 1), BACKOFF_MAX_MS)
        self._due[channel] = mono_ms + backoff

    def _ingest(self, channel, result, fetched_mono_ms):
        if not isinstance(result, ClientResult):
            raise TypeError("result must be a ClientResult")
        snapshot = self._snapshot
        old = getattr(snapshot, channel)
        if result.state in LIVE_STATES:
            new = _Record(
                state=result.state,
                last=result.value,
                last_age_ms=result.age_ms,
                last_mono=fetched_mono_ms,
            )
        else:
            new = replace(old, state=result.state, error_code=result.error_code, message=result.message)
        changed = (new.state, new.last) != (old.state, old.last)
        self._snapshot = replace(
            snapshot,
            revision=snapshot.revision + (1 if changed else 0),
            **{channel: new},
        )


_shared = None


def get_shared(**kwargs):
    global _shared
    if _shared is None:
        _shared = TelemetryCache(**kwargs)
    return _shared
