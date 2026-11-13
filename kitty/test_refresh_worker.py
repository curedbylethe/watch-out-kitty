import threading
import time
import unittest

import syswatch_cache
from syswatch_cache import TelemetryCache, get_shared, stop_shared
from syswatch_client import ClientResult, Memory, Status

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=1.5,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=4),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)
OTHER_STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=2.5,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=5),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)


class Clock:
    def __init__(self):
        self.ms = 0

    def __call__(self):
        return self.ms


class FakeReader:
    """Returns `result`; advances the clock by `advance_ms` or raises `error` when set."""

    def __init__(self, clock, result=None, advance_ms=0, error=None):
        self.clock = clock
        self.result = result
        self.advance_ms = advance_ms
        self.error = error
        self.calls = 0

    def __call__(self, socket_path=None, *, timeout_ms, now_ms, max_age_ms):
        self.calls += 1
        self.clock.ms += self.advance_ms
        if self.error is not None:
            raise self.error
        return self.result


def make_cache(clock, status=None, suggestions=None):
    status = status or FakeReader(clock, ClientResult("available", value=STATUS, age_ms=0))
    suggestions = suggestions or FakeReader(clock, ClientResult("warming_up"))
    cache = TelemetryCache(
        monotonic_ms=clock,
        now_ms=lambda: WALL_MS + clock.ms,
        read_status=status,
        read_suggestions=suggestions,
    )
    return cache, status, suggestions


def refresh_threads():
    return [t for t in threading.enumerate() if t.name == "syswatch-refresh"]


class StepScheduleTest(unittest.TestCase):
    def test_both_channels_fetch_at_zero_then_each_on_its_own_interval(self):
        clock = Clock()
        cache, status, suggestions = make_cache(clock)
        self.assertEqual(cache._step(0), 1000)
        self.assertEqual((status.calls, suggestions.calls), (1, 1))
        clock.ms = 999
        self.assertEqual(cache._step(999), 1000)
        self.assertEqual((status.calls, suggestions.calls), (1, 1))
        clock.ms = 1000
        self.assertEqual(cache._step(1000), 2000)
        self.assertEqual((status.calls, suggestions.calls), (2, 1))
        clock.ms = 5000
        self.assertEqual(cache._step(5000), 6000)
        self.assertEqual((status.calls, suggestions.calls), (3, 2))

    def test_next_fetch_is_due_from_completion_not_from_schedule(self):
        clock = Clock()
        status = FakeReader(clock, ClientResult("available", value=STATUS, age_ms=0), advance_ms=400)
        cache, status, _ = make_cache(clock, status=status)
        self.assertEqual(cache._step(0), 1400)
        clock.ms = 1399
        cache._step(1399)
        self.assertEqual(status.calls, 1)
        clock.ms = 1400
        cache._step(1400)
        self.assertEqual(status.calls, 2)

    def test_identical_timestamp_fetches_once(self):
        clock = Clock()
        cache, status, _ = make_cache(clock)
        cache._step(0)
        cache._step(0)
        self.assertEqual(status.calls, 1)

    def test_failure_keeps_last_value_then_recovers(self):
        clock = Clock()
        status = FakeReader(clock, ClientResult("available", value=STATUS, age_ms=0))
        cache, status, _ = make_cache(clock, status=status)
        cache._step(0)
        revision = cache.view().revision
        status.result = ClientResult("daemon_unavailable")
        cache._step(1000)
        view = cache.view().status
        self.assertEqual((view.state, view.value), ("daemon_unavailable", None))
        self.assertIs(view.last_value, STATUS)
        status.result = ClientResult("available", value=OTHER_STATUS, age_ms=0)
        cache._step(2000)
        view = cache.view()
        self.assertIs(view.status.value, OTHER_STATUS)
        self.assertGreater(view.revision, revision)

    def test_unexpected_reader_error_keeps_view_and_retries(self):
        clock = Clock()
        status = FakeReader(clock, ClientResult("available", value=STATUS, age_ms=0))
        cache, status, _ = make_cache(clock, status=status)
        cache._step(0)
        before = cache.view()
        status.error = RuntimeError("bug in reader")
        clock.ms = 1000
        cache._step(1000)
        self.assertIs(cache.view().status.last_value, STATUS)
        self.assertEqual(cache.view().revision, before.revision)
        status.error = None
        status.result = ClientResult("available", value=OTHER_STATUS, age_ms=0)
        clock.ms = 2000
        cache._step(2000)
        self.assertIs(cache.view().status.value, OTHER_STATUS)


class ThreadLifecycleTest(unittest.TestCase):
    def setUp(self):
        self.fetched = threading.Event()
        self.cache = TelemetryCache(
            read_status=self.reader, read_suggestions=self.reader
        )

    def tearDown(self):
        self.cache.stop()

    def reader(self, socket_path=None, *, timeout_ms, now_ms, max_age_ms):
        self.fetched.set()
        return ClientResult("available", value=STATUS, age_ms=0)

    def test_module_import_creates_no_thread(self):
        self.assertEqual(refresh_threads(), [])

    def test_start_twice_keeps_one_thread_and_first_fetch_is_prompt(self):
        self.cache.start()
        self.cache.start()
        self.assertTrue(self.fetched.wait(1))
        self.assertEqual(len(refresh_threads()), 1)
        self.cache.stop()
        self.assertEqual(refresh_threads(), [])

    def test_stop_is_prompt_and_idempotent_and_restart_works(self):
        self.cache.start()
        self.assertTrue(self.fetched.wait(1))
        started = time.monotonic()
        self.cache.stop()
        self.assertLess(time.monotonic() - started, 1.5)
        self.cache.stop()
        self.assertEqual(refresh_threads(), [])
        self.fetched.clear()
        self.cache.start()
        self.assertTrue(self.fetched.wait(1))
        self.assertEqual(len(refresh_threads()), 1)

    def test_view_stays_readable_after_stop(self):
        self.cache.start()
        self.assertTrue(self.fetched.wait(1))
        self.cache.stop()
        self.assertIs(self.cache.view().status.value, STATUS)


class SharedInstanceTest(unittest.TestCase):
    def tearDown(self):
        stop_shared()

    def test_get_shared_returns_one_object_and_ignores_later_kwargs(self):
        first = get_shared(status_interval_ms=2000)
        self.assertIs(get_shared(status_interval_ms=1000), first)
        self.assertEqual(first._intervals["status"], 2000)

    def test_stop_shared_clears_the_singleton(self):
        first = get_shared()
        stop_shared()
        self.assertIsNot(get_shared(), first)
        self.assertIsInstance(get_shared(), syswatch_cache.TelemetryCache)


if __name__ == "__main__":
    unittest.main()
