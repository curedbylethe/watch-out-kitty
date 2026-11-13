"""Read-only client for the syswatch daemon's cached status and suggestions.

Requires Python 3.11 or newer. Standard library only. Read-only: it sends
`status` and `suggestions` requests over the local Unix socket. It never opens
TCP, never runs shell commands, and never requests a process action.

The socket path resolution mirrors `crates/syswatch-core/src/ipc.rs`.
"""

import os


class SocketPathError(Exception):
    """Raised when no usable socket path can be resolved."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason  # "relative_path" or "not_configured"


def resolve_socket_path(environ=None):
    """Return the daemon socket path, using the same precedence as the Rust core."""
    env = os.environ if environ is None else environ

    explicit = env.get("SYSWATCH_SOCKET")
    if explicit:
        if os.path.isabs(explicit):
            return explicit
        raise SocketPathError("relative_path")

    runtime_dir = env.get("XDG_RUNTIME_DIR")
    if runtime_dir and os.path.isabs(runtime_dir):
        return os.path.join(runtime_dir, "syswatch.sock")

    home = env.get("HOME")
    if home and os.path.isabs(home):
        return os.path.join(home, ".local", "run", "syswatch.sock")

    raise SocketPathError("not_configured")
