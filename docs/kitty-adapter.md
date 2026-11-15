# Kitty adapter internals

Part of [Watch Out Kitty](../README.md).

### Python client

`kitty/syswatch_client.py` reads the daemon's cached status and suggestions for Kitty integration code. It needs Python 3.11 or newer and the standard library only. Add the `kitty/` directory to `sys.path`, then `import syswatch_client`.

The client sends one `status` or `suggestions` request per call over the socket described in [Daemon socket](protocol.md). It never sends `processes` or a close request, opens no TCP connection, runs no shell command, polls no OS metric, and writes nothing to disk.

- `read_status(socket_path=None, *, timeout_ms=1000, now_ms=None, max_age_ms=None)` returns a `ClientResult`.
- `read_suggestions(...)` takes the same arguments and returns a `ClientResult`.

`ClientResult.state` is one of `available`, `warming_up`, `stale`, `daemon_unavailable`, `timeout`, `permission_denied`, `socket_path_invalid`, `protocol_mismatch`, `malformed_response`, or `daemon_error`. Expected failures return a state and never raise. Only `available` and `stale` carry `value`; a `stale` result keeps its value so a caller can show how old it is.

The client sets no freshness threshold. With `max_age_ms=None`, `age_ms` reports the age and the state is never `stale`. With an integer, the state is `stale` when the primary timestamp is older than that many milliseconds: `aggregate.timestamp_ms` for status, `evaluated_at_ms` for suggestions.

Run its suite from the repository root with the command in [Development](../AGENTS.md#build-and-validation).

### Kitty cache and refresh

`kitty/syswatch_cache.py` keeps the latest `status` and `suggestions` results in memory for Kitty drawing code. `TelemetryCache.view()` reads one immutable snapshot and never does I/O, so a tab-bar `draw_tab` can call it directly.

- Refresh intervals: `status` every 1000 ms and `suggestions` every 5000 ms by default. A shorter interval raises `ValueError`. The daemon samples at 1 s and 5 s, so actual refresh periods are about one second for `status` and about five seconds for `suggestions`, plus scan time.
- Staleness: a channel is `stale` three intervals after its data was produced (3000 ms for `status`, 15000 ms for `suggestions`). The check runs at read time, so a stopped refresh cannot present old data as live.
- Failures: a failed fetch clears `value`. The last good value stays in `last_value` with `last_age_ms`.
- States: `not_refreshed` before the first fetch, then each `ClientResult.state`. `warming_up` is normal for `suggestions` for about the first two minutes after the daemon starts.
- Driver: `install_refresh_timer(cache)` runs one chain of one-shot Kitty main-loop timers. It starts no threads. Each step starts at most one nonblocking socket request and advances it without waiting. A request has a 250 ms deadline. A failed request backs off from 1 s to 10 s. `get_shared()` returns the process-wide cache, and `stop_refresh_timer()` stops the chain.

`kitty/syswatch_redraw.py` repaints the tab bar when the drawn state changes: a new cache revision, or a channel turning `stale` while no new reply arrives. It uses four Kitty symbols that are not public API: `kitty.fast_data_types.add_timer`, `kitty.fast_data_types.wakeup_main_loop`, `kitty.boss.get_boss().os_window_map`, and `TabManager.mark_tab_bar_dirty()`. Kitty 0.49.2 provides them. On that version `mark_tab_bar_dirty()` only marks the bar; it does not wake Kitty's render loop, so an idle window would not repaint. The shim calls `wakeup_main_loop()` once per change to request that repaint. `install_redraw_timer()` checks the symbols and returns `False` if any is missing or fails. Then the bar updates only when Kitty redraws it for its own reasons, such as a tab change, focus change, or title change, and the data can be older than the polling interval. After a Kitty upgrade, check that the indicator still updates on an idle window. [Issue #80](https://github.com/curedbylethe/watch-out-kitty/issues/80) tracks a repeatable upgrade check, and [issue #78](https://github.com/curedbylethe/watch-out-kitty/issues/78) tracks dropping this workaround when Kitty offers a public repaint call.

Kitty does not render a hidden, minimized, or fully covered window. A change made while the window is not visible is drawn when the window becomes visible again.

### Manual Kitty integration checklist

The suite cannot reach a real Kitty window. Run these checks in order, in a macOS Kitty session with the [Install](../README.md#install) steps done. Each item states what you should see.

1. Fresh install, no existing `tab_bar.py`, `tab_bar_min_tabs 1`, one tab: after a restart the bar shows `CPU n%  RAM n%` at the right edge.
2. Set `tab_bar_min_tabs` back to the default `2` (or remove the line) and restart: with one tab the bar and indicator are hidden; open a second tab and both appear.
3. Ten or more tabs: every title is readable and none is overwritten. The indicator is at the right edge or hidden, as the limits in [Install](../README.md#limits) describe.
4. Narrow the window until the indicator disappears: the tabs still draw. Widen it: the indicator returns.
5. `tab_bar_edge top`, then `bottom`: the indicator is present on both. `left` or `right`: the tabs draw and no indicator appears.
6. Kitty follows the macOS appearance only when its config directory holds `dark-theme.auto.conf` or `light-theme.auto.conf` (see Kitty's themes kitten). If neither file exists, switch Kitty's theme yourself: run `kitty +kitten themes`, choose a dark theme, restart Kitty, then choose a light theme and restart again. If the files exist, switch the macOS appearance and restart Kitty in each mode. Either way the text stays readable in both, with no fixed color.
7. No daemon running: the indicator shows `syswatch offline`.
8. Start `syswatch-daemon`: the indicator shows `CPU --  RAM --`, then numbers. `cargo run -q -p syswatch-cli -- status` shows the same values.
9. Leave one window idle for several minutes: the numbers keep changing. If they change only when you interact, record that as the repaint fallback. [Issue #80](https://github.com/curedbylethe/watch-out-kitty/issues/80) tracks this check; run it here once and do not file a duplicate.
10. Stop the daemon while Kitty runs: the indicator shows `syswatch offline` without a restart. Start the daemon again: the numbers return.
11. With a current finding, the marker shows `!N`. After the finding clears, the marker disappears. With the daemon down, no marker appears beside `syswatch offline`. A finding needs a process that holds at least 256 MiB, uses at most 1% CPU, and is not started from a terminal, so `yes > /dev/null` never produces one. Start a process from launchd that holds memory and sleeps, and wait about 2–3 minutes:

    ```sh
    cat > "${TMPDIR:-/tmp}/syswatch-hold.py" <<'EOF'
    import ctypes, mmap, time
    size = 300 * 1024 * 1024
    m = mmap.mmap(-1, size)
    addr = ctypes.addressof(ctypes.c_char.from_buffer(m))
    if ctypes.CDLL(None).mlock(ctypes.c_void_p(addr), ctypes.c_size_t(size)) != 0:
        raise SystemExit("mlock failed")
    for i in range(0, size, 4096):
        m[i] = 1
    time.sleep(3600)
    EOF
    launchctl submit -l syswatch.test -- python3 "${TMPDIR:-/tmp}/syswatch-hold.py"
    cargo run -q -p syswatch-cli -- suggestions   # expect a memory finding for Python
    ```

    Check that the marker reads `!1`. When you finish, run `launchctl remove syswatch.test`. If `launchctl list syswatch.test` shows a non-zero `LastExitStatus`, the script stopped and printed nothing: run the `python3` line above in a terminal to see the error, then stop it with Ctrl-C. If `suggestions` still reports no finding after 5 minutes, record the item as failed with that output.
12. Start the daemon when a published release is newer than your build: the terminal prints one line, `syswatch-daemon: version 0.2.0 is available at https://…; set SYSWATCH_DISABLE_UPDATE_CHECK=1 to silence`, and the tab bar ends with one caret, for example `CPU 18%  RAM 42%  !2 ^`. No release exists yet, so record this item as pending the first tag.
13. `cargo run -q -p syswatch-cli -- status` shows the same release on one line. Run `cargo run -q -p syswatch-cli -- update dismiss`: it prints `dismissed update notice for 0.2.0`. The next `status` shows no notice, and the caret leaves the tab bar within one sampling interval. Restart the daemon: both stay gone.
14. Stop the daemon and start it with `SYSWATCH_DISABLE_UPDATE_CHECK=1`: the terminal prints nothing about a release, the tab bar never gains a caret, and `status` never carries the notice line.
15. Disconnect the network and restart the daemon with the check enabled: no error line appears, and nothing retries before the next daily check.
16. Remove the integration by the [Removal](../README.md#removal) steps. After the Kitty restart the bar has its previous appearance.

Record each item as pass, fail, or skipped with the reason, with the Kitty and macOS versions, in the pull request. A failed item is a documentation change or a code bug; do not change the checklist to match a failure.

### Tab bar internals

`kitty/tab_bar.py` draws the tabs with Kitty's own code and shows `CPU 18%  RAM 42%` at the right edge of the tab bar. The text comes from the cached view; the drawing path does no I/O.

- Values: whole percent, rounded half-up and clamped to 0-100. RAM is occupied physical RAM as a share of total RAM.
- The tab bar adds a marker after the status text, two spaces apart: `CPU 18%  RAM 42%  !2`. The marker reads only the cached advisory list that `syswatch suggestions` prints.
- A caret after the marker means the daemon found a newer release: `CPU 18%  RAM 42%  !2 ^`. It is one space after the advisory marker, or two spaces after the values when there is no marker. It needs a status reply, so it never appears beside `syswatch offline`. See [Upgrade](../README.md#upgrade).
- No value appears until `syswatch-daemon` is running; the tab bar reads its cached replies and starts nothing. Polling starts with the first bar draw, so with the default `tab_bar_min_tabs 2` it begins when a second tab opens. Until the first reply, the bar shows `CPU --  RAM --`. The advisory marker shows `!?` while the advisory list has no answer yet, which lasts about two minutes after a daemon start; see [Kitty cache and refresh](#kitty-cache-and-refresh).
- If the whole text does not fit, the whole indicator is hidden; the marker is never cut off on its own. The last tab's title is shortened with Kitty's `…` to leave room for the widest text, `CPU ~100%  RAM ~100%  !~64`, but only when at least nine columns remain for the title. That room stays reserved while no marker is shown. If the title cannot be shortened, the indicator is hidden rather than drawn over the title.
- The last tab's title is shortened with Kitty's `…` to leave room for the widest text, including an advisory marker and an update caret, plus one free column, and only when at least nine columns remain for the title. That widest text is `CPU ~100%  RAM ~100%  !~64 ^`, 28 characters, and the bar reserves 30 columns for it. The indicator is hidden, never truncated, only when there is still no room, for example with many tabs.
- It uses the theme's inactive tab foreground on the inactive tab background, with no icon font.
