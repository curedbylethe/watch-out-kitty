import os
import unittest
from unittest.mock import patch

from syswatch_client import SocketPathError, resolve_socket_path


def resolve(**env):
    with patch.dict(os.environ, env, clear=True):
        return resolve_socket_path()


class SocketPathTests(unittest.TestCase):
    def test_absolute_explicit_wins_over_fallbacks(self):
        self.assertEqual(
            resolve(SYSWATCH_SOCKET="/a/s.sock", XDG_RUNTIME_DIR="/x", HOME="/h"),
            "/a/s.sock",
        )

    def test_relative_explicit_raises_even_with_valid_xdg(self):
        with self.assertRaises(SocketPathError) as ctx:
            resolve(SYSWATCH_SOCKET="rel.sock", XDG_RUNTIME_DIR="/x")
        self.assertEqual(ctx.exception.reason, "relative_path")

    def test_empty_explicit_falls_through_to_xdg(self):
        self.assertEqual(
            resolve(SYSWATCH_SOCKET="", XDG_RUNTIME_DIR="/x"), "/x/syswatch.sock"
        )

    def test_absolute_xdg_yields_socket_in_runtime_dir(self):
        self.assertEqual(resolve(XDG_RUNTIME_DIR="/run/user/501"), "/run/user/501/syswatch.sock")

    def test_relative_xdg_is_ignored_and_home_is_used(self):
        self.assertEqual(
            resolve(XDG_RUNTIME_DIR="run", HOME="/home/u"),
            "/home/u/.local/run/syswatch.sock",
        )

    def test_absolute_home_yields_local_run_socket(self):
        self.assertEqual(resolve(HOME="/home/u"), "/home/u/.local/run/syswatch.sock")

    def test_relative_home_alone_is_not_configured(self):
        with self.assertRaises(SocketPathError) as ctx:
            resolve(HOME="home/u")
        self.assertEqual(ctx.exception.reason, "not_configured")

    def test_all_unset_is_not_configured(self):
        with self.assertRaises(SocketPathError) as ctx:
            resolve()
        self.assertEqual(ctx.exception.reason, "not_configured")

    def test_trailing_slash_on_xdg_yields_one_separator(self):
        self.assertEqual(resolve(XDG_RUNTIME_DIR="/run/user/501/"), "/run/user/501/syswatch.sock")


if __name__ == "__main__":
    unittest.main()
