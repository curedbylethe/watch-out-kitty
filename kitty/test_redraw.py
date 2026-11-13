import ast
import pathlib
import types
import unittest

import syswatch_redraw
from syswatch_redraw import install_redraw_timer


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


if __name__ == "__main__":
    unittest.main()
