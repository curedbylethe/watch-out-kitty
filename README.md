# watch-out-kitty

System telemetry core for a kitty CPU and RAM indicator.

## Development

Run the workspace checks:

```sh
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Snapshot example

Run the diagnostic example:

```sh
cargo run -p syswatch-core --example snapshot
```

Example output:

```text
CPU: 12.3%
Memory: 8.0 GiB / 16.0 GiB (50.0%)
Processes: 42

PID     NAME                         MEMORY      CPU
1234    example                      128.0 MiB    2.0%
```

This example is a diagnostic tool, not a permanent CLI.

## Comparing with Activity Monitor

Global CPU usage is an aggregate from 0% to 100%. Per-process CPU can exceed
100% because one fully used core counts as 100%. Used RAM reports occupied
physical memory, not macOS memory pressure. Values are single samples, not
averages. Process names may contain replacement characters and are truncated
to 24 characters. Expect differences; do not chase exact equality.
