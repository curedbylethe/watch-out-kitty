import ast
import json
import os
import unittest
from unittest.mock import patch

import syswatch_client as client
from syswatch_client import TransportError, read_status, read_suggestions

SOCKET = "/s.sock"
NOW = 2_000


def status_body(**overrides):
    body = {
        "kind": "status",
        "version": 1,
        "aggregate": {
            "timestamp_ms": 1_000,
            "cpu_usage_percent": 12.5,
            "cpu_updated_at_ms": 900,
            "memory": {"total_bytes": 8, "used_bytes": 4},
        },
        "process_count": 3,
        "processes_updated_at_ms": 950,
    }
    body.update(overrides)
    return body


def suggestions_body(findings=(), **overrides):
    body = {
        "kind": "suggestions",
        "version": 1,
        "evaluated_at_ms": 1_000,
        "processes_updated_at_ms": 950,
        "findings": list(findings),
    }
    body.update(overrides)
    return body


def finding(**overrides):
    item = {
        "id": {"pid": 42, "start_time_secs": 7, "kind": "memory"},
        "confidence": "high",
        "observed_at_ms": 990,
        "explanation": "pid 42 held memory",
        "evidence": {
            "sample_count": 6,
            "span_ms": 120_000,
            "cpu": {"max_percent": 0.5, "mean_percent": 0.25, "valid_samples": 6},
            "memory": {"latest_bytes": 10, "max_bytes": 12},
        },
    }
    item.update(overrides)
    return item


def run(reply, read=read_status, **kwargs):
    """Decode `reply` (a dict, or raw bytes) as the daemon's answer to `read`."""
    body = reply if isinstance(reply, bytes) else json.dumps(reply).encode()
    with patch("syswatch_client.request_line", return_value=body):
        return read(SOCKET, now_ms=kwargs.pop("now_ms", NOW), **kwargs)


class StatusDecodingTests(unittest.TestCase):
    def test_clean_status_decodes_every_field(self):
        result = run(status_body())
        self.assertEqual(result.state, "available")
        status = result.value
        self.assertEqual(status.timestamp_ms, 1_000)
        self.assertEqual(status.cpu_usage_percent, 12.5)
        self.assertEqual(status.cpu_updated_at_ms, 900)
        self.assertEqual(status.memory, client.Memory(total_bytes=8, used_bytes=4))
        self.assertEqual(status.process_count, 3)
        self.assertEqual(status.processes_updated_at_ms, 950)

    def test_warm_up_status_decodes_nullable_fields_as_none(self):
        body = status_body(process_count=None, processes_updated_at_ms=None)
        body["aggregate"]["cpu_updated_at_ms"] = None
        status = run(body).value
        self.assertIsNone(status.cpu_updated_at_ms)
        self.assertIsNone(status.process_count)
        self.assertIsNone(status.processes_updated_at_ms)

    def test_missing_memory_total_is_malformed(self):
        body = status_body()
        del body["aggregate"]["memory"]["total_bytes"]
        self.assertEqual(run(body).state, "malformed_response")

    def test_boolean_process_count_is_malformed(self):
        self.assertEqual(run(status_body(process_count=True)).state, "malformed_response")

    def test_nan_cpu_is_malformed(self):
        raw = json.dumps(status_body()).replace("12.5", "NaN").encode()
        self.assertEqual(run(raw).state, "malformed_response")

    def test_unavailable_error_is_warming_up_with_message(self):
        result = run({"kind": "error", "version": 1, "code": "unavailable", "message": "no snapshot yet"})
        self.assertEqual(result.state, "warming_up")
        self.assertEqual(result.message, "no snapshot yet")

    def test_unsupported_version_error_is_protocol_mismatch(self):
        result = run({"kind": "error", "version": 1, "code": "unsupported_version", "message": "v"})
        self.assertEqual(result.state, "protocol_mismatch")

    def test_unsupported_command_error_is_daemon_error_with_code(self):
        result = run({"kind": "error", "version": 1, "code": "unsupported_command", "message": "m"})
        self.assertEqual(result.state, "daemon_error")
        self.assertEqual(result.error_code, "unsupported_command")

    def test_processes_kind_in_status_reply_is_malformed(self):
        self.assertEqual(run(status_body(kind="processes")).state, "malformed_response")

    def test_version_two_is_protocol_mismatch(self):
        self.assertEqual(run(status_body(version=2)).state, "protocol_mismatch")

    def test_json_array_is_malformed(self):
        self.assertEqual(run(b"[1, 2]").state, "malformed_response")

    def test_truncated_json_is_malformed(self):
        self.assertEqual(run(b'{"kind":"status"').state, "malformed_response")

    def test_deep_nesting_and_huge_integers_are_malformed(self):
        huge = json.dumps(status_body()).replace("12.5", "1" + "0" * 400)
        for body in (b"[" * 200_000, huge.encode()):
            with self.subTest(body=body[:20]):
                self.assertEqual(run(body).state, "malformed_response")


class SuggestionsDecodingTests(unittest.TestCase):
    def test_clean_empty_suggestions_are_available(self):
        result = run(suggestions_body(), read=read_suggestions)
        self.assertEqual(result.state, "available")
        self.assertEqual(result.value.findings, ())

    def test_null_evidence_halves_stay_none_in_their_slots(self):
        cpu_only = finding(id={"pid": 1, "start_time_secs": 1, "kind": "cpu"})
        cpu_only["evidence"]["memory"] = None
        memory_only = finding()
        memory_only["evidence"]["cpu"] = None
        result = run(suggestions_body([cpu_only, memory_only]), read=read_suggestions)
        first, second = result.value.findings
        self.assertIsInstance(first.evidence.cpu, client.CpuEvidence)
        self.assertIsNone(first.evidence.memory)
        self.assertIsNone(second.evidence.cpu)
        self.assertEqual(second.evidence.memory, client.MemoryEvidence(latest_bytes=10, max_bytes=12))

    def test_unknown_finding_kind_is_kept_verbatim(self):
        item = finding(id={"pid": 1, "start_time_secs": 1, "kind": "disk"})
        result = run(suggestions_body([item]), read=read_suggestions)
        self.assertEqual(result.value.findings[0].kind, "disk")

    def test_unknown_confidence_is_malformed(self):
        result = run(suggestions_body([finding(confidence="certain")]), read=read_suggestions)
        self.assertEqual(result.state, "malformed_response")

    def test_suggestions_read_rejects_status_kind(self):
        result = run(status_body(), read=read_suggestions)
        self.assertEqual(result.state, "malformed_response")


class FreshnessTests(unittest.TestCase):
    def test_age_beyond_max_is_stale_with_value_and_age(self):
        result = run(status_body(), max_age_ms=500)  # primary 1_000, now 2_000
        self.assertEqual(result.state, "stale")
        self.assertEqual(result.age_ms, 1_000)
        self.assertEqual(result.value.timestamp_ms, 1_000)

    def test_age_within_max_is_available(self):
        result = run(status_body(), max_age_ms=1_500)
        self.assertEqual(result.state, "available")
        self.assertEqual(result.age_ms, 1_000)

    def test_no_max_age_never_marks_stale(self):
        result = run(status_body())
        self.assertEqual(result.state, "available")
        self.assertEqual(result.age_ms, 1_000)

    def test_future_timestamp_has_zero_age(self):
        result = run(status_body(), now_ms=500)
        self.assertEqual(result.age_ms, 0)

    def test_suggestions_age_uses_evaluated_at(self):
        result = run(suggestions_body(evaluated_at_ms=1_900), read=read_suggestions, max_age_ms=50)
        self.assertEqual(result.state, "stale")
        self.assertEqual(result.age_ms, 100)

    def test_negative_max_age_raises_before_connecting(self):
        with patch("syswatch_client.request_line") as request:
            with self.assertRaises(ValueError):
                read_status(SOCKET, max_age_ms=-1)
            request.assert_not_called()


class TransportMappingTests(unittest.TestCase):
    def test_transport_reasons_map_to_states(self):
        for reason in ("timeout", "permission_denied", "daemon_unavailable"):
            with self.subTest(reason=reason), patch(
                "syswatch_client.request_line", side_effect=TransportError(reason)
            ):
                self.assertEqual(read_status(SOCKET).state, reason)

    def test_relative_socket_env_is_socket_path_invalid(self):
        with patch.dict(os.environ, {"SYSWATCH_SOCKET": "rel.sock"}, clear=True), patch(
            "syswatch_client.request_line"
        ) as request:
            result = read_status()
            request.assert_not_called()
        self.assertEqual(result.state, "socket_path_invalid")
        self.assertEqual(result.reason, "relative_path")


class StaticGuardTests(unittest.TestCase):
    ALLOWED = {"__future__", "dataclasses", "json", "math", "os", "socket", "time"}
    FORBIDDEN = ("subprocess", "os.system", "os.popen", "AF_INET", "urllib")

    def test_module_imports_only_allowed_stdlib_and_opens_no_tcp(self):
        with open(client.__file__, encoding="utf-8") as handle:
            source = handle.read()
        imported = set()
        for node in ast.walk(ast.parse(source)):
            if isinstance(node, ast.Import):
                imported.update(alias.name.split(".")[0] for alias in node.names)
            elif isinstance(node, ast.ImportFrom):
                imported.add(node.module.split(".")[0])
        self.assertLessEqual(imported, self.ALLOWED)
        for text in self.FORBIDDEN:
            self.assertNotIn(text, source)


if __name__ == "__main__":
    unittest.main()
