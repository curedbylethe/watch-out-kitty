# Socket protocol

Part of [Watch Out Kitty](../README.md).

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

A `status` reply also carries `build_version`, and `update` with `latest_version`
and `url` when the daemon found a newer release. Both are additive: version `1`
is unchanged, an older daemon omits them, and a reader that does not know them
ignores them. The fields are informational. They never carry a process action.

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
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100,"build_version":"0.1.0"}
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100,"build_version":"0.1.0","update":{"latest_version":"0.2.0","url":"https://github.com/curedbylethe/watch-out-kitty/releases/tag/v0.2.0"}}
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
{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":0.0,"cpu_updated_at_ms":null,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":null,"processes_updated_at_ms":null,"build_version":"0.1.0"}
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
daemon creates a missing final directory with mode `0700` and refuses a directory
other users can access. A client sends one JSON request line per connection; the
server closes the connection after its reply. The
monitoring requests are read-only and take no arguments. The close requests are
the only process-control operations, and they carry only one finding identity.
The protocol has no filesystem paths, argv, environment data, history request,
retained-observation payload, or bulk command.
A suggestions reply carries finding identity, confidence, timestamps,
explanation, and aggregate numeric evidence; it contains no process names,
paths, argv, or environment data. Types live in `syswatch_core::protocol`.

`syswatch status` shows `warming up` before the first CPU sample and `not yet
sampled` before the first process scan. Displayed ages use the sample times.
