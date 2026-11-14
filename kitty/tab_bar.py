"""Kitty tab bar with the syswatch CPU/RAM indicator at the right edge.

Install: symlink this file to `<kitty config dir>/tab_bar.py` and set
`tab_bar_style custom`. Use a symlink, not a copy: `resolve()` follows it to
this directory, where the `syswatch_*` adapter modules live.

Every tab is drawn by Kitty's own default `fade` style. After the last tab, the
indicator text is painted at the right edge, or nothing when it does not fit.
The drawing path only reads the in-memory cached view: no I/O, no threads, no
blocking. Refresh and redraw run on Kitty's main-loop timers.
"""

import sys
from pathlib import Path

from kitty.tab_bar import as_rgb, draw_tab_with_fade

sys.path.insert(0, str(Path(__file__).resolve().parent))

from syswatch_cache import get_shared  # noqa: E402
from syswatch_indicator import GAP_COLUMNS, fit, render  # noqa: E402
from syswatch_redraw import install_redraw_timer  # noqa: E402
from syswatch_refresh import install_refresh_timer  # noqa: E402

_cache = None
# Widest indicator text, the gap, and Kitty's trailing space after the last tab.
RESERVE = len("CPU 100%  RAM 100%") + GAP_COLUMNS + 1
# Narrower than this, a clamped title would be unreadable, so Kitty draws it whole.
MIN_TITLE = 9


def draw_tab(draw_data, screen, tab, before, max_tab_length, index, is_last, extra_data):
    global _cache
    if _cache is None:
        _cache = get_shared()
        install_refresh_timer(_cache)
        install_redraw_timer(_cache.view)
    shown = is_last and not extra_data.for_layout and draw_data.tab_bar_edge in ("top", "bottom")
    budget = screen.columns - RESERVE - before
    if shown and budget >= MIN_TITLE:
        # Kitty truncates a long title to this budget; the text keeps its columns.
        max_tab_length = min(max_tab_length, budget)
    # The extent stays Kitty's, so click targets and layout widths are unchanged.
    end = draw_tab_with_fade(draw_data, screen, tab, before, max_tab_length, index, is_last, extra_data)
    if not shown:
        return end
    try:
        text = render(_cache.view())
        if text is not None:
            text = fit(text, screen.columns - max(end, screen.cursor.x) - GAP_COLUMNS)
        if text is not None:
            # A long title can draw past its reported end; blank that overflow first.
            screen.cursor.x = end
            screen.cursor.bg = as_rgb(int(draw_data.default_bg))
            screen.erase_in_line(0, False)
            screen.cursor.bold = screen.cursor.italic = False
            screen.cursor.fg = as_rgb(int(draw_data.inactive_fg))
            screen.cursor.bg = as_rgb(int(draw_data.inactive_bg))
            screen.cursor.x = screen.columns - len(text)
            # The cursor stays after the text: Kitty erases the rest of the line from the cursor.
            screen.draw(text)
    except Exception:  # an indicator error must never cost the tabs
        return end
    return end
