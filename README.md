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
```

The example prints one snapshot of aggregate CPU, physical memory, and the
processes using the most memory, then exits. The daemon runs in the foreground.
While it runs, `syswatch status` prints the latest CPU, memory, process count,
and sample ages. It exits with status 1 when the daemon is unavailable or its
connection times out, protocol version differs, or response is invalid. It
exits with status 2 for invalid command-line arguments.
Ctrl-C (SIGINT) or SIGTERM stops it cleanly, prints `syswatch-daemon: stopped`,
and exits with status 0. CI currently runs on Ubuntu 24.04.

### Daemon socket

The daemon socket path is `$SYSWATCH_SOCKET` when set to an absolute path,
otherwise `$XDG_RUNTIME_DIR/syswatch.sock`, otherwise
`$HOME/.local/run/syswatch.sock`. The daemon creates a missing final directory
with mode `0700` and refuses directories accessible by other users. The socket
has mode `0600`. The daemon does not listen on TCP. A client sends one JSON
request line per connection; the server closes the connection after its reply.

## Current scope

This repository contains the telemetry crate, snapshot example, foreground
daemon, and `syswatch status` command; it does not yet display an indicator in
Kitty. A background service and advisory process recommendations are planned. See the
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
| `crates/syswatch-core/src/config.rs` | Validated monitoring intervals |
| `crates/syswatch-core/src/snapshot.rs` | CPU, memory, and process data types |
| `crates/syswatch-core/src/report.rs` | Human-readable formatting |
| `crates/syswatch-core/src/protocol.rs` | Versioned read-only monitoring protocol |
| `crates/syswatch-core/examples/snapshot.rs` | One-time snapshot example |
| `crates/syswatch-daemon` | Foreground monitoring daemon |
| `crates/syswatch-daemon/src/clock.rs` | Interruptible monotonic clock |
| `crates/syswatch-daemon/src/scheduler.rs` | Independent sampling schedule |
| `crates/syswatch-daemon/src/latest.rs` | Cached latest snapshot |
| `crates/syswatch-daemon/src/sampler.rs` | Collector and process-history owner |
| `crates/syswatch-daemon/src/signals.rs` | SIGINT and SIGTERM handling |
| `crates/syswatch-cli` | Status command-line client |
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

## Protocol

The socket server and planned command-line client exchange one UTF-8 JSON
value per line. Each request and reply uses one line with no embedded newline.
Clients send one request per line and read each reply through its line ending.
Requests over 4096 bytes are rejected. A transport reader must cap buffered
input at `MAX_REQUEST_BYTES + 1` bytes while waiting for LF; if no LF appears
within that bound, it must return `malformed_request`. Lines end with LF only;
CR is rejected.

Protocol version is `1`. Clients send the version they speak. The server reports
`unsupported_version` with both version numbers when they differ. Changes to
required fields, field meanings, or command behavior require a version bump.

The two read-only requests and example replies:

```text
{"version":1,"command":"status"}
{"version":1,"command":"processes"}
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100}
{"kind":"processes","version":1,"observed_at_ms":1700000000123,"processes_updated_at_ms":1700000000100,"processes":[{"pid":42,"parent_pid":1,"name":"worker","cpu_usage_percent":12.5,"memory_bytes":4096,"start_time_secs":1700000000}]}
{"kind":"error","version":1,"code":"unsupported_version","message":"unsupported protocol version 2; this server speaks 1"}
{"kind":"error","version":1,"code":"unsupported_command","message":"unsupported command \"kill\""}
{"kind":"error","version":1,"code":"malformed_request","message":"request is not a JSON object with integer version and string command"}
{"kind":"error","version":1,"code":"unavailable","message":"process data has not been collected yet"}
```

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
| `unavailable` | Process data has not been collected yet. |

The protocol has no filesystem paths, process-control operations, authentication,
remote access, history, or write requests. Types live in
`syswatch_core::protocol`.

`syswatch status` shows `warming up` before the first CPU sample and `not yet
sampled` before the first process scan. Displayed ages use the sample times.

## License

Apache-2.0. See [LICENSE](LICENSE).

Copyright 2026 Parsa
