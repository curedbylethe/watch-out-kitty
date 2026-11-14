"""Tab-bar indicator text for the syswatch Kitty adapter's cached view.

Pure and read-only: it formats the cached `status` channel into a short ASCII
string. It imports nothing from Kitty, performs no I/O, reads no clock or
environment, and never requests a process action.
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


def _percent(value, prefix):
    """Return `<prefix>NN%` for a finite percent, clamped to 0..100 and rounded half-up."""
    return f"{prefix}{math.floor(min(max(value, 0.0), 100.0) + 0.5)}%"


def render(view) -> str | None:
    """Return the indicator text for `view.status`, or None when the state is not recognised."""
    status = view.status
    if status.state in ("not_refreshed", "warming_up"):
        return PLACEHOLDER
    if status.state in OFFLINE_STATES:
        return "syswatch offline"
    if status.state not in ("available", "stale") or status.value is None:
        return None
    value = status.value
    prefix = "~" if status.state == "stale" else ""
    cpu = "--" if value.cpu_updated_at_ms is None else _percent(value.cpu_usage_percent, prefix)
    memory = value.memory
    ram = "--" if memory.total_bytes <= 0 else _percent(memory.used_bytes / memory.total_bytes * 100, prefix)
    return f"CPU {cpu}  RAM {ram}"


def fit(text: str, available_width: int) -> str | None:
    """Return `text` when it fits in `available_width` columns, else None; never truncates."""
    return text if 0 < available_width and len(text) <= available_width else None
