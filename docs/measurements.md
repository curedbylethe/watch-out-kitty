# Measurement semantics

Part of [Watch Out Kitty](../README.md).

## Reading the measurements

Per-process CPU can exceed 100% when a process uses multiple cores. The first
CPU sample is not representative. Memory reports occupied physical RAM, not
macOS memory pressure. All values are point-in-time samples. Process names may
contain replacement characters and are shortened to 24 characters; finding
explanations are shortened to 160 characters. The daemon
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
the cached findings. The Kitty tab bar shows only their count; see [Advisory badge](../README.md#advisory-badge). A finding describes
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
