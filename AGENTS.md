# Agent guidance

## Workspace and crate scope

The Rust 2024 workspace contains `crates/syswatch-core` for telemetry,
`crates/syswatch-daemon` for foreground sampling, and `crates/syswatch-cli` for status
output. The core exposes system collection, snapshot data types, and text
reporting through `src/collector.rs`, `src/snapshot.rs`, and `src/report.rs`. Its
`examples/snapshot.rs` prints one diagnostic snapshot. See `README.md` for
setup and the example.

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
operating systems reuse PIDs. Process names use lossy decoding and may contain
replacement characters.

## Process lifecycle policy

The crate does not detect stale processes or terminate processes. Any future
stale-process detection must use evidence. Parent PID 1 alone does not show
that a process is abandoned. Process termination requires explicit safeguards
and confirmation.

## Change discipline

Keep diffs narrowly scoped and do not modify unrelated files. Describe
tradeoffs in pull requests. Add a dependency only for a scoped issue.
