import ast
import dataclasses
import pathlib
import types
import unittest

from syswatch_cache import CacheView, ChannelView, TelemetryCache
from syswatch_client import ClientResult, Evidence, Finding, Memory, Status, Suggestions, read_suggestions
from syswatch_indicator import MAX_INDICATOR_TEXT, fit, render
from test_responses import finding as wire_finding
from test_responses import run, suggestions_body

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=18.0,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=100, used_bytes=42),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)
OFFLINE = (
    "daemon_unavailable",
    "timeout",
    "permission_denied",
    "socket_path_invalid",
    "malformed_response",
    "protocol_mismatch",
    "daemon_error",
)


def channel(state, value=None):
    return ChannelView(state, value, None, None, None, None, None, None)


def view(state, value=None, advisory=None):
    if advisory is None:
        advisory = channel("available", Suggestions(WALL_MS, WALL_MS, ()))
    return CacheView(status=channel(state, value), suggestions=advisory, revision=0)


def _finding(kind="memory", explanation="pid 42 held memory"):
    return Finding(42, 7, kind, "high", WALL_MS, explanation, Evidence(6, 120_000, None, None))


def _suggestions(count):
    return Suggestions(WALL_MS, WALL_MS, tuple(_finding() for _ in range(count)))


def fresh(count):
    return channel("available", _suggestions(count))


def stale(count):
    return channel("stale", _suggestions(count))


def status(cpu=18.0, cpu_at=WALL_MS, used=42, total=100):
    return dataclasses.replace(
        STATUS, cpu_usage_percent=cpu, cpu_updated_at_ms=cpu_at, memory=Memory(total_bytes=total, used_bytes=used)
    )


class RenderTest(unittest.TestCase):
    def test_placeholder_states(self):
        for state in ("not_refreshed", "warming_up"):
            with self.subTest(state=state):
                self.assertEqual(render(view(state)), "CPU --  RAM --")

    def test_offline_states(self):
        for state in OFFLINE:
            with self.subTest(state=state):
                self.assertEqual(render(view(state)), "syswatch offline")

    def test_available_rows(self):
        cases = [
            (status(), "CPU 18%  RAM 42%"),
            (status(cpu_at=None), "CPU --  RAM 42%"),
            (status(used=0, total=0), "CPU 18%  RAM --"),
            (status(total=-1), "CPU 18%  RAM --"),
        ]
        for value, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(render(view("available", value)), expected)

    def test_stale_prefixes_only_numbers(self):
        cases = [
            (status(), "CPU ~18%  RAM ~42%"),
            (status(cpu_at=None), "CPU --  RAM ~42%"),
            (status(total=0), "CPU ~18%  RAM --"),
        ]
        for value, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(render(view("stale", value)), expected)

    def test_unknown_state_or_missing_value_gives_none(self):
        self.assertIsNone(render(view("bogus", status())))
        self.assertIsNone(render(view("available")))
        self.assertIsNone(render(view("stale")))

    def test_cpu_clamps_and_rounds_half_up(self):
        cases = [(0, "0%"), (0.4, "0%"), (0.5, "1%"), (99.5, "100%"), (100.0, "100%"), (123.4, "100%"), (-3, "0%")]
        for cpu, expected in cases:
            with self.subTest(cpu=cpu):
                self.assertEqual(render(view("available", status(cpu=cpu))), f"CPU {expected}  RAM 42%")

    def test_memory_percent(self):
        cases = [((0, 0), "--"), ((1, 3), "33%"), ((100, 100), "100%"), ((3, 2), "100%")]
        for (used, total), expected in cases:
            with self.subTest(used=used, total=total):
                self.assertEqual(render(view("available", status(used=used, total=total))), f"CPU 18%  RAM {expected}")

    def test_output_is_ascii(self):
        self.assertTrue(render(view("stale", status())).isascii())

    def test_badge_table(self):
        cases = [
            (fresh(1), "CPU 18%  RAM 42%  !1"),
            (fresh(2), "CPU 18%  RAM 42%  !2"),
            (fresh(64), "CPU 18%  RAM 42%  !64"),
            (fresh(0), "CPU 18%  RAM 42%"),
            (stale(0), "CPU 18%  RAM 42%  !~0"),
            (stale(2), "CPU 18%  RAM 42%  !~2"),
            (channel("not_refreshed"), "CPU 18%  RAM 42%  !?"),
            (channel("warming_up"), "CPU 18%  RAM 42%  !?"),
            (channel("timeout"), "CPU 18%  RAM 42%  !?"),
            (channel("daemon_error"), "CPU 18%  RAM 42%  !?"),
            (channel("bogus"), "CPU 18%  RAM 42%  !?"),
            (channel("available"), "CPU 18%  RAM 42%  !?"),
            (channel("available", object()), "CPU 18%  RAM 42%  !?"),
            (channel("available", types.SimpleNamespace(findings=None)), "CPU 18%  RAM 42%  !?"),
        ]
        for advisory, expected in cases:
            with self.subTest(advisory=advisory.state, expected=expected):
                self.assertEqual(render(view("available", status(), advisory)), expected)

    def test_badge_composes_with_placeholder_and_stale_status(self):
        self.assertEqual(render(view("not_refreshed", None, fresh(2))), "CPU --  RAM --  !2")
        self.assertEqual(render(view("not_refreshed", None, channel("warming_up"))), "CPU --  RAM --  !?")
        self.assertEqual(render(view("stale", status(), fresh(2))), "CPU ~18%  RAM ~42%  !2")

    def test_offline_status_has_no_badge(self):
        for state in OFFLINE:
            with self.subTest(state=state):
                self.assertEqual(render(view(state, None, fresh(2))), "syswatch offline")
        self.assertIsNone(render(view("bogus", status(), fresh(2))))

    def test_worst_case_is_the_reserved_text(self):
        text = render(view("stale", status(cpu=100.0, used=100, total=100), stale(64)))
        self.assertEqual(text, MAX_INDICATOR_TEXT)
        self.assertLessEqual(len(text), len(MAX_INDICATOR_TEXT))

    def test_hostile_finding_text_never_reaches_the_badge(self):
        hostile = Finding(42, 7, "\x1b[31mcpu", "high", WALL_MS, "a\x00b\nc\x1b[0m�", Evidence(6, 120_000, None, None))
        advisory = channel("available", Suggestions(WALL_MS, WALL_MS, (hostile,)))
        text = render(view("available", status(), advisory))
        self.assertEqual(text, "CPU 18%  RAM 42%  !1")
        self.assertTrue(text.isprintable())


class WireBadgeTest(unittest.TestCase):
    def cache_with(self, reply, **kwargs):
        cache = TelemetryCache(monotonic_ms=lambda: 0)
        cache._ingest("status", ClientResult("available", value=status(), age_ms=0), 0)
        cache._ingest("suggestions", run(reply, read=read_suggestions, **kwargs), 0)
        return cache

    def test_decoded_reply_sets_the_count(self):
        second = wire_finding(id={"pid": 43, "start_time_secs": 8, "kind": "memory"})
        cache = self.cache_with(suggestions_body(findings=[wire_finding(), second]))
        self.assertEqual(render(cache.view()), "CPU 18%  RAM 42%  !2")

    def test_decoded_empty_reply_has_no_badge(self):
        self.assertEqual(render(self.cache_with(suggestions_body()).view()), "CPU 18%  RAM 42%")

    def test_decoded_stale_reply_keeps_its_count(self):
        cache = self.cache_with(suggestions_body(findings=[wire_finding()]), max_age_ms=500)
        self.assertEqual(render(cache.view()), "CPU 18%  RAM 42%  !~1")


class FitTest(unittest.TestCase):
    def test_all_or_nothing(self):
        text = "CPU 18%  RAM 42%"
        self.assertEqual(fit(text, len(text)), text)
        self.assertEqual(fit(text, len(text) + 5), text)
        self.assertIsNone(fit(text, len(text) - 1))
        self.assertIsNone(fit(text, 0))
        self.assertIsNone(fit(text, -4))
        self.assertIsNone(fit("", 0))

    def test_badge_text_uses_the_same_rule(self):
        text = "CPU 18%  RAM 42%  !2"
        self.assertEqual(fit(text, len(text)), text)
        self.assertIsNone(fit(text, len(text) - 1))


class PurityTest(unittest.TestCase):
    def test_module_has_no_kitty_import_or_io(self):
        path = pathlib.Path(__file__).with_name("syswatch_indicator.py")
        tree = ast.parse(path.read_text())
        names = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names |= {alias.name.split(".")[0] for alias in node.names}
            elif isinstance(node, ast.ImportFrom) and node.module:
                names.add(node.module.split(".")[0])
        self.assertEqual(names, {"math"})
        calls = {
            node.func.id if isinstance(node.func, ast.Name) else node.func.attr
            for node in ast.walk(tree)
            if isinstance(node, ast.Call) and isinstance(node.func, (ast.Name, ast.Attribute))
        }
        self.assertFalse(calls & {"open", "notify", "advance", "request_line", "close", "signal", "kill", "sleep"})


if __name__ == "__main__":
    unittest.main()
