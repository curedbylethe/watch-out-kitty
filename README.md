# watch-out-kitty

Watch Out Kitty is a macOS-first system monitor for showing CPU and RAM use in
Kitty. It currently provides a Rust telemetry core; advisory process inspection
is planned.

## Status

### Implemented

- Samples aggregate CPU, physical memory, and process metadata.
- Exposes serializable snapshots and helpers for readable CPU, memory, and
  process reports.
- Includes a diagnostic snapshot example and CI checks for formatting, Clippy,
  and tests.
- Uses the Apache-2.0 license; see [LICENSE](LICENSE).

### Planned

- Background daemon and command-line interface.
- Kitty UI and rendering.
- Safe, advisory process recommendations.
- Contributor guidance in [issue #13](https://github.com/curedbylethe/watch-out-kitty/issues/13).

See the [issue tracker](https://github.com/curedbylethe/watch-out-kitty/issues)
for planned work. CI currently runs on Ubuntu 24.04; macOS is the primary target.

## Prerequisites

- Rust stable toolchain (edition 2024).
- macOS is the primary target; CI runs on Ubuntu 24.04.

## Build and check

Run from the repository root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p syswatch-core --example snapshot
```

The snapshot example prints current CPU, memory, and top processes. It is a
diagnostic tool, not a permanent CLI.

## Repository layout

| Path | Purpose |
| --- | --- |
| `crates/syswatch-core` | Telemetry library crate |
| `crates/syswatch-core/src/snapshot.rs` | Serializable system, memory, and process snapshots |
| `crates/syswatch-core/src/collector.rs` | Collects system telemetry |
| `crates/syswatch-core/src/report.rs` | Formats CPU, memory, and process reports |
| `crates/syswatch-core/examples/snapshot.rs` | Diagnostic snapshot example |
| `.github/workflows/ci.yml` | Ubuntu formatting, lint, and test checks |
| `LICENSE` | Apache-2.0 license text |

## Telemetry notes

Global CPU usage is an aggregate from 0% to 100%. Per-process CPU can exceed
100% because one fully used core counts as 100%. The first CPU sample is not
representative. Used RAM reports occupied physical memory, not macOS memory
pressure. Values are single samples, not averages. Process names may contain
replacement characters and are truncated to 24 characters. Expect differences
from Activity Monitor; do not chase exact equality.

Copyright 2026 Parsa
