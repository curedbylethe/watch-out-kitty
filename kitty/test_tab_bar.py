import ast
import pathlib
import sys
import types
import unittest
from unittest import mock

from syswatch_cache import TelemetryCache
from syswatch_client import ClientResult, Evidence, Finding, Memory, Status, Suggestions


def _as_rgb(color):
    return (color << 8) | 2


def _fade(draw_data, screen, tab, before, max_tab_length, index, is_last, extra_data):
    # Stands in for Kitty's draw_tab_with_fade: title plus a trailing space.
    _fade.calls.append((draw_data, screen, tab, before, max_tab_length, index, is_last, extra_data))
    screen.draw(tab.title)
    end = screen.cursor.x
    screen.draw(" ")
    return end


_fade.calls = []
_fake_tab_bar = types.ModuleType("kitty.tab_bar")
_fake_tab_bar.as_rgb = _as_rgb
_fake_tab_bar.draw_tab_with_fade = _fade
# Only for the import: other test modules rely on `kitty` being absent.
with mock.patch.dict(sys.modules, {"kitty": types.ModuleType("kitty"), "kitty.tab_bar": _fake_tab_bar}):
    import tab_bar

WALL_MS = 1_700_000_000_000
STATUS = Status(
    timestamp_ms=WALL_MS,
    cpu_usage_percent=12.4,
    cpu_updated_at_ms=WALL_MS,
    memory=Memory(total_bytes=8, used_bytes=4),
    process_count=3,
    processes_updated_at_ms=WALL_MS,
)
LIVE = "CPU 12%  RAM 50%"
FG, BG = 0x112233, 0x445566


class FakeScreen:
    def __init__(self, columns):
        self.columns = columns
        self.cursor = types.SimpleNamespace(x=0, fg=7, bg=9, bold=True, italic=True)
        self.drawn = []
        self.erased = []

    def erase_in_line(self, mode, private):
        self.erased.append((self.cursor.x, len(self.drawn)))

    def draw(self, text):
        self.drawn.append((text, self.cursor.x, self.cursor.fg, self.cursor.bg))
        self.cursor.x += len(text)


class TabBarTest(unittest.TestCase):
    def setUp(self):
        tab_bar._cache = None
        _fade.calls.clear()
        self.cache = TelemetryCache(monotonic_ms=lambda: 0)
        self.cache._ingest("status", ClientResult("available", value=STATUS, age_ms=0), 0)
        self.cache._ingest("suggestions", ClientResult("available", value=Suggestions(0, 0, ()), age_ms=0), 0)
        self.refresh = self.patch("install_refresh_timer", mock.Mock(return_value=True))
        self.redraw = self.patch("install_redraw_timer", mock.Mock(return_value=True))
        self.patch("get_shared", lambda **kwargs: self.cache)

    def patch(self, name, replacement):
        patcher = mock.patch.object(tab_bar, name, replacement)
        patcher.start()
        self.addCleanup(patcher.stop)
        return replacement

    def draw_bar(self, columns=40, edge="top", for_layout=False):
        """Draw tabs "one" and "two"; return the screen and each tab's extent."""
        screen = FakeScreen(columns)
        draw_data = types.SimpleNamespace(tab_bar_edge=edge, inactive_fg=FG, default_bg=0x778899, inactive_bg=BG)
        extra = types.SimpleNamespace(for_layout=for_layout)
        ends = []
        for index, title in enumerate(("one", "two"), start=1):
            tab = types.SimpleNamespace(title=title)
            ends.append(tab_bar.draw_tab(draw_data, screen, tab, screen.cursor.x, 20, index, index == 2, extra))
        return screen, ends

    def indicator(self, screen):
        return [entry for entry in screen.drawn if entry[0] not in ("one", "two", " ")]

    def suggest(self, count):
        finding = Finding(42, 7, "memory", "high", 0, "held memory", Evidence(6, 120_000, None, None))
        suggestions = Suggestions(0, 0, (finding,) * count)
        self.cache._ingest("suggestions", ClientResult("available", value=suggestions, age_ms=0), 0)

    def test_every_tab_is_delegated_and_keeps_its_extent(self):
        for for_layout in (False, True):
            with self.subTest(for_layout=for_layout):
                _fade.calls.clear()
                screen, ends = self.draw_bar(for_layout=for_layout)
                self.assertEqual(ends, [3, 7])
                draw_data, extra = _fade.calls[0][0], _fade.calls[0][7]
                self.assertEqual((draw_data.tab_bar_edge, extra.for_layout), ("top", for_layout))
                self.assertEqual(
                    [(c[0], c[1], c[2].title) + c[3:] for c in _fade.calls],
                    [
                        (draw_data, screen, "one", 0, 20, 1, False, extra),
                        (draw_data, screen, "two", 4, 20, 2, True, extra),
                    ],
                )

    def test_last_tab_paints_indicator_at_right_edge(self):
        screen, ends = self.draw_bar(columns=40)
        self.assertEqual(ends[-1], 7)
        self.assertEqual(self.indicator(screen), [(LIVE, 40 - len(LIVE), _as_rgb(FG), _as_rgb(BG))])
        self.assertEqual((screen.cursor.x, screen.cursor.bold, screen.cursor.italic), (40, False, False))

    def test_fresh_findings_paint_badge_at_right_edge(self):
        self.suggest(2)
        screen, ends = self.draw_bar(columns=40)
        text = "CPU 12%  RAM 50%  !2"
        self.assertEqual(self.indicator(screen), [(text, 40 - len(text), _as_rgb(FG), _as_rgb(BG))])
        self.assertEqual(ends[-1], 7)
        self.assertEqual(self.indicator(self.draw_bar(for_layout=True)[0]), [])

    def test_badge_fit_boundary(self):
        self.suggest(2)
        exact = 8 + 1 + len("CPU 12%  RAM 50%  !2")  # tabs end at 8; one gap column before the text
        self.assertEqual(len(self.indicator(self.draw_bar(columns=exact)[0])), 1)
        screen, ends = self.draw_bar(columns=exact - 1)
        self.assertEqual(self.indicator(screen), [])
        self.assertEqual(ends, [3, 7])

    def test_last_tab_budget_leaves_room_for_indicator(self):
        for for_layout, budget in ((False, 60 - 28 - 4), (True, 50)):
            with self.subTest(for_layout=for_layout):
                _fade.calls.clear()
                draw_data = types.SimpleNamespace(tab_bar_edge="top", inactive_fg=FG, default_bg=BG)
                tab = types.SimpleNamespace(title="x" * 50)
                tab_bar.draw_tab(draw_data, FakeScreen(60), tab, 4, 50, 2, True, types.SimpleNamespace(for_layout=for_layout))
                self.assertEqual(_fade.calls[0][4], budget)

    def test_no_clamp_without_room_for_a_readable_title(self):
        _fade.calls.clear()
        draw_data = types.SimpleNamespace(tab_bar_edge="top", inactive_fg=FG, default_bg=BG, inactive_bg=BG)
        tab = types.SimpleNamespace(title="x" * 50)
        tab_bar.draw_tab(draw_data, FakeScreen(30), tab, 12, 50, 2, True, types.SimpleNamespace(for_layout=False))
        self.assertEqual(_fade.calls[0][4], 50)

    def test_gap_before_indicator_is_erased(self):
        screen, ends = self.draw_bar(columns=40)
        self.assertEqual(screen.erased, [(ends[-1], 4)])

    def test_fit_boundary(self):
        # Tabs leave the cursor at 8; one gap column precedes the text.
        exact = 8 + 1 + len(LIVE)
        self.assertEqual(len(self.indicator(self.draw_bar(columns=exact)[0])), 1)
        self.assertEqual(self.indicator(self.draw_bar(columns=exact - 1)[0]), [])

    def test_indicator_omitted_for_layout_and_vertical_edges(self):
        self.assertEqual(self.indicator(self.draw_bar(for_layout=True)[0]), [])
        for edge in ("left", "right"):
            self.assertEqual(self.indicator(self.draw_bar(edge=edge)[0]), [])

    def test_non_last_tab_draws_no_indicator(self):
        self.draw_bar()
        self.assertEqual(len(_fade.calls), 2)
        screen = FakeScreen(40)
        draw_data = types.SimpleNamespace(tab_bar_edge="top", inactive_fg=FG, default_bg=BG)
        tab = types.SimpleNamespace(title="one")
        tab_bar.draw_tab(draw_data, screen, tab, 0, 20, 1, False, types.SimpleNamespace(for_layout=False))
        self.assertEqual(self.indicator(screen), [])

    def test_not_refreshed_placeholder_with_failed_installers(self):
        self.cache = TelemetryCache(monotonic_ms=lambda: 0)
        self.refresh.return_value = self.redraw.return_value = False
        screen, ends = self.draw_bar()
        self.assertEqual([entry[0] for entry in self.indicator(screen)], ["CPU --  RAM --  !?"])
        self.assertEqual(ends, [3, 7])

    def test_render_error_keeps_tabs_and_extent(self):
        self.patch("render", mock.Mock(side_effect=RuntimeError("bad view")))
        screen, ends = self.draw_bar()
        self.assertEqual(ends, [3, 7])
        self.assertEqual([entry[0] for entry in screen.drawn], ["one", " ", "two", " "])

    def test_drawing_never_opens_a_socket(self):
        with mock.patch("socket.socket", side_effect=AssertionError("socket opened")) as sock:
            screen, ends = self.draw_bar()
        self.assertFalse(sock.called)
        self.assertEqual(len(self.indicator(screen)), 1)

    def test_startup_runs_once_and_again_after_reload(self):
        self.draw_bar()
        self.draw_bar()
        self.assertEqual((self.refresh.call_count, self.redraw.call_count), (1, 1))
        self.refresh.assert_called_with(self.cache)
        self.redraw.assert_called_with(self.cache.view)
        tab_bar._cache = None  # a config reload re-runs the module
        self.draw_bar()
        self.assertEqual((self.refresh.call_count, self.redraw.call_count), (2, 2))

    def test_source_does_no_io(self):
        source = pathlib.Path(__file__).with_name("tab_bar.py").read_text()
        for forbidden in ("sleep(", "open(", "request_line", "kitten", ".advance(", ".close(", "stop_refresh_timer"):
            self.assertNotIn(forbidden, source)
        imported = set()
        for node in ast.walk(ast.parse(source)):
            if isinstance(node, ast.Import):
                imported |= {alias.name.split(".")[0] for alias in node.names}
            elif isinstance(node, ast.ImportFrom) and node.module:
                imported.add(node.module.split(".")[0])
        self.assertFalse(imported & {"socket", "subprocess", "threading", "time", "os"})


if __name__ == "__main__":
    unittest.main()
