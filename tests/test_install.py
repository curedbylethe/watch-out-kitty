import io
import pathlib
import stat
import subprocess
import tarfile
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "install.sh"
BASE_URL = "https://github.com/curedbylethe/watch-out-kitty/releases/latest/download"

UNAME_STUB = """#!/bin/sh
case $1 in
-s) printf '%s\\n' "$WOK_STUB_UNAME_S" ;;
-m) printf '%s\\n' "$WOK_STUB_UNAME_M" ;;
*) exit 1 ;;
esac
"""

CURL_STUB = """#!/bin/sh
out=
prev=
for arg in "$@"; do
	if [ "$prev" = "-o" ]; then out=$arg; fi
	case $arg in
	https://*) printf '%s\\n' "$arg" >>"$WOK_STUB_URL_LOG" ;;
	esac
	prev=$arg
done
cp "$WOK_STUB_ARCHIVE" "$out"
"""


class Fixture:
    """A temporary HOME with stubbed uname and curl on PATH."""

    def __init__(self, root, os_name, machine, target, shell="/bin/zsh"):
        self.root = pathlib.Path(root)
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.url_log = self.root / "url.txt"
        self.archive = self.root / f"wok-{target}.tar.gz"
        self.target = target
        self.env = {
            "HOME": str(self.home),
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "SHELL": shell,
            "WOK_STUB_UNAME_S": os_name,
            "WOK_STUB_UNAME_M": machine,
            "WOK_STUB_ARCHIVE": str(self.archive),
            "WOK_STUB_URL_LOG": str(self.url_log),
        }
        self._stub("uname", UNAME_STUB)
        self._stub("curl", CURL_STUB)
        self.write_archive("one")

    def _stub(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def write_archive(self, marker):
        """Build the release archive; the marker distinguishes two installs."""
        with tarfile.open(self.archive, "w:gz") as tar:
            for name in ("wok", "wok-daemon"):
                data = f"#!/bin/sh\nprintf '%s\\n' {name}-{marker}\n".encode()
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755
                tar.addfile(info, io.BytesIO(data))

    def run(self, *args):
        return subprocess.run(
            ["sh", str(SCRIPT), *args],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def urls(self):
        if not self.url_log.exists():
            return []
        return self.url_log.read_text().splitlines()


class InstallTests(unittest.TestCase):
    def fixture(self, os_name, machine, target, shell="/bin/zsh"):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Fixture(tmp.name, os_name, machine, target, shell)

    def assert_installed(self, directory, marker):
        for name in ("wok", "wok-daemon"):
            path = directory / name
            self.assertTrue(path.is_file(), f"{path} missing")
            self.assertIn(f"{name}-{marker}", path.read_text())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o755)

    def assert_output(self, result, directory, profile):
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 5, result.stdout)
        self.assertEqual(lines[0], f"Installed {directory}/wok")
        self.assertEqual(lines[1], f"Installed {directory}/wok-daemon")
        self.assertIn(str(directory), lines[2])
        self.assertIn(str(profile), lines[2])
        self.assertTrue(lines[3].startswith("Next:"), lines[3])
        self.assertIn("kitty/", lines[3])
        self.assertTrue(
            lines[4].startswith("Only the two binaries were installed"), lines[4]
        )

    def test_darwin_arm64_installs_the_apple_silicon_archive(self):
        fx = self.fixture("Darwin", "arm64", "aarch64-apple-darwin")
        result = fx.run()
        directory = fx.home / ".local" / "bin"
        zshrc = fx.home / ".zshrc"
        self.assert_output(result, directory, zshrc)
        self.assertEqual(fx.urls(), [f"{BASE_URL}/wok-aarch64-apple-darwin.tar.gz"])
        self.assert_installed(directory, "one")
        self.assertEqual(zshrc.read_text(), f'export PATH="{directory}:$PATH"\n')

    def test_darwin_x86_64_installs_the_intel_archive(self):
        fx = self.fixture("Darwin", "x86_64", "x86_64-apple-darwin")
        result = fx.run()
        directory = fx.home / ".local" / "bin"
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(fx.urls(), [f"{BASE_URL}/wok-x86_64-apple-darwin.tar.gz"])
        self.assert_installed(directory, "one")

    def test_linux_aarch64_installs_the_gnu_archive_and_edits_bashrc(self):
        fx = self.fixture(
            "Linux", "aarch64", "aarch64-unknown-linux-gnu", shell="/bin/bash"
        )
        result = fx.run()
        directory = fx.home / ".local" / "bin"
        bashrc = fx.home / ".bashrc"
        self.assert_output(result, directory, bashrc)
        self.assertEqual(fx.urls(), [f"{BASE_URL}/wok-aarch64-unknown-linux-gnu.tar.gz"])
        self.assert_installed(directory, "one")
        self.assertEqual(bashrc.read_text(), f'export PATH="{directory}:$PATH"\n')
        self.assertFalse((fx.home / ".bash_profile").exists())

    def test_linux_x86_64_keeps_the_literal_path_variable(self):
        fx = self.fixture("Linux", "x86_64", "x86_64-unknown-linux-gnu")
        result = fx.run()
        directory = fx.home / ".local" / "bin"
        zshrc = fx.home / ".zshrc"
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(fx.urls(), [f"{BASE_URL}/wok-x86_64-unknown-linux-gnu.tar.gz"])
        self.assert_installed(directory, "one")
        self.assertEqual(zshrc.read_text(), f'export PATH="{directory}:$PATH"\n')

    def test_unsupported_platform_exits_before_downloading(self):
        fx = self.fixture("FreeBSD", "amd64", "x86_64-unknown-linux-gnu")
        result = fx.run()
        self.assertEqual(result.returncode, 1)
        self.assertIn("FreeBSD", result.stderr)
        self.assertIn("amd64", result.stderr)
        for triple in (
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
        ):
            self.assertIn(triple, result.stderr)
        self.assertFalse((fx.home / ".local" / "bin").exists())
        self.assertFalse((fx.home / ".zshrc").exists())
        self.assertEqual(fx.urls(), [])

    def test_custom_dir_with_a_space_is_quoted_everywhere(self):
        fx = self.fixture("Darwin", "arm64", "aarch64-apple-darwin")
        directory = fx.root / "custom bin"
        result = fx.run("--dir", str(directory))
        zshrc = fx.home / ".zshrc"
        self.assert_output(result, directory, zshrc)
        self.assert_installed(directory, "one")
        self.assertEqual(zshrc.read_text(), f'export PATH="{directory}:$PATH"\n')

    def test_second_run_replaces_binaries_and_keeps_one_path_line(self):
        fx = self.fixture("Darwin", "arm64", "aarch64-apple-darwin")
        directory = fx.home / ".local" / "bin"
        zshrc = fx.home / ".zshrc"

        first = fx.run()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assert_installed(directory, "one")

        fx.write_archive("two")
        second = fx.run()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assert_installed(directory, "two")

        line = f'export PATH="{directory}:$PATH"'
        self.assertEqual(zshrc.read_text().splitlines(), [line])
        self.assertEqual(len(fx.urls()), 2)

    def test_install_script_names_no_package_manager_or_privilege_escalation(self):
        text = SCRIPT.read_text().lower()
        for word in ("python", "pip", "brew", "apt", "sudo"):
            self.assertNotIn(word, text)


if __name__ == "__main__":
    unittest.main()
