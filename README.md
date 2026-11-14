# Watch Out Kitty

Watch Out Kitty is a macOS-first telemetry library and foreground daemon for
a CPU and RAM indicator in Kitty. Run the example to print one snapshot, or
run the daemon to keep sampling system usage.

## Run it

Install the Rust stable toolchain, then clone and run the example:

```sh
git clone https://github.com/curedbylethe/watch-out-kitty.git
cd watch-out-kitty
cargo run -p syswatch-core --example snapshot
cargo run -p syswatch-daemon
cargo run -p syswatch-cli -- status
cargo run -p syswatch-cli -- top
cargo run -p syswatch-cli -- suggestions
cargo run -p syswatch-cli -- inspect 42-1700000000-memory
cargo run -p syswatch-cli -- close 42-1700000000-memory   # needs an interactive terminal
```

The example prints one snapshot of aggregate CPU, physical memory, and the
processes using the most memory, then exits. The daemon runs in the foreground.
While it runs, `syswatch status` prints the latest CPU, memory, process count,
and sample ages. It exits with status 1 when the daemon is unavailable or its
connection times out, protocol version differs, or response is invalid. It
exits with status 2 for invalid command-line arguments. `syswatch top` prints
cached process data ranked by memory (default) or CPU. Use `--sort memory|cpu`
to choose order and `--limit N` to set a positive row limit (default 10).
`status` and `top` use exit code 1 for runtime errors and 2 for usage errors.
`close` exits 0 only when the daemon accepted a SIGTERM request; see
[Requesting a graceful stop](#requesting-a-graceful-stop).
Ctrl-C (SIGINT) or SIGTERM stops it cleanly, prints `syswatch-daemon: stopped`,
and exits with status 0. CI currently runs on Ubuntu 24.04.

`syswatch suggestions` prints the daemon's cached advisory findings. Each finding
has a reference (`<pid>-<start_time_secs>-<kind>`), a confidence, the process
name, and an explanation, followed by the ages of the evaluation and the process
data. Names are shortened to 24 characters and explanations to 160. The command
reads cached replies only and never scans processes. Before the daemon has enough
history it prints `still collecting evidence: …`; when no finding qualifies it
prints `(no current suggestions)`. Both exit with status 0. Daemon and IPC
failures exit with status 1, and invalid arguments exit with status 2. The command
requests no process action; a finding invites inspection or an explicitly
confirmed `close`.

`syswatch inspect <reference>` prints the cached evidence for one finding, selected by its full
reference `<pid>-<start_time_secs>-<kind>`. It exits with status 1 when the daemon is unavailable or
the finding is no longer cached, and with status 2 for a malformed reference. It reads cached replies
only and never scans processes.

`syswatch close <reference>` previews one finding and, only after you type `yes`, asks the
daemon to request one SIGTERM. See [Requesting a graceful stop](#requesting-a-graceful-stop).

### Daemon socket

The daemon socket path is `$SYSWATCH_SOCKET` when set to an absolute path,
otherwise `$XDG_RUNTIME_DIR/syswatch.sock`, otherwise
`$HOME/.local/run/syswatch.sock`. The daemon creates a missing final directory
with mode `0700` and refuses directories accessible by other users. The socket
has mode `0600`. The daemon does not listen on TCP. A client sends one JSON
request line per connection; the server closes the connection after its reply.

## Current scope

This repository contains the telemetry crate, snapshot example, foreground
daemon, and `syswatch status`, `syswatch top`, `syswatch suggestions`, `syswatch inspect`,
and `syswatch close`
commands. `syswatch close` is the only command that can request a process action. The daemon evaluates advisory findings on process-sampling ticks,
caches them separately from aggregate telemetry, and serves them through the
read-only `suggestions` request. A Kitty tab bar shows CPU and RAM; it does not
show advisory findings, which are planned in
[issue #69](https://github.com/curedbylethe/watch-out-kitty/issues/69). A background
service is planned. See the
[issue tracker](https://github.com/curedbylethe/watch-out-kitty/issues) for
planned work and [issue #13](https://github.com/curedbylethe/watch-out-kitty/issues/13)
for contributor guidance.

## Development

The workspace uses Rust edition 2024. From the repository root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
python3 -m unittest discover -s kitty -t kitty -p 'test_*.py'
```

`cargo test --workspace` also runs the end-to-end daemon and socket tests; they spawn isolated daemon processes in temporary directories and require no running daemon.

### Python client

`kitty/syswatch_client.py` reads the daemon's cached status and suggestions for Kitty integration code. It needs Python 3.11 or newer and the standard library only. Add the `kitty/` directory to `sys.path`, then `import syswatch_client`.

The client sends one `status` or `suggestions` request per call over the socket described in [Daemon socket](#daemon-socket). It never sends `processes` or a close request, opens no TCP connection, runs no shell command, polls no OS metric, and writes nothing to disk.

- `read_status(socket_path=None, *, timeout_ms=1000, now_ms=None, max_age_ms=None)` returns a `ClientResult`.
- `read_suggestions(...)` takes the same arguments and returns a `ClientResult`.

`ClientResult.state` is one of `available`, `warming_up`, `stale`, `daemon_unavailable`, `timeout`, `permission_denied`, `socket_path_invalid`, `protocol_mismatch`, `malformed_response`, or `daemon_error`. Expected failures return a state and never raise. Only `available` and `stale` carry `value`; a `stale` result keeps its value so a caller can show how old it is.

The client sets no freshness threshold. With `max_age_ms=None`, `age_ms` reports the age and the state is never `stale`. With an integer, the state is `stale` when the primary timestamp is older than that many milliseconds: `aggregate.timestamp_ms` for status, `evaluated_at_ms` for suggestions.

Run its suite from the repository root with the command in [Development](#development).

### Kitty cache and refresh

`kitty/syswatch_cache.py` keeps the latest `status` and `suggestions` results in memory for Kitty drawing code. `TelemetryCache.view()` reads one immutable snapshot and never does I/O, so a tab-bar `draw_tab` can call it directly.

- Refresh intervals: `status` every 1000 ms and `suggestions` every 5000 ms by default. A shorter interval raises `ValueError`. The daemon samples at 1 s and 5 s, so actual refresh periods are about one second for `status` and about five seconds for `suggestions`, plus scan time.
- Staleness: a channel is `stale` three intervals after its data was produced (3000 ms for `status`, 15000 ms for `suggestions`). The check runs at read time, so a stopped refresh cannot present old data as live.
- Failures: a failed fetch clears `value`. The last good value stays in `last_value` with `last_age_ms`.
- States: `not_refreshed` before the first fetch, then each `ClientResult.state`. `warming_up` is normal for `suggestions` for about the first two minutes after the daemon starts.
- Driver: `install_refresh_timer(cache)` runs one chain of one-shot Kitty main-loop timers. It starts no threads. Each step starts at most one nonblocking socket request and advances it without waiting. A request has a 250 ms deadline. A failed request backs off from 1 s to 10 s. `get_shared()` returns the process-wide cache, and `stop_refresh_timer()` stops the chain.

`kitty/syswatch_redraw.py` repaints the tab bar when the drawn state changes: a new cache revision, or a channel turning `stale` while no new reply arrives. It uses four Kitty symbols that are not public API: `kitty.fast_data_types.add_timer`, `kitty.fast_data_types.wakeup_main_loop`, `kitty.boss.get_boss().os_window_map`, and `TabManager.mark_tab_bar_dirty()`. Kitty 0.49.2 provides them. On that version `mark_tab_bar_dirty()` only marks the bar; it does not wake Kitty's render loop, so an idle window would not repaint. The shim calls `wakeup_main_loop()` once per change to request that repaint. `install_redraw_timer()` checks the symbols and returns `False` if any is missing or fails. Then the bar updates only when Kitty redraws it for its own reasons, such as a tab change, focus change, or title change, and the data can be older than the polling interval. After a Kitty upgrade, check that the indicator still updates on an idle window. [Issue #80](https://github.com/curedbylethe/watch-out-kitty/issues/80) tracks a repeatable upgrade check, and [issue #78](https://github.com/curedbylethe/watch-out-kitty/issues/78) tracks dropping this workaround when Kitty offers a public repaint call.

Kitty does not render a hidden, minimized, or fully covered window. A change made while the window is not visible is drawn when the window becomes visible again.

### Kitty tab bar

`kitty/tab_bar.py` draws the tabs with Kitty's own code and shows `CPU 18%  RAM 42%` at the right edge of the tab bar. The text comes from the cached view; the drawing path does no I/O.

- Values: whole percent, rounded half-up and clamped to 0-100. RAM is occupied physical RAM as a share of total RAM.
- `CPU --  RAM --`: before the first reply, and while the daemon warms up.
- `CPU --`: while the first CPU sample is not ready.
- `RAM --`: the daemon reported no total memory.
- `~` before each value: the data is stale (older than 3 s).
- `syswatch offline`: the daemon cannot be reached or replies with an error.

A dash is not a measurement.

To install, symlink `kitty/tab_bar.py` as `tab_bar.py` in the directory of the `kitty.conf` that Kitty loads. By default that is `~/.config/kitty`, or the directory named by `KITTY_CONFIG_DIRECTORY`. Use a symlink, not a copy: the file finds the adapter modules next to its real path. Kitty loads `tab_bar.py` once, so restart Kitty after you install it.

Set these options in `kitty.conf`:

```conf
tab_bar_style custom
tab_bar_edge top
tab_bar_min_tabs 1
```

- `tab_bar_style custom` (default `fade`): required. Kitty then calls `draw_tab` in `tab_bar.py`. Tabs keep Kitty's fade look and your `tab_title_template`. Another style (`powerline`, `slant`, `separator`, `hidden`) is replaced by the fade look.
- `tab_bar_edge top` (default `bottom`): optional. The indicator works on `top` and `bottom`. `top` moves the bar, and the windows start one row lower. On `left` or `right`, tabs draw normally and the indicator is not shown.
- `tab_bar_min_tabs 1` (default `2`): shows the bar, and so the indicator, with a single tab. With the default, the bar and the indicator stay hidden until a second tab opens.

Limits:

- The last tab's title is shortened with Kitty's `…` to leave room for the text plus one free column. The indicator is hidden, never truncated, only when there is still no room, for example with many tabs.
- When tabs overflow the bar, the indicator is hidden.
- `tab_bar_align` must stay `start` (the default); `center` and `end` shift the finished line and can clip the text.
- The indicator is not clickable.
- It uses the theme's inactive tab foreground on the inactive tab background, with no icon font.

Kitty loads one `tab_bar.py` per config directory. If you already have one, do not replace it. Import this file's `draw_tab` from yours and call it, or keep yours and call this one for the last tab. Nothing is merged automatically. Further installation steps, troubleshooting, and removal are planned in [issue #70](https://github.com/curedbylethe/watch-out-kitty/issues/70).

## Project files

| Path | Contents |
| --- | --- |
| `crates/syswatch-core` | Telemetry library |
| `crates/syswatch-core/src/collector.rs` | System sampling |
| `crates/syswatch-core/src/provenance.rs` | Internal process provenance metadata |
| `crates/syswatch-core/src/config.rs` | Validated monitoring intervals |
| `crates/syswatch-core/src/detector.rs` | Advisory idle memory detector |
| `crates/syswatch-core/src/findings.rs` | Advisory findings, evidence, and cached suggestions payload |
| `crates/syswatch-core/src/eligibility.rs` | Protected-process and ancestry eligibility |
| `crates/syswatch-core/src/history.rs` | Bounded process observation history |
| `crates/syswatch-core/src/ipc.rs` | Daemon socket path resolution |
| `crates/syswatch-core/src/lib.rs` | Public core modules and exports |
| `crates/syswatch-core/src/snapshot.rs` | CPU, memory, and process data types |
| `crates/syswatch-core/src/report.rs` | Human-readable formatting |
| `crates/syswatch-core/src/preflight.rs` | Dry-run process-action assessment and preview |
| `crates/syswatch-core/src/action.rs` | Typed action service; requests one SIGTERM after a fresh allowed preflight |
| `crates/syswatch-core/src/alerts.rs` | Alert eligibility policy with a fixed cooldown |
| `crates/syswatch-core/src/protocol.rs` | Versioned protocol: monitoring requests are read-only; close requests are the only action requests |
| `crates/syswatch-core/examples/snapshot.rs` | One-time snapshot example |
| `crates/syswatch-daemon` | Foreground monitoring daemon |
| `crates/syswatch-daemon/src/action.rs` | Daemon close handler: preview and guarded confirmation from the latest scan |
| `crates/syswatch-daemon/src/clock.rs` | Interruptible monotonic clock |
| `crates/syswatch-daemon/src/ipc.rs` | Unix socket server |
| `crates/syswatch-daemon/src/scheduler.rs` | Independent sampling schedule |
| `crates/syswatch-daemon/src/dismissal.rs` | Bounded in-memory registry of dismissed advisory findings |
| `crates/syswatch-daemon/src/latest.rs` | Cached latest snapshot and advisory suggestions |
| `crates/syswatch-daemon/src/lib.rs` | Daemon lifecycle and server startup |
| `crates/syswatch-daemon/src/main.rs` | Foreground daemon entry point |
| `crates/syswatch-daemon/src/sampler.rs` | Collector, process history, and advisory evaluation |
| `crates/syswatch-daemon/src/signals.rs` | SIGINT and SIGTERM handling |
| `crates/syswatch-daemon/tests` | End-to-end daemon and socket tests |
| `crates/syswatch-cli` | Status, process-ranking, advisory-inspection, and confirmed-close command-line client |
| `crates/syswatch-cli/src/args.rs` | Command-line argument parsing |
| `crates/syswatch-cli/src/client.rs` | Daemon IPC client |
| `crates/syswatch-cli/src/main.rs` | Command dispatch and process entry point |
| `crates/syswatch-cli/src/output.rs` | Status, process, and advisory report rendering |
| `kitty/syswatch_client.py` | Read-only Python client for cached status and suggestions |
| `kitty/syswatch_cache.py` | In-memory cache of status and suggestions, request scheduler, shared instance |
| `kitty/syswatch_transport.py` | Nonblocking one-request socket state machine with a whole-request deadline |
| `kitty/syswatch_refresh.py` | Main-loop timer chain that drives the cache; no threads |
| `kitty/syswatch_redraw.py` | Tab-bar repaint timer on non-public Kitty symbols, with a fallback |
| `kitty/tab_bar.py` | Installable Kitty tab bar: Kitty's tab drawing plus the CPU/RAM indicator |
| `kitty/syswatch_indicator.py` | Indicator text from the cached view; pure, no I/O |
| `kitty/test_*.py` | Python client tests (`unittest`) |
| `.github/workflows/ci.yml` | Ubuntu build checks |
| `LICENSE` | Apache-2.0 license |

## Reading the measurements

Per-process CPU can exceed 100% when a process uses multiple cores. The first
CPU sample is not representative. Memory reports occupied physical RAM, not
macOS memory pressure. All values are point-in-time samples. Process names may
contain replacement characters and are shortened to 24 characters. The daemon
keeps its last successful sample in memory between refreshes. The daemon exposes
cached samples through the per-user socket described above. Process data can be
older than aggregate data; the `*_updated_at_ms` fields show sample times.
`syswatch top` reads the cached process list and does not scan on demand. The
age line under the table (for example, `process data: 4.2 s old`) shows its age;
before the first scan it prints `process data: unavailable`. Per-process CPU
can exceed 100%; nonfinite CPU values render as `n/a`. Rows sort by the selected
key, then by PID and start time.

The collector retains best-effort process provenance keyed by process identity:
executable path, capped file name of `argv[0]`, numeric owner UID, and parent
identity. Only the capped `argv[0]` file name is copied into provenance; full
arguments are not exposed or serialized through snapshots or IPC. Environment
variables are not collected. Provenance is not part of `ProcessSnapshot` or the
`processes` response. The daemon keeps each scan's provenance with that scan's
processes, and preflight reads both from one scan. Provenance never crosses the
socket.

### Advisory findings

An advisory finding describes a process identity, finding kind, confidence,
observation time, explanation, and evidence aggregated from retained process
history samples. Evidence covers sampled CPU and memory activity only.

Confidence describes evidence coverage for the finding's kind. Low means fewer
than two retained observations support the finding, or no valid measurement
exists. Medium means at least two observations support the finding, but one or
more retained observations lack a valid measurement. High means every retained
observation has a valid measurement and at least two observations are retained.
Confidence does not say whether a process is stale, safe, or safe to terminate.
A finding invites inspection.

A finding reference is `<pid>-<start_time_secs>-<kind>`. It names the same `(pid, start_time_secs)`
identity the rest of the tool uses, so a reference cannot select a process whose PID was reused.
`syswatch inspect` matches all three fields. A reference absent from the current reply is reported
as not found and may have expired.

The detector considers eligible processes holding at least 256 MiB, with CPU
usage at or below 1.0% across at least six valid samples spanning at least
120 seconds. The latest supporting observation must be no more than 30 seconds
old. At the default 5-second interval, the 60-observation cap permits about
295 seconds of retained evidence. The detector reports the observed span.
Warm-up and invalid CPU samples do not count; genuine zero readings can still
be cold, so this evidence does not prove idleness. A process whose start time
is unknown (zero) is not assessed as a candidate, because `(pid, start_time_secs)`
is the identity and a matching PID or executable name does not establish it.
This exclusion affects advice only. The `processes` reply and `syswatch top` still
include the process; the reply keeps its reported `start_time_secs`. Explanations
omit process names; any renderer adding them must sanitize them as `report.rs`
does. Findings
do not claim a process is safe or safe to terminate. The detector does not
detect orphaned, stale, or abandoned processes and performs no process control.
The daemon evaluates findings on process-sampling ticks and serves the cached
result over IPC without recomputing it per request. `syswatch suggestions` prints
the cached findings; Kitty rendering does not show them yet. A finding describes
observed evidence and never authorizes a process action.

### Advisory eligibility

Eligibility assessment decides whether a later advisory rule may consider a
process at all. It has exactly three states: `Protected`, `Eligible`, and
`Unknown`. Unknown terminal association means evidence is missing, not that a
process is detached. A zero start time is always `Unknown`, regardless of owner
or ancestry evidence, because its identity cannot be verified.
A process's own program label counts as terminal evidence, reported separately
from ancestor evidence; missing ancestry is not evidence of detachment.

### Process-action preflight

A preflight assesses one requested process action. It takes the full finding
identity, a fresh process scan with provenance, the monitor's identity, the
current time, and the caller's maximum observation age. It returns one of three outcomes: allowed for
confirmation, denied, or needs fresh evidence. A zero start time, a missing or
reused identity, and every protected, monitor, terminal, ownership, or
incomplete-evidence result denies.

Passing preflight is not permission. It means only that a later confirmation
step may be offered. The preflight sends no signal. A separate action service
in `crates/syswatch-core/src/action.rs` may, on explicit instruction, request
one SIGTERM after it obtains a fresh allowed preflight result. Immediately
before the request, it revalidates the identity, existence, and eligibility of
the process, and that the process belongs to the caller's UID. The caller
supplies its real UID. A requested signal does not mean the process exited. The kernel
applies the signal to whichever process holds the PID when the signal is sent;
macOS has no `pidfd`, so identity-safe signalling is not atomic. Prohibited:
SIGKILL, escalation, retry, force, sudo, bulk actions, and advisor-triggered
actions. The preflight performs no lookup of its own. The close requests carry its
typed result over the socket, and protocol version `1` is unchanged.

The daemon runs preflight itself, on its latest cached scan, which supplies the
processes, provenance, and monitor identity together. A separate read-only
provenance reply was rejected. Two requests can read two different scans, and
matching them would need a scan token and a retry loop. Provenance also stays
off the socket.

The preview prints the same finding reference that `suggestions` and `inspect`
use, described above, and states that nothing was executed.

### Requesting a graceful stop

`syswatch close <pid>-<start_time_secs>-<kind>` is the only command that can lead to
a signal. The reference is the complete finding identity, as in `inspect`. A PID
alone, a list number, or an unspecified target is a usage error (exit 2). The
command has no `--yes`, `--force`, `--all`, or `--kill` option.

Flow:

1. Without an interactive terminal on stdin, the command exits 1 before it
   connects to the daemon.
2. The daemon runs the preflight on its latest cached scan and returns a preview:
   the reference, process name, owner UID when known, verdict, reasons,
   observation age, and the line `nothing was executed; no signal was sent`.
   Evidence older than two process intervals (10 seconds at the default
   interval) is stale.
3. A denied or stale preview ends with `aborted: no action was requested` and
   exit 1. No prompt appears.
4. Otherwise the command prints `Type yes to request SIGTERM for <reference>:`.
   Only the exact line `yes` continues. Empty input, end of input, `y`, `YES`,
   and padded input abort with exit 1 and send nothing more.
5. The daemon revalidates the identity, the scan age, the monitor and ancestor
   protection, and the owner UID at delivery. A reused PID, a changed scan, or
   protected context denies without a signal. The daemon requests at most one
   SIGTERM.

A finding that the daemon no longer caches is reported as `not_found`.

Exit codes:

| Exit | Meaning |
| :--- | :--- |
| 0 | The daemon accepted a SIGTERM request. This is not confirmed exit. |
| 1 | Aborted, denied, failed, already exited, not found, unavailable, or a connection or protocol error. |
| 2 | Usage error. |

Output for an accepted request states `signal requested: SIGTERM`, names the process,
and says that the daemon accepted the request but did not confirm an exit. Run
`syswatch inspect <reference>` to check again.

Failure modes that exit 1: cancellation at the prompt, end of input, non-terminal
stdin, stale evidence, a finding that is no longer cached, PID reuse, protected
or monitor context, a different owner UID, a permission refusal from the operating
system, a daemon that is down, and an older daemon that answers
`unsupported_command`.

Safety limits. The command sends at most one SIGTERM per run. It never sends
SIGKILL, retries, escalates, uses sudo, or acts on several processes. Background
sampling, advisory findings, and any Kitty rendering stay read-only. A requested
signal is not an exit: the kernel applies it to whichever process holds the PID at
delivery, and macOS has no `pidfd` to close that gap. A dismissed finding can still
be closed by its full reference; the preview shows its current evidence.

## Protocol

The socket server and command-line client exchange one UTF-8 JSON
value per line. Each request and reply uses one line with no embedded newline.
Clients send one request per line and read each reply through its line ending.
Requests over 4096 bytes are rejected. A transport reader must cap buffered
input at `MAX_REQUEST_BYTES + 1` bytes while waiting for LF; if no LF appears
within that bound, it must return `malformed_request`. Lines end with LF only;
CR is rejected.

Protocol version is `1`. Clients send the version they speak. The server reports
`unsupported_version` with both version numbers when they differ. Changes to
required fields, field meanings, or command behavior require a version bump.
The `suggestions` operation is additive, so the version remains `1`: existing
fields and command behavior are unchanged, older servers reply
`unsupported_command`, and older clients never send the request.

`syswatch inspect` reads the same cached reply and adds no request type; a finding reference is not a
wire field and adds nothing to the reply. `syswatch close` adds two requests, `close_preview`
and `close_confirm`. Each carries one complete `finding` object and nothing else. Monitoring
requests reject a `finding` field. The close requests are additive, so the version remains
`1`; an older daemon answers `unsupported_command`.

The three monitoring requests, the two close requests, and example replies:

```text
{"version":1,"command":"status"}
{"version":1,"command":"processes"}
{"version":1,"command":"suggestions"}
{"version":1,"command":"close_preview","finding":{"pid":42,"start_time_secs":1700000000,"kind":"memory"}}
{"version":1,"command":"close_confirm","finding":{"pid":42,"start_time_secs":1700000000,"kind":"memory"}}
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100}
{"kind":"processes","version":1,"observed_at_ms":1700000000123,"processes_updated_at_ms":1700000000100,"processes":[{"pid":42,"parent_pid":1,"name":"worker","cpu_usage_percent":12.5,"memory_bytes":4096,"start_time_secs":1700000000}]}
{"kind":"suggestions","version":1,"evaluated_at_ms":7,"processes_updated_at_ms":6,"findings":[]}
{"kind":"close_preview","version":1,"assessment":{...},"process":{...}}
{"kind":"close_result","version":1,"outcome":"sigterm_requested"}
{"kind":"error","version":1,"code":"unsupported_version","message":"unsupported protocol version 2; this server speaks 1"}
{"kind":"error","version":1,"code":"unsupported_command","message":"unsupported command \"kill\""}
{"kind":"error","version":1,"code":"malformed_request","message":"request is not a JSON object with integer version and string command"}
{"kind":"error","version":1,"code":"unavailable","message":"process data has not been collected yet"}
{"kind":"error","version":1,"code":"unavailable","message":"suggestions are not available until enough process history has been collected"}
```

Suggestions contain at most 64 findings, sorted by finding identity and then
observation time. The existing per-user socket permissions apply. A suggestions
reply contains a list, possibly empty when nothing qualified, or an unavailable
error before enough history is collected; `findings` is never `null`.

`cpu_updated_at_ms`, `process_count`, and `processes_updated_at_ms` can be
`null` during warm-up. For example:

```text
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":0.0,"cpu_updated_at_ms":null,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":null,"processes_updated_at_ms":null}
```

| Error code | Meaning |
| :--- | :--- |
| `unsupported_version` | Requested protocol version is not supported. |
| `unsupported_command` | Request names an unsupported command. |
| `malformed_request` | Request line is malformed or too large. |
| `unavailable` | Requested data has not been collected yet. |
| `not_found` | The finding identity is not among the current advisory findings. |

The transport is local: the owner-only socket (`0600`, inside a `0700` directory) is
the only access control, with no authentication layer and no remote access. The
monitoring requests are read-only and take no arguments. The close requests are
the only process-control operations, and they carry only one finding identity.
The protocol has no filesystem paths, argv, environment data, history request,
retained-observation payload, or bulk command.
A suggestions reply carries finding identity, confidence, timestamps,
explanation, and aggregate numeric evidence; it contains no process names,
paths, argv, or environment data. Types live in `syswatch_core::protocol`.

`syswatch status` shows `warming up` before the first CPU sample and `not yet
sampled` before the first process scan. Displayed ages use the sample times.

## License

Apache-2.0. See [LICENSE](LICENSE).

Copyright 2026 Parsa
