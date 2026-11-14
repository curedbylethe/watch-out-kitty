"""Drive the telemetry cache from Kitty's main loop. No threads.

One chain of one-shot timers calls `cache.advance()`, then schedules the next
call for the delay it returns. A generation token stops an outdated timer from
reviving a stopped cache. Kitty's `add_timer` is looked up lazily, so this
module imports without Kitty installed.
"""

import sys
import time

BACKOFF_ON_ERROR_MS = 1000
MAX_REPORTED_ERRORS = 32

_cache = None
_generation = 0
_reported = set()


def report_error(where, exc):
    """Print each distinct error once to stderr (Kitty's log). Never raises."""
    key = (where, type(exc).__name__, str(exc))
    if key in _reported or len(_reported) >= MAX_REPORTED_ERRORS:
        return
    _reported.add(key)
    try:
        print(f"syswatch: {where}: {type(exc).__name__}: {exc}", file=sys.stderr)
    except Exception:  # a closed stderr must not break the timer chain
        pass


def _mono_ms():
    return int(time.monotonic() * 1000)


def _schedule(delay_ms, add_timer):
    generation = _generation
    add_timer(lambda timer_id: _tick(generation, add_timer), delay_ms / 1000.0, False)


def _tick(generation, add_timer):
    if generation != _generation or _cache is None:
        return
    try:
        delay_ms = _cache.advance(_mono_ms())
    except Exception as exc:  # a cache bug drops the request and retries later
        report_error("refresh", exc)
        _cache.close()
        delay_ms = BACKOFF_ON_ERROR_MS
    _schedule(delay_ms, add_timer)


def install_refresh_timer(cache, *, add_timer=None):
    """Start driving `cache` from the main loop. Returns False when Kitty's timer is unavailable."""
    global _cache
    if _cache is not None:
        return True
    try:
        if add_timer is None:
            from kitty.fast_data_types import add_timer
        _cache = cache
        _schedule(0, add_timer)
    except Exception:  # ImportError and missing Kitty surface are the fallback
        _cache = None
        return False
    return True


def stop_refresh_timer():
    """Stop the chain. A timer already queued fires once and does nothing."""
    global _cache, _generation
    _generation += 1
    if _cache is not None:
        _cache.close()
        _cache = None
