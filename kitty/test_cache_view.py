import ast
import dataclasses
import pathlib
import sys
import unittest

from syswatch_cache import TelemetryCache
from syswatch_client import ClientResult, Memory, Status, Suggestions

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=1.5,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=4),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)
OTHER_STATUS = dataclasses.replace(STATUS, cpu_usage_percent=2.5)
SUGGESTIONS = Suggestions(evaluated_at_ms=WALL_MS, processes_updated_at_ms=WALL_MS, findings=())


class CacheViewTest(unittest.TestCase):
    def setUp(self):
        self.ms = 0
        self.cache = TelemetryCache(monotonic_ms=lambda: self.ms)

    def test_initial_view_is_not_refreshed(self):
        view = self.cache.view()
        self.assertEqual(view.revision, 0)
        for channel in (view.status, view.suggestions):
            self.assertEqual(channel.state, "not_refreshed")
            self.assertIsNone(channel.value)
            self.assertIsNone(channel.last_value)
            self.assertIsNone(channel.refreshed_age_ms)

    def test_success_reports_age_and_grows_with_time(self):
        self.ms = 1000
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=50), self.ms)
        view = self.cache.view().status
        self.assertEqual(view.state, "available")
        self.assertIs(view.value, STATUS)
        self.assertEqual((view.age_ms, view.refreshed_age_ms), (50, 0))
        self.ms = 1500
        view = self.cache.view().status
        self.assertEqual((view.age_ms, view.refreshed_age_ms, view.last_age_ms), (550, 500, 550))
        self.assertIs(view.last_value, STATUS)

    def test_suggestions_refresh_leaves_status_alone(self):
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 0)
        before = self.cache.view()
        self.cache._ingest("suggestions", ClientResult("available", value=SUGGESTIONS, age_ms=0), 0)
        after = self.cache.view()
        self.assertEqual(after.status, before.status)
        self.assertEqual(after.revision, before.revision + 1)

    def test_identical_result_keeps_revision(self):
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 0)
        revision = self.cache.view().revision
        self.ms = 400
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 400)
        self.assertEqual(self.cache.view().revision, revision)
        self.ms = 500
        self.assertEqual(self.cache.view().status.age_ms, 100)

    def test_changed_result_bumps_revision_once(self):
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 0)
        revision = self.cache.view().revision
        self.cache._ingest("status", ClientResult("available", value=OTHER_STATUS, age_ms=0), 0)
        self.assertEqual(self.cache.view().revision, revision + 1)

    def test_failure_after_success_keeps_last_value_and_ages(self):
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=10), 0)
        self.ms = 200
        self.cache._ingest(
            "status", ClientResult("daemon_error", error_code="internal", message="boom"), 200
        )
        view = self.cache.view().status
        self.assertEqual(view.state, "daemon_error")
        self.assertIsNone(view.value)
        self.assertIsNone(view.age_ms)
        self.assertIs(view.last_value, STATUS)
        self.assertEqual((view.last_age_ms, view.refreshed_age_ms), (210, 200))
        self.assertEqual((view.error_code, view.message), ("internal", "boom"))

    def test_failure_before_success_has_no_last_value(self):
        self.cache._ingest("status", ClientResult("timeout"), 0)
        view = self.cache.view().status
        self.assertEqual(view.state, "timeout")
        self.assertIsNone(view.last_value)
        self.assertIsNone(view.last_age_ms)
        self.assertIsNone(view.refreshed_age_ms)

    def test_live_value_turns_stale_at_view_time_without_fetch(self):
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 0)
        self.ms = 3000
        self.assertEqual(self.cache.view().status.state, "available")
        self.ms = 3001
        view = self.cache.view().status
        self.assertEqual(view.state, "stale")
        self.assertIs(view.value, STATUS)

    def test_interval_floors_and_types(self):
        for bad in (999, 0, -1):
            with self.assertRaises(ValueError):
                TelemetryCache(status_interval_ms=bad)
        for bad in (4999, 0, -1):
            with self.assertRaises(ValueError):
                TelemetryCache(suggestions_interval_ms=bad)
        TelemetryCache(status_interval_ms=1000, suggestions_interval_ms=5000)
        TelemetryCache(status_interval_ms=60_000, suggestions_interval_ms=300_000)
        with self.assertRaises(TypeError):
            TelemetryCache(status_interval_ms=1000.5)

    def test_warming_up_and_protocol_mismatch_have_no_value(self):
        self.cache._ingest("status", ClientResult("warming_up"), 0)
        self.assertEqual(self.cache.view().status.state, "warming_up")
        self.assertIsNone(self.cache.view().status.value)
        self.cache._ingest("suggestions", ClientResult("protocol_mismatch"), 0)
        self.assertEqual(self.cache.view().suggestions.state, "protocol_mismatch")
        self.assertIsNone(self.cache.view().suggestions.value)

    def test_views_are_frozen(self):
        view = self.cache.view()
        with self.assertRaises(dataclasses.FrozenInstanceError):
            view.revision = 1
        with self.assertRaises(dataclasses.FrozenInstanceError):
            view.status.state = "available"

    def test_module_imports_only_allowed_modules(self):
        path = pathlib.Path(__file__).with_name("syswatch_cache.py")
        names = set()
        for node in ast.walk(ast.parse(path.read_text())):
            if isinstance(node, ast.Import):
                names |= {alias.name.split(".")[0] for alias in node.names}
            elif isinstance(node, ast.ImportFrom) and node.level == 0:
                names.add(node.module.split(".")[0])
        self.assertFalse(names & {"kitty", "socket", "subprocess", "asyncio"})
        self.assertLessEqual(names, set(sys.stdlib_module_names) | {"syswatch_client"})


if __name__ == "__main__":
    unittest.main()
