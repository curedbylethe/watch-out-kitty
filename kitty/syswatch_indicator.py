"""Tab-bar indicator text for the syswatch Kitty adapter's cached view.

Pure and read-only: it formats the cached `status` channel and the advisory
`suggestions` count into a short ASCII string. It imports nothing from Kitty,
performs no I/O, reads no clock or environment, and never requests a process
action.
"""

import math

GAP_COLUMNS = 1
OFFLINE_STATES = frozenset(
    {
        "daemon_unavailable",
        "timeout",
        "permission_denied",
        "socket_path_invalid",
        "malformed_response",
        "protocol_mismatch",
        "daemon_error",
    }
)
PLACEHOLDER = "CPU --  RAM --"
# Worst case: both stale prefixes and the widest stale badge (the daemon caps the count at 64).
MAX_INDICATOR_TEXT = "CPU ~100%  RAM ~100%  !~64"


def _percent(value, prefix):
    """Return `<prefix>NN%` for a finite percent, clamped to 0..100 and rounded half-up."""
    return f"{prefix}{math.floor(min(max(value, 0.0), 100.0) + 0.5)}%"


def _badge(view) -> str:
    """Return `!N` fresh, `!~N` stale, `!?` pending or unknown, or "" for a fresh empty list."""
    channel = view.suggestions
    if channel.state not in ("available", "stale"):
        return "!?"
    try:
        count = len(channel.value.findings)
    except (AttributeError, TypeError):  # an unexpected payload shape must not raise on Kitty's drawing path
        return "!?"
    if channel.state == "stale":
        return f"!~{count}"
    return f"!{count}" if count else ""


def _with_badge(text, view) -> str:
    badge = _badge(view)
    return f"{text}  {badge}" if badge else text


def render(view) -> str | None:
    """Return the indicator text for `view`, or None when the status state is not recognised."""
    status = view.status
    if status.state in ("not_refreshed", "warming_up"):
        return _with_badge(PLACEHOLDER, view)
    if status.state in OFFLINE_STATES:
        return "syswatch offline"
    if status.state not in ("available", "stale") or status.value is None:
        return None
    value = status.value
    prefix = "~" if status.state == "stale" else ""
    cpu = "--" if value.cpu_updated_at_ms is None else _percent(value.cpu_usage_percent, prefix)
    memory = value.memory
    ram = "--" if memory.total_bytes <= 0 else _percent(memory.used_bytes / memory.total_bytes * 100, prefix)
    return _with_badge(f"CPU {cpu}  RAM {ram}", view)


def fit(text: str, available_width: int) -> str | None:
    """Return `text` when it fits in `available_width` columns, else None; never truncates."""
    return text if 0 < available_width and len(text) <= available_width else None
