"""Demonstration tab bar: plain ASCII from the cached view. Not an installed indicator.

Symlink this file to `<config>/tab_bar.py`. The symlink lets `resolve()` find the
adapter modules in this directory. Only the last tab is drawn. The text reads
the cache and never does I/O.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from syswatch_cache import get_shared  # noqa: E402
from syswatch_redraw import install_redraw_timer  # noqa: E402

_started = False
_timer_ok = False
_cache = None


def _label(view):
    status = view.status
    if status.value is None:
        return "..." if status.state == "not_refreshed" else status.state
    memory = status.value.memory
    used = 100 * memory.used_bytes / memory.total_bytes if memory.total_bytes else 0
    prefix = "" if status.state == "available" else status.state + " "
    return f"{prefix}r{view.revision} {status.value.cpu_usage_percent:.0f}% {used:.0f}%"


def draw_tab(draw_data, screen, tab, before, max_tab_length, index, is_last, extra_data):
    global _started, _timer_ok, _cache
    if not _started:
        _cache = get_shared()
        _cache.start()
        _timer_ok = install_redraw_timer(_cache.view)
        _started = True
    if not is_last:
        return before
    marker = "T" if _timer_ok else "E"
    screen.draw(f"SW {_label(_cache.view())} {marker}")
    return screen.cursor.x
