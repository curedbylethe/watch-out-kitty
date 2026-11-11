# Watch Out Kitty

Watch Out Kitty is a macOS-first telemetry library for a CPU and RAM indicator
in Kitty. Run the included example to print a snapshot of CPU, memory, and
process usage.

## Run it

Install the Rust stable toolchain, then clone and run the example:

```sh
git clone https://github.com/curedbylethe/watch-out-kitty.git
cd watch-out-kitty
cargo run -p syswatch-core --example snapshot
```

The example prints one snapshot of aggregate CPU, physical memory, and the
processes using the most memory, then exits. CI currently runs on Ubuntu 24.04.

## Current scope

This repository contains the telemetry crate and snapshot example; it does not
yet display an indicator in Kitty. A background service, command-line
interface, and advisory process recommendations are planned. See the
[issue tracker](https://github.com/curedbylethe/watch-out-kitty/issues) for
planned work and [issue #13](https://github.com/curedbylethe/watch-out-kitty/issues/13)
for contributor guidance.

## Development

The workspace uses Rust edition 2024. From the repository root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Project files

| Path | Contents |
| --- | --- |
| `crates/syswatch-core` | Telemetry library |
| `crates/syswatch-core/src/collector.rs` | System sampling |
| `crates/syswatch-core/src/snapshot.rs` | CPU, memory, and process data types |
| `crates/syswatch-core/src/report.rs` | Human-readable formatting |
| `crates/syswatch-core/examples/snapshot.rs` | One-time snapshot example |
| `.github/workflows/ci.yml` | Ubuntu build checks |
| `LICENSE` | Apache-2.0 license |

## Reading the measurements

Per-process CPU can exceed 100% when a process uses multiple cores. The first
CPU sample is not representative. Memory reports occupied physical RAM, not
macOS memory pressure. All values are point-in-time samples. Process names may
contain replacement characters and are shortened to 24 characters.

## License

Apache-2.0. See [LICENSE](LICENSE).

Copyright 2026 Parsa
