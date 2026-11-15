# Architecture

Part of [Watch Out Kitty](../README.md).

The Rust 2024 workspace contains `crates/syswatch-core` for telemetry,
`crates/syswatch-daemon` for foreground sampling, and `crates/syswatch-cli` for status,
process-ranking, and advisory findings output. The Rust core is independent of Kitty
rendering, daemon runtime, and CLI frameworks. Run
`cargo run -p syswatch-core --example snapshot` for one diagnostic snapshot.

## Update check

The daemon reads the build version from one workspace value and reports it in
`syswatch --version` and in its startup line. It checks once at startup and then
once a day whether GitHub publishes a newer release. The check runs on its own
thread and never delays sampling or a reply. It makes one bounded request with a
3-second deadline, sends no machine identifier, and stays silent on every
failure: offline, DNS failure, and rate limiting print nothing and are not
retried before the next daily check. Set `SYSWATCH_DISABLE_UPDATE_CHECK=1` to
stop the check entirely.

## Module map

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
| `crates/syswatch-core/src/update.rs` | Release tag comparison and the one-key dismissal state file |
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
| `crates/syswatch-daemon/src/update_check.rs` | Detached daily release check and its one-line prompt |
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
| `kitty/tab_bar.py` | Installable Kitty tab bar: Kitty's tab drawing plus the CPU/RAM indicator and advisory marker |
| `kitty/syswatch_indicator.py` | Indicator and advisory-marker text from the cached view; pure, no I/O |
| `kitty/test_*.py` | Python client tests (`unittest`) |
| `.github/workflows/ci.yml` | Ubuntu checks: Rust formatting, Clippy, build and tests, and the Python suite |
| `.github/workflows/release.yml` | Creates a GitHub release with generated notes from a `v*` tag |
| `LICENSE` | Apache-2.0 license |
