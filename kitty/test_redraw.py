import ast
import pathlib
import types
import unittest
from unittest import mock

import probe_tab_bar
import syswatch_redraw
from syswatch_cache import TelemetryCache
from syswatch_client import ClientResult, Memory, Status
from syswatch_redraw import install_redraw_timer

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=12.4,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=4),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)


class FakeManager:
    def __init__(self):
        self.marked = 0

    def mark_tab_bar_dirty(self):
        self.marked += 1


class FakeTimer:
    def __init__(self, result=7):
        self.calls = []
        self.result = result

    def __call__(self, callback, interval, repeats):
        self.calls.append((callback, interval, repeats))
        return self.result


class RedrawTest(unittest.TestCase):
    def setUp(self):
        syswatch_redraw._timer_id = None
        syswatch_redraw._last_revision = None
        self.view = types.SimpleNamespace(revision=0)
        self.get_view = lambda: self.view
        self.timer = FakeTimer()
        self.managers = [FakeManager()]

    def install(self, **overrides):
        options = {
            "add_timer": self.timer,
            "get_tab_managers": lambda: self.managers,
        }
        options.update(overrides)
        return install_redraw_timer(self.get_view, **options)

    def tick(self):
        self.timer.calls[0][0](7)

    def test_install_adds_one_repeating_timer_in_seconds(self):
        self.assertTrue(self.install())
        self.assertEqual(len(self.timer.calls), 1)
        callback, interval, repeats = self.timer.calls[0]
        self.assertTrue(callable(callback))
        self.assertEqual((interval, repeats), (1.0, True))

    def test_second_install_adds_nothing(self):
        self.install()
        self.assertTrue(self.install(get_tab_managers=lambda: []))
        self.assertEqual(len(self.timer.calls), 1)

    def test_without_kitty_install_returns_false(self):
        self.assertFalse(install_redraw_timer(self.get_view))

    def test_timer_failure_returns_false_and_allows_retry(self):
        def broken(callback, interval, repeats):
            raise RuntimeError("no timer")

        self.assertFalse(self.install(add_timer=broken))
        self.assertTrue(self.install())

    def test_manager_lookup_failure_returns_false_without_timer(self):
        def broken():
            raise AttributeError("os_window_map")

        self.assertFalse(self.install(get_tab_managers=broken))
        self.assertEqual(self.timer.calls, [])

    def test_install_seeds_revision_so_first_tick_is_quiet(self):
        self.view.revision = 5
        self.install()
        self.tick()
        self.assertEqual([m.marked for m in self.managers], [0])

    def test_changed_revision_marks_every_manager_once(self):
        self.managers.append(FakeManager())
        self.install()
        self.view.revision = 1
        self.tick()
        self.tick()
        self.assertEqual([m.marked for m in self.managers], [1, 1])

    def test_failing_manager_does_not_stop_the_others(self):
        class Broken(FakeManager):
            def mark_tab_bar_dirty(self):
                raise RuntimeError("closed window")

        self.managers[:0] = [Broken()]
        self.managers.append(FakeManager())
        self.install()
        self.view.revision = 1
        self.tick()
        self.assertEqual(self.managers[1].marked, 1)

    def test_manager_without_method_is_skipped(self):
        self.managers.append(object())
        self.install()
        self.view.revision = 1
        self.tick()
        self.assertEqual(self.managers[0].marked, 1)

    def test_view_error_is_swallowed_and_next_tick_works(self):
        calls = []

        def flaky_view():
            calls.append(1)
            if len(calls) == 2:  # call 1 seeds the revision during install
                raise RuntimeError("cache not ready")
            return self.view

        self.get_view = flaky_view
        self.install()
        self.view.revision = 1
        self.tick()
        self.assertEqual(self.managers[0].marked, 0)
        self.tick()
        self.assertEqual(self.managers[0].marked, 1)

    def test_redraw_module_has_no_kitty_import_or_io(self):
        path = pathlib.Path(__file__).with_name("syswatch_redraw.py")
        tree = ast.parse(path.read_text())
        top_level = {
            alias.name.split(".")[0]
            for node in tree.body
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        top_level |= {
            node.module.split(".")[0]
            for node in tree.body
            if isinstance(node, ast.ImportFrom) and node.module
        }
        self.assertFalse(top_level & {"kitty", "socket", "subprocess", "threading"})


class FakeScreen:
    def __init__(self):
        self.drawn = []
        self.cursor = types.SimpleNamespace(x=9)

    def draw(self, text):
        self.drawn.append(text)


class FakeCache:
    def __init__(self, view):
        self._view = view
        self.started = 0

    def start(self):
        self.started += 1

    def view(self):
        return self._view


class ProbeTest(unittest.TestCase):
    def setUp(self):
        probe_tab_bar._started = False
        probe_tab_bar._timer_ok = False
        probe_tab_bar._cache = None
        self.screen = FakeScreen()
        self.installs = []
        self.cache = FakeCache(TelemetryCache(monotonic_ms=lambda: 0).view())
        self.patch("get_shared", lambda **kwargs: self.cache)
        self.patch("install_redraw_timer", self.install)

    def patch(self, name, replacement):
        patcher = mock.patch.object(probe_tab_bar, name, replacement)
        patcher.start()
        self.addCleanup(patcher.stop)

    def install(self, view):
        self.installs.append(view)
        return False

    def draw(self, is_last):
        return probe_tab_bar.draw_tab(None, self.screen, None, 3, 10, 1, is_last, None)

    def test_non_last_tab_returns_before_and_draws_nothing(self):
        self.assertEqual(self.draw(is_last=False), 3)
        self.assertEqual(self.screen.drawn, [])

    def test_last_tab_draws_placeholder_before_first_refresh(self):
        self.assertEqual(self.draw(is_last=True), 9)
        self.assertEqual(self.screen.drawn, ["SW ... E"])

    def test_last_tab_draws_live_values_and_revision(self):
        cache = TelemetryCache(monotonic_ms=lambda: 0)
        cache._ingest(
            "status",
            ClientResult("available", value=STATUS, age_ms=0),
            0,
        )
        self.cache._view = cache.view()
        self.draw(is_last=True)
        self.assertEqual(self.screen.drawn, ["SW r1 12% 50% E"])

    def test_failed_refresh_shows_state_not_stale_numbers(self):
        cache = TelemetryCache(monotonic_ms=lambda: 0)
        cache._ingest("status", ClientResult("daemon_unavailable"), 0)
        self.cache._view = cache.view()
        self.draw(is_last=True)
        self.assertEqual(self.screen.drawn, ["SW daemon_unavailable E"])

    def test_timer_is_installed_once_across_draws(self):
        self.draw(is_last=True)
        self.draw(is_last=True)
        self.assertEqual(self.cache.started, 1)
        self.assertEqual(len(self.installs), 1)

    def test_probe_source_does_no_io_or_config_writes(self):
        path = pathlib.Path(__file__).with_name("probe_tab_bar.py")
        source = path.read_text()
        for forbidden in ("socket", "subprocess", "sleep(", "open(", "request_line", "kitten", "tab_bar_style"):
            self.assertNotIn(forbidden, source)
        tree = ast.parse(source)
        imported = {
            alias.name.split(".")[0]
            for node in ast.walk(tree)
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        self.assertFalse(imported & {"kitty", "socket", "subprocess", "asyncio", "time"})


if __name__ == "__main__":
    unittest.main()
