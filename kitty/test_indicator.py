import ast
import dataclasses
import pathlib
import unittest

from syswatch_cache import CacheView, ChannelView
from syswatch_client import Memory, Status
from syswatch_indicator import GAP_COLUMNS, fit, render

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


def view(state, value=None):
    channel = ChannelView(state, value, None, None, None, None, None, None)
    empty = ChannelView("not_refreshed", None, None, None, None, None, None, None)
    return CacheView(status=channel, suggestions=empty, revision=0)


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


class FitTest(unittest.TestCase):
    def test_all_or_nothing(self):
        text = "CPU 18%  RAM 42%"
        self.assertEqual(fit(text, len(text)), text)
        self.assertEqual(fit(text, len(text) + 5), text)
        self.assertIsNone(fit(text, len(text) - 1))
        self.assertIsNone(fit(text, 0))
        self.assertIsNone(fit(text, -4))
        self.assertIsNone(fit("", 0))

    def test_gap_columns(self):
        self.assertEqual(GAP_COLUMNS, 1)


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
        self.assertFalse(names & {"kitty", "socket", "subprocess", "time", "threading", "os"})
        calls = {
            node.func.id for node in ast.walk(tree) if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
        }
        self.assertNotIn("open", calls)


if __name__ == "__main__":
    unittest.main()
