# Agent guidance

## Workspace and crate scope

The Rust 2024 workspace contains `crates/syswatch-core` for telemetry,
`crates/syswatch-daemon` for foreground sampling, and `crates/syswatch-cli` for status,
process-ranking, and advisory findings output. The core exposes system collection, bounded process
history, socket path resolution, snapshot data types, text reporting, and
advisory findings and eligibility through `src/collector.rs`, `src/detector.rs`, `src/findings.rs`,
`src/eligibility.rs`, `src/history.rs`, `src/ipc.rs`, `src/preflight.rs`, `src/snapshot.rs`, and
`src/report.rs`.
The CLI modules are `src/args.rs`, `src/client.rs`, `src/output.rs`, and `src/main.rs`.
The core's `examples/snapshot.rs` prints one diagnostic snapshot. See `README.md`
for setup and the example.

## Architecture boundary

Keep the Rust core independent of Kitty rendering, daemon runtime, and CLI
frameworks unless a scoped issue changes that architecture.

## Build and validation

Use Rust stable. Run these commands from the workspace root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Add or update relevant tests for behavior changes. CI runs formatting, Clippy,
and tests on Ubuntu 24.04; it does not run a separate build job.

## Data semantics

Memory fields and process memory use bytes. Usage fields are percentages.
`MemorySnapshot` reports occupied physical RAM, not macOS memory pressure.
The first CPU sample is a warm-up and is not representative; CPU sampling
requires an interval between refreshes. Per-process CPU can exceed 100% when a
process uses multiple cores. Process identity is `(pid, start_time_secs)`;
operating systems reuse PIDs. A finding reference is `pid-start_time_secs-kind`; `syswatch inspect`
matches the full identity, so a reference cannot select a process whose PID was reused. The command is
read-only and performs no process action. Process names use lossy decoding and may contain
replacement characters.
Process provenance is collector advisory metadata keyed by `(pid, start_time_secs)`;
`None` means denied or unavailable. Only the capped file name of `argv[0]` is
retained in provenance; full arguments are not exposed or serialized through
snapshots or IPC. Environment variables are not collected. Provenance is not
serialized. A parent relationship is trustworthy only when the parent's start
time comes from the same scan. A zero start time is an unverifiable identity:
advisory findings and eligibility exclude it, while read-only telemetry still
shows it. A matching PID or program label does not validate identity.

The daemon evaluates advisory findings during process sampling and serves the
most recent cached result through the read-only `suggestions` request. Requests
never recompute findings. The reply carries finding identity, confidence,
timestamps, explanation, and aggregate numeric evidence only; it contains no
process names, paths, argv, or environment data.

## Process lifecycle policy

The crate does not detect stale processes or terminate processes. Any future
stale-process detection must use evidence. Parent PID 1 alone does not show
that a process is abandoned. Process termination requires explicit safeguards
and confirmation. Advisory findings describe observed evidence and never
authorize a process action. The eligibility assessment decides only whether a
process may be considered by a later advisory rule. It reads no CPU, memory,
or lifetime signal, and unknown terminal association does not mean detached.
A zero start time is insufficient identity for advice; `Unknown` there is not
evidence that the process is abandoned. A preflight assessment decides only
whether a later confirmation may be offered; it sends no signal and is never
permission to act.

## Change discipline

Keep diffs narrowly scoped and do not modify unrelated files. Describe
tradeoffs in pull requests. Add a dependency only for a scoped issue.
