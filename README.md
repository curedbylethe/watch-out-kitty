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
offers no process action; a finding invites inspection.

`syswatch inspect <reference>` prints the cached evidence for one finding, selected by its full
reference `<pid>-<start_time_secs>-<kind>`. It exits with status 1 when the daemon is unavailable or
the finding is no longer cached, and with status 2 for a malformed reference. It reads cached replies
only and never scans processes.

### Daemon socket

The daemon socket path is `$SYSWATCH_SOCKET` when set to an absolute path,
otherwise `$XDG_RUNTIME_DIR/syswatch.sock`, otherwise
`$HOME/.local/run/syswatch.sock`. The daemon creates a missing final directory
with mode `0700` and refuses directories accessible by other users. The socket
has mode `0600`. The daemon does not listen on TCP. A client sends one JSON
request line per connection; the server closes the connection after its reply.

## Current scope

This repository contains the telemetry crate, snapshot example, foreground
daemon, and `syswatch status`, `syswatch top`, `syswatch suggestions`, and `syswatch inspect`
commands, and the core exposes a dry-run process-action preflight and an action service that can request one SIGTERM; no command uses either yet. The daemon evaluates advisory findings on process-sampling ticks,
caches them separately from aggregate telemetry, and serves them through the
read-only `suggestions` request. No Kitty indicator shows them yet. A background
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
```

`cargo test --workspace` also runs the end-to-end daemon and socket tests; they spawn isolated daemon processes in temporary directories and require no running daemon.

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
| `crates/syswatch-core/src/protocol.rs` | Versioned read-only monitoring protocol |
| `crates/syswatch-core/examples/snapshot.rs` | One-time snapshot example |
| `crates/syswatch-daemon` | Foreground monitoring daemon |
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
| `crates/syswatch-cli` | Status, process-ranking, and advisory-inspection command-line client |
| `crates/syswatch-cli/src/args.rs` | Command-line argument parsing |
| `crates/syswatch-cli/src/client.rs` | Daemon IPC client |
| `crates/syswatch-cli/src/main.rs` | Command dispatch and process entry point |
| `crates/syswatch-cli/src/output.rs` | Status, process, and advisory report rendering |
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
`processes` response.

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
actions. The preflight performs no lookup of its own and adds no request or
reply, so protocol version `1` is unchanged.

The preview prints the same finding reference that `suggestions` and `inspect`
use, described above, and states that nothing was executed. No command uses
preflight or the action service yet.

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
wire field and adds nothing to the reply.

The three read-only requests and example replies:

```text
{"version":1,"command":"status"}
{"version":1,"command":"processes"}
{"version":1,"command":"suggestions"}
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100}
{"kind":"processes","version":1,"observed_at_ms":1700000000123,"processes_updated_at_ms":1700000000100,"processes":[{"pid":42,"parent_pid":1,"name":"worker","cpu_usage_percent":12.5,"memory_bytes":4096,"start_time_secs":1700000000}]}
{"kind":"suggestions","version":1,"evaluated_at_ms":7,"processes_updated_at_ms":6,"findings":[]}
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

The protocol has no filesystem paths, process-control operations, authentication,
remote access, history request, retained-observation payload, or write requests.
A suggestions reply carries finding identity, confidence, timestamps,
explanation, and aggregate numeric evidence; it contains no process names,
paths, argv, or environment data. Types live in `syswatch_core::protocol`.

`syswatch status` shows `warming up` before the first CPU sample and `not yet
sampled` before the first process scan. Displayed ages use the sample times.

## License

Apache-2.0. See [LICENSE](LICENSE).

Copyright 2026 Parsa
