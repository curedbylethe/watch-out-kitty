"""Main-loop bridge from cache changes to Kitty's tab bar.

Four non-public Kitty symbols are used, and only here:
`kitty.fast_data_types.add_timer`, `kitty.fast_data_types.wakeup_main_loop`,
`kitty.boss.get_boss().os_window_map`, and `TabManager.mark_tab_bar_dirty()`.
Re-run the probe after a Kitty upgrade. Remove or adapt this module when Kitty
offers a public repaint call. `install_redraw_timer` returning False is the
fallback: the bar then repaints only when Kitty already redraws.

Why the wakeup: on Kitty 0.49.2 `mark_tab_bar_dirty()` only clears a flag, and a
timer callback does not request a main-loop tick. The flag is consumed at the
next render, which an idle window may not have for minutes. `wakeup_main_loop()`
requests that tick. Kitty itself calls it after marking a window dirty.

The timer runs on Kitty's main loop and starts no thread. It repaints only when
the presentation changes: the cache revision, or a channel state such as a
value turning stale while no new reply arrives. Module state lives in
`sys.modules`, so a `tab_bar.py` reload does not install a second timer.
"""

REDRAW_INTERVAL_MS = 1000

_timer_id = None
_last_key = None


def _kitty_tab_managers():
    from kitty.boss import get_boss

    return list(get_boss().os_window_map.values())


def _presentation_key(view):
    return (view.revision, view.status.state, view.suggestions.state)


def _make_tick(get_view, get_tab_managers, wakeup):
    def tick(timer_id):
        global _last_key
        try:
            key = _presentation_key(get_view())
            if key == _last_key:
                return
            _last_key = key
            for manager in get_tab_managers():
                try:
                    manager.mark_tab_bar_dirty()
                except Exception:  # shortcut: a broken window is skipped silently, add logging when diagnosing
                    pass
            wakeup()
        except Exception:  # shortcut: a broken view is skipped silently, add logging when diagnosing
            pass

    return tick


def install_redraw_timer(
    get_view, *, interval_ms=REDRAW_INTERVAL_MS, add_timer=None, get_tab_managers=None, wakeup=None
):
    global _timer_id, _last_key
    if _timer_id is not None:
        return True
    try:
        if add_timer is None:
            from kitty.fast_data_types import add_timer
        if wakeup is None:
            from kitty.fast_data_types import wakeup_main_loop as wakeup
        if get_tab_managers is None:
            get_tab_managers = _kitty_tab_managers
        get_tab_managers()
        _last_key = _presentation_key(get_view())
        _timer_id = add_timer(_make_tick(get_view, get_tab_managers, wakeup), interval_ms / 1000.0, True)
    except Exception:  # ImportError and AttributeError included: missing Kitty surface is the fallback
        return False
    return True
