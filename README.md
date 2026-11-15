# Watch Out Kitty

Watch Out Kitty puts live CPU and memory numbers in the Kitty tab bar on macOS. A foreground daemon samples the system and answers the tab bar and a small command-line client. It also lists processes that hold a lot of memory while idle, and it signals nothing unless you confirm a `close`.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/curedbylethe/watch-out-kitty/main/install.sh | sh
```

It installs `wok` and `wok-daemon` into `~/.local/bin`, adds that directory to your shell profile, and installs no Python package and no Rust toolchain. It handles macOS on Apple silicon and Intel, and Linux on x86_64 and aarch64; any other machine stops with a clear error. The profile line is added once: a re-run replaces the two binaries and adds nothing more.

You need macOS (tested with Kitty 0.49.2) or Linux with glibc, and Kitty. Nothing needs installing for Python: Kitty runs the adapter with its own interpreter. `--dir <directory>` installs somewhere else; the default is `~/.local/bin`. This document uses `wok` for commands and `wok-daemon` for the sampler.

1. Open a new terminal so `wok` is on your PATH. The installer edits `.zshrc` for zsh, and `.bash_profile` on macOS or `.bashrc` on Linux for bash; for any other shell, add the line it prints.
2. Link the adapter as `tab_bar.py` into the directory that holds the `kitty.conf` Kitty loads. The `ln -s` line below needs a repository checkout, as [Build from source](#build-from-source) shows; the checkout serves the adapter only, and nothing is built from it. Kitty uses `$KITTY_CONFIG_DIRECTORY` when set; otherwise the first existing file of `$XDG_CONFIG_HOME/kitty/kitty.conf`, `~/.config/kitty/kitty.conf`, `~/Library/Preferences/kitty/kitty.conf` and `$XDG_CONFIG_DIRS/kitty/kitty.conf`. The command below assumes `~/.config/kitty`; if your loaded `kitty.conf` is elsewhere, use that directory. Kitty's `edit_config_file` action (⌘, on macOS) shows the path.

   ```sh
   mkdir -p "${KITTY_CONFIG_DIRECTORY:-$HOME/.config/kitty}"
   ln -s "$PWD/kitty/tab_bar.py" "${KITTY_CONFIG_DIRECTORY:-$HOME/.config/kitty}/tab_bar.py"
   ```

   Use a symlink, not a copy: the file finds the adapter modules next to its real path. If `ln` reports `File exists`, read [If you already have a tab_bar.py](#if-you-already-have-a-tab_barpy).
3. Set the three options below in `kitty.conf`.
4. Start the daemon in a terminal other than the Kitty window you restart, then restart Kitty. Kitty loads `tab_bar.py` once, so a restart is needed after each change.

Options for `kitty.conf`:

```conf
tab_bar_style custom
tab_bar_edge top
tab_bar_min_tabs 1
```

- `tab_bar_style custom` (default `fade`): required. Kitty then calls `draw_tab` in `tab_bar.py`. Tabs keep Kitty's fade look and your `tab_title_template`. Any other style (`powerline`, `slant`, `separator`, `hidden`) falls back to the fade look.
- `tab_bar_edge top` (default `bottom`): optional. The indicator works on `top` and `bottom`. `top` moves the bar, and the windows start one row lower. On `left` or `right`, tabs draw normally and the indicator is not shown.
- `tab_bar_min_tabs 1` (default `2`): shows the bar, and so the indicator, with a single tab. With the default, both stay hidden until a second tab opens.

### Limits

- If the whole text does not fit, or the tabs overflow the bar, the whole indicator is hidden. It is never truncated.
- `tab_bar_align` must stay `start` (the default); `center` and `end` shift the finished line and can clip the text.
- The indicator is not clickable.

### If you already have a tab_bar.py

Kitty loads one `tab_bar.py` per config directory, and nothing is merged automatically. If you already have one, do not delete or replace it. Choose one of these cases:

- **No `tab_bar.py` yet.** Use the link from step 2. Keep your customization in `tab_title_template`; the adapter keeps it.
- **Your `tab_bar.py` must stay as it is.** Do not install this adapter.
- **Your `tab_bar.py` must keep drawing the tabs.** Do not install this adapter over it; it does not compose with a custom `tab_bar.py`.

## Build from source

The Rust toolchain is needed only to build from source. Clone the repository to build it, or just to get the adapter files install step 2 links; the checkout alone needs no build.

```sh
git clone https://github.com/curedbylethe/watch-out-kitty.git
cd watch-out-kitty
cargo build --workspace
export PATH="$PWD/target/debug:$PATH"
```

The build writes `wok` and `wok-daemon` into `target/debug`. Use Rust stable. The `export` line lasts for that terminal; add it to your shell profile to keep it.

## Use

Start the daemon:

```sh
wok-daemon
```

It runs in the foreground. Ctrl-C or SIGTERM stops it cleanly and prints `wok-daemon: stopped`.

```sh
wok status
wok top --sort memory --limit 10
wok suggestions
wok inspect <reference>
wok close <reference>
wok update dismiss
wok --version
```

- `status` prints the latest CPU, memory, process count and sample ages.
- `top` prints cached processes ranked by memory (the default) or CPU. `--sort memory|cpu` picks the key and `--limit N` sets a positive row limit, 10 by default.
- `suggestions` prints the daemon's cached advisory findings: a reference, confidence, process name and explanation, then the ages of the evaluation and the process data. Before there is enough history it prints `still collecting evidence: …`; with nothing to report it prints `(no current suggestions)`.
- `inspect <reference>` prints the cached evidence for one finding, selected by its full reference `<pid>-<start_time_secs>-<kind>`.
- `close <reference>` previews one finding and, only after you type `yes`, asks the daemon to request one SIGTERM. See [Closing a process](#closing-a-process).
- `update dismiss` silences the current update notice. It prints `dismissed update notice for 1.2.3`, or `no update to dismiss` when the daemon reports no newer release.
- `--version` prints the build version, for example `wok 0.1.0`.

`wok --help`, `wok -h` and `wok help` print the same block: the commands, the global options and the exit codes. Every command answers `-h`/`--help` with its own block, `wok help <COMMAND>` prints one command's block, and `-v` is not a version flag; use `-V` or `--version`.

Exit status is `0` for success, `1` for a runtime failure such as a missing daemon, and `2` for a usage error.

`status` shows `warming up` before the first CPU sample and `not yet sampled` before the first process scan.

The daemon and the tab bar must resolve the same socket path: `$WOK_SOCKET` when set to an absolute path, otherwise `$XDG_RUNTIME_DIR/wok.sock`, otherwise `$HOME/.local/run/wok.sock`.

Every command reads the daemon's cached replies; none of them scans processes on demand.

### What the tab bar shows

The bar shows `CPU 18%  RAM 42%` at the right edge of the tab bar.

- `CPU --  RAM --`: before the first reply, and while the daemon warms up.
- `CPU --`: while the first CPU sample is not ready.
- `RAM --`: the daemon reported no total memory.
- `~` before a value: the data is stale.
- `wok offline`: the daemon cannot be reached or replies with an error.

A dash is not a measurement. No value appears until `wok-daemon` is running; the tab bar reads cached replies and starts nothing.

#### Advisory badge

The tab bar adds a marker after the status text, two spaces apart: `CPU 18%  RAM 42%  !2`. The marker reads only the cached advisory list that `wok suggestions` prints.

| Cached suggestions | Right edge |
| :--- | :--- |
| fresh, one or more findings | `CPU 18%  RAM 42%  !2` |
| fresh, no findings | `CPU 18%  RAM 42%` |
| stale, last known count 2 | `CPU 18%  RAM 42%  !~2` |
| stale, last known count 0 | `CPU 18%  RAM 42%  !~0` |
| pending evidence or unknown advisory state | `CPU 18%  RAM 42%  !?` |
| daemon unavailable | `wok offline` |

`!N` is a current count. `!~N` is a retained count that is stale, so it is not a current count. `!?` means the advisory list has no current answer yet, for example while the daemon warms up. A fresh empty list shows no marker. The count never exceeds 64, the daemon's cap on the served list.

A caret after the marker means the daemon found a newer release: `CPU 18%  RAM 42%  !2 ^`. It is one space after the advisory marker, or two spaces after the values when there is no marker. It needs a status reply, so it never appears beside `wok offline`. See [Upgrade](#upgrade).

The marker is passive. It never opens a popup or notification, sends a signal, dismisses a finding, or runs `wok close`. It shows a count and a state only, never a process name, path, argument, explanation, or finding reference. No command dismisses a finding, so the marker changes only when the served list changes. The marker is not clickable; use the keyboard mappings below.

### Keyboard shortcuts

These optional mappings open the advisory output from the keyboard. No key is bound by default. Add them to `kitty.conf`:

```conf
map kitty_mod+alt+g launch --type=overlay --title "wok suggestions" --hold wok suggestions

map kitty_mod+alt+i launch --type=overlay --title "wok inspect" sh -c 'printf "finding reference: "; read -r ref; wok inspect "$$ref"; printf "\nPress Enter to close"; read -r dummy'
```

`kitty_mod` is `ctrl+shift` by default, so the keys are Ctrl+Shift+Alt+G and Ctrl+Shift+Alt+I. Both are unbound in stock Kitty.

- The first mapping opens `wok suggestions` in an overlay. `--hold` keeps the overlay at a shell prompt. Close it with `ctrl+shift+w`, or type `exit`.
- The second mapping prompts for a reference, runs `wok inspect` on it, and waits for Enter. Copy the reference from the `wok suggestions` output; it has the form `<pid>-<start_time_secs>-<kind>`. `$$ref` is correct: Kitty consumes one `$`, so the shell receives `$ref`.

Neither key touches a process. Neither mapping sends a signal or runs `wok close`.

`launch` does not run a login shell, so `wok` must be on the PATH that Kitty inherits; the installer puts it in `~/.local/bin`. If it is not there, replace `wok` in both lines with the binary's absolute path, for example `/path/to/wok`.

### Closing a process

`wok close <reference>` is the only command that can lead to a signal. It previews the finding first and needs an interactive terminal. Only the exact line `yes` continues; empty input, end of input, `y`, `YES` and padded input abort and send nothing. A refused preview prints `nothing was executed; no signal was sent`.

The daemon revalidates the process at delivery and requests at most one SIGTERM. It never sends SIGKILL, retries, escalates, or acts on several processes. `close` exits `0` only when the daemon accepted a SIGTERM request, `1` when it aborted, denied, failed, or could not reach the daemon, and `2` for a usage error.

A requested signal is not a confirmed exit: the kernel delivers it to whichever process holds the PID at that moment. Run `wok inspect <reference>` to check again.

## Upgrade

Releases are tags of the form `vMAJOR.MINOR.PATCH`, for example `v0.2.0`, and each release carries one archive per target. Run the installer again:

```sh
curl -fsSL https://raw.githubusercontent.com/curedbylethe/watch-out-kitty/main/install.sh | sh
```

Then stop the daemon with Ctrl-C and start it again. Kitty needs no restart unless the adapter files changed.

A newer release appears in three places:

- The daemon prints one line at the terminal: `wok-daemon: version 0.2.0 is available at https://…; set WOK_DISABLE_UPDATE_CHECK=1 to silence`.
- `wok status` adds one line naming the version and the release URL. The line suggests `wok update dismiss`.
- The tab bar ends with one caret: `CPU 18%  RAM 42% ^`, or `CPU 18%  RAM 42%  !2 ^` beside an advisory badge. The caret is passive; it opens nothing and sends no signal.

The daemon asks GitHub for the latest release once at startup and then once a
day, and the request carries nothing about the machine. Set
`WOK_DISABLE_UPDATE_CHECK=1` to stop the check entirely.

Each version prompts at most once. `wok update dismiss` records the noticed version in a one-key file: `$WOK_UPDATE_STATE` when set to an absolute path, otherwise `$XDG_STATE_HOME/wok/update.json`, otherwise `~/.local/state/wok/update.json`. A dismissal survives a daemon restart and hides the notice until a newer release appears; then the notice returns. Remove the file to restore the notice.

## Troubleshooting

| Symptom | Check | Fix |
| :--- | :--- | :--- |
| No indicator | Kitty's `edit_config_file` action shows the `kitty.conf` Kitty loads. `ls -l` on `tab_bar.py` in that directory shows a link to `kitty/tab_bar.py`, and that `kitty.conf` has the three options | Set the options above, restart Kitty, and widen the window if the title is too long. A regular file under that name is another `tab_bar.py`; see [If you already have a tab_bar.py](#if-you-already-have-a-tab_barpy). |
| `wok offline` | `wok status; echo "exit=$?"` prints `exit=1` when no daemon is running | Start `wok-daemon` in a terminal and leave it running. |
| Socket permission error | `ls -ld` on the socket directory and the socket shows `drwx------` and `srw-------` | Remove other users' access. The daemon and the adapter must resolve the same path, so set `WOK_SOCKET` to the same absolute path for both. |
| Stale or frozen numbers | `wok status` prints `Sampled: <age> ago`. A small age with an unchanged indicator means Kitty has not repainted | A large age means the daemon is not sampling; restart it. A small age means the window was idle; the bar catches up on the next redraw. |
| Changes not applied | Kitty loads `tab_bar.py` once, at startup | Restart Kitty after each change to `kitty.conf` or `tab_bar.py`. |

## Removal

Reverse the install in this order:

1. Stop the daemon: press Ctrl-C in its terminal.
2. Remove the two binaries and the profile line: `rm ~/.local/bin/wok ~/.local/bin/wok-daemon`, then delete the `export PATH=` line naming that directory from the profile the installer edited. Use your `--dir` directory instead of `~/.local/bin` if you chose one.
3. Remove the link, not the file it points to, from the directory of the `kitty.conf` Kitty loads (see install step 2): `rm "<that directory>/tab_bar.py"`, with `~/.config/kitty` as the default. Run `ls -l` first; if the entry is a regular file, it is not this adapter, so leave it.
4. Remove the three lines from `kitty.conf`.
5. Restart Kitty. Removing the link while Kitty runs does not unload the bar until the restart.

The daemon keeps its telemetry in memory, and `wok update dismiss` is the only command that writes a file. On a clean stop the daemon removes the socket file and leaves its directory.

## Where the rest lives

- [Module map](docs/architecture.md)
- [Measurement semantics, advisory findings and process actions](docs/measurements.md)
- [Kitty adapter internals and the manual checklist](docs/kitty-adapter.md)
- [Socket protocol](docs/protocol.md)

## License

Apache-2.0. See [LICENSE](LICENSE).

Copyright 2026 Parsa
