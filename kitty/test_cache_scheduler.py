import unittest

import syswatch_refresh
from unittest import mock
from syswatch_cache import PROGRESS_MS, TelemetryCache
from syswatch_client import ClientResult, Memory, Status

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=1.0,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=4),
    process_count=1,
    processes_updated_at_ms=WALL_MS,
)
NEVER = 10**12


class FakeRequest:
    """Scripted request: returns `result` on its `steps`-th advance."""

    def __init__(self, command, deadline, max_age_ms, result, steps):
        self.command, self.deadline, self.max_age_ms = command, deadline, max_age_ms
        self.result, self.steps, self.calls, self.cancelled = result, steps, 0, False

    def advance(self, mono_ms):
        self.calls += 1
        return self.result if self.calls >= self.steps else None

    def cancel(self):
        self.cancelled = True


class Script:
    """Pops one (result, steps) per request and records every request made."""

    def __init__(self, *items):
        self.items = list(items)
        self.made = []

    def __call__(self, command, socket_path, deadline, wall_ms, max_age_ms):
        result, steps = self.items.pop(0)
        request = FakeRequest(command, deadline, max_age_ms, result, steps)
        self.made.append(request)
        return request


def live(value=STATUS):
    return ClientResult("available", value=value, age_ms=0)


def cache_with(script, clock=None, **kwargs):
    kwargs.setdefault("suggestions_interval_ms", 5000)
    cache = TelemetryCache(request_factory=script, monotonic_ms=lambda: clock[0] if clock else 0, **kwargs)
    cache._due["suggestions"] = NEVER  # tests that need suggestions set its due time themselves
    return cache


class SchedulerTest(unittest.TestCase):
    def test_status_runs_first_and_progress_is_short(self):
        script = Script((live(), 3))
        cache = cache_with(script, clock=[0], suggestions_interval_ms=5000)
        self.assertEqual(cache.advance(0), PROGRESS_MS)
        self.assertEqual(cache.advance(1), PROGRESS_MS)
        cache.advance(2)
        self.assertEqual(script.made[0].command, "status")
        self.assertEqual(cache._due["status"], 1002)  # completion + interval
        self.assertEqual(cache.view().status.state, "available")

    def test_only_one_request_in_flight(self):
        script = Script((live(), 5))
        cache = cache_with(script)
        for t in range(4):
            cache.advance(t)
        self.assertEqual(len(script.made), 1)

    def test_next_request_is_due_from_completion_not_from_schedule(self):
        script = Script((live(), 1), (ClientResult("warming_up"), 1))
        clock = [0]
        cache = cache_with(script, clock=clock)
        cache.advance(0)  # status completes at t=0: next due at 1000
        clock[0] = 400
        cache._due["suggestions"] = 0
        delay = cache.advance(400)  # suggestions starts late at t=400 and completes at once
        self.assertEqual(cache._due["status"], 1000)
        self.assertEqual(delay, 600)

    def test_warming_up_retries_on_the_channel_interval(self):
        script = Script((ClientResult("warming_up"), 1))
        cache = cache_with(script, status_interval_ms=1000)
        cache.advance(0)
        self.assertEqual(cache._due["status"], 1000)
        self.assertEqual(cache._failures["status"], 0)

    def test_failures_back_off_then_success_resets(self):
        down = ClientResult("daemon_unavailable")
        script = Script(*[(down, 1)] * 5 + [(live(), 1)])
        cache = cache_with(script)
        gaps, t = [], 0
        for _ in range(5):
            cache.advance(t)
            gaps.append(cache._due["status"] - t)
            t = cache._due["status"]
        self.assertEqual(gaps, [1000, 2000, 4000, 8000, 10000])
        cache.advance(t)
        self.assertEqual(cache._failures["status"], 0)

    def test_request_gets_deadline_and_stale_bound(self):
        script = Script((live(), 1))
        cache = cache_with(script, clock=[100], deadline_ms=250, status_interval_ms=1000)
        cache.advance(100)
        self.assertEqual(script.made[0].deadline, 350)
        self.assertEqual(script.made[0].max_age_ms, 3000)

    def test_close_cancels_the_request_and_the_next_one_starts_fresh(self):
        script = Script((live(), 9), (live(), 1))
        cache = cache_with(script)
        cache.advance(0)
        cache.close()
        self.assertTrue(script.made[0].cancelled)
        cache.advance(1)
        self.assertEqual(len(script.made), 2)

    def test_failure_keeps_last_value_visible_as_last(self):
        clock = [0]
        script = Script((live(), 1), (ClientResult("timeout"), 1))
        cache = cache_with(script, clock=clock)
        cache.advance(0)
        clock[0] = 1000
        cache._due["status"] = 1000
        cache.advance(1000)
        view = cache.view().status
        self.assertEqual(view.state, "timeout")
        self.assertIsNone(view.value)
        self.assertIsNotNone(view.last_value)


class RefreshDriverTest(unittest.TestCase):
    def setUp(self):
        syswatch_refresh._cache = None
        self.timers = []
        self.cache = mock.Mock()
        self.cache.advance.return_value = 1000

    def add_timer(self, callback, interval, repeats):
        self.timers.append((callback, interval, repeats))
        return len(self.timers)

    def fire(self):
        self.timers[-1][0](0)

    def test_install_schedules_a_one_shot_and_advances_on_fire(self):
        self.assertTrue(syswatch_refresh.install_refresh_timer(self.cache, add_timer=self.add_timer))
        self.assertEqual(self.timers[0][1:], (0.0, False))
        self.fire()
        self.cache.advance.assert_called_once()
        self.assertEqual(self.timers[-1][1:], (1.0, False))

    def test_second_install_adds_no_timer(self):
        syswatch_refresh.install_refresh_timer(self.cache, add_timer=self.add_timer)
        self.assertTrue(syswatch_refresh.install_refresh_timer(mock.Mock(), add_timer=self.add_timer))
        self.assertEqual(len(self.timers), 1)

    def test_stop_makes_a_queued_timer_do_nothing(self):
        syswatch_refresh.install_refresh_timer(self.cache, add_timer=self.add_timer)
        syswatch_refresh.stop_refresh_timer()
        self.fire()
        self.cache.advance.assert_not_called()
        self.cache.close.assert_called()

    def test_advance_exception_closes_request_and_retries_later(self):
        syswatch_refresh.install_refresh_timer(self.cache, add_timer=self.add_timer)
        self.cache.advance.side_effect = RuntimeError("bug")
        self.fire()
        self.cache.close.assert_called()
        self.assertEqual(self.timers[-1][1], 1.0)

    def test_missing_kitty_timer_returns_false(self):
        self.assertFalse(syswatch_refresh.install_refresh_timer(self.cache))
        self.assertIsNone(syswatch_refresh._cache)


if __name__ == "__main__":
    unittest.main()
