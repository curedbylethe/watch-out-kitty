"""Client-side cache for the `status` and `suggestions` daemon replies.

Drawing code reads `TelemetryCache.view()`, which only reads one immutable
snapshot and does no I/O. `_ingest` replaces the whole snapshot with a single
attribute assignment. CPython makes that swap atomic, so readers never see a
half-written snapshot. Only one writer (the refresh worker) exists.

Polling limits: `status` at most every 1000 ms, `suggestions` at most every
5000 ms. A shorter interval raises `ValueError`. A channel is `stale` when its
data is older than three intervals (3000 ms, 15000 ms).
"""

import time
from dataclasses import dataclass, replace

from syswatch_client import DEFAULT_TIMEOUT_MS, ClientResult, read_status, read_suggestions

DEFAULT_STATUS_INTERVAL_MS = 1000
DEFAULT_SUGGESTIONS_INTERVAL_MS = 5000
MIN_STATUS_INTERVAL_MS = 1000
MIN_SUGGESTIONS_INTERVAL_MS = 5000
STALE_FACTOR = 3

LIVE_STATES = ("available", "stale")
FLOORS = {"status": MIN_STATUS_INTERVAL_MS, "suggestions": MIN_SUGGESTIONS_INTERVAL_MS}


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
        timeout_ms=DEFAULT_TIMEOUT_MS,
        socket_path=None,
        read_status=read_status,
        read_suggestions=read_suggestions,
        now_ms=None,
        monotonic_ms=None,
    ):
        intervals = {"status": status_interval_ms, "suggestions": suggestions_interval_ms}
        for channel, interval in intervals.items():
            if isinstance(interval, bool) or not isinstance(interval, int):
                raise TypeError(f"{channel} interval must be an integer")
            if interval < FLOORS[channel]:
                raise ValueError(f"{channel} interval must be at least {FLOORS[channel]} ms")
        if not (callable(read_status) and callable(read_suggestions)):
            raise TypeError("readers must be callable")
        self._intervals = intervals
        self._timeout_ms = timeout_ms
        self._socket_path = socket_path
        self._read_status = read_status
        self._read_suggestions = read_suggestions
        self._now_ms = now_ms or _wall_ms
        self._mono_ms = monotonic_ms or _mono_ms
        self._snapshot = _Snapshot()

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
